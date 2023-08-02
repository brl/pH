use std::sync::{Arc, atomic};
use std::sync::atomic::Ordering;
use crate::io::virtio::Error;
use crate::io::virtio::features::ReservedFeatureBit;
use crate::io::virtio::queues::InterruptLine;
use crate::io::virtio::vq::chain::DescriptorList;
use crate::io::virtio::vq::descriptor::Descriptor;
use crate::io::virtio::vq::SharedIndex;
use crate::io::virtio::vq::virtqueue::QueueBackend;
use crate::memory::GuestRam;


pub struct SplitQueue {
    memory: GuestRam,
    interrupt: Arc<InterruptLine>,

    queue_size: u16,
    features: u64,

    descriptor_base: u64,
    avail_base: u64,
    used_base: u64,
    /// last seen avail_idx loaded from guest memory
    cached_avail_idx: SharedIndex,
    /// The index in the avail ring where the next available entry will be read
    next_avail: SharedIndex,
    /// The index in the used ring where the next used entry will be placed
    next_used_idx: SharedIndex,
}

impl SplitQueue {
    pub fn new(memory: GuestRam, interrupt: Arc<InterruptLine>) -> Self {
        SplitQueue {
            memory,
            interrupt,
            queue_size: 0,
            features: 0,
            descriptor_base: 0,
            avail_base: 0,
            used_base: 0,

            cached_avail_idx: SharedIndex::new(),
            next_avail: SharedIndex::new(),
            next_used_idx: SharedIndex::new(),
        }
    }

    ///
    /// Load the descriptor table entry at `idx` from guest memory and return it.
    ///
    fn load_descriptor(&self, idx: u16) -> Option<Descriptor> {
        if idx >= self.queue_size {
            panic!("load_descriptor called with index larger than queue size");
        }
        let head = self.descriptor_base + (idx as u64 * 16);

        let addr = self.memory.read_int::<u64>(head).unwrap();
        let len= self.memory.read_int::<u32>(head + 8).unwrap();
        let flags = self.memory.read_int::<u16>(head + 12).unwrap();
        let next = self.memory.read_int::<u16>(head + 14).unwrap();

        if self.memory.is_valid_range(addr, len as usize) && next < self.queue_size {
            return Some(Descriptor::new(addr, len, flags, next));
        }
        None
    }

    fn load_descriptor_lists(&self, head: u16) -> (DescriptorList,DescriptorList) {
        let mut readable = DescriptorList::new(self.memory.clone());
        let mut writeable = DescriptorList::new(self.memory.clone());
        let mut idx = head;
        let mut ttl = self.queue_size;

        while let Some(d) = self.load_descriptor(idx) {
            if ttl == 0 {
                warn!("Descriptor chain length exceeded ttl");
                break;
            } else {
                ttl -= 1;
            }
            if d.is_write() {
                writeable.add_descriptor(d);
            } else {
                if !writeable.is_empty() {
                    warn!("Guest sent readable virtqueue descriptor after writeable descriptor in violation of specification");
                }
                readable.add_descriptor(d);
            }
            if !d.has_next() {
                break;
            }
            idx = d.next();
        }

        readable.reverse();
        writeable.reverse();
        return (readable, writeable)
    }

    ///
    /// Load `avail_ring.idx` from guest memory and store it in `cached_avail_idx`.
    ///
    fn load_avail_idx(&self) -> u16 {
        let avail_idx = self.memory.read_int::<u16>(self.avail_base + 2).unwrap();
        self.cached_avail_idx.set(avail_idx);
        avail_idx
    }

    ///
    /// Read from guest memory and return the Avail ring entry at
    /// index `ring_idx % queue_size`.
    ///
    fn load_avail_entry(&self, ring_idx: u16) -> u16 {
        let offset = (4 + (ring_idx % self.queue_size) * 2) as u64;
        self.memory.read_int(self.avail_base + offset).unwrap()
    }

    /// Queue is empty if `next_avail` is same value as
    /// `avail_ring.idx` value in guest memory If `cached_avail_idx`
    /// currently matches `next_avail` it is reloaded from
    /// memory in case guest has updated field since last
    /// time it was loaded.
    ///
    fn is_empty(&self) -> bool {
        let next_avail = self.next_avail.get();
        if self.cached_avail_idx.get() != next_avail {
            return false;
        }
        next_avail == self.load_avail_idx()
    }

    ///
    /// If queue is not empty, read and return the next Avail ring entry
    /// and increment `next_avail`.  If queue is empty return `None`
    ///
    fn pop_avail_entry(&self) -> Option<u16> {
        if self.is_empty() {
            return None
        }
        let next_avail = self.next_avail.get();
        let avail_entry = self.load_avail_entry(next_avail);
        self.next_avail.inc();
        if self.has_event_idx() {
            self.write_avail_event(self.next_avail.get());
        }
        Some(avail_entry)
    }

