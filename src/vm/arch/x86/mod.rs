mod cpuid;
mod gdt;
mod interrupts;
mod memory;
mod mptable;
mod registers;
mod kernel;
mod setup;

pub use setup::X86ArchSetup;
pub use memory::PCI_MMIO_RESERVED_BASE;