use std::fs::File;
use std::io;
use crate::util::BitSet;
use crate::disk::{Result, Error, SECTOR_SIZE, DiskImage};
use std::io::{Seek, SeekFrom};
use memfd::MemfdOptions;
use vm_memory::{ReadVolatile, VolatileSlice, WriteVolatile};

pub struct MemoryOverlay {
    memory: File,
    written_sectors: BitSet,
}

impl MemoryOverlay {
    pub fn new() -> Result<Self> {
        let memory = MemfdOptions::new()
            .allow_sealing(true)
            .create("disk-overlay-memfd")
            .map_err(Error::MemoryOverlayCreate)?;
        let memory = memory.into_file();
        let written_sectors = BitSet::new();
        Ok(MemoryOverlay { memory, written_sectors })
    }

    pub fn write_sectors(&mut self, start: u64, buffer: &VolatileSlice) -> Result<()> {
        let sector_count = buffer.len() / SECTOR_SIZE;
        let len = sector_count * SECTOR_SIZE;
        let seek_offset = SeekFrom::Start(start * SECTOR_SIZE as u64);

        self.memory.seek(seek_offset)
            .map_err(Error::DiskSeek)?;

        let slice = buffer.subslice(0, len)
            .expect("Out of bounds in MemoryOverlay::write_sectors()");

        self.memory.write_all_volatile(&slice)
            .map_err(io::Error::other)
            .map_err(Error::DiskWrite)?;

        for n in 0..sector_count {
            let idx = start as usize + n;
            self.written_sectors.insert(idx);
        }
        Ok(())
    }

    pub fn read_sectors<D: DiskImage>(&mut self, disk: &mut D, start: u64, buffer: &mut VolatileSlice) -> Result<()> {
        let sector_count = buffer.len() / SECTOR_SIZE;
        if (0..sector_count).all(|i| !self.written_sectors.get(i)) {
            return disk.read_sectors(start, buffer);
        }

        for n in 0..sector_count {
            let sector = start + n as u64;
            let offset = n * SECTOR_SIZE;
            let mut sector_buffer = buffer.subslice(offset, SECTOR_SIZE)
                .expect("Out of bounds in MemoryOverlay::read_sectors()");
            if self.written_sectors.get(sector as usize) {
                self.read_single_sector(sector, &mut sector_buffer)?;
            } else {
                disk.read_sectors(sector, &mut sector_buffer)?;
            }
        }
        Ok(())
    }

    fn read_single_sector(&mut self, sector: u64, buffer: &mut VolatileSlice) -> Result<()> {
        assert_eq!(buffer.len(), SECTOR_SIZE);
        let offset = SeekFrom::Start(sector * SECTOR_SIZE as u64);
        self.memory.seek(offset)
            .map_err(Error::DiskSeek)?;
        self.memory.read_exact_volatile(buffer).map_err(io::Error::other)
            .map_err(Error::DiskRead)?;
        Ok(())
    }

}