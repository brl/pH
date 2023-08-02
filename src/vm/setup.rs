use crate::vm::{VmConfig, Result, Error, PHINIT, SOMMELIER};
use crate::vm::arch::ArchSetup;
use crate::vm::kernel_cmdline::KernelCmdLine;
use termios::Termios;
use crate::devices::{SyntheticFS, VirtioBlock, VirtioNet, VirtioP9, VirtioRandom, VirtioSerial, VirtioWayland};
use std::{env, fs, thread};
use crate::system::{Tap, NetlinkSocket};
use crate::disk::DiskImage;
use std::sync::Arc;
use crate::memory::MemoryManager;
use std::sync::atomic::AtomicBool;
use kvm_ioctls::VmFd;
use vmm_sys_util::eventfd::EventFd;
use crate::devices::ac97::{Ac97Dev, Ac97Parameters};
use crate::devices::serial::SerialPort;
use crate::io::manager::IoManager;
use crate::{Logger, LogLevel};
use crate::vm::kvm_vm::KvmVm;
use crate::vm::vcpu::Vcpu;

pub struct Vm {
    kvm_vm: KvmVm,
    vcpus: Vec<Vcpu>,
    memory: MemoryManager,
    io_manager: IoManager,
    termios: Option<Termios>,
}

impl Vm {
    fn create<A: ArchSetup>(arch: &mut A) -> Result<Self> {
        let kvm_vm = KvmVm::open()?;
        kvm_vm.create_irqchip()?;
        kvm_vm.vm_fd().set_tss_address(0xfffbd000)
            .map_err(Error::KvmError)?;

        let memory = arch.create_memory(kvm_vm.clone())
            .map_err(Error::ArchError)?;

        let io_manager = IoManager::new(memory.clone());

        Ok(Vm {
            kvm_vm,
            memory,
            io_manager,
            vcpus: Vec::new(),
            termios: None,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        let mut handles = Vec::new();
        for vcpu in self.vcpus.drain(..) {
            let h = thread::spawn(move || {
                vcpu.run();
            });
            handles.push(h);
        }

        for h in handles {
            h.join().expect("...");
        }
        if let Some(termios) = self.termios {
            let _ = termios::tcsetattr(0, termios::TCSANOW, &termios)
                .map_err(Error::TerminalTermios)?;
        }
        Ok(())

    }

    pub fn vm_fd(&self) -> &VmFd {
        self.kvm_vm.vm_fd()
    }

}

pub struct VmSetup <T: ArchSetup> {
    config: VmConfig,
    cmdline: KernelCmdLine,
    arch: T,
}

impl <T: ArchSetup> VmSetup <T> {

    pub fn new(config: VmConfig, arch: T) -> Self {
        VmSetup {
            config,
            cmdline: KernelCmdLine::new_default(),
            arch,
        }
    }

    pub fn create_vm(&mut self) -> Result<Vm> {
        let exit_evt = EventFd::new(libc::EFD_NONBLOCK)?;
        let mut vm = Vm::create(&mut self.arch)?;

        let reset_evt = exit_evt.try_clone()?;
        vm.io_manager.register_legacy_devices(reset_evt);


        if self.config.verbose() {
            Logger::set_log_level(LogLevel::Info);
            self.cmdline.push("earlyprintk=serial");
            vm.io_manager.register_serial_port(SerialPort::COM1);
        } else {
            self.cmdline.push("quiet");
        }
        if self.config.rootshell() {
            self.cmdline.push("phinit.rootshell");
        }
        if vm.memory.drm_available() && self.config.is_dmabuf_enabled() {
            self.cmdline.push("phinit.virtwl_dmabuf");
        }

        if let Some(realm) = self.config.realm_name() {
            self.cmdline.push_set_val("phinit.realm", realm);
        }

        let saved= Termios::from_fd(0)
            .map_err(Error::TerminalTermios)?;
        vm.termios = Some(saved);

        self.setup_synthetic_bootfs(&mut vm.io_manager)?;
        self.setup_virtio(&mut vm.io_manager)?;

        if self.config.is_audio_enable() {

            if unsafe { libc::geteuid() } == 0 {
                self.drop_privs();
            }
            env::set_var("HOME", "/home/citadel");
            env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
            let irq = vm.io_manager.allocator().allocate_irq();
            let mem = vm.memory.guest_ram().clone();
            // XXX expect()
            let ac97 = Ac97Dev::try_new(&vm.kvm_vm, irq, mem, Ac97Parameters::new_pulseaudio()).expect("audio initialize error");
            vm.io_manager.add_pci_device(Arc::new(Mutex::new(ac97)));

        }

        if let Some(init_cmd) = self.config.get_init_cmdline() {
            self.cmdline.push_set_val("init", init_cmd);
        }

        let pci_irqs = vm.io_manager.pci_irqs();
        self.arch.setup_memory(&self.cmdline, &pci_irqs)
            .map_err(Error::ArchError)?;

        let shutdown = Arc::new(AtomicBool::new(false));
        for id in 0..self.config.ncpus() {
            let vcpu = vm.kvm_vm.create_vcpu(id as u64, vm.io_manager.clone(), shutdown.clone(), &mut self.arch)?;
            vm.vcpus.push(vcpu);
        }
        Ok(vm)
    }

    fn setup_virtio(&mut self, io_manager: &mut IoManager) -> Result<()> {
        io_manager.add_virtio_device(VirtioSerial::new())?;
        io_manager.add_virtio_device(VirtioRandom::new())?;

        if self.config.is_wayland_enabled() {
            io_manager.add_virtio_device(VirtioWayland::new(self.config.is_dmabuf_enabled()))?;
        }

        let homedir = self.config.homedir();
        io_manager.add_virtio_device(VirtioP9::new_filesystem("home", homedir, false, false))?;
        if homedir != "/home/user" && !self.config.is_realm() {
            self.cmdline.push_set_val("phinit.home", homedir);
        }

        let mut block_root = None;

        for disk in self.config.get_realmfs_images() {
            if block_root == None {
                block_root = Some(disk.read_only());
            }
            io_manager.add_virtio_device(VirtioBlock::new(disk))?;
        }

        for disk in self.config.get_raw_disk_images() {
            if block_root == None {
                block_root = Some(disk.read_only());
            }
            io_manager.add_virtio_device(VirtioBlock::new(disk))?;
        }

        if let Some(read_only) = block_root {
            if !read_only {
                self.cmdline.push("phinit.root_rw");
            }
            self.cmdline.push("phinit.root=/dev/vda");
            self.cmdline.push("phinit.rootfstype=ext4");
        } else {
            io_manager.add_virtio_device(VirtioP9::new_filesystem("9proot", "/", true, false))?;
            self.cmdline.push_set_val("phinit.root", "9proot");
            self.cmdline.push_set_val("phinit.rootfstype", "9p");
            self.cmdline.push_set_val("phinit.rootflags", "trans=virtio");
        }

        if self.config.network() {
            self.setup_network(io_manager)?;
            self.drop_privs();

        }
        Ok(())
    }

    fn drop_privs(&self) {
        unsafe {
            libc::setgid(1000);
            libc::setuid(1000);
            libc::setegid(1000);
            libc::seteuid(1000);
        }

    }

    fn setup_synthetic_bootfs(&mut self, io_manager: &mut IoManager) -> Result<()> {
        let bootfs = self.create_bootfs()
            .map_err(Error::SetupBootFs)?;

        io_manager.add_virtio_device(VirtioP9::new(bootfs, "/dev/root", "/", false))?;

        self.cmdline.push_set_val("init", "/usr/bin/ph-init");
        self.cmdline.push_set_val("root", "/dev/root");
        self.cmdline.push("ro");
        self.cmdline.push_set_val("rootfstype", "9p");
        self.cmdline.push_set_val("rootflags", "trans=virtio");
        Ok(())
    }

    fn create_bootfs(&self) -> std::io::Result<SyntheticFS> {
        let mut s = SyntheticFS::new();
        s.mkdirs(&["/tmp", "/proc", "/sys", "/dev", "/home/user", "/bin", "/etc"]);

        fs::write("/tmp/ph-init", PHINIT)?;
        s.add_library_dependencies("/tmp/ph-init")?;
        fs::remove_file("/tmp/ph-init")?;

        s.add_memory_file("/usr/bin", "ph-init", 0o755, PHINIT)?;
        s.add_memory_file("/usr/bin", "sommelier", 0o755, SOMMELIER)?;

        s.add_file("/etc", "ld.so.cache", 0o644, "/etc/ld.so.cache");
        Ok(s)
    }

    fn setup_network(&mut self, io_manager: &mut IoManager) -> Result<()> {
        let tap = match self.setup_tap() {
            Ok(tap) => tap,
            Err(e) => {
                warn!("failed to create tap device: {}", e);
                return Ok(());
            }
        };
        io_manager.add_virtio_device(VirtioNet::new(tap))?;
        self.cmdline.push("phinit.ip=172.17.0.22");
        Ok(())
    }

    fn setup_tap(&self) -> Result<Tap> {
        let bridge_name = self.config.bridge();
        let tap = Tap::new_default()?;
        let nl = NetlinkSocket::open()?;

        if !nl.interface_exists(bridge_name) {
            nl.create_bridge(bridge_name)?;
            nl.set_interface_up(bridge_name)?;
        }
        nl.add_interface_to_bridge(tap.name(), bridge_name)?;
        nl.set_interface_up(tap.name())?;
        Ok(tap)
    }
}