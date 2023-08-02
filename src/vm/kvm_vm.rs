use std::result;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use kvm_bindings::{CpuId, KVM_MAX_CPUID_ENTRIES, kvm_pit_config, KVM_PIT_SPEAKER_DUMMY, kvm_userspace_memory_region};
use kvm_ioctls::{Cap, Kvm, VmFd};
use kvm_ioctls::Cap::*;
use crate::io::manager::IoManager;
use crate::vm::vcpu::Vcpu;
use crate::vm::{Result, Error, ArchSetup};

const KVM_API_VERSION: i32 = 12;
type KvmResult<T> = result::Result<T, kvm_ioctls::Error>;

static REQUIRED_EXTENSIONS: &[Cap] = &[
    AdjustClock,
    Debugregs,
    ExtCpuid,
    Hlt,
    Ioeventfd,
    IoeventfdNoLength,
    Irqchip,
    MpState,
    Pit2,
    PitState2,
    SetTssAddr,
    UserMemory,
    VcpuEvents,
    Xcrs,
    Xsave,
];

fn check_extensions_and_version(kvm: &Kvm) -> Result<()> {
    if kvm.get_api_version() != KVM_API_VERSION {
        return Err(Error::BadVersion);
    }

    for &e in REQUIRED_EXTENSIONS {
        if !kvm.check_extension(e) {
            return Err(Error::MissingRequiredExtension(e));
        }
    }
    Ok(())
}


#[derive(Clone)]
pub struct KvmVm {
    vm_fd: Arc<VmFd>,
    supported_cpuid: Arc<CpuId>,
    //supported_msrs: MsrList,
}

impl KvmVm {
    pub fn open() -> Result<Self> {
        let kvm = Kvm::new()
            .map_err(Error::KvmOpenError)?;

        check_extensions_and_version(&kvm)?;

        let vm_fd = kvm.create_vm()
            .map_err(Error::VmFdOpenError)?;

        let supported_cpuid = kvm.get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
            .map_err(Error::KvmError)?;

        Ok(KvmVm {
            vm_fd: Arc::new(vm_fd),
            supported_cpuid : Arc::new(supported_cpuid)
        })
    }

    pub fn vm_fd(&self) -> &VmFd {
        &self.vm_fd
    }

    fn set_memory_region(&self, slot: u32, guest_phys_addr: u64, userspace_addr: u64, memory_size: u64) -> KvmResult<()> {
        let memory_region = kvm_userspace_memory_region {
            slot,
            flags: 0,
            guest_phys_addr,
            memory_size,
            userspace_addr,
        };

        unsafe {
            self.vm_fd.set_user_memory_region(memory_region)?;
        }
        Ok(())
    }

    pub fn add_memory_region(&self, slot: u32, guest_address: u64, host_address: u64, size: usize) -> KvmResult<()> {
        self.set_memory_region(slot, guest_address, host_address, size as u64)
    }

    pub fn remove_memory_region(&self, slot: u32) -> KvmResult<()> {
        self.set_memory_region(slot, 0, 0, 0)
    }

    pub fn set_irq_line(&self, irq: u32, active: bool) -> KvmResult<()> {
        self.vm_fd.set_irq_line(irq, active)
    }

    pub fn supported_cpuid(&self) -> CpuId {
        (*self.supported_cpuid).clone()
    }

    pub fn create_irqchip(&self) -> Result<()> {
        self.vm_fd.create_irq_chip()
            .map_err(Error::VmSetup)?;

        let pit_config = kvm_pit_config {
            flags: KVM_PIT_SPEAKER_DUMMY,
            ..Default::default()
        };
        self.vm_fd.create_pit2(pit_config)
            .map_err(Error::VmSetup)
    }

    pub fn create_vcpu<A: ArchSetup>(&self, id: u64, io_manager: IoManager, shutdown: Arc<AtomicBool>, arch: &mut A) -> Result<Vcpu> {
        let vcpu_fd = self.vm_fd.create_vcpu(id)
            .map_err(Error::CreateVcpu)?;
        let vcpu = Vcpu::new(vcpu_fd, io_manager, shutdown);
        arch.setup_vcpu(vcpu.vcpu_fd(), self.supported_cpuid().clone()).map_err(Error::ArchError)?;
        Ok(vcpu)
    }
}