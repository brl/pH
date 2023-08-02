
#[derive(Copy,Clone,Eq,PartialEq,Debug)]
#[repr(u32)]
pub enum VirtioDeviceType {
    Net = 1,
    Block = 2,
    Console = 3,
    Rng = 4,
    NineP = 9,
    Wl = 63,
}

impl VirtioDeviceType {
    // Base PCI device id for Virtio devices
    const PCI_VIRTIO_DEVICE_ID_BASE: u16 = 0x1040;

    const PCI_CLASS_NETWORK_ETHERNET: u16 = 0x0200;
    const PCI_CLASS_STORAGE_SCSI: u16 = 0x0100;
    const PCI_CLASS_COMMUNICATION_OTHER: u16 = 0x0780;
    const PCI_CLASS_OTHERS: u16 = 0xff;
    const PCI_CLASS_STORAGE_OTHER: u16 = 0x0180;

    pub fn device_id(&self) -> u16 {
        Self::PCI_VIRTIO_DEVICE_ID_BASE + (*self as u16)
    }

    pub fn class_id(&self) -> u16 {
        match self {
            VirtioDeviceType::Net => Self::PCI_CLASS_NETWORK_ETHERNET,
            VirtioDeviceType::Block => Self::PCI_CLASS_STORAGE_SCSI,
            VirtioDeviceType::Console => Self::PCI_CLASS_COMMUNICATION_OTHER,
            VirtioDeviceType::Rng => Self::PCI_CLASS_OTHERS,
            VirtioDeviceType::NineP => Self::PCI_CLASS_STORAGE_OTHER,
            VirtioDeviceType::Wl => Self::PCI_CLASS_OTHERS,
        }
    }
}

pub const VIRTIO_MMIO_AREA_SIZE: usize = 4096;

// Offsets and sizes for each structure in MMIO area

pub const VIRTIO_MMIO_OFFSET_COMMON_CFG : u64 = 0;     // Common configuration offset
pub const VIRTIO_MMIO_OFFSET_ISR        : u64 = 56;    // ISR register offset
pub const VIRTIO_MMIO_OFFSET_NOTIFY     : u64 = 0x400; // Notify area offset
pub const VIRTIO_MMIO_OFFSET_DEV_CFG    : u64 = 0x800; // Device specific configuration offset

pub const VIRTIO_MMIO_COMMON_CFG_SIZE: u64 = 56;       // Common configuration size
pub const VIRTIO_MMIO_NOTIFY_SIZE    : u64 = 0x400;    // Notify area size
pub const VIRTIO_MMIO_ISR_SIZE       : u64 = 4;        // ISR register size


// Common configuration status bits

pub const _VIRTIO_CONFIG_S_ACKNOWLEDGE : u8 = 1;
pub const _VIRTIO_CONFIG_S_DRIVER      : u8 = 2;
pub const VIRTIO_CONFIG_S_DRIVER_OK   : u8 = 4;
pub const VIRTIO_CONFIG_S_FEATURES_OK : u8 = 8;
pub const VIRTIO_CONFIG_S_FAILED      : u8 = 0x80;

pub const MAX_QUEUE_SIZE: u16 = 1024;

pub const VIRTIO_NO_MSI_VECTOR: u16 = 0xFFFF;

// Bar number 0 is used for Virtio MMIO area

pub const VIRTIO_MMIO_BAR: usize = 0;

// Virtio PCI capability types

pub const VIRTIO_PCI_CAP_COMMON_CFG : u8 = 1;
pub const VIRTIO_PCI_CAP_NOTIFY_CFG : u8 = 2;
pub const VIRTIO_PCI_CAP_ISR_CFG    : u8 = 3;
pub const VIRTIO_PCI_CAP_DEVICE_CFG : u8 = 4;
