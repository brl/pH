use crate::vm::arch::{Error, Result};
use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};
use crate::io::PciIrq;
use crate::vm::kernel_cmdline::KernelCmdLine;
use crate::vm::arch::x86::kernel::{load_pm_kernel, KERNEL_CMDLINE_ADDRESS};
use crate::system;
use crate::vm::arch::x86::mptable::setup_mptable;

pub const HIMEM_BASE: u64 = 1 << 32;
pub const PCI_MMIO_RESERVED_SIZE: usize = 512 << 20;
pub const PCI_MMIO_RESERVED_BASE: u64 = HIMEM_BASE - PCI_MMIO_RESERVED_SIZE as u64;
pub const IRQ_BASE: u32 = 5;
pub const IRQ_MAX: u32 = 23;

const BOOT_GDT_OFFSET: usize = 0x500;
const BOOT_IDT_OFFSET: usize = 0x520;

const BOOT_PML4: u64 = 0x9000;
const BOOT_PDPTE: u64 = 0xA000;
const BOOT_PDE: u64 = 0xB000;

pub fn x86_setup_memory(ram_size: usize, memory: &GuestMemoryMmap, cmdline: &KernelCmdLine, ncpus: usize, pci_irqs: &[PciIrq]) -> Result<()> {
    load_pm_kernel(ram_size, memory, KERNEL_CMDLINE_ADDRESS, cmdline.size())
        .map_err(Error::LoadKernel)?;
    setup_gdt(memory)?;
    setup_boot_pagetables(memory).map_err(Error::SystemError)?;
    setup_mptable(memory, ncpus, pci_irqs).map_err(Error::SystemError)?;
    write_cmdline(memory, cmdline).map_err(Error::SystemError)?;
    Ok(())
}

fn setup_boot_pagetables(memory: &GuestMemoryMmap) -> system::Result<()> {
    memory.write_obj(BOOT_PDPTE | 0x3, GuestAddress(BOOT_PML4))?;
    memory.write_obj(BOOT_PDE | 0x3, GuestAddress(BOOT_PDPTE))?;
    for i in 0..512_u64 {
        let entry = (i << 21) | 0x83;
        memory.write_obj(entry, GuestAddress(BOOT_PDE + (i * 8)))?;
    }
    Ok(())
}

fn write_gdt_table(table: &[u64], memory: &GuestMemoryMmap) -> system::Result<()> {
    for i in 0..table.len() {
        memory.write_obj(table[i], GuestAddress((BOOT_GDT_OFFSET + i * 8) as u64))?;
    }
    Ok(())
}

pub fn gdt_entry(flags: u16, base: u32, limit: u32) -> u64 {
    (((base as u64) & 0xff000000u64) << (56 - 24)) | (((flags as u64) & 0x0000f0ffu64) << 40) |
        (((limit as u64) & 0x000f0000u64) << (48 - 16)) |
        (((base as u64) & 0x00ffffffu64) << 16) | ((limit as u64) & 0x0000ffffu64)
}

pub fn setup_gdt(memory: &GuestMemoryMmap) -> Result<()> {
    let table = [
        gdt_entry(0,0,0),
        gdt_entry(0xa09b,0,0xfffff),
        gdt_entry(0xc093,0,0xfffff),
        gdt_entry(0x808b,0,0xfffff),
    ];
    write_gdt_table(&table, memory)
        .map_err(Error::SystemError)?;

    memory.write_obj(0u64, GuestAddress(BOOT_IDT_OFFSET as u64))
        .map_err(Error::GuestMemory)?;

    Ok(())
}

fn write_cmdline(memory: &GuestMemoryMmap, cmdline: &KernelCmdLine) -> system::Result<()> {
    let bytes = cmdline.as_bytes();
    let len = bytes.len() as u64;
    memory.write_slice(bytes, GuestAddress(KERNEL_CMDLINE_ADDRESS))?;
    memory.write_obj(0u8, GuestAddress(KERNEL_CMDLINE_ADDRESS + len))?;
    Ok(())
}
