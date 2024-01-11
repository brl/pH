pub mod bus;
pub mod busdata;
pub mod pci;
pub mod manager;
pub mod virtio;
mod address;
pub mod shm_mapper;

pub use virtio::{VirtioDevice,FeatureBits,VirtioDeviceType,VirtQueue,Chain,Queues};
pub use virtio::Error as VirtioError;
pub use busdata::ReadableInt;
pub use pci::PciIrq;

// PCI Vendor id for Virtio devices

pub const PCI_VENDOR_ID_REDHAT: u16 = 0x1af4;
