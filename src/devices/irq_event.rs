use std::{io, result};
use vmm_sys_util::eventfd::EventFd;
use crate::vm::KvmVm;

pub struct IrqLevelEvent {
    trigger_event: EventFd,
    resample_event: EventFd,
}

type Result<T> = result::Result<T, io::Error>;

impl IrqLevelEvent {
    pub fn register(kvm_vm: &KvmVm, irq: u8) -> Result<Self> {
        let ev = Self::new()?;
        kvm_vm.vm_fd()
            .register_irqfd_with_resample(&ev.trigger_event, &ev.resample_event, irq as u32)?;
        Ok(ev)
    }

    pub fn new() -> Result<Self> {
        let trigger_event = EventFd::new(0)?;
        let resample_event = EventFd::new(0)?;
        Ok(IrqLevelEvent {
            trigger_event, resample_event,
        })
    }

    pub fn try_clone(&self) -> Result<IrqLevelEvent> {
        let trigger_event = self.trigger_event.try_clone()?;
        let resample_event = self.resample_event.try_clone()?;
        Ok(IrqLevelEvent {
            trigger_event,
            resample_event,
        })
    }

    pub fn trigger(&self) -> Result<()> {
        self.trigger_event.write(1)
    }

    pub fn wait_resample(&self) -> Result<()> {
        let _ = self.resample_event.read()?;
        Ok(())
    }
}