    fn read_avail_flags(&self) -> u16 {
        self.memory.read_int::<u16>(self.avail_base).unwrap()
    }

    ///
    /// Write an entry into the Used ring.
    ///
    /// The entry is written into the ring structure at offset
    /// `next_used_idx % queue_size`.  The value of `next_used_idx`
    /// is then incremented and the new value is written into
    /// guest memory into the `used_ring.idx` field.
    ///
    fn put_used_entry(&self, idx: u16, len: u32) {
        if idx >= self.queue_size {
            return;
        }

        let used_idx = (self.next_used_idx.get() % self.queue_size) as u64;
        let elem_addr = self.used_base + (4 + used_idx * 8);
        // write descriptor index to 'next used' slot in used ring
        self.memory.write_int(elem_addr, idx as u32).unwrap();
        // write length to 'next used' slot in ring
        self.memory.write_int(elem_addr + 4, len as u32).unwrap();

        self.next_used_idx.inc();
        atomic::fence(Ordering::Release);
        // write updated next_used
        self.memory.write_int(self.used_base + 2, self.next_used_idx.get()).unwrap();
    }

    ///
    /// Write `val` to the `avail_event` field of Used ring.
    ///
    /// If `val` is not a valid index for this virtqueue this
    /// function does nothing.
    ///
    pub fn write_avail_event(&self, val: u16) {
        if val > self.queue_size {
            return;
        }
        let addr = self.used_base + 4 + (self.queue_size as u64 * 8);
        self.memory.write_int::<u16>(addr, val).unwrap();
        atomic::fence(Ordering::Release);
    }

    fn has_event_idx(&self) -> bool {
        ReservedFeatureBit::EventIdx.is_set_in(self.features)
    }

    ///
    /// Read and return the `used_event` field from the Avail ring
    fn read_used_event(&self) -> u16 {
        let addr = self.avail_base + 4 + (self.queue_size as u64  * 2);
        self.memory.read_int::<u16>(addr).unwrap()
    }

    fn need_interrupt(&self, first_used: u16) -> bool {
        if self.has_event_idx() {
            first_used == self.read_used_event()
        } else {
            self.read_avail_flags() & 0x1 == 0
        }
    }
}

impl QueueBackend for SplitQueue {
    fn configure(&mut self, descriptor_area: u64, driver_area: u64, device_area: u64, size: u16, features: u64) -> crate::io::virtio::Result<()> {
        let desc_table_sz = 16 * size as usize;
        let avail_ring_sz = 6 + 2 * size as usize;
        let used_ring_sz = 6 + 8 * size as usize;

        if !self.memory.is_valid_range(descriptor_area, desc_table_sz) {
            return Err(Error::RangeInvalid(descriptor_area));
        }
        if !self.memory.is_valid_range(driver_area, avail_ring_sz) {
            return Err(Error::AvailInvalid(driver_area));
        }
        if !self.memory.is_valid_range(device_area, used_ring_sz) {
            return Err(Error::UsedInvalid(device_area));
        }

        self.descriptor_base = descriptor_area;
        self.avail_base = driver_area;
        self.used_base = device_area;
        self.queue_size = size;
        self.features = features;

        Ok(())
    }

    fn reset(&mut self) {
        self.queue_size = 0;
        self.features = 0;
        self.descriptor_base = 0;
        self.avail_base = 0;
        self.used_base = 0;
        self.next_avail.set(0);
        self.cached_avail_idx.set(0);
        self.next_used_idx.set(0);
    }

    /// Queue is empty if `next_avail` is same value as
    /// `avail_ring.idx` value in guest memory If `cached_avail_idx`
    /// currently matches `next_avail` it is reloaded from
    /// memory in case guest has updated field since last
    /// time it was loaded.
    ///
    fn is_empty(&self) -> bool {
        let next_avail = self.next_avail.get();
        if self.cached_avail_idx.get() != next_avail {
            return false;
        }
        next_avail == self.load_avail_idx()
    }

    fn next_descriptors(&self) -> Option<(u16, DescriptorList, DescriptorList)> {
        self.pop_avail_entry().map(|head| {
            let (r,w) = self.load_descriptor_lists(head);
            (head, r, w)
        })
    }

    fn put_used(&self, id: u16, size: u32) {
        let used = self.next_used_idx.get();
        self.put_used_entry(id, size);
        if self.need_interrupt(used) {
            self.interrupt.notify_queue();
        }
    }
}