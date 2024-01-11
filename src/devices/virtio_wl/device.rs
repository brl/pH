use std::os::unix::io::{AsRawFd, RawFd};
use std::thread;

use crate::system;
use crate::system::EPoll;
use crate::system::drm::DrmDescriptor;

use crate::devices::virtio_wl::{vfd::VfdManager, consts::*, Error, Result, VfdObject};
use crate::system::ioctl::ioctl_with_ref;
use std::os::raw::{c_ulong, c_uint, c_ulonglong};
use vmm_sys_util::eventfd::EventFd;
use crate::io::{Chain, FeatureBits, Queues, VirtioDevice, VirtioDeviceType, VirtQueue};
use crate::io::shm_mapper::DeviceSharedMemoryManager;

#[repr(C)]
struct dma_buf_sync {
    flags: c_ulonglong,
}
const DMA_BUF_IOCTL_BASE: c_uint = 0x62;
const DMA_BUF_IOCTL_SYNC: c_ulong = iow!(DMA_BUF_IOCTL_BASE, 0, ::std::mem::size_of::<dma_buf_sync>() as i32);

pub struct VirtioWayland {
    dev_shm_manager: Option<DeviceSharedMemoryManager>,
    features: FeatureBits,
    enable_dmabuf: bool,
}

impl VirtioWayland {
    pub fn new(enable_dmabuf: bool , dev_shm_manager: DeviceSharedMemoryManager) -> Self {
        let features = FeatureBits::new_default(VIRTIO_WL_F_TRANS_FLAGS as u64);
        VirtioWayland {
            dev_shm_manager: Some(dev_shm_manager),
            features,
            enable_dmabuf
        }
    }

    fn transition_flags(&self) -> bool {
        self.features.has_guest_bit(VIRTIO_WL_F_TRANS_FLAGS as u64)
    }

    fn create_device(in_vq: VirtQueue, out_vq: VirtQueue, transition: bool, enable_dmabuf: bool, dev_shm_manager: DeviceSharedMemoryManager) -> Result<WaylandDevice> {
        let kill_evt = EventFd::new(0).map_err(Error::EventFdCreate)?;
        let dev = WaylandDevice::new(in_vq, out_vq, kill_evt, transition, enable_dmabuf, dev_shm_manager)?;
        Ok(dev)
    }
}

impl VirtioDevice for VirtioWayland {
    fn features(&self) -> &FeatureBits {
        &self.features
    }

    fn queue_sizes(&self) -> &[u16] {
        &[VirtQueue::DEFAULT_QUEUE_SIZE, VirtQueue::DEFAULT_QUEUE_SIZE]
    }

    fn device_type(&self) -> VirtioDeviceType {
        VirtioDeviceType::Wl
    }

    fn start(&mut self, queues: &Queues) {
        thread::spawn({
            let transition = self.transition_flags();
            let enable_dmabuf = self.enable_dmabuf;
            let dev_shm_manager = self.dev_shm_manager.take().expect("No dev_shm_manager");
            let in_vq = queues.get_queue(0);
            let out_vq = queues.get_queue(1);
            move || {
                let mut dev = match Self::create_device(in_vq, out_vq,transition, enable_dmabuf, dev_shm_manager) {
                    Err(e) => {
                        warn!("Error creating virtio wayland device: {}", e);
                        return;
                    }
                    Ok(dev) => dev,
                };
                if let Err(e) = dev.run() {
                    warn!("Error running virtio-wl device: {}", e);
                };
            }
        });
    }
}

struct WaylandDevice {
    vfd_manager: VfdManager,
    out_vq: VirtQueue,
    kill_evt: EventFd,
    enable_dmabuf: bool,
}

impl WaylandDevice {
    const IN_VQ_TOKEN: u64 = 0;
    const OUT_VQ_TOKEN:u64 = 1;
    const KILL_TOKEN: u64 = 2;
    const VFDS_TOKEN: u64 = 3;

