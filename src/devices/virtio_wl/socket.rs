use std::fs::File;
use std::io;
use std::os::fd::FromRawFd;
use std::path::Path;
use std::os::unix::{net::UnixStream, io::{AsRawFd, RawFd}};
use vm_memory::{VolatileSlice, WriteVolatile};

use crate::system::ScmSocket;
use crate::devices::virtio_wl::{consts:: *, Error, Result, VfdObject, VfdRecv};

pub struct VfdSocket {
    vfd_id: u32,
    flags: u32,
    socket: Option<UnixStream>,
}

impl VfdSocket {
    pub fn open<P: AsRef<Path>>(vfd_id: u32, transition_flags: bool, path: P) -> Result<Self> {
        let flags = if transition_flags {
            VIRTIO_WL_VFD_READ | VIRTIO_WL_VFD_WRITE
        } else {
            VIRTIO_WL_VFD_CONTROL
        };
        let socket = UnixStream::connect(path)
            .map_err(Error::SocketConnect)?;
        socket.set_nonblocking(true)
            .map_err(Error::SocketConnect)?;

        Ok(VfdSocket{
            vfd_id,
            flags,
            socket: Some(socket),
        })
    }
    fn socket_recv(socket: &mut UnixStream) -> Result<(Vec<u8>, Vec<File>)> {
        let mut buf = vec![0; IN_BUFFER_LEN];
        let mut fd_buf = [0; VIRTWL_SEND_MAX_ALLOCS];
        let (len, fd_len) = socket.recv_with_fds(&mut buf, &mut fd_buf)
            .map_err(Error::SocketReceive)?;
        buf.truncate(len);
        let files = fd_buf[..fd_len].iter()
            .map(|&fd| unsafe {
                File::from_raw_fd(fd)
            }).collect();
        Ok((buf, files))
    }
}
impl VfdObject for VfdSocket {
    fn id(&self) -> u32 {
        self.vfd_id
    }

    fn send_fd(&self) -> Option<RawFd> {
        self.socket.as_ref().map(|s| s.as_raw_fd())
    }

    fn poll_fd(&self) -> Option<RawFd> {
        self.socket.as_ref().map(|s| s.as_raw_fd())
    }

    fn recv(&mut self) -> Result<Option<VfdRecv>> {
        if let Some(mut sock) = self.socket.take() {
            let (buf,files) = Self::socket_recv(&mut sock)?;
            if !(buf.is_empty() && files.is_empty()) {
                self.socket.replace(sock);
                return if files.is_empty() {
                    Ok(Some(VfdRecv::new(buf)))
                } else {
                    Ok(Some(VfdRecv::new_with_fds(buf, files)))
                }
            }
        }
        Ok(None)
    }

    fn send(&mut self, data: &VolatileSlice) -> Result<()> {
        if let Some(s) = self.socket.as_mut() {
            s.write_all_volatile(data).map_err(Error::VolatileSendVfd)
        } else {
            Err(Error::InvalidSendVfd)
        }
    }

    fn send_with_fds(&mut self, data: &VolatileSlice, fds: &[RawFd]) -> Result<()> {
        if let Some(s) = self.socket.as_mut() {
            let mut buffer = vec![0u8; data.len()];
            data.copy_to(&mut buffer);
            s.send_with_fds(&buffer, fds)
                .map_err(|_| Error::SendVfd(io::Error::last_os_error()))?;
            Ok(())
        } else {
            Err(Error::InvalidSendVfd)
        }
    }

    fn flags(&self) -> u32 {
        if self.socket.is_some() {
            self.flags
        } else {
            0
        }
    }
    fn close(&mut self) -> Result<()> {
        self.socket = None;
        Ok(())
    }
}