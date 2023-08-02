use std::{cmp, io};
use std::io::Read;
use crate::memory::GuestRam;

#[repr(u16)]
enum DescriptorFlag {
    Next = 1,
    Write = 2,
    Indirect = 4,
    PackedAvail = 1<<7,
    PackedUsed = 1<<15,
}

#[derive(Copy,Clone)]
pub struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    // 'next' field for split virtqueue, 'buffer_id' for packed virtqueue
    extra: u16,
}

impl Descriptor {

    pub fn new(address: u64, length: u32, flags: u16, extra: u16) -> Self {
        Descriptor {
            address, length, flags, extra
        }
    }

    pub fn length(&self) -> usize {
        self.length as usize
    }

    pub fn address(&self) -> u64 {
        self.address
    }

    ///
    /// Test if `flag` is set in `self.flags`
    ///
    fn has_flag(&self, flag: DescriptorFlag) -> bool {
        self.flags & (flag as u16) != 0
    }

    ///
    /// Is VRING_DESC_F_NEXT set in `self.flags`?
    ///
    pub fn has_next(&self) -> bool {
        self.has_flag(DescriptorFlag::Next)
    }

    pub fn next(&self) -> u16 {
        self.extra
    }

    ///
    /// Is VRING_DESC_F_WRITE set in `self.flags`?
    ///
    pub fn is_write(&self) -> bool {
        self.has_flag(DescriptorFlag::Write)
    }

    ///
    /// Is VRING_DESC_F_INDIRECT set in `self.flags`?
    ///
    pub fn is_indirect(&self) -> bool {
        self.has_flag(DescriptorFlag::Indirect)
    }


    pub fn remaining(&self, offset: usize) -> usize {
        if offset >= self.length as usize {
            0
        } else {
            self.length as usize - offset
        }
    }

    pub fn is_desc_avail(&self, wrap_counter: bool) -> bool {
        let used = self.has_flag(DescriptorFlag::PackedUsed);
        let avail = self.has_flag(DescriptorFlag::PackedAvail);
        (used != avail) && (avail == wrap_counter)
    }

    pub fn read_from(&self, memory: &GuestRam, offset: usize, buf: &mut[u8]) -> usize {
        let sz = cmp::min(buf.len(), self.remaining(offset));
        if sz > 0 {
            memory.read_bytes(self.address + offset as u64, &mut buf[..sz]).unwrap();
        }
        sz
    }

    pub fn write_to(&self, memory: &GuestRam, offset: usize, buf: &[u8]) -> usize {
        let sz = cmp::min(buf.len(), self.remaining(offset));
        if sz > 0 {
            memory.write_bytes(self.address + offset as u64, &buf[..sz]).unwrap();
        }
        sz
    }

    pub fn write_from_reader<R: Read+Sized>(&self, memory: &GuestRam, offset: usize, mut r: R, size: usize) -> io::Result<usize> {
        let sz = cmp::min(size, self.remaining(offset));
        if sz > 0 {
            let slice = memory.mut_slice(self.address + offset as u64, sz).unwrap();
            return r.read(slice);
        }
        Ok(0)
    }

}