    fn new(in_vq: VirtQueue, out_vq: VirtQueue, kill_evt: EventFd, use_transition: bool, enable_dmabuf: bool, dev_shm_manager: DeviceSharedMemoryManager) -> Result<Self> {
        let vfd_manager = VfdManager::new(dev_shm_manager, use_transition, in_vq, "/run/user/1000/wayland-0")?;

        Ok(WaylandDevice {
            vfd_manager,
            out_vq,
            kill_evt,
            enable_dmabuf,
        })
    }

    pub fn get_vfd(&self, vfd_id: u32) -> Option<&dyn VfdObject> {
        self.vfd_manager.get_vfd(vfd_id)
    }

    pub fn get_mut_vfd(&mut self, vfd_id: u32) -> Option<&mut dyn VfdObject> {
        self.vfd_manager.get_mut_vfd(vfd_id)
    }

    fn setup_poll(&mut self) -> system::Result<EPoll> {
        let poll = EPoll::new()?;
        poll.add_read(self.vfd_manager.in_vq_poll_fd(), Self::IN_VQ_TOKEN as u64)?;
        poll.add_read(self.out_vq.ioevent().as_raw_fd(), Self::OUT_VQ_TOKEN as u64)?;
        poll.add_read(self.kill_evt.as_raw_fd(), Self::KILL_TOKEN as u64)?;
        poll.add_read(self.vfd_manager.poll_fd(), Self::VFDS_TOKEN as u64)?;
        Ok(poll)
    }
    fn run(&mut self) -> Result<()> {
        let mut poll = self.setup_poll().map_err(Error::FailedPollContextCreate)?;

        'poll: loop {
            let events = match poll.wait() {
                Ok(v) => v,
                Err(e) => {
                    warn!("virtio_wl: error waiting for poll events: {}", e);
                    break;
                }
            };
            for ev in events.iter() {
                match ev.id() {
                    Self::IN_VQ_TOKEN => {
                        self.vfd_manager.in_vq_ready()?;
                    },
                    Self::OUT_VQ_TOKEN => {
                        self.out_vq.ioevent().read().map_err(Error::IoEventError)?;
                        if let Some(chain) = self.out_vq.next_chain() {
                            let mut handler = MessageHandler::new(self, chain, self.enable_dmabuf);
                            match handler.run() {
                                Ok(()) => {
                                },
                                Err(err) => {
                                    warn!("virtio_wl: error handling request: {}", err);
                                    if !handler.responded {
                                        let _ = handler.send_err();
                                    }
                                },
                            }
                            handler.chain.flush_chain();
                        }
                    },
                    Self::KILL_TOKEN => break 'poll,
                    Self::VFDS_TOKEN => self.vfd_manager.process_poll_events(),
                    _ =>  warn!("virtio_wl: unexpected poll token value"),
                }
            };
        }
        Ok(())
    }
}

struct MessageHandler<'a> {
    device: &'a mut WaylandDevice,
    chain: Chain,
    responded: bool,
    enable_dmabuf: bool,
}

impl <'a> MessageHandler<'a> {

    fn new(device: &'a mut WaylandDevice, chain: Chain, enable_dmabuf: bool) -> Self {
        MessageHandler { device, chain, responded: false, enable_dmabuf }
    }

    fn run(&mut self) -> Result<()> {
        let msg_type = self.chain.r32()?;
        // Flags are always zero
        let _flags = self.chain.r32()?;

        match msg_type {
            VIRTIO_WL_CMD_VFD_NEW => self.cmd_new_alloc(),
            VIRTIO_WL_CMD_VFD_CLOSE => self.cmd_close(),
            VIRTIO_WL_CMD_VFD_SEND => self.cmd_send(),
            VIRTIO_WL_CMD_VFD_NEW_DMABUF  if self.enable_dmabuf => self.cmd_new_dmabuf(),
            VIRTIO_WL_CMD_VFD_DMABUF_SYNC if self.enable_dmabuf => self.cmd_dmabuf_sync(),
            VIRTIO_WL_CMD_VFD_NEW_CTX => self.cmd_new_ctx(),
            VIRTIO_WL_CMD_VFD_NEW_PIPE => self.cmd_new_pipe(),
            v => {
                self.send_invalid_command()?;
                if v == VIRTIO_WL_CMD_VFD_NEW_DMABUF && !self.enable_dmabuf {
                    // Sommelier probes this command to determine if dmabuf is supported
                    // so if dmabuf is not enabled don't throw an error.
                    Ok(())
                } else {
                    Err(Error::UnexpectedCommand(v))
                }
            },
        }
    }

