use crate::system;
use crate::system::ErrnoError;
use std::result;
use kvm_ioctls::Cap;
use thiserror::Error;
use vm_memory::guest_memory;

#[derive(Debug,Error)]
pub enum Error {
    #[error("failed to create memory manager: {0}")]
    MemoryManagerCreate(vm_memory::Error),
    #[error("failed to register memory region: {0}")]
    MemoryRegister(kvm_ioctls::Error),
    #[error("failed to create memory region: {0}")]
    MemoryRegionCreate(system::Error),
    #[error("error loading kernel: {0}")]
    LoadKernel(system::Error),
    #[error("{0}")]
    KvmError(kvm_ioctls::Error),
    #[error("kernel does not support a required kvm extension: {0:?}")]
    KvmMissingExtension(Cap),
    #[error("{0}")]
    SystemError(system::Error),
    #[error("failed to call {0} ioctl: {1}")]
    IoctlError(&'static str, ErrnoError),
    #[error("error setting up vm: {0}")]
    SetupError(kvm_ioctls::Error),
    #[error("guest memory error: {0}")]
    GuestMemory(guest_memory::Error),
}

pub type Result<T> = result::Result<T, Error>;
