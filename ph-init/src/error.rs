use std::{result, io};
use crate::netlink;
use thiserror::Error;

#[derive(Debug,Error)]
pub enum Error {
    #[error("not running as pid 1")]
    Pid1,
    #[error("failed to load kernel command line from /proc/cmdline: {0}")]
    KernelCmdLine(io::Error),
    #[error("Cannot mount rootfs because no phinit.root is set")]
    NoRootVar,
    #[error("Cannot mount rootfs because no phinit.rootfs is set")]
    NoRootFsVar,
    #[error("Failed to mount rootfs {0}: {1}")]
    RootFsMount(String, io::Error),
    #[error("unable to mount procfs: {0}")]
    MountProcFS(io::Error),
    #[error("failed to mount tmpfs at {0}: {1}")]
    MountTmpFS(String, io::Error),
    #[error("failed to mount sysfs at /sys: {0}")]
    MountSysFS(io::Error),
    #[error("failed to mount cgroup at /sys/fs/cgroup: {0}")]
    MountCGroup(io::Error),
    #[error("failed to mount devtmpfs at /dev: {0}")]
    MountDevTmpFS(io::Error),
    #[error("failed to mount /dev/pts: {0}")]
    MountDevPts(io::Error),
    #[error("failed to mount overlayfs: {0}")]
    MountOverlay(io::Error),
    #[error("failed to move mount from {0} to {1}: {2}")]
    MoveMount(String, String, io::Error),
    #[error("failed to mount 9p volume {0} at {1}: {2}")]
    Mount9P(String, String, io::Error),
    #[error("failed to unmount {0}: {1}")]
    Umount(String, io::Error),
    #[error("failed to mkdir {0}: {1}")]
    MkDir(String, io::Error),
    #[error("sethostname() failed: {0}")]
    SetHostname(io::Error),
    #[error("call to setsid() failed: {0}")]
    SetSid(io::Error),
    #[error("failed to set controlling terminal: {0}")]
    SetControllingTty(io::Error),
    #[error("failed to pivot_root({0}, {1}): {2}")]
    PivotRoot(String, String, io::Error),
    #[error("failed to waitpid(): {0}")]
    WaitPid(io::Error),
    #[error("failed to write /etc/hosts: {0}")]
    WriteEtcHosts(io::Error),
    #[error("error launching shell: {0}")]
    RunShell(io::Error),
    #[error("failed to create CString")]
    CStringConv,
    #[error("failed to chmod: {0}")]
    ChmodFailed(io::Error),
    #[error("failed to chown: {0}")]
    ChownFailed(io::Error),
    #[error("unable to execute {0}: {1}")]
    LaunchFailed(String, io::Error),
    #[error("could not reboot system: {0}")]
    RebootFailed(io::Error),
    #[error("failed to open log file: {0}")]
    OpenLogFailed(io::Error),
    #[error("error creating .Xauthority file: {0}")]
    XAuthFail(io::Error),
    #[error("error writing bashrc file: {0}")]
    WriteBashrc(io::Error),
    #[error("error configuring network: {0}")]
    NetworkConfigure(netlink::Error),
}

pub type Result<T> = result::Result<T, Error>;