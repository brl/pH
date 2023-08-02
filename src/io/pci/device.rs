use std::sync::{Arc, Mutex};
use crate::io::bus::BusDevice;
use crate::io::pci::PciConfiguration;

#[derive(Copy,Clone,Eq,PartialEq)]
#[repr(u8)]
pub enum PciBar {
    Bar0 = 0,
    Bar1 = 1,
    Bar2 = 2,
    Bar3 = 3,
    Bar4 = 4,
    Bar5 = 5,
}

impl PciBar {
    pub fn idx(&self) -> usize {
        *self as usize
    }
}

pub enum PciBarAllocation {
    Mmio(PciBar, usize),
}

pub trait PciDevice: Send {
    fn config(&self) -> &PciConfiguration;
    fn config_mut(&mut self) -> &mut PciConfiguration;

    fn read_bar(&mut self, bar: PciBar, offset: u64, data: &mut [u8]) {
        let (_,_,_) = (bar, offset, data);
    }

    fn write_bar(&mut self, bar: PciBar, offset: u64, data: &[u8]) {
        let (_,_,_) = (bar,offset, data);
    }

    fn irq(&self) -> Option<u8> { None }

    fn bar_allocations(&self) -> Vec<PciBarAllocation> { vec![] }

    fn configure_bars(&mut self, allocations: Vec<(PciBar, u64)>) { let _ = allocations; }
}

pub struct MmioHandler {
    bar: PciBar,
    device: Arc<Mutex<dyn PciDevice+Send>>
}

impl MmioHandler {
    pub fn new(bar: PciBar, device: Arc<Mutex<dyn PciDevice+Send>>) -> Self {
        MmioHandler {
            bar, device,
        }
    }
}

impl BusDevice for MmioHandler {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        let mut lock = self.device.lock().unwrap();
        lock.read_bar(self.bar, offset, data)
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        let mut lock = self.device.lock().unwrap();
        lock.write_bar(self.bar, offset, data)
    }
}