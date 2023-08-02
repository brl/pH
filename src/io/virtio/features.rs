use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Copy,Clone)]
#[repr(u64)]
pub enum ReservedFeatureBit {
    _IndirectDesc = 1 << 28,
    EventIdx = 1 << 29,
    Version1 = 1 << 32,
}

impl ReservedFeatureBit {
    pub fn is_set_in(&self, flags: u64) -> bool {
        flags & (*self as u64) != 0
    }
}

#[derive(Clone)]
pub struct FeatureBits {
    device_bits: Arc<Mutex<Inner>>,
    guest_bits: Arc<Mutex<Inner>>,
}

struct Inner {
    bits: u64,
    selected: u32,
}

impl Inner {
    fn new(bits: u64) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Inner { bits, selected: 0 }))
    }
}

impl FeatureBits {

    pub fn new_default(device_bits: u64) -> Self {
        FeatureBits {
            guest_bits: Inner::new(0),
            device_bits: Inner::new(ReservedFeatureBit::Version1 as u64 | device_bits),
        }
    }

    pub fn reset(&self) {
        let mut guest = self.guest();
        guest.bits = 0;
        guest.selected = 0;
    }

    fn guest(&self) -> MutexGuard<Inner> {
        self.guest_bits.lock().unwrap()
    }
    fn device(&self) -> MutexGuard<Inner> {
        self.device_bits.lock().unwrap()
    }

    pub fn guest_selected(&self) -> u32 {
        self.guest().selected
    }

    pub fn guest_value(&self) -> u64 {
        self.guest().bits
    }

    pub fn has_guest_bit(&self, bit: u64) -> bool {
        self.guest_value() & bit == bit
    }

    pub fn set_guest_selected(&self, val: u32) {
        self.guest().selected = val;
    }

    pub fn write_guest_word(&self, val: u32) {
        const MASK_LOW_32: u64 = (1u64 << 32) - 1;
        const MASK_HI_32: u64 = MASK_LOW_32 << 32;
        let mut inner = self.guest();
        let val = u64::from(val);

        match inner.selected {
            0 => inner.bits = (inner.bits & MASK_HI_32) | val,
            1 => inner.bits = val << 32 | (inner.bits & MASK_LOW_32),
            _ => (),
        }
    }

    pub fn read_guest_word(&self) -> u32 {
        let inner = self.guest();
        match inner.selected {
            0 => inner.bits as u32,
            1 => (inner.bits >> 32) as u32,
            _ => 0,
        }
    }

    pub fn set_device_selected(&self, val: u32) {
        self.device().selected = val;
    }

    pub fn device_selected(&self) -> u32 {
        self.device().selected
    }

    pub fn read_device_word(&self) -> u32 {
        let inner = self.device();
        match inner.selected {
            0 => inner.bits as u32,
            1 => (inner.bits >> 32) as u32,
            _ => 0,
        }
    }
}