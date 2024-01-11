use kvm_bindings::CpuId;
use kvm_ioctls::VcpuFd;
use vm_memory::{Address, GuestAddress, GuestMemory, GuestMemoryMmap, GuestMemoryRegion};
use crate::io::PciIrq;
use crate::vm::VmConfig;
use crate::vm::arch::{ArchSetup, Error, PCI_MMIO_RESERVED_BASE, Result};
use crate::vm::kernel_cmdline::KernelCmdLine;
use crate::vm::arch::x86::memory::{x86_setup_memory, HIMEM_BASE};
use crate::vm::arch::x86::cpuid::setup_cpuid;
use crate::vm::arch::x86::registers::{setup_pm_sregs, setup_pm_regs, setup_fpu, setup_msrs};
use crate::vm::arch::x86::interrupts::setup_lapic;
use crate::vm::arch::x86::kernel::KVM_KERNEL_LOAD_ADDRESS;
use crate::vm::kvm_vm::KvmVm;

pub struct X86ArchSetup {
    ram_size: usize,
    ncpus: usize,
    memory: Option<GuestMemoryMmap>,
}

impl X86ArchSetup {
    pub fn create(config: &VmConfig) -> Self {
        let ram_size = config.ram_size();
        X86ArchSetup {
            ram_size,
            ncpus: config.ncpus(),
            memory: None,
        }
    }
}

fn x86_memory_ranges(mem_size: usize) -> Vec<(GuestAddress, usize)> {
    match mem_size.checked_sub(PCI_MMIO_RESERVED_BASE as usize) {
        None | Some(0) => vec![(GuestAddress(0), mem_size)],
        Some(remaining) => vec![
            (GuestAddress(0), PCI_MMIO_RESERVED_BASE as usize),
            (GuestAddress(HIMEM_BASE), remaining),
        ],
    }
}


impl ArchSetup for X86ArchSetup {
    fn create_memory(&mut self, kvm_vm: KvmVm) -> Result<GuestMemoryMmap> {
        let ranges = x86_memory_ranges(self.ram_size);
        let guest_memory = GuestMemoryMmap::from_ranges(&ranges)
            .map_err(Error::MemoryManagerCreate)?;

        for (i, r) in guest_memory.iter().enumerate() {
            let slot = i as u32;
            let guest_address = r.start_addr().raw_value();
            let size = r.len() as usize;
            let host_address = guest_memory.get_host_address(r.start_addr()).unwrap() as u64;
            kvm_vm.add_memory_region(slot, guest_address, host_address, size).map_err(Error::MemoryRegister)?;
        }
        self.memory = Some(guest_memory.clone());
        Ok(guest_memory)
    }

    fn setup_memory(&mut self, cmdline: &KernelCmdLine, pci_irqs: &[PciIrq]) -> Result<()> {
        let memory = self.memory.as_mut().expect("No memory created");
        x86_setup_memory(self.ram_size, memory, cmdline, self.ncpus, pci_irqs)?;
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


