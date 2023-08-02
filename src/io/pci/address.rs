
#[derive(Copy,Clone,Debug,PartialEq,Eq,PartialOrd,Ord,Hash)]
pub struct PciAddress(u16);

impl PciAddress {
    pub fn empty() -> Self {
        Self::new(0,0,0)
    }

    pub fn new(bus: u8, device: u8, function: u8) -> Self {
        const DEVICE_MASK: u16 = 0x1f;
        const FUNCTION_MASK: u16 = 0x07;

        let bus = bus as u16;
        let device = device as u16;
        let function = function as u16;

        let addr = bus << 8
            | (device & DEVICE_MASK) << 3
            | (function & FUNCTION_MASK);

        PciAddress(addr)
    }

    pub fn device(&self) -> u8 {
        ((self.0 & 0xF) >> 3) as u8
    }

    pub fn address(&self) -> u16 {
        self.0
    }
}