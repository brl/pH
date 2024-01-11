use std::collections::HashMap;
use std::fs::File;
use std::os::fd::{AsRawFd, RawFd};
use std::result;
use std::sync::{Arc, Mutex, MutexGuard};
use vm_allocator::{AddressAllocator, AllocPolicy, RangeInclusive};
use vm_memory::{Address, FileOffset, GuestAddress, GuestMemory, GuestMemoryMmap, GuestMemoryRegion, MmapRegion};
use crate::system::drm::{DrmBufferAllocator, DrmDescriptor};
use crate::system::drm;
use crate::util::BitSet;
use crate::vm::KvmVm;

use thiserror::Error;
use std::io::{Seek, SeekFrom};
use memfd::{FileSeal, MemfdOptions};
use crate::system;

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug,Error)]
pub enum Error {
    #[error("failed to create SharedMemory: {0}")]
    SharedMemoryCreation(system::Error),
    #[error("failed to allocate DRM buffer: {0}")]
    DrmAllocateFailed(drm::Error),
    #[error("no DRM memory allocator")]
    NoDrmAllocator,
    #[error("failed to register memory with hypervisor: {0}")]
    RegisterMemoryFailed(kvm_ioctls::Error),
    #[error("failed to unregister memory with hypervisor: {0}")]
    UnregisterMemoryFailed(kvm_ioctls::Error),
    #[error("failed to allocate memory for device")]
    DeviceMemoryAllocFailed,

}

/// Tracks graphic buffer memory allocations shared between host and guest.
///
/// The allocated buffers are opaque to the hypervisor and are referred to only
/// by file descriptor. These memory regions are passed to or received from the
/// wayland compositor (ie GNOME Shell) and are mapped into guest memory space so
/// that a device driver in the guest can make the shared memory available for
/// rendering application windows.
///
#[derive(Clone)]
pub struct DeviceSharedMemoryManager {
    device_memory: Arc<Mutex<DeviceSharedMemory>>,
}

impl DeviceSharedMemoryManager {

    pub fn new(kvm_vm: &KvmVm, memory: &GuestMemoryMmap) -> Self {
        let device_memory = DeviceSharedMemory::new(kvm_vm.clone(), memory);
        DeviceSharedMemoryManager {
            device_memory: Arc::new(Mutex::new(device_memory)),
        }
    }

    pub fn free_buffer(&self, slot: u32) -> Result<()> {
        self.dev_memory().unregister(slot)
    }

    pub fn allocate_buffer_from_file(&self, fd: File) -> Result<SharedMemoryAllocation> {
        let memory = SharedMemoryMapping::from_file(fd)
            .map_err(Error::SharedMemoryCreation)?;

        self.dev_memory().register(memory)
    }

    pub fn allocate_buffer(&self, size: usize) -> Result<SharedMemoryAllocation> {
        let memory = SharedMemoryMapping::create_memfd(size, "ph-dev-shm")
            .map_err(Error::SharedMemoryCreation)?;

        self.dev_memory().register(memory)
    }

    pub fn allocate_drm_buffer(&self, width: u32, height: u32, format: u32) -> Result<SharedMemoryAllocation> {
        self.dev_memory().allocate_drm_buffer(width, height, format)
    }

    fn dev_memory(&self) -> MutexGuard<DeviceSharedMemory> {
        self.device_memory.lock().unwrap()
    }
}

#[derive(Copy,Clone)]
pub struct SharedMemoryAllocation {
    pfn: u64,
    size: usize,
    slot: u32,
    raw_fd: RawFd,
    drm_descriptor: Option<DrmDescriptor>,
}

impl SharedMemoryAllocation {
    fn new(pfn: u64, size: usize, slot: u32, raw_fd: RawFd) -> Self {
        SharedMemoryAllocation {
            pfn, size, slot, raw_fd,
            drm_descriptor: None,
        }
    }

    fn set_drm_descriptor(&mut self, drm_descriptor: DrmDescriptor) {
        self.drm_descriptor.replace(drm_descriptor);
    }

    pub fn pfn(&self) -> u64 {
        self.pfn
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn slot(&self) -> u32 {
        self.slot
    }

    pub fn raw_fd(&self) -> RawFd {
        self.raw_fd
    }

    pub fn drm_descriptor(&self) -> Option<DrmDescriptor> {
        self.drm_descriptor
    }
}

struct DeviceSharedMemory {
    kvm_vm: KvmVm,
    slots: BitSet,
    mappings: HashMap<u32, SharedMemoryMapping>,
    allocator: AddressAllocator,
    drm_allocator: Option<DrmBufferAllocator>
}

impl DeviceSharedMemory {
    const WL_SHM_SIZE: u64 = 1 << 32;

    fn create_allocator(memory: &GuestMemoryMmap) -> AddressAllocator {
        let device_memory_base = || -> GuestAddress {
            // Put device memory at a 2MB boundary after physical memory or at 4GB,
            // whichever is higher.
            const ONE_MB: u64 = 1 << 20;
            const FOUR_GB: u64 = 4 * 1024 * ONE_MB;

            memory.iter()
                .map(|addr| addr.last_addr().unchecked_align_up(2 * ONE_MB))
                .max()
                .map(|top| std::cmp::max(top, GuestAddress(FOUR_GB)))
                .expect("Failed to compute device memory base")
        };
        let base = device_memory_base();
        AddressAllocator::new(base.raw_value(), Self::WL_SHM_SIZE)
            .expect("Failed to create wayland shared memory allocator")

    }

