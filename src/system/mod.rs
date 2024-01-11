#[macro_use]pub mod ioctl;
mod epoll;
pub mod errno;
mod socket;
mod tap;
pub mod netlink;
pub mod drm;

pub use epoll::{EPoll,Event};
pub use socket::ScmSocket;
pub use netlink::NetlinkSocket;
pub use tap::Tap;
use std::{result, io};

pub use errno::Error as ErrnoError;

use thiserror::Error;
use vm_memory::guest_memory;
use vm_memory::mmap::MmapRegionError;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug,Error)]
pub enum Error {
    #[error("{0}")]
    Errno(errno::Error),
    #[error("failed to open /dev/kvm: {0}")]
    OpenKvmFailed(errno::Error),
    #[error("attempt to access invalid offset into mapping")]
    InvalidOffset,
    #[error("attempt to access invalid address: {0:16x}")]
    InvalidAddress(u64),
    #[error("failed to call {0} ioctl: {1}")]
    IoctlError(&'static str, errno::Error),
    #[error("failed writing to eventfd")]
    EventFdWrite,
    #[error("failed reading from eventfd")]
    EventFdRead,
    #[error("guest memory error: {0}")]
    GuestMemory(guest_memory::Error),
    #[error("failed to allocate shared memory: {0}")]
    ShmAllocFailed(memfd::Error),
    #[error("failed to create mmap region: {0}")]
    MmapRegionCreate(MmapRegionError)

}

impl Error {
    pub fn last_os_error() -> Error {
        Error::Errno(errno::Error::last_os_error())
    }

    pub fn last_errno() -> i32 {
        errno::Error::last_errno()
    }

    pub fn from_raw_os_error(e: i32) -> Error {
        Error::Errno(errno::Error::from_raw_os_error(e))
    }

    pub fn inner_err(&self) -> Option<&errno::Error> {
        match self {
            Error::IoctlError(_,e) => Some(e),
            Error::Errno(e) => Some(e),
            Error::OpenKvmFailed(e) => Some(e),
            _ => None,
        }
    }

    pub fn is_interrupted(&self) -> bool {
        self.inner_err()
            .map(|e| e.is_interrupted())
            .unwrap_or(false)
    }
}

impl From<guest_memory::Error> for Error {
    fn from(err: guest_memory::Error) -> Error {
        Error::GuestMemory(err)
    }
}

impl From<errno::Error> for Error {
    fn from(err: errno::Error) -> Error {
        Error::Errno(err)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::from_raw_os_error(e.raw_os_error().unwrap_or_default())
    }
}

impl From<Error> for io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Errno(e) => io::Error::from_raw_os_error(e.errno()),
            e => io::Error::new(io::ErrorKind::Other, e),
        }
    }
}