    fn cmd_new_alloc(&mut self) -> Result<()> {
        let id = self.chain.r32()?;
        let flags = self.chain.r32()?;
        let _pfn = self.chain.r64()?;
        let size = self.chain.r32()?;

        match self.device.vfd_manager.create_shm(id, size) {
            Ok((pfn,size)) => self.resp_vfd_new(id, flags, pfn, size as u32),
            Err(Error::ShmAllocFailed(_)) => self.send_simple_resp(VIRTIO_WL_RESP_OUT_OF_MEMORY),
            Err(e) => Err(e),
        }
    }

    fn resp_vfd_new(&mut self, id: u32, flags: u32, pfn: u64, size: u32) -> Result<()> {
        self.chain.w32(VIRTIO_WL_RESP_VFD_NEW)?;
        self.chain.w32(0)?;
        self.chain.w32(id)?;
        self.chain.w32(flags)?;
        self.chain.w64(pfn)?;
        self.chain.w32(size)?;
        self.responded = true;
        Ok(())
    }

    fn cmd_new_dmabuf(&mut self) -> Result<()> {
        let id = self.chain.r32()?;
        let _flags = self.chain.r32()?;
        let _pfn = self.chain.r64()?;
        let _size = self.chain.r32()?;
        let width = self.chain.r32()?;
        let height = self.chain.r32()?;
        let format = self.chain.r32()?;

        match self.device.vfd_manager.create_dmabuf(id, width,height, format) {
            Ok((pfn, size, desc)) => self.resp_dmabuf_new(id, pfn, size as u32, desc),
            Err(e) => {
                if !(height == 0 && width == 0) {
                    warn!("virtio_wl: Failed to create dmabuf: {}", e);
                }
                self.responded = true;
                self.send_err()
            }
        }
    }

    fn resp_dmabuf_new(&mut self, id: u32, pfn: u64, size: u32, desc: DrmDescriptor) -> Result<()> {
        self.chain.w32(VIRTIO_WL_RESP_VFD_NEW_DMABUF)?;
        self.chain.w32(0)?;
        self.chain.w32(id)?;
        self.chain.w32(0)?;
        self.chain.w64(pfn)?;
        self.chain.w32(size)?;
        self.chain.w32(0)?;
        self.chain.w32(0)?;
        self.chain.w32(0)?;
        self.chain.w32(desc.planes[0].stride)?;
        self.chain.w32(desc.planes[1].stride)?;
        self.chain.w32(desc.planes[2].stride)?;
        self.chain.w32(desc.planes[0].offset)?;
        self.chain.w32(desc.planes[1].offset)?;
        self.chain.w32(desc.planes[2].offset)?;
        self.responded = true;
        Ok(())
    }

    fn cmd_dmabuf_sync(&mut self) -> Result<()> {
        let id = self.chain.r32()?;
        let flags = self.chain.r32()?;

        let vfd = match self.device.get_mut_vfd(id) {
            Some(vfd) => vfd,
            None => return self.send_invalid_id(),
        };
        let fd = match vfd.send_fd() {
            Some(fd) => fd,
            None => return self.send_invalid_id(),
        };

        unsafe {
            let sync = dma_buf_sync {
                flags: flags as u64,
            };
            ioctl_with_ref(fd, DMA_BUF_IOCTL_SYNC, &sync).map_err(Error::DmaSync)?;
        }

        self.send_ok()
    }

    fn cmd_close(&mut self) -> Result<()> {
        let id = self.chain.r32()?;
        self.device.vfd_manager.close_vfd(id)?;
        self.send_ok()
    }

    fn cmd_send(&mut self) -> Result<()> {
        let id = self.chain.r32()?;

        let send_fds = self.read_vfd_ids()?;
        let data = self.chain.current_read_slice();

        let vfd = match self.device.get_mut_vfd(id) {
            Some(vfd) => vfd,
            None => return self.send_invalid_id(),
        };

        if let Some(fds) = send_fds.as_ref() {
            vfd.send_with_fds(&data, fds)?;
        } else {
            vfd.send(&data)?;
        }
        self.send_ok()
    }

