use kvm_bindings::{kvm_fpu, kvm_msr_entry, kvm_regs, Msrs};
use kvm_ioctls::VcpuFd;

use crate::vm::arch::{Error, Result};
use crate::vm::arch::x86::gdt::GdtEntry;
use crate::vm::arch::x86::kernel::KERNEL_ZERO_PAGE;

const MSR_IA32_SYSENTER_CS: u32  = 0x00000174;
const MSR_IA32_SYSENTER_ESP: u32 = 0x00000175;
const MSR_IA32_SYSENTER_EIP: u32 = 0x00000176;
const MSR_STAR: u32              = 0xc0000081;
const MSR_LSTAR: u32             = 0xc0000082;
const MSR_CSTAR: u32             = 0xc0000083;
const MSR_SYSCALL_MASK: u32      = 0xc0000084;
const MSR_KERNEL_GS_BASE: u32    = 0xc0000102;
const MSR_IA32_TSC: u32          = 0x00000010;
const MSR_IA32_MISC_ENABLE: u32  = 0x000001a0;

const MSR_IA32_MISC_ENABLE_FAST_STRING: u64 = 0x01;

pub fn setup_fpu(vcpu: &VcpuFd) -> Result<()> {
    let fpu = kvm_fpu {
        fcw:  0x37f,
        mxcsr:  0x1f80,
        ..Default::default()
    };
    vcpu.set_fpu(&fpu)
        .map_err(Error::SetupError)?;
    Ok(())
}

pub fn setup_msrs(vcpu: &VcpuFd) -> Result<()> {
    let msr = | index, data| kvm_msr_entry {
        index, data, ..Default::default()
    };
    let entries = vec![
        msr(MSR_IA32_SYSENTER_CS, 0),
        msr(MSR_IA32_SYSENTER_ESP, 0),
        msr(MSR_IA32_SYSENTER_EIP, 0),
        msr(MSR_STAR, 0),
        msr(MSR_CSTAR, 0),
        msr(MSR_KERNEL_GS_BASE, 0),
        msr(MSR_SYSCALL_MASK, 0),
        msr(MSR_LSTAR, 0),
        msr(MSR_IA32_TSC, 0),
        msr(MSR_IA32_MISC_ENABLE, MSR_IA32_MISC_ENABLE_FAST_STRING),
    ];

    let msrs = Msrs::from_entries(&entries)
        .expect("Failed to create msr entries");
    vcpu.set_msrs(&msrs)
        .map_err(Error::SetupError)?;
    Ok(())
}

const BOOT_GDT_OFFSET: usize = 0x500;
const BOOT_IDT_OFFSET: usize = 0x520;

const BOOT_STACK: u64 = 0x8000;
const BOOT_PML4: u64 = 0x9000;

const X86_CR0_PE: u64 = 0x1;
const X86_CR0_PG: u64 = 0x80000000;
const X86_CR4_PAE: u64 = 0x20;

const EFER_LME: u64 = 0x100;
const EFER_LMA: u64 = 1 << 10;

pub fn setup_pm_sregs(vcpu: &VcpuFd) -> Result<()> {

    let code = GdtEntry::new(0xa09b, 0, 0xFFFFF)
        .kvm_segment(1);
    let data = GdtEntry::new(0xc093, 0, 0xFFFFF)
        .kvm_segment(2);
    let tss = GdtEntry::new(0x808b, 0, 0xFFFFF)
        .kvm_segment(3);

    let mut regs = vcpu.get_sregs()
        .map_err(Error::SetupError)?;

    regs.gdt.base = BOOT_GDT_OFFSET as u64;
    regs.gdt.limit = 32 - 1;

    regs.idt.base = BOOT_IDT_OFFSET as u64;
    regs.idt.limit = 8 - 1;

    regs.cs = code;
    regs.ds = data;
    regs.es = data;
    regs.fs = data;
    regs.gs = data;
    regs.ss = data;
    regs.tr = tss;

    // protected mode
    regs.cr0 |= X86_CR0_PE;
    regs.efer |= EFER_LME;

    regs.cr3 = BOOT_PML4;
    regs.cr4 |= X86_CR4_PAE;
    regs.cr0 |= X86_CR0_PG;
    regs.efer |= EFER_LMA;

    vcpu.set_sregs(&regs)
        .map_err(Error::SetupError)?;

    Ok(())
}

pub fn setup_pm_regs(vcpu: &VcpuFd, kernel_entry: u64) -> Result<()> {
    let regs = kvm_regs {
        rflags:  0x0000000000000002,
        rip:  kernel_entry,
        rsp:  BOOT_STACK,
        rbp:  BOOT_STACK,
        rsi:  KERNEL_ZERO_PAGE,
        ..Default::default()
    };

    vcpu.set_regs(&regs)
        .map_err(Error::SetupError)?;

    Ok(())
}