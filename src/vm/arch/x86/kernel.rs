use std::io;
use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};

use crate::system;
use crate::util::ByteBuffer;
use crate::vm::arch::PCI_MMIO_RESERVED_BASE;
use crate::vm::arch::x86::memory::HIMEM_BASE;
use crate::vm::KERNEL;

pub const KVM_KERNEL_LOAD_ADDRESS: u64 = 0x1000000;
pub const KERNEL_CMDLINE_ADDRESS: u64 = 0x20000;
pub const KERNEL_ZERO_PAGE: u64 = 0x7000;

// Documentation/x86/boot.txt

const HDR_BOOT_FLAG: usize           = 0x1fe;  // u16
const HDR_HEADER: usize              = 0x202;  // u32
const HDR_TYPE_LOADER: usize         = 0x210;  // u8
const HDR_CMDLINE_PTR: usize         = 0x228;  // u32
const HDR_CMDLINE_SIZE: usize        = 0x238;  // u32
const HDR_KERNEL_ALIGNMENT: usize    = 0x230;  // u32

// Documentation/x86/zero-page.txt

const BOOT_PARAM_E820_ENTRIES: usize = 0x1e8;
const BOOT_PARAM_E820_MAP: usize     = 0x2d0;

const KERNEL_BOOT_FLAG_MAGIC: u16 = 0xaa55;
const EBDA_START: u64 = 0x0009fc00;
const KERNEL_HDR_MAGIC: u32 = 0x53726448;
const KERNEL_LOADER_OTHER: u8 = 0xff;
const KERNEL_MIN_ALIGNMENT_BYTES: u32 = 0x1000000;

const E820_RAM: u32 = 1;

fn setup_e820(ram_size: usize, zero: &mut ByteBuffer<Vec<u8>>) -> system::Result<()> {
    let ram_size = ram_size as u64;

    let mut e820_ranges = Vec::new();
    e820_ranges.push((0u64, EBDA_START));

    if ram_size < PCI_MMIO_RESERVED_BASE {
        e820_ranges.push((KVM_KERNEL_LOAD_ADDRESS, ram_size - KVM_KERNEL_LOAD_ADDRESS));
    } else {
        e820_ranges.push((KVM_KERNEL_LOAD_ADDRESS, PCI_MMIO_RESERVED_BASE - KVM_KERNEL_LOAD_ADDRESS));
        e820_ranges.push((HIMEM_BASE, ram_size - HIMEM_BASE));
    }
    zero.write_at(BOOT_PARAM_E820_ENTRIES , e820_ranges.len() as u8);

    zero.set_offset(BOOT_PARAM_E820_MAP);
    for i in 0..e820_ranges.len() {
        zero.write(e820_ranges[i].0)
            .write(e820_ranges[i].1)
            .write(E820_RAM);
    }
    Ok(())
}

fn setup_zero_page(ram_size: usize, memory: &GuestMemoryMmap, cmdline_addr: u64, cmdline_size: usize) -> system::Result<()> {
    let mut zero = ByteBuffer::new(4096);
    zero.write_at(HDR_BOOT_FLAG, KERNEL_BOOT_FLAG_MAGIC)
        .write_at(HDR_HEADER, KERNEL_HDR_MAGIC)
        .write_at(HDR_TYPE_LOADER, KERNEL_LOADER_OTHER)
        .write_at(HDR_CMDLINE_PTR, cmdline_addr as u32)
        .write_at(HDR_CMDLINE_SIZE, cmdline_size as u32)
        .write_at(HDR_KERNEL_ALIGNMENT, KERNEL_MIN_ALIGNMENT_BYTES);

    setup_e820(ram_size, &mut zero)?;
    memory.write_slice(zero.as_ref(), GuestAddress(KERNEL_ZERO_PAGE))?;
    Ok(())

}

pub fn load_pm_kernel(ram_size: usize, memory: &GuestMemoryMmap, cmdline_addr: u64, cmdline_size: usize) -> system::Result<()> {
    load_elf_kernel(memory)?;
    setup_zero_page(ram_size, memory,  cmdline_addr, cmdline_size)
}

fn load_elf_segment(memory: &GuestMemoryMmap, hdr: ElfPhdr) {
    let addr = hdr.p_paddr + KVM_KERNEL_LOAD_ADDRESS;
    let size = hdr.p_filesz as usize;
    let off = hdr.p_offset as usize;

    let src = &KERNEL[off..off+size];
    memory.write_slice(src, GuestAddress(addr)).unwrap();
}

pub fn load_elf_kernel(memory: &GuestMemoryMmap) -> io::Result<()> {
    let mut k = ByteBuffer::from_bytes(KERNEL);
    let phoff = k.read_at::<u64>(32);
    let phnum = k.read_at::<u16>(56);

    k.set_offset(phoff as usize);

    for _ in 0..phnum {
        let hdr = ElfPhdr::load_from(&mut k);
        if hdr.is_pt_load() {
            load_elf_segment(memory, hdr);
        }
    }
    Ok(())
}

struct ElfPhdr {
    pub p_type: u32,
    pub _p_flags: u32,
    pub p_offset: u64,
    pub _p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub _p_memsz: u64,
    pub _p_align: u64,
}

impl ElfPhdr {
    fn load_from(buf: &mut ByteBuffer<&[u8]>) -> Self {
        ElfPhdr {
            p_type: buf.read(),
            _p_flags: buf.read(),
            p_offset: buf.read(),
            _p_vaddr: buf.read(),
            p_paddr: buf.read(),
            p_filesz: buf.read(),
            _p_memsz: buf.read(),
            _p_align: buf.read(),
        }
    }

    fn is_pt_load(&self) -> bool {
        self.p_type == 1
    }
}