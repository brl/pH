use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use crate::io::bus::BusDevice;
use crate::io::pci::address::PciAddress;
use crate::io::pci::config::PciConfiguration;
use crate::io::pci::consts::{PCI_CLASS_BRIDGE_HOST, PCI_MAX_DEVICES, PCI_VENDOR_ID_INTEL};
use crate::io::pci::PciDevice;

/// Current address to read/write from (io port 0xcf8)
struct PciConfigAddress([u8; 4]);

impl PciConfigAddress {
    fn new() -> Self {
        PciConfigAddress([0u8; 4])
    }
    fn bus(&self) -> u8 {
        self.0[2]
    }

    fn function(&self) -> u8 {
        self.0[1] & 0x7
    }
    fn device(&self) -> u8 {
        self.0[1] >> 3
    }
    fn offset(&self) -> u8 {
        self.0[0] & !0x3
    }

    fn enabled(&self) -> bool {
        self.0[3] & 0x80 != 0
    }

    fn pci_address(&self) -> PciAddress {
        PciAddress::new(self.bus(), self.device(), self.function())
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        let offset = offset as usize;
        if offset + data.len() <= 4 {
            self.0[offset..offset+data.len()]
                .copy_from_slice(data)
        }
    }

    fn read(&self, offset: u64, data: &mut [u8]) {
        let offset = offset as usize;
        if offset + data.len() <= 4 {
            data.copy_from_slice(&self.0[offset..offset+data.len()])
        }
    }
}

struct PciRootDevice(PciConfiguration);

impl PciRootDevice {
    fn new() -> Self {
        let config = PciConfiguration::new(0, PCI_VENDOR_ID_INTEL, 0, PCI_CLASS_BRIDGE_HOST);
        PciRootDevice(config)
    }
}
impl PciDevice for PciRootDevice {

    fn config(&self) -> &PciConfiguration {
        &self.0
    }

    fn config_mut(&mut self) -> &mut PciConfiguration {
        &mut self.0
    }
}

pub struct PciBus {
    devices: BTreeMap<PciAddress, Arc<Mutex<dyn PciDevice>>>,

    config_address: PciConfigAddress,
    used_device_ids: Vec<bool>,

}

impl PciBus {
    pub const PCI_CONFIG_ADDRESS: u16 = 0xcf8;

    pub fn new() -> PciBus {
        let mut pci = PciBus {
            devices: BTreeMap::new(),
            config_address: PciConfigAddress::new(),
            used_device_ids: vec![false; PCI_MAX_DEVICES],
        };

        let root = PciRootDevice::new();
        pci.add_device(Arc::new(Mutex::new(root)));
        pci

    }

    pub fn add_device(&mut self, device: Arc<Mutex<dyn PciDevice>>) {
        let id = self.allocate_id().unwrap();
        let address = PciAddress::new(0, id, 0);
        device.lock().unwrap().config_mut().set_address(address);
        self.devices.insert(address, device);
    }

    pub fn pci_irqs(&self) -> Vec<PciIrq> {
        let mut irqs = Vec::new();
        for (addr, dev)  in &self.devices {
            let lock = dev.lock().unwrap();
            if let Some(irq) = lock.irq() {
                irqs.push(PciIrq::new(addr.device(), irq));
            }
        }
        irqs
    }

    fn allocate_id(&mut self) -> Option<u8> {
        for i in 0..PCI_MAX_DEVICES {
            if !self.used_device_ids[i] {
                self.used_device_ids[i] = true;
                return Some(i as u8)
            }
        }
        None
    }

    fn is_in_range(base: u64, offset: u64, len: usize) -> bool {
        let end = offset + len as u64;
        offset >= base && end <= (base + 4)
    }

    fn is_config_address(offset: u64, len: usize) -> bool {
        Self::is_in_range(0, offset, len)
    }

    fn is_config_data(offset: u64, len: usize) -> bool {
        Self::is_in_range(4, offset, len)
    }

    fn current_config_device(&self) -> Option<Arc<Mutex<dyn PciDevice>>> {
        if self.config_address.enabled() {
            let addr = self.config_address.pci_address();
            self.devices.get(&addr).cloned()
        } else {
            None
        }
    }
}

impl BusDevice for PciBus {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        if PciBus::is_config_address(offset, data.len()) {
            self.config_address.read(offset, data);
        } else if PciBus::is_config_data(offset, data.len()) {
            if let Some(dev) = self.current_config_device() {
                let lock = dev.lock().unwrap();
                let offset = (offset - 4) + self.config_address.offset() as u64;
                lock.config().read(offset, data)
            } else {
                data.fill(0xff)
            }
        }
    }
    fn write(&mut self, offset: u64, data: &[u8]) {
        if PciBus::is_config_address(offset, data.len()) {
            self.config_address.write(offset, data)
        } else if PciBus::is_config_data(offset, data.len()) {
            if let Some(dev) = self.current_config_device() {
                let mut lock = dev.lock().unwrap();
                let offset = (offset - 4) + self.config_address.offset() as u64;
                lock.config_mut().write(offset, data)
            }
        }
    }
}

#[derive(Debug)]
pub struct PciIrq {
    pci_id: u8,
    int_pin: u8,
    irq: u8,
}

impl PciIrq {
    fn new(pci_id: u8, irq: u8) -> PciIrq {
        PciIrq {
            pci_id,
            int_pin: 1,
            irq,
        }
    }

    pub fn src_bus_irq(&self) -> u8 {
        (self.pci_id << 2) | (self.int_pin - 1)
    }

    pub fn irq_line(&self) -> u8 {
        self.irq
    }
}
