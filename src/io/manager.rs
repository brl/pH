use std::sync::{Arc, Mutex, MutexGuard};
use vm_allocator::{AddressAllocator, AllocPolicy, IdAllocator, RangeInclusive};
use vm_memory::GuestMemoryMmap;
use vmm_sys_util::eventfd::EventFd;
use crate::devices::rtc::Rtc;
use crate::devices::serial::{SerialDevice, SerialPort};
use crate::io::bus::{Bus, BusDevice};
use crate::io::pci::{MmioHandler, PciBarAllocation, PciBus, PciDevice};
use crate::io::{PciIrq, virtio};
use crate::io::address::AddressRange;
use crate::io::shm_mapper::DeviceSharedMemoryManager;
use crate::io::virtio::{VirtioDeviceState,VirtioDevice};
use crate::vm::{arch, KvmVm};

#[derive(Clone)]
pub struct IoAllocator {
    mmio_allocator: Arc<Mutex<AddressAllocator>>,
    irq_allocator: Arc<Mutex<IdAllocator>>,
}

impl IoAllocator {
    fn new() -> Self {
        let mmio_allocator = AddressAllocator::new(arch::PCI_MMIO_RESERVED_BASE, arch::PCI_MMIO_RESERVED_SIZE as u64)
            .expect("Failed to create address allocator");
        let irq_allocator = IdAllocator::new(arch::IRQ_BASE, arch::IRQ_MAX)
            .expect("Failed to create IRQ allocator");
        IoAllocator {
            mmio_allocator: Arc::new(Mutex::new(mmio_allocator)),
            irq_allocator: Arc::new(Mutex::new(irq_allocator)),
        }
    }

    pub fn allocate_mmio(&self, size: usize) -> RangeInclusive {
        let mut allocator = self.mmio_allocator.lock().unwrap();
        allocator.allocate(size as u64, 4096, AllocPolicy::FirstMatch).unwrap()
    }

    pub fn allocate_irq(&self) -> u8 {
        let mut allocator = self.irq_allocator.lock().unwrap();
        allocator.allocate_id().unwrap() as u8
    }
}

#[derive(Clone)]
pub struct IoManager {
    kvm_vm: KvmVm,
    memory: GuestMemoryMmap,
    dev_shm_manager: DeviceSharedMemoryManager,
    pio_bus: Bus,
    mmio_bus: Bus,
    pci_bus: Arc<Mutex<PciBus>>,
    allocator: IoAllocator,
}

impl IoManager {
    pub fn new(kvm_vm: KvmVm, memory: GuestMemoryMmap) -> IoManager {
        let pci_bus = Arc::new(Mutex::new(PciBus::new()));
        let mut pio_bus = Bus::new();
        pio_bus.insert(pci_bus.clone(), PciBus::PCI_CONFIG_ADDRESS as u64, 8)
            .expect("Failed to add PCI configuration to PIO");

        let dev_shm_manager = DeviceSharedMemoryManager::new(&kvm_vm, &memory);

        IoManager {
            kvm_vm,
            memory,
            dev_shm_manager,
            pio_bus,
            mmio_bus: Bus::new(),
            pci_bus,
            allocator: IoAllocator::new(),
        }
    }

    pub fn register_legacy_devices(&mut self, reset_evt: EventFd) {
        let rtc = Arc::new(Mutex::new(Rtc::new()));
        self.pio_bus.insert(rtc, 0x0070, 2).unwrap();

        let i8042 = Arc::new(Mutex::new(I8042Device::new(reset_evt)));
        self.pio_bus.insert(i8042, 0x0060, 8).unwrap();
    }

    pub fn register_serial_port(&mut self, port: SerialPort) {
        let serial = SerialDevice::new(self.kvm_vm.clone(), port.irq());
        let serial = Arc::new(Mutex::new(serial));
        self.pio_bus.insert(serial, port.io_port() as u64, 8).unwrap();

    }

    pub fn allocator(&self) -> IoAllocator {
        self.allocator.clone()
    }

    pub fn mmio_read(&self, addr: u64, data: &mut [u8]) -> bool {
        self.mmio_bus.read(addr, data)
    }

    pub fn mmio_write(&self, addr: u64, data: &[u8]) -> bool {
        self.mmio_bus.write(addr, data)
    }

    pub fn pio_read(&self, port: u16, data: &mut [u8]) -> bool {
        self.pio_bus.read(port as u64, data)
    }

    pub fn pio_write(&self, port: u16, data: &[u8]) -> bool {
        self.pio_bus.write(port as u64, data)
    }

    fn pci_bus(&self) -> MutexGuard<PciBus> {
        self.pci_bus.lock().unwrap()
    }

    pub fn pci_irqs(&self) -> Vec<PciIrq> {
        self.pci_bus().pci_irqs()
    }

    fn allocate_pci_bars(&mut self, dev: &Arc<Mutex<dyn PciDevice+Send>>) {
        let allocations = dev.lock().unwrap().bar_allocations();
        if allocations.is_empty() {
            return;
        }

        for a in allocations {
            let mut allocated = Vec::new();
            match a {
                PciBarAllocation::Mmio(bar, size) => {
                    let range = self.allocator.allocate_mmio(size);
                    let mmio = AddressRange::new(range.start(), range.len() as usize);
                    dev.lock().unwrap().config_mut().set_mmio_bar(bar, mmio);
                    allocated.push((bar,range.start()));
                    let handler = Arc::new(Mutex::new(MmioHandler::new(bar, dev.clone())));
                    self.mmio_bus.insert(handler, range.start(), range.len()).unwrap();
                }
            }
            dev.lock().unwrap().configure_bars(allocated);
        }
    }

    pub fn add_pci_device(&mut self, device: Arc<Mutex<dyn PciDevice+Send>>) {
        self.allocate_pci_bars(&device);
        let mut pci = self.pci_bus.lock().unwrap();
        pci.add_device(device);
    }

    pub fn add_virtio_device<D: VirtioDevice+'static>(&mut self, dev: D) -> virtio::Result<()> {
        let irq = self.allocator.allocate_irq();
        let devstate = VirtioDeviceState::new(dev, self.kvm_vm.clone(), self.memory.clone(), irq)?;
        self.add_pci_device(Arc::new(Mutex::new(devstate)));
        Ok(())
    }

    pub fn dev_shm_manager(&self) -> &DeviceSharedMemoryManager {
        &self.dev_shm_manager
    }
}

pub struct I8042Device {
    reset_evt: EventFd,
}
impl I8042Device {
    fn new(reset_evt: EventFd) -> Self {
        I8042Device { reset_evt }
    }
}

impl BusDevice for I8042Device {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        if data.len() == 1 {
            match offset {
                0 => data[0] = 0x20,
                1 => data[0] = 0x00,
                _ => {},
            }
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        if data.len() == 1 {
            if offset == 3 && data[0] == 0xfe {
                if let Err(err) = self.reset_evt.write(1) {
                    warn!("Error triggering i8042 reset event: {}", err);
                }
            }
        }
    }
}