use std::os::unix::io::RawFd;
use std::{result, io};
use std::fs::File;

use thiserror::Error;
use vm_memory::{VolatileMemoryError, VolatileSlice};

use crate::system;

mod vfd;
mod shm;
mod pipe;
mod socket;
mod device;

mod consts {
    use std::mem;

    pub const VIRTWL_SEND_MAX_ALLOCS: usize = 28;
    pub const VIRTIO_WL_CMD_VFD_NEW: u32 = 256;
    pub const VIRTIO_WL_CMD_VFD_CLOSE: u32 = 257;
    pub const VIRTIO_WL_CMD_VFD_SEND: u32 = 258;
    pub const VIRTIO_WL_CMD_VFD_RECV: u32 = 259;
    pub const VIRTIO_WL_CMD_VFD_NEW_CTX: u32 = 260;
    pub const VIRTIO_WL_CMD_VFD_NEW_PIPE: u32 = 261;
    pub const VIRTIO_WL_CMD_VFD_HUP: u32 = 262;
    pub const VIRTIO_WL_CMD_VFD_NEW_DMABUF: u32 = 263;
    pub const VIRTIO_WL_CMD_VFD_DMABUF_SYNC: u32 = 264;
    pub const VIRTIO_WL_RESP_OK: u32 = 4096;
    pub const VIRTIO_WL_RESP_VFD_NEW: u32 = 4097;
    pub const VIRTIO_WL_RESP_VFD_NEW_DMABUF: u32 = 4098;
    pub const VIRTIO_WL_RESP_ERR: u32 = 4352;
    pub const VIRTIO_WL_RESP_OUT_OF_MEMORY: u32 = 4353;
    pub const VIRTIO_WL_RESP_INVALID_ID: u32 = 4354;
    pub const VIRTIO_WL_RESP_INVALID_TYPE: u32 = 4355;
    pub const VIRTIO_WL_RESP_INVALID_FLAGS: u32 = 4356;
    pub const VIRTIO_WL_RESP_INVALID_CMD: u32 = 4357;

    pub const VIRTIO_WL_VFD_WRITE: u32 = 0x1;          // Intended to be written by guest
    pub const VIRTIO_WL_VFD_READ: u32 = 0x2;           // Intended to be read by guest

    pub const VIRTIO_WL_VFD_MAP: u32 = 0x2;
    pub const VIRTIO_WL_VFD_CONTROL: u32 = 0x4;
    pub const VIRTIO_WL_F_TRANS_FLAGS: u32 = 0x01;

    pub const NEXT_VFD_ID_BASE: u32 = 0x40000000;
    pub const VFD_ID_HOST_MASK: u32 = NEXT_VFD_ID_BASE;

    pub const VFD_RECV_HDR_SIZE: usize = 16;
    pub const IN_BUFFER_LEN: usize =
        0x1000 - VFD_RECV_HDR_SIZE - VIRTWL_SEND_MAX_ALLOCS * mem::size_of::<u32>();
}

pub use device::VirtioWayland;
use crate::devices::virtio_wl::shm_mapper::SharedMemoryAllocation;
use crate::io::shm_mapper;

pub type Result<T> = result::Result<T, Error>;

pub struct VfdRecv {
    buf: Vec<u8>,
    fds: Option<Vec<File>>,
}

impl VfdRecv {
    fn new(buf: Vec<u8>) -> Self {
        VfdRecv { buf, fds: None }
    }
    fn new_with_fds(buf: Vec<u8>, fds: Vec<File>) -> Self {
        VfdRecv { buf, fds: Some(fds) }
    }
}

pub trait VfdObject {
    fn id(&self) -> u32;
    fn send_fd(&self) -> Option<RawFd> { None }
    fn poll_fd(&self) -> Option<RawFd> { None }
    fn recv(&mut self) -> Result<Option<VfdRecv>> { Ok(None) }
    fn send(&mut self, _data: &VolatileSlice) -> Result<()> { Err(Error::InvalidSendVfd) }
    fn send_with_fds(&mut self, _data: &VolatileSlice, _fds: &[RawFd]) -> Result<()> { Err(Error::InvalidSendVfd) }
    fn flags(&self) -> u32;
    fn shared_memory(&self) -> Option<SharedMemoryAllocation> { None }
    fn close(&mut self) -> Result<()> { Ok(()) }
}


#[derive(Debug,Error)]
pub enum Error {
    #[error("error reading from ioevent fd: {0}")]
    IoEventError(io::Error),
    #[error("error creating eventfd: {0}")]
    EventFdCreate(io::Error),
    #[error("i/o error on virtio chain operation: {0}")]
    ChainIoError(#[from] io::Error),
    #[error("unexpected virtio wayland command: {0}")]
    UnexpectedCommand(u32),
    #[error("failed to allocate shared memory: {0}")]
    ShmAllocFailed(shm_mapper::Error),
    #[error("failed to free shared memory allocation: {0}")]
    ShmFreeFailed(shm_mapper::Error),
    #[error("failed to create pipes: {0}")]
    CreatePipesFailed(system::Error),
    #[error("error reading from socket: {0}")]
    SocketReceive(system::ErrnoError),
    #[error("error connecting to socket: {0}")]
    SocketConnect(io::Error),
    #[error("error reading from pipe: {0}")]
    PipeReceive(io::Error),
    #[error("error writing to vfd: {0}")]
    SendVfd(io::Error),
    #[error("error writing volatile memory to vfd: {0}")]
    VolatileSendVfd(VolatileMemoryError),
    #[error("attempt to send to incorrect vfd type")]
    InvalidSendVfd,
    #[error("message has too many vfd ids: {0}")]
    TooManySendVfds(usize),
    #[error("failed creating poll context: {0}")]
    FailedPollContextCreate(system::Error),
    #[error("failed adding fd to poll context: {0}")]
    FailedPollAdd(system::Error),
    #[error("error calling dma sync: {0}")]
    DmaSync(system::ErrnoError),
}
