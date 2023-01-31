use std::fmt;
use kvm_bindings::kvm_segment;

pub struct GdtEntry(u64);

impl GdtEntry {

    pub fn new(flags: u16, base: u32, limit: u32) -> Self {
        let flags = flags as u64;
        let base = base as u64;
        let limit = limit as u64;

        GdtEntry(
            ((base & 0xff00_0000_u64) << (56 - 24))
                | ((flags & 0x0000_f0ff_u64) << 40)
                | ((limit & 0x000f_0000_u64) << (48 - 16))
                | ((base & 0x00ff_ffff_u64) << 16)
                | (limit & 0x0000_ffff_u64))
    }

    fn get_base(&self) -> u64 {
        (((self.0) & 0xFF00_0000_0000_0000) >> 32)
            | (((self.0) & 0x0000_00ff_ffff_0000) >> 16)
    }

    fn get_limit(&self) -> u32 {
        ((((self.0) & 0x000f_0000_0000_0000) >> 32)
            | ((self.0) & 0x0000_0000_0000_ffff)) as u32
    }

    const BIT_G: usize  = 55;
    const BIT_DB: usize  = 54;
    const BIT_L: usize  = 53;
    const BIT_AVL: usize  = 52;
    const BIT_P: usize  = 47;
    const BIT_S: usize = 44;
    const BITS_DPL: usize = 45;
    const BITS_TYPE: usize = 40;

    fn get_type(&self) -> u8 {
        ((self.0 & 0x0000_0f00_0000_0000) >> GdtEntry::BITS_TYPE) as u8
    }

    fn get_dpl(&self) -> u8 {
        ((self.0 & 0x0000_6000_0000_0000) >> GdtEntry::BITS_DPL) as u8
    }

    fn get_bit(&self, bit: usize) -> u8 {
        ((self.0 & (1u64 << bit)) >> bit) as u8
    }

    pub fn kvm_segment(&self, table_index: u16) -> kvm_segment {
        kvm_segment {
            base: self.get_base(),
            limit: self.get_limit(),
            selector: table_index * 8,
            type_: self.get_type(),
            present: self.get_bit(GdtEntry::BIT_P),
            dpl: self.get_dpl(),
            db: self.get_bit(GdtEntry::BIT_DB),
            s: self.get_bit(GdtEntry::BIT_S),
            l: self.get_bit(GdtEntry::BIT_L),
            g: self.get_bit(GdtEntry::BIT_G),
            avl: self.get_bit(GdtEntry::BIT_AVL),
            padding: 0,
            unusable: if self.get_bit(GdtEntry::BIT_P) == 0 { 1 } else { 0 },
        }
    }
}

impl fmt::Debug for GdtEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(base: {:x} limit {:x} type: {:x} p: {} dpl: {})",
               self.get_base(), self.get_limit(), self.get_type(), self.get_bit(GdtEntry::BIT_P), self.get_dpl())
    }
}
