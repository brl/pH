use std::sync::{Arc, Mutex, MutexGuard};
use vm_memory::GuestMemoryMmap;

use vmm_sys_util::eventfd::EventFd;

use crate::io::virtio::{Error, Result};
use crate::io::virtio::consts::MAX_QUEUE_SIZE;
use crate::io::virtio::queues::InterruptLine;
use crate::io::virtio::vq::chain::{Chain, DescriptorList};
use crate::io::virtio::vq::splitqueue::SplitQueue;

pub trait QueueBackend: Send {

    fn configure(&mut self, descriptor_area: u64, driver_area: u64, device_area: u64, size: u16, features: u64) -> Result<()>;

    fn reset(&mut self);
    fn is_empty(&self) -> bool;


    fn next_descriptors(&self) -> Option<(u16, DescriptorList,DescriptorList)>;
    fn put_used(&self, id: u16, size: u32);
}

#[derive(Clone)]
pub struct VirtQueue {
    ioeventfd: Arc<EventFd>,

    /// Default queue_size for this virtqueue
    default_size: u16,

    /// Number of elements in the virtqueue ring
    queue_size: u16,

    descriptor_area: u64,
    driver_area: u64,
    device_area: u64,

    backend: Arc<Mutex<dyn QueueBackend>>,

    /// Has this virtqueue been enabled?
    enabled: bool,
}

impl VirtQueue {
    pub const DEFAULT_QUEUE_SIZE: u16 = 128;

    pub fn new(memory: GuestMemoryMmap, default_size: u16, interrupt: Arc<InterruptLine>, ioeventfd: Arc<EventFd>) -> Self {
        let backend = Arc::new(Mutex::new(SplitQueue::new(memory, interrupt)));
        VirtQueue {
            ioeventfd,
            default_size,
            queue_size: default_size,
            descriptor_area: 0,
            driver_area: 0,
            device_area: 0,
            backend,
            enabled: false,
        }
    }

    fn backend(&self) -> MutexGuard<dyn QueueBackend+'static> {
        self.backend.lock().unwrap()
    }

    pub fn descriptor_area(&self) -> u64 {
        self.descriptor_area
    }

    pub fn set_descriptor_area(&mut self, address: u64) {
        self.descriptor_area = address;
    }

    pub fn driver_area(&self) -> u64 {
        self.driver_area
    }

    pub fn set_driver_area(&mut self, address: u64) {
        self.driver_area = address;
    }

    pub fn device_area(&self) -> u64 {
        self.device_area
    }

    pub fn set_device_area(&mut self, address: u64) {
        self.device_area = address
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn enable(&mut self) {
        self.enabled = true
    }

    ///
    /// Set the queue size of this `VirtQueue`.  If `sz` is an invalid value
    /// ignore the request.  It is illegal to change the queue size after
    /// a virtqueue has been enabled, so ignore requests if enabled.
    ///
    /// Valid sizes are less than or equal to `MAX_QUEUE_SIZE` and must
    /// be a power of 2.
    ///
    pub fn set_size(&mut self, sz: u16) {
        if self.is_enabled() || sz > MAX_QUEUE_SIZE || (sz & (sz - 1) != 0) {
            return;
        }
        self.queue_size = sz;
    }

    pub fn size(&self) -> u16 {
        self.queue_size
    }

    ///
    /// Reset `VirtQueue` to the initial state.  `queue_size` is set to the `default_size`
    /// and all other fields are cleared.  `enabled` is set to false.
    ///
    pub fn reset(&mut self) {
        self.queue_size = self.default_size;
        self.descriptor_area = 0;
        self.driver_area = 0;
        self.device_area = 0;
        self.enabled = false;
        self.backend().reset();
    }

    pub fn configure(&self, features: u64) -> Result<()> {
        if !self.enabled {
            return Err(Error::QueueNotEnabled);
        }
        self.backend().configure(self.descriptor_area, self.driver_area, self.device_area, self.size(), features)
    }

    ///
    /// Does `VirtQueue` currently have available entries?
    ///
    pub fn is_empty(&self) -> bool {
        self.backend().is_empty()
    }

    pub fn wait_ready(&self) -> Result<()> {
        if self.is_empty() {
            let _ = self.ioeventfd.read()
                .map_err(Error::ReadIoEventFd)?;
        }
        Ok(())
    }

    pub fn wait_next_chain(&self) -> Result<Chain> {
        loop {
            self.wait_ready()?;
            if let Some(chain) = self.next_chain() {
                return Ok(chain)
            }
        }
    }

    pub fn next_chain(&self) -> Option<Chain> {
        self.backend().next_descriptors().map(|(id, r, w)| {
            Chain::new(self.backend.clone(), id, r, w)
        })
    }

    pub fn on_each_chain<F>(&self, mut f: F)
        where F: FnMut(Chain) {
        loop {
            self.wait_ready().unwrap();
            for chain in self.iter() {
                f(chain);
            }
        }
    }

    pub fn iter(&self) -> QueueIter {
        QueueIter { vq: self.clone() }
    }

    pub fn ioevent(&self) -> &EventFd {
        &self.ioeventfd
    }
}

pub struct QueueIter {
    vq: VirtQueue
}

impl Iterator for QueueIter {
    type Item = Chain;

    fn next(&mut self) -> Option<Self::Item> {
        self.vq.next_chain()
    }
}