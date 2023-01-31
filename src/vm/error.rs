use std::{result, io};
use kvm_ioctls::Cap;
use crate::{system, virtio};
use crate::system::netlink;
use crate::vm::arch;

use thiserror::Error;
pub type Result<T> = result::Result<T, Error>;

#[derive(Error,Debug)]
pub enum Error {
    #[error("failed to open kvm instance: {0}")]
    KvmOpenError(kvm_ioctls::Error),
    #[error("failed to create VM file descriptor: {0}")]
    VmFdOpenError(kvm_ioctls::Error),
    #[error("error on KVM operation: {0}")]
    KvmError(kvm_ioctls::Error),
    #[error("unexpected KVM version")]
    BadVersion,
    #[error("kernel does not support a required kvm extension: {0:?}")]
    MissingRequiredExtension(Cap),
    #[error("error configuring VM: {0}")]
    VmSetup(kvm_ioctls::Error),
    #[error("memory mapping failed: {0}")]
    MappingFailed(system::Error),
    #[error("error reading/restoring terminal state: {0}")]
    TerminalTermios(io::Error),
    #[error("i/o error: {0}")]
    IoError(#[from] io::Error),
    #[error("{0}")]
    ArchError(arch::Error),
    #[error("error setting up network: {0}")]
    NetworkSetup(#[from] netlink::Error),
    #[error("setting up boot fs failed: {0}")]
    SetupBootFs(io::Error),
    #[error("setting up virtio devices failed: {0}")]
    SetupVirtio(virtio::Error),
    #[error("failed to create Vcpu: {0}")]
    CreateVcpu(kvm_ioctls::Error),
}