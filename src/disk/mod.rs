use std::{io, result, cmp};
use std::fs::File;
use std::os::linux::fs::MetadataExt;
use std::io::{SeekFrom, Seek};

use crate::system;

mod realmfs;
mod raw;
mod memory;

pub use raw::RawDiskImage;
pub use realmfs::RealmFSImage;
use std::path::PathBuf;
use thiserror::Error;

const SECTOR_SIZE: usize = 512;

#[derive(Debug,PartialEq)]
pub enum OpenType {
    ReadOnly,
    ReadWrite,
    MemoryOverlay,
}

pub trait DiskImage: Sync+Send {
    fn open(&mut self) -> Result<()>;
    fn read_only(&self) -> bool;
    fn sector_count(&self) -> u64;
    fn disk_file(&mut self) -> Result<&mut File>;

    fn seek_to_sector(&mut self, sector: u64) -> Result<()> {
        if sector > self.sector_count() {
            return Err(Error::BadSectorOffset(sector));
        }
        let offset = SeekFrom::Start(sector * SECTOR_SIZE as u64);
        let file = self.disk_file()?;
        file.seek(offset)
            .map_err(Error::DiskSeek)?;
        Ok(())
    }
    fn write_sectors(&mut self, start_sector: u64, buffer: &[u8]) -> Result<()>;
    fn read_sectors(&mut self, start_sector: u64, buffer: &mut [u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()> { Ok(()) }

    fn disk_image_id(&self) -> &[u8];
}

fn generate_disk_image_id(disk_file: &File) -> Vec<u8> {
    const VIRTIO_BLK_ID_BYTES: usize = 20;
    let meta = match disk_file.metadata() {
        Ok(meta) => meta,
        Err(_) => return vec![0u8; VIRTIO_BLK_ID_BYTES]
    };
    let dev_id = format!("{}{}{}", meta.st_dev(), meta.st_rdev(), meta.st_ino());
    let bytes = dev_id.as_bytes();
    let len = cmp::min(bytes.len(), VIRTIO_BLK_ID_BYTES);
    Vec::from(&bytes[..len])
}

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug,Error)]
pub enum Error {
    #[error("attempted write to read-only device")]
    ReadOnly,
    #[error("disk image {0} does not exist")]
    ImageDoesntExit(PathBuf),
    #[error("failed to open disk image {0:?}: {1}")]
    DiskOpen(PathBuf,io::Error),
    #[error("failed to open disk image {0} because the file is too short")]
    DiskOpenTooShort(PathBuf),
    #[error("error reading from disk image: {0}")]
    DiskRead(io::Error),
    #[error("error writing to disk image: {0}")]
    DiskWrite(io::Error),
    #[error("error seeking to offset on disk image: {0}")]
    DiskSeek(io::Error),
    #[error("attempt to access invalid sector offset {0}")]
    BadSectorOffset(u64),
    #[error("failed to create memory overlay: {0}")]
    MemoryOverlayCreate(system::Error),
    #[error("disk not open")]
    NotOpen,
}