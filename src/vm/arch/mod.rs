use kvm_bindings::CpuId;
use kvm_ioctls::VcpuFd;
pub use crate::vm::arch::x86::X86ArchSetup;
use crate::memory::MemoryManager;

mod error;
mod x86;

pub use x86::{PCI_MMIO_RESERVED_BASE,PCI_MMIO_RESERVED_SIZE,IRQ_BASE,IRQ_MAX};


pub use error::{Error,Result};
use crate::io::PciIrq;
use crate::vm::kernel_cmdline::KernelCmdLine;
use crate::vm::VmConfig;
use crate::vm::kvm_vm::KvmVm;

pub fn create_setup(config: &VmConfig) -> X86ArchSetup {
    X86ArchSetup::create(config)
}

pub trait ArchSetup {
    fn create_memory(&mut self, kvm_vm: KvmVm) -> Result<MemoryManager>;
    fn setup_memory(&mut self, cmdline: &KernelCmdLine, pci_irqs: &[PciIrq]) -> Result<()>;
    fn setup_vcpu(&self, vcpu: &VcpuFd, cpuid: CpuId) -> Result<()>;
}


