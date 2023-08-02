use std::sync::{Arc, Barrier};
use std::sync::atomic::{AtomicBool,Ordering};
use kvm_ioctls::{VcpuExit, VcpuFd};
use crate::io::manager::IoManager;


pub struct Vcpu {
    vcpu_fd: VcpuFd,
    io_manager: IoManager,
    shutdown: Arc<AtomicBool>,
}


impl Vcpu {
    pub fn new(vcpu_fd: VcpuFd, io_manager: IoManager, shutdown: Arc<AtomicBool>) -> Self {
        Vcpu {
            vcpu_fd,
            io_manager,
            shutdown,
        }
    }

    pub fn vcpu_fd(&self) -> &VcpuFd {
        &self.vcpu_fd
    }


    fn handle_io_out(&self, port: u16, data: &[u8]) {
        let _ok = self.io_manager.pio_write(port, data);
    }

    fn handle_io_in(&self, port: u16, data: &mut [u8]) {
        let _ok = self.io_manager.pio_read(port, data);
    }

    fn handle_mmio_read(&self, addr: u64, data: &mut [u8]) {
        let _ok = self.io_manager.mmio_read(addr, data);
    }

    fn handle_mmio_write(&self, addr: u64, data: &[u8]) {
        let _ok = self.io_manager.mmio_write(addr,data);
    }

    fn handle_shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    pub fn run(&self, barrier: &Arc<Barrier>) {
        barrier.wait();
        loop {
            match self.vcpu_fd.run() {
                Ok(VcpuExit::IoOut(port, data)) => self.handle_io_out(port, data),
                Ok(VcpuExit::IoIn(port, data)) => self.handle_io_in(port, data),
                Ok(VcpuExit::MmioRead(addr, data)) => self.handle_mmio_read(addr, data),
                Ok(VcpuExit::MmioWrite(addr, data)) => self.handle_mmio_write(addr, data),
                Ok(VcpuExit::Shutdown) => self.handle_shutdown(),
                Ok(exit) => {
                    println!("unhandled exit: {:?}", exit);
                },
                Err(err) => {
                    if err.errno() == libc::EAGAIN {}
                    else {
                        warn!("VCPU run() returned error: {}", err);
                        return;
                    }
                }
            }
            if self.shutdown.load(Ordering::Relaxed) {
                return;
            }
        }
    }
}