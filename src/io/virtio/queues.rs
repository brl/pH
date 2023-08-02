use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use kvm_ioctls::{IoEventAddress, NoDatamatch};
use vmm_sys_util::eventfd::EventFd;
use crate::memory::MemoryManager;
use crate::io::virtio::{Error, Result};
use crate::io::virtio::consts::VIRTIO_MMIO_OFFSET_NOTIFY;
use crate::io::VirtQueue;
use crate::vm::KvmVm;

pub struct InterruptLine {
    irqfd: EventFd,
    irq: u8,
    isr: AtomicUsize,
}

impl InterruptLine {
    fn new(kvm_vm: &KvmVm, irq: u8) -> Result<InterruptLine> {
        let irqfd = EventFd::new(0)
            .map_err(Error::CreateEventFd)?;
        kvm_vm.vm_fd().register_irqfd(&irqfd, irq as u32)
            .map_err(Error::IrqFd)?;
        Ok(InterruptLine{
            irqfd,
            irq,
            isr: AtomicUsize::new(0)
        })

    }

    fn irq(&self) -> u8 {
        self.irq
    }


    fn isr_read(&self) -> u64 {
        self.isr.swap(0, Ordering::SeqCst) as u64
    }

    pub fn notify_queue(&self) {
        self.isr.fetch_or(0x1, Ordering::SeqCst);
        self.irqfd.write(1).unwrap();
    }

    pub fn notify_config(&self) {
        self.isr.fetch_or(0x2, Ordering::SeqCst);
        self.irqfd.write(1).unwrap();
    }
}

pub struct Queues {
    memory: MemoryManager,
    selected_queue: u16,
    queues: Vec<VirtQueue>,
    interrupt: Arc<InterruptLine>,
}

impl Queues {
    pub fn new(memory: MemoryManager, irq: u8) -> Result<Self> {
        let interrupt = InterruptLine::new(memory.kvm_vm(), irq)?;
        let queues = Queues {
            memory,
            selected_queue: 0,
            queues: Vec::new(),
            interrupt: Arc::new(interrupt),
        };
        Ok(queues)
    }

    pub fn get_queue(&self, idx: usize) -> VirtQueue {
        self.queues
            .get(idx)
            .cloned()
            .expect(&format!("Virtio device requested VQ index {} that does not exist", idx))
    }

    pub fn queues(&self) -> Vec<VirtQueue> {
        self.queues.clone()
    }

    pub fn memory(&self) -> &MemoryManager {
        &self.memory
    }

    pub fn configure_queues(&self, features: u64) -> Result<()> {
        for q in &self.queues {
            q.configure(features)?;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.selected_queue = 0;
        let _ = self.isr_read();
        for vr in &mut self.queues {
            vr.reset();
        }
    }

    pub fn irq(&self) -> u8 {
        self.interrupt.irq()
    }

    pub fn isr_read(&self) -> u64 {
        self.interrupt.isr_read()
    }

    pub fn num_queues(&self) -> u16 {
        self.queues.len() as u16
    }

    pub fn create_queues(&mut self, mmio_base: u64, queue_sizes: &[u16]) -> Result<()> {
        let mut idx = 0;
        for &sz in queue_sizes {
            let ioevent = self.create_ioevent(idx, mmio_base)?;
            let vq = VirtQueue::new(self.memory.guest_ram().clone(), sz, self.interrupt.clone(), ioevent);
            self.queues.push(vq);
            idx += 1;
        }
        Ok(())
    }

    fn create_ioevent(&self, index: usize, mmio_base: u64) -> Result<Arc<EventFd>> {
        let evt = EventFd::new(0)
            .map_err(Error::CreateEventFd)?;

        let notify_address = mmio_base +
            VIRTIO_MMIO_OFFSET_NOTIFY +
            (4 * index as u64);

        let addr = IoEventAddress::Mmio(notify_address);

        self.memory.kvm_vm().vm_fd().register_ioevent(&evt, &addr, NoDatamatch)
            .map_err(Error::CreateIoEventFd)?;

        Ok(Arc::new(evt))
    }


    fn current_queue(&self) -> Option<&VirtQueue> {
        self.queues.get(self.selected_queue as usize)
    }

    fn with_current<F>(&mut self, f: F)
        where F: FnOnce(&mut VirtQueue)
    {
        if let Some(vq) = self.queues.get_mut(self.selected_queue as usize) {
            if !vq.is_enabled() {
                f(vq)
            }
        }
    }

    pub fn selected_queue(&self) -> u16 {
        self.selected_queue
    }

    pub fn select(&mut self, index: u16) {
        self.selected_queue = index;
    }

    pub fn is_current_enabled(&self) -> bool {
        self.current_queue()
            .map(|q| q.is_enabled())
            .unwrap_or(false)
    }

    pub fn queue_size(&self) -> u16 {
        self.current_queue()
            .map(|q| q.size())
            .unwrap_or(0)
    }

    pub fn set_size(&mut self, size: u16) {
        self.with_current(|q| q.set_size(size))
    }

    pub fn enable_current(&mut self) {
        self.with_current(|q| q.enable())
    }

    pub fn get_current_descriptor_area(&self, hi_word: bool) -> u32 {
        self.current_queue().map(|q| if hi_word {
            Self::get_hi32(q.descriptor_area())
        } else {
            Self::get_lo32(q.descriptor_area())
        }).unwrap_or(0)

    }
    pub fn set_current_descriptor_area(&mut self, val: u32, hi_word: bool) {
        self.with_current(|q| {
            let mut addr = q.descriptor_area();
            if hi_word { Self::set_hi32(&mut addr, val) } else { Self::set_lo32(&mut addr, val) }
            q.set_descriptor_area(addr);
        });
    }

    pub fn get_avail_area(&self, hi_word: bool) -> u32 {
       self.current_queue().map(|q| if hi_word {
           Self::get_hi32(q.driver_area())
       } else {
           Self::get_lo32(q.driver_area())
       }).unwrap_or(0)
    }

    fn set_hi32(val: &mut u64, dword: u32) {
        const MASK_LO_32: u64 = (1u64 << 32) - 1;
        *val = (*val & MASK_LO_32) | (u64::from(dword) << 32)
    }

    fn set_lo32(val: &mut u64, dword: u32) {
        const MASK_HI_32: u64 = ((1u64 << 32) - 1) << 32;
        *val = (*val & MASK_HI_32) | u64::from(dword)
    }

    fn get_hi32(val: u64) -> u32 {
        (val >> 32) as u32
    }

    fn get_lo32(val: u64) -> u32 {
        val as u32
    }

    pub fn set_avail_area(&mut self, val: u32, hi_word: bool) {
        self.with_current(|q| {
            let mut addr = q.driver_area();
            if hi_word { Self::set_hi32(&mut addr, val) } else { Self::set_lo32(&mut addr, val) }
            q.set_driver_area(addr);
        });
    }

    pub fn set_used_area(&mut self, val: u32, hi_word: bool) {
        self.with_current(|q| {
            let mut addr = q.device_area();
            if hi_word { Self::set_hi32(&mut addr, val) } else { Self::set_lo32(&mut addr, val) }
            q.set_device_area(addr);
        });
    }

    pub fn get_used_area(&self, hi_word: bool) -> u32 {
        self.current_queue().map(|q| if hi_word {
            Self::get_hi32(q.device_area())
        } else {
            Self::get_lo32(q.device_area())
        }).unwrap_or(0)
    }
}