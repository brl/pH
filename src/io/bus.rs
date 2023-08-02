use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::result;
use std::sync::{Arc, Mutex};

use thiserror::Error;


#[derive(Debug,Error)]
pub enum Error {
    #[error("New device overlaps with an old device.")]
    Overlap,
}

pub type Result<T> = result::Result<T, Error>;

pub trait BusDevice {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        let (_,_) = (offset, data);

    }
    fn write(&mut self, offset: u64, data: &[u8]) {
        let (_,_) = (offset, data);
    }
}

#[derive(Debug,Copy,Clone)]
struct BusRange(u64, u64);

impl Eq for BusRange {}

impl PartialEq for BusRange {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Ord for BusRange {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for BusRange {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

/// A device container for routing reads and writes over some address space.
///
/// This doesn't have any restrictions on what kind of device or address space this applies to. The
/// only restriction is that no two devices can overlap in this address space.
#[derive(Clone,Default)]
pub struct Bus {
    devices: BTreeMap<BusRange, Arc<Mutex<dyn BusDevice + Send>>>,
}

impl Bus {
    /// Constructs an a bus with an empty address space.
    pub fn new() -> Bus {
        Bus {
            devices: BTreeMap::new(),
        }
    }
    fn first_before(&self, addr: u64) -> Option<(BusRange, &Arc<Mutex<dyn BusDevice+Send>>)> {
        for (range, dev) in self.devices.iter().rev() {
            if range.0 <= addr {
                return Some((*range, dev))
            }
        }
        None
    }

    /// Puts the given device at the given address space.
    pub fn get_device(&self, addr: u64) -> Option<(u64, &Arc<Mutex<dyn BusDevice+Send>>)> {
        if let Some((BusRange(start, len), dev)) = self.first_before(addr) {
            let offset = addr - start;
            if offset < len {
                return Some((offset, dev))
            }
        }
        None
    }
    /// Puts the given device at the given address space.
    pub fn insert(&mut self, device: Arc<Mutex<dyn BusDevice+Send>>, base: u64, len: u64) -> Result<()> {
        if len == 0 {
            return Err(Error::Overlap);
        }

        // Reject all cases where the new device's base is within an old device's range.
        if self.get_device(base).is_some() {
            return Err(Error::Overlap);
        }

        // The above check will miss an overlap in which the new device's base address is before the
        // range of another device. To catch that case, we search for a device with a range before
        // the new device's range's end. If there is no existing device in that range that starts
        // after the new device, then there will be no overlap.
        if let Some((BusRange(start, _), _)) = self.first_before(base + len - 1) {
            // Such a device only conflicts with the new device if it also starts after the new
            // device because of our initial `get_device` check above.
            if start >= base {
                return Err(Error::Overlap);
            }
        }

        if self.devices.insert(BusRange(base, len), device).is_some() {
            return Err(Error::Overlap);
        }

        Ok(())
    }

    /// Reads data from the device that owns the range containing `addr` and puts it into `data`.
    ///
    /// Returns true on success, otherwise `data` is untouched.
    pub fn read(&self, addr: u64, data: &mut [u8]) -> bool {
        if let Some((offset, dev)) = self.get_device(addr) {
            // OK to unwrap as lock() failing is a serious error condition and should panic.
            dev.lock()
                .expect("Failed to acquire device lock")
                .read(offset, data);
            true
        } else {
            false
        }
    }

    /// Writes `data` to the device that owns the range containing `addr`.
    ///
    /// Returns true on success, otherwise `data` is untouched.
    pub fn write(&self, addr: u64, data: &[u8]) -> bool {
        if let Some((offset, dev)) = self.get_device(addr) {
            // OK to unwrap as lock() failing is a serious error condition and should panic.
            dev.lock()
                .expect("Failed to acquire device lock")
                .write(offset, data);
            true
        } else {
            false
        }
    }
}