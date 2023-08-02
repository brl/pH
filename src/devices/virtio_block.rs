use std::io::Write;
use std::{result, io, thread};

use crate::disk;
use crate::disk::DiskImage;

use thiserror::Error;
use crate::io::{Chain, FeatureBits, Queues, VirtioDevice, VirtioDeviceType, VirtioError, VirtQueue};
use crate::io::virtio::DeviceConfigArea;

const VIRTIO_BLK_F_RO: u64 = 1 << 5;
const VIRTIO_BLK_F_BLK_SIZE: u64 = 1 << 6;
const VIRTIO_BLK_F_FLUSH: u64 = 1 << 9;
const VIRTIO_BLK_F_SEG_MAX: u64 = 1 << 2;

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_FLUSH: u32 = 4;
const VIRTIO_BLK_T_GET_ID: u32 = 8;

const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_BLK_S_IOERR: u8 = 1;
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

const SECTOR_SHIFT: usize = 9;
const SECTOR_SIZE: usize = 1 << SECTOR_SHIFT;

const QUEUE_SIZE: usize = 256;

#[derive(Debug,Error)]
enum Error {
    #[error("i/o error on virtio chain operation: {0}")]
    IoChainError(#[from] io::Error),
    #[error("error reading disk image: {0}")]
    DiskRead(disk::Error),
    #[error("error writing disk image: {0}")]
    DiskWrite(disk::Error),
    #[error("error flushing disk image: {0}")]
    DiskFlush(disk::Error),
    #[error("error waiting on virtqueue: {0}")]
    VirtQueueWait(VirtioError),
    #[error("virtqueue read descriptor size ({0}) is invalid. Not a multiple of sector size")]
    InvalidReadDescriptor(usize),
}

type Result<T> = result::Result<T, Error>;

pub struct VirtioBlock<D: DiskImage+'static> {
    disk_image: Option<D>,
    config: DeviceConfigArea,
    features: FeatureBits,
}

const HEADER_SIZE: usize = 16;

const CAPACITY_OFFSET: usize = 0;
const SEG_MAX_OFFSET: usize = 12;
const BLK_SIZE_OFFSET: usize = 20;
const CONFIG_SIZE: usize = 24;
impl <D: DiskImage + 'static> VirtioBlock<D> {

    pub fn new(disk_image: D) -> Self {
        let mut config = DeviceConfigArea::new(CONFIG_SIZE);
        config.write_u64(CAPACITY_OFFSET, disk_image.sector_count());
        config.write_u32(SEG_MAX_OFFSET, QUEUE_SIZE as u32 - 2);
        config.write_u32(BLK_SIZE_OFFSET, 1024);
        let features = FeatureBits::new_default( VIRTIO_BLK_F_FLUSH |
                VIRTIO_BLK_F_BLK_SIZE |
                VIRTIO_BLK_F_SEG_MAX  |
                if disk_image.read_only() {
                    VIRTIO_BLK_F_RO
                } else {
                    0
                }
        );
        VirtioBlock {
            disk_image: Some(disk_image),
            config,
            features,
        }
    }
}

impl <D: DiskImage> VirtioDevice for VirtioBlock<D> {
    fn features(&self) -> &FeatureBits {
        &self.features
    }

    fn queue_sizes(&self) -> &[u16] {
        &[QUEUE_SIZE as u16]
    }

    fn device_type(&self) -> VirtioDeviceType {
        VirtioDeviceType::Block
    }

    fn config_size(&self) -> usize {
        CONFIG_SIZE
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        self.config.read_config(offset, data);
    }

    fn write_config(&mut self, offset: u64, data: &[u8]) {
        self.config.write_config(offset, data);
    }

    fn start(&mut self, queues: &Queues) {
        let vq = queues.get_queue(0);

        let mut disk = self.disk_image.take().expect("No disk image?");
        if let Err(err) = disk.open() {
            warn!("Unable to start virtio-block device: {}", err);
            return;
        }
        let mut dev = VirtioBlockDevice::new(vq, disk);
        thread::spawn(move || {
            if let Err(err) = dev.run() {
                warn!("Error running virtio block device: {}", err);
            }
        });
    }
}

