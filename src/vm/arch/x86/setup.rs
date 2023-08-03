use kvm_bindings::CpuId;
use kvm_ioctls::VcpuFd;
use crate::io::PciIrq;
use crate::memory::{MemoryManager, GuestRam, SystemAllocator, AddressRange};
use crate::vm::VmConfig;
use crate::vm::arch::{ArchSetup, Error, PCI_MMIO_RESERVED_BASE, Result};
use crate::vm::kernel_cmdline::KernelCmdLine;
use crate::vm::arch::x86::memory::{x86_setup_memory_regions, x86_setup_memory, HIMEM_BASE};
use crate::vm::arch::x86::cpuid::setup_cpuid;
use crate::vm::arch::x86::registers::{setup_pm_sregs, setup_pm_regs, setup_fpu, setup_msrs};
use crate::vm::arch::x86::interrupts::setup_lapic;
use crate::vm::arch::x86::kernel::KVM_KERNEL_LOAD_ADDRESS;
use crate::vm::kvm_vm::KvmVm;

pub struct X86ArchSetup {
    ram_size: usize,
    use_drm: bool,
    ncpus: usize,
    memory: Option<MemoryManager>,
}

impl X86ArchSetup {
    pub fn create(config: &VmConfig) -> Self {
        let ram_size = config.ram_size();
        let use_drm = config.is_wayland_enabled() && config.is_dmabuf_enabled();
        X86ArchSetup {
            ram_size,
            use_drm,
            ncpus: config.ncpus(),
            memory: None,
        }
    }
}

fn arch_memory_regions(mem_size: usize) -> Vec<(u64, usize)> {
    match mem_size.checked_sub(PCI_MMIO_RESERVED_BASE as usize) {
        None | Some(0) => vec![(0, mem_size)],
        Some(remaining) => vec![
            (0, PCI_MMIO_RESERVED_BASE as usize),
            (HIMEM_BASE, remaining),
        ],
    }
}

fn device_memory_start(regions: &[(u64, usize)]) -> u64 {
    let top = regions.last().map(|&(base, size)| {
        base + size as u64
    }).unwrap();
    // Put device memory at a 2MB boundary after physical memory or 4gb, whichever is greater.
    const MB: u64 = 1 << 20;
    const TWO_MB: u64 = 2 * MB;
    const FOUR_GB: u64 = 4 * 1024 * MB;
    let dev_base_round_2mb = (top + TWO_MB - 1) & !(TWO_MB - 1);
    std::cmp::max(dev_base_round_2mb, FOUR_GB)
}


impl ArchSetup for X86ArchSetup {
    fn create_memory(&mut self, kvm_vm: KvmVm) -> Result<MemoryManager> {
        let ram = GuestRam::new(self.ram_size);
        let regions = arch_memory_regions(self.ram_size);
        let dev_addr_start = device_memory_start(&regions);

        let dev_addr_size = u64::MAX - dev_addr_start;
        let allocator = SystemAllocator::new(AddressRange::new(dev_addr_start,dev_addr_size as usize));
        let mut mm = MemoryManager::new(kvm_vm, ram, allocator, self.use_drm)
            .map_err(Error::MemoryManagerCreate)?;
        x86_setup_memory_regions(&mut mm, self.ram_size)?;
        self.memory = Some(mm.clone());
        Ok(mm)
    }

    fn setup_memory(&mut self, cmdline: &KernelCmdLine, pci_irqs: &[PciIrq]) -> Result<()> {
        let memory = self.memory.as_mut().expect("No memory created");
        x86_setup_memory(memory, cmdline, self.ncpus, pci_irqs)?;
        Ok(())
    }

    fn setup_vcpu(&self, vcpu_fd: &VcpuFd, cpuid: CpuId) -> Result<()> {
        setup_cpuid(vcpu_fd, cpuid)?;
        setup_pm_sregs(vcpu_fd)?;
        setup_pm_regs(&vcpu_fd, KVM_KERNEL_LOAD_ADDRESS)?;
        setup_fpu(vcpu_fd)?;
        setup_msrs(vcpu_fd)?;
        setup_lapic(vcpu_fd)?;
        Ok(())
    }
}


