mod device;
mod consts;
mod vq;
mod queues;
mod features;

use std::result;
pub use device::{VirtioDeviceState, VirtioDevice, DeviceConfigArea};
pub use queues::Queues;
pub use features::FeatureBits;
pub use consts::VirtioDeviceType;
pub use vq::virtqueue::VirtQueue;
pub use vq::chain::Chain;
use crate::io::bus::Error as BusError;

use thiserror::Error;
use vmm_sys_util::errno;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug,Error)]
pub enum Error {
    #[error("failed to create EventFd for VirtQueue: {0}")]
    CreateEventFd(std::io::Error),
    #[error("failed to create IoEventFd for VirtQueue: {0}")]
    CreateIoEventFd(kvm_ioctls::Error),
    #[error("failed to read from IoEventFd: {0}")]
    ReadIoEventFd(std::io::Error),
    #[error("VirtQueue not enabled")]
    QueueNotEnabled,
    #[error("VirtQueue descriptor table range is invalid 0x{0:x}")]
    RangeInvalid(u64),
    #[error("VirtQueue avail ring range range is invalid 0x{0:x}")]
    AvailInvalid(u64),
    #[error("VirtQueue used ring range is invalid 0x{0:x}")]
    UsedInvalid(u64),
    #[error("{0}")]
    BusInsert(#[from]BusError),
    #[error("Error registering irqfd: {0}")]
    IrqFd(errno::Error),
}