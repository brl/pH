use std::convert::TryInto;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool,Ordering};
use kvm_ioctls::{VcpuExit, VcpuFd};
use vmm_sys_util::sock_ctrl_msg::IntoIovec;
use crate::vm::io::IoDispatcher;


/*
pub enum VcpuEvent {
    Exit,
}

pub struct VcpuHandle {
    sender: Sender<VcpuEvent>,
    thread: thread::JoinHandle<()>,
}

 */

pub struct Vcpu {
    vcpu_fd: VcpuFd,
    io: Arc<IoDispatcher>,
    shutdown: Arc<AtomicBool>,
}


impl Vcpu {
    pub fn new(vcpu_fd: VcpuFd, io: Arc<IoDispatcher>, shutdown: Arc<AtomicBool>) -> Self {
        Vcpu {
            vcpu_fd, io, shutdown,
        }
    }

    pub fn vcpu_fd(&self) -> &VcpuFd {
        &self.vcpu_fd
    }

    fn data_to_int(data: &[u8]) -> u64 {
        match data.len() {
            1 =>  data[0] as u64,
            2 =>  u16::from_le_bytes(data.try_into().unwrap()) as u64,
            4 =>  u32::from_le_bytes(data.try_into().unwrap()) as u64,
            8 =>  u64::from_le_bytes(data.try_into().unwrap()),
            _ => 0,
        }
    }

    fn int_to_data(n: u64, data: &mut[u8]) {
        match data.len() {
            1 => data[0] = n as u8,
            2 => data.copy_from_slice((n as u16).to_le_bytes().as_slice()),
            4 => data.copy_from_slice((n as u32).to_le_bytes().as_slice()),
            8 => data.copy_from_slice((n as u64).to_le_bytes().as_slice()),
            _ => {},
        }
    }

    fn handle_io_out(&self, port: u16, data: &[u8]) {
        let val = Self::data_to_int(data) as u32;
        self.io.emulate_io_out(port, data.size(), val);
    }

    fn handle_io_in(&self, port: u16, data: &mut [u8]) {
        let val = self.io.emulate_io_in(port, data.len());
        Self::int_to_data(val as u64, data);
    }

    fn handle_mmio_read(&self, addr: u64, data: &mut [u8]) {
        let val = self.io.emulate_mmio_read(addr, data.len());
        Self::int_to_data(val, data);
    }

    fn handle_mmio_write(&self, addr: u64, data: &[u8]) {
        let val = Self::data_to_int(data);
        self.io.emulate_mmio_write(addr, data.size(), val);
    }

    fn handle_shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    pub fn run(&self) {
        loop {
            match self.vcpu_fd.run().expect("fail") {
                VcpuExit::IoOut(port, data) => self.handle_io_out(port, data),
                VcpuExit::IoIn(port, data) => self.handle_io_in(port, data),
                VcpuExit::MmioRead(addr, data) => self.handle_mmio_read(addr, data),
                VcpuExit::MmioWrite(addr, data) => self.handle_mmio_write(addr, data),
                VcpuExit::Shutdown => self.handle_shutdown(),
                exit => {
                    println!("unhandled exit: {:?}", exit);
                }
            }
            if self.shutdown.load(Ordering::Relaxed) {
                return;
            }
        }
    }
}