struct VirtioBlockDevice<D: DiskImage> {
    vq: VirtQueue,
    disk: D,
}

impl <D: DiskImage> VirtioBlockDevice<D> {
    fn new(vq: VirtQueue, disk: D) -> Self {
        VirtioBlockDevice { vq, disk }
    }

    fn run(&mut self) -> Result<()> {
        loop {
            let mut chain = self.vq.wait_next_chain()
                .map_err(Error::VirtQueueWait)?;

            while chain.remaining_read() >= HEADER_SIZE {
                match MessageHandler::read_header(&mut self.disk, &mut chain) {
                    Ok(mut handler) => handler.process_message(),
                    Err(e) => {
                        warn!("Error handling virtio_block message: {}", e);
                    }
                }
            }
        }
    }
}

struct MessageHandler<'a,'b, D: DiskImage> {
    disk: &'a mut D,
    chain: &'b mut Chain,
    msg_type: u32,
    sector: u64,
}

impl <'a,'b, D: DiskImage> MessageHandler<'a,'b, D> {

    fn read_header(disk: &'a mut D, chain: &'b mut Chain) -> Result<Self> {
        let msg_type = chain.r32()?;
        let _ = chain.r32()?;
        let sector = chain.r64()?;
        Ok(MessageHandler { disk, chain, msg_type, sector })
    }

    fn process_message(&mut self)  {
        let r = match self.msg_type {
            VIRTIO_BLK_T_IN => self.handle_io_in(),
            VIRTIO_BLK_T_OUT => self.handle_io_out(),
            VIRTIO_BLK_T_FLUSH => self.handle_io_flush(),
            VIRTIO_BLK_T_GET_ID => self.handle_get_id(),
            cmd => {
                warn!("virtio_block: unexpected command: {}", cmd);
                self.write_status(VIRTIO_BLK_S_UNSUPP);
                Ok(())
            },
        };
        self.process_result(r);
    }

    fn process_result(&mut self, result: Result<()>) {
        match result {
            Ok(()) => self.write_status(VIRTIO_BLK_S_OK),
            Err(e) => {
                warn!("virtio_block: disk error: {}", e);
                self.write_status(VIRTIO_BLK_S_IOERR);
            }
        }
    }

    fn handle_io_in(&mut self) -> Result<()> {
        loop {
            let current = self.chain.current_write_slice();
            let nsectors = current.len() >> SECTOR_SHIFT;
            if nsectors == 0 {
                return Ok(())
            }
            let len = nsectors << SECTOR_SHIFT;
            let buffer = &mut current[..len];

            self.disk.read_sectors(self.sector, buffer)
                .map_err(Error::DiskRead)?;
            self.chain.inc_write_offset(len);
            self.sector += nsectors as u64;
        }
    }

    fn handle_io_out(&mut self) -> Result<()> {
        loop {
            let current = self.chain.current_read_slice();
            if current.len() & (SECTOR_SIZE-1) != 0 {
                return Err(Error::InvalidReadDescriptor(current.len()));
            }
            let nsectors = current.len() >> SECTOR_SHIFT;
            if nsectors == 0 {
                return Ok(())
            }
            self.disk.write_sectors(self.sector, current)
                .map_err(Error::DiskWrite)?;

            self.chain.inc_read_offset(nsectors << SECTOR_SHIFT);
            self.sector += nsectors as u64;
        }
    }

    fn handle_io_flush(&mut self) -> Result<()> {
        self.disk.flush().map_err(Error::DiskFlush)
    }

    fn handle_get_id(&mut self) -> Result<()> {
        self.chain.write_all(self.disk.disk_image_id())?;
        Ok(())
    }

    fn write_status(&mut self, status: u8) {
        if let Err(e) = self.chain.w8(status) {
           warn!("Error writing block device status: {}", e);
        }
        self.chain.flush_chain();
    }
}