    fn new(kvm_vm: KvmVm, memory: &GuestMemoryMmap) -> Self {
        let allocator = Self::create_allocator(memory);
        let mut slots = BitSet::new();
        for idx in 0..memory.num_regions() {
            slots.insert(idx);
        }

        DeviceSharedMemory {
            kvm_vm,
            slots,
            mappings: HashMap::new(),
            allocator,
            drm_allocator: None,
        }
    }

    fn is_drm_enabled(&self) -> bool {
        self.drm_allocator.is_some()
    }

    fn enable_drm_allocator(&mut self) {
        if !self.is_drm_enabled() {
            match DrmBufferAllocator::open() {
                Ok(allocator) => { self.drm_allocator.replace(allocator); },
                Err(err) => {
                    warn!("failed to open DRM buffer allocator: {}", err);
                },
            };
        }
    }

    fn allocate_drm_buffer(&mut self, width: u32, height: u32, format: u32) -> Result<SharedMemoryAllocation> {
        if !self.is_drm_enabled() {
            self.enable_drm_allocator();
        }

        if let Some(drm_allocator) = self.drm_allocator.as_ref() {
            let (fd, desc) = drm_allocator.allocate(width, height, format)
                .map_err(Error::DrmAllocateFailed)?;
            let memory = SharedMemoryMapping::from_file(fd)
                .map_err(Error::SharedMemoryCreation)?;

            let mut registration = self.register(memory)?;
            registration.set_drm_descriptor(desc);
            Ok(registration)
        } else {
            Err(Error::NoDrmAllocator)
        }
    }

    fn register(&mut self, mut memory: SharedMemoryMapping) -> Result<SharedMemoryAllocation> {

        fn round_to_page_size(n: usize) -> usize {
            let mask = 4096 - 1;
            (n + mask) & !mask
        }


        let size = round_to_page_size(memory.size());
        let (range, slot) = self.allocate_addr_and_slot(size)?;
        memory.set_guest_range(range.clone());

        if let Err(e) = self.kvm_vm.add_memory_region(slot, range.start(), memory.mapping_host_address(), size) {
            self.free_range_and_slot(&range, slot);
            Err(Error::RegisterMemoryFailed(e))
        } else {
            let pfn = range.start() >> 12;
            let size = memory.size();
            let raw_fd = memory.raw_fd();
            self.mappings.insert(slot, memory);
            Ok(SharedMemoryAllocation::new(pfn, size, slot, raw_fd))
        }
    }

    fn unregister(&mut self, slot: u32) -> Result<()> {
        if let Some(registration) = self.mappings.remove(&slot) {
            self.kvm_vm.remove_memory_region(slot)
                .map_err(Error::UnregisterMemoryFailed)?;
            if let Some(range) = registration.guest_range() {
                self.free_range_and_slot(range, slot);
            } else {
                // XXX
            }
        }
        Ok(())
    }

    fn allocate_addr_and_slot(&mut self, size: usize) -> Result<(RangeInclusive, u32)> {
        let range = self.allocator.allocate(
            size as u64,
            4096,
            AllocPolicy::FirstMatch
        ).map_err(|_| Error::DeviceMemoryAllocFailed)?;
        Ok((range, self.allocate_slot()))
    }

    fn free_range_and_slot(&mut self, range: &RangeInclusive, slot: u32) {
        if let Err(err) = self.allocator.free(range) {
            warn!("Error freeing memory range from allocator: {}", err);
        }
        self.free_slot(slot);
    }

    fn allocate_slot(&mut self) -> u32 {
        for i in 0.. {
            if !self.slots.get(i) {
                self.slots.insert(i);
                return i as u32;
            }
        }
        unreachable!()
    }

    fn free_slot(&mut self, slot: u32) {
        self.slots.remove(slot as usize)
    }
}

struct SharedMemoryMapping {
    mapping: MmapRegion,
    guest_range: Option<RangeInclusive>,
}

impl SharedMemoryMapping {
    fn from_file(fd: File) -> system::Result<Self> {
        let size = (&fd).seek(SeekFrom::End(0))? as usize;

        let file_offset = FileOffset::new(fd, 0);
        let mapping = MmapRegion::from_file(file_offset, size)
            .map_err(system::Error::MmapRegionCreate)?;
        Ok(SharedMemoryMapping {
            mapping,
            guest_range: None,
        })
    }

    fn create_memfd(size: usize, name: &str) -> system::Result<Self> {
        let memfd = MemfdOptions::default()
            .allow_sealing(true)
            .create(name)
            .map_err(system::Error::ShmAllocFailed)?;

        memfd.as_file().set_len(size as u64)?;
        memfd.add_seals(&[
            FileSeal::SealShrink,
            FileSeal::SealGrow,
        ]).map_err(system::Error::ShmAllocFailed)?;
        memfd.add_seal(FileSeal::SealSeal)
            .map_err(system::Error::ShmAllocFailed)?;

        let fd = memfd.into_file();
        let file_offset = FileOffset::new(fd, 0);
        let mapping = MmapRegion::from_file(file_offset, size)
            .map_err(system::Error::MmapRegionCreate)?;

        Ok(SharedMemoryMapping {
            mapping,
            guest_range: None,
        })
    }

    fn size(&self) -> usize {
        self.mapping.size()
    }

    fn mapping_host_address(&self) -> u64 {
        self.mapping.as_ptr() as u64
    }

    fn set_guest_range(&mut self, range: RangeInclusive) {
        self.guest_range.replace(range);
    }

    fn guest_range(&self) -> Option<&RangeInclusive> {
        self.guest_range.as_ref()
    }

    fn raw_fd(&self) -> RawFd {
        self.mapping.file_offset()
            .expect("SharedMemory mapping does not have a file!")
            .file()
            .as_raw_fd()
    }
}
