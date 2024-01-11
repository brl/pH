use std::os::unix::io::RawFd;

use crate::devices::virtio_wl::{
    consts::{VIRTIO_WL_VFD_MAP, VIRTIO_WL_VFD_WRITE},
    Error, Result, VfdObject
};
use crate::io::shm_mapper::{DeviceSharedMemoryManager, SharedMemoryAllocation};

pub struct VfdSharedMemory {
    vfd_id: u32,
    flags: u32,
    shm: SharedMemoryAllocation,
}

impl VfdSharedMemory {
    fn round_to_page_size(n: usize) -> usize {
        let mask = 4096 - 1;
        (n + mask) & !mask
    }

    pub fn new(vfd_id: u32, transition_flags: bool, shm: SharedMemoryAllocation) -> Self {
        let flags = if transition_flags { 0 } else { VIRTIO_WL_VFD_WRITE | VIRTIO_WL_VFD_MAP};
        VfdSharedMemory { vfd_id, flags, shm }
    }

    pub fn create(vfd_id: u32, transition_flags: bool, size: u32, dev_shm_manager: &DeviceSharedMemoryManager) -> Result<Self> {
        let size = Self::round_to_page_size(size as usize);
        let shm = dev_shm_manager.allocate_buffer(size)
            .map_err(Error::ShmAllocFailed)?;
        Ok(Self::new(vfd_id, transition_flags, shm))
    }

    pub fn create_dmabuf(vfd_id: u32, tflags: bool, width: u32, height: u32, format: u32, dev_shm_manager: &DeviceSharedMemoryManager) -> Result<Self> {
        let shm  = dev_shm_manager.allocate_drm_buffer(width, height, format)
            .map_err(Error::ShmAllocFailed)?;
        Ok(Self::new(vfd_id, tflags, shm))
    }
}

impl VfdObject for VfdSharedMemory {
    fn id(&self) -> u32 {
        self.vfd_id
    }

    fn send_fd(&self) -> Option<RawFd> {
        Some(self.shm.raw_fd())
    }

    fn flags(&self) -> u32 {
        self.flags
    }

    fn shared_memory(&self) -> Option<SharedMemoryAllocation> {
        Some(self.shm)
    }
}
