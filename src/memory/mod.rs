mod ram;
mod drm;
mod manager;
mod mmap;
mod address;
mod allocator;

pub use self::allocator::SystemAllocator;
pub use self::address::AddressRange;
pub use self::mmap::Mapping;
pub use self::ram::{GuestRam,MemoryRegion};
pub use manager::MemoryManager;

pub use drm::{DrmDescriptor,DrmPlaneDescriptor};

use std::{result, io};
use crate::system;

use thiserror::Error;

#[derive(Debug,Error)]
pub enum Error {
    #[error("failed to allocate memory for device")]
    DeviceMemoryAllocFailed,
    #[error("failed to create memory mapping for device memory: {0}")]
    MappingFailed(system::Error),
    #[error("failed to register memory for device memory: {0}")]
    RegisterMemoryFailed(kvm_ioctls::Error),
    #[error("failed to unregister memory for device memory: {0}")]
    UnregisterMemoryFailed(kvm_ioctls::Error),
    #[error("failed to open device with libgbm: {0}")]
    GbmCreateDevice(system::Error),
    #[error("failed to allocate buffer with libgbm: {0}")]
    GbmCreateBuffer(system::Error),
    #[error("error opening render node: {0}")]
    OpenRenderNode(io::Error),
    #[error("exporting prime handle to fd failed: {0}")]
    PrimeHandleToFD(system::ErrnoError),
    #[error("failed to create buffer: {0}")]
    CreateBuffer(io::Error),
    #[error("no DRM allocator is available")]
    NoDrmAllocator,
}

pub type Result<T> = result::Result<T, Error>;


