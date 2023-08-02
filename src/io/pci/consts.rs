
// Maximum number of logical devices on a PCI bus

pub const PCI_MAX_DEVICES: usize = 32;

// Vendor specific PCI capabilities

pub const PCI_CAP_ID_VENDOR: u8 = 0x09;

pub const PCI_CAP_BASE_OFFSET: usize = 0x40;

pub const PCI_VENDOR_ID: usize = 0x00;
pub const PCI_DEVICE_ID: usize = 0x02;
pub const PCI_COMMAND: usize = 0x04;
pub const PCI_COMMAND_IO: u16 = 0x01;
pub const PCI_COMMAND_MEMORY: u16 = 0x02;
pub const PCI_STATUS: usize = 0x06;
pub const PCI_BAR0: usize = 0x10;
pub const PCI_BAR5: usize = 0x24;
pub const PCI_STATUS_CAP_LIST: u16 = 0x10;
pub const PCI_CLASS_REVISION: usize = 0x08;
pub const PCI_CLASS_DEVICE: usize = 0x0a;
pub const PCI_CACHE_LINE_SIZE: usize = 0x0c;

pub const _PCI_SUBSYSTEM_VENDOR_ID: usize = 0x2c;
pub const PCI_SUBSYSTEM_ID: usize = 0x2e;
pub const PCI_CAPABILITY_LIST: usize = 0x34;
pub const PCI_INTERRUPT_LINE: usize = 0x3C;
pub const PCI_INTERRUPT_PIN: usize = 0x3D;

pub const PCI_VENDOR_ID_INTEL: u16 = 0x8086;
pub const PCI_CLASS_BRIDGE_HOST: u16 = 0x0600;

