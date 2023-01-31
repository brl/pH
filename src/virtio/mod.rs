mod bus;
mod chain;
mod config;
mod consts;
mod device;
mod pci;
mod virtqueue;
mod vring;
mod device_config;

pub use self::virtqueue::VirtQueue;
pub use self::pci::PciIrq;
pub use self::bus::VirtioBus;
pub use self::device::{VirtioDevice,VirtioDeviceOps};
pub use self::chain::Chain;
pub use self::device_config::DeviceConfigArea;

use byteorder::{ByteOrder,LittleEndian};
use std::result;

use thiserror::Error;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug,Error)]
pub enum Error {
    #[error("failed to create EventFd for VirtQueue: {0}")]
    CreateEventFd(std::io::Error),
    #[error("failed to create IoEventFd for VirtQueue: {0}")]
    CreateIoEventFd(kvm_ioctls::Error),
    #[error("failed to read from IoEventFd: {0}")]
    ReadIoEventFd(std::io::Error),
    #[error("VirtQueue: {0}")]
    IrqFd(kvm_ioctls::Error),
    #[error("vring not enabled")]
    VringNotEnabled,
    #[error("vring descriptor table range is invalid 0x{0:x}")]
    VringRangeInvalid(u64),
    #[error("vring avail ring range range is invalid 0x{0:x}")]
    VringAvailInvalid(u64),
    #[error("vring used ring range is invalid 0x{0:x}")]
    VringUsedInvalid(u64),
}

pub fn read_config_buffer(config: &[u8], offset: usize, size: usize) -> u64 {
    if offset + size > config.len() {
        return 0;
    }
    match size {
        1 => config[offset] as u64,
        2 => LittleEndian::read_u16(&config[offset..]) as u64,
        4 => LittleEndian::read_u32(&config[offset..]) as u64,
        8 => LittleEndian::read_u64(&config[offset..]) as u64,
        _ => 0,
    }
}