    fn read_vfd_ids(&mut self) -> Result<Option<Vec<RawFd>>> {
        let vfd_count = self.chain.r32()? as usize;
        if vfd_count > VIRTWL_SEND_MAX_ALLOCS {
            return Err(Error::TooManySendVfds(vfd_count))
        }
        if vfd_count == 0 {
            return Ok(None);
        }

        let mut raw_fds = Vec::with_capacity(vfd_count);
        for _ in 0..vfd_count {
            let vfd_id = self.chain.r32()?;
            if let Some(fd) = self.vfd_id_to_raw_fd(vfd_id)? {
                raw_fds.push(fd);
            }
        }
        Ok(Some(raw_fds))
    }

    fn vfd_id_to_raw_fd(&mut self, vfd_id: u32) -> Result<Option<RawFd>> {
        let vfd = match self.device.get_vfd(vfd_id) {
            Some(vfd) => vfd,
            None => {
                warn!("virtio_wl: Received unexpected vfd id 0x{:08x}", vfd_id);
                return Ok(None);
            }
        };

        if let Some(fd) = vfd.send_fd() {
            Ok(Some(fd))
        } else {
            self.send_invalid_type()?;
            Err(Error::InvalidSendVfd)
        }
    }

    fn cmd_new_ctx(&mut self) -> Result<()> {
        let id = self.chain.r32()?;
        if !Self::is_valid_id(id) {
            return self.send_invalid_id();
        }
        let flags = self.device.vfd_manager.create_socket(id)?;
        self.resp_vfd_new(id, flags, 0, 0)?;
        Ok(())
    }

    fn cmd_new_pipe(&mut self) -> Result<()> {
        let id = self.chain.r32()?;
        let flags = self.chain.r32()?;

        if !Self::is_valid_id(id) {
            return self.send_invalid_id();
        }
        if !Self::valid_new_pipe_flags(flags) {
            notify!("invalid flags: 0x{:08}", flags);
            return self.send_invalid_flags();
        }

        let is_write = Self::is_flag_set(flags, VIRTIO_WL_VFD_WRITE);

        self.device.vfd_manager.create_pipe(id, is_write)?;

        self.resp_vfd_new(id, 0, 0, 0)
    }

    fn valid_new_pipe_flags(flags: u32) -> bool {
        // only VFD_READ and VFD_WRITE may be set
        if flags & !(VIRTIO_WL_VFD_WRITE|VIRTIO_WL_VFD_READ) != 0 {
            return false;
        }
        let read = Self::is_flag_set(flags, VIRTIO_WL_VFD_READ);
        let write = Self::is_flag_set(flags, VIRTIO_WL_VFD_WRITE);
        // exactly one of them must be set
        !(read && write) && (read || write)
    }

    fn is_valid_id(id: u32) -> bool {
        id & VFD_ID_HOST_MASK == 0
    }

    fn is_flag_set(flags: u32, bit: u32) -> bool {
        flags & bit != 0
    }

    fn send_invalid_flags(&mut self) -> Result<()> {
        self.send_simple_resp(VIRTIO_WL_RESP_INVALID_FLAGS)
    }

    fn send_invalid_id(&mut self) -> Result<()> {
        self.send_simple_resp(VIRTIO_WL_RESP_INVALID_ID)
    }

    fn send_invalid_type(&mut self) -> Result<()> {
        self.send_simple_resp(VIRTIO_WL_RESP_INVALID_TYPE)
    }

    fn send_invalid_command(&mut self) -> Result<()> {
        self.send_simple_resp(VIRTIO_WL_RESP_INVALID_CMD)
    }

    fn send_ok(&mut self) -> Result<()> {
        self.send_simple_resp(VIRTIO_WL_RESP_OK)
    }

    fn send_err(&mut self) -> Result<()> {
        self.send_simple_resp(VIRTIO_WL_RESP_ERR)
    }

    fn send_simple_resp(&mut self, code: u32) -> Result<()> {
        self.chain.w32(code)?;
        self.responded = true;
        Ok(())
    }
}
