use crate::io::address::AddressRange;
use crate::io::pci::address::PciAddress;
use crate::io::pci::consts::{PCI_BAR0, PCI_BAR5, PCI_CACHE_LINE_SIZE, PCI_CAP_BASE_OFFSET, PCI_CAP_ID_VENDOR, PCI_CAPABILITY_LIST, PCI_CLASS_DEVICE, PCI_CLASS_REVISION, PCI_COMMAND, PCI_COMMAND_IO, PCI_COMMAND_MEMORY, PCI_DEVICE_ID, PCI_INTERRUPT_LINE, PCI_INTERRUPT_PIN, PCI_STATUS, PCI_STATUS_CAP_LIST, PCI_SUBSYSTEM_ID, PCI_VENDOR_ID};
use crate::io::pci::device::PciBar;
use crate::util::{ByteBuffer,Writeable};

const PCI_CONFIG_SPACE_SIZE: usize = 256;
const MAX_CAPABILITY_COUNT:usize  = 16; // arbitrary

pub struct PciCapability<'a> {
    config: &'a mut PciConfiguration,
    buffer: ByteBuffer<Vec<u8>>,
}

impl <'a> PciCapability<'a> {
    pub fn new_vendor_capability(config: &'a mut PciConfiguration) -> Self {
        let mut buffer = ByteBuffer::new_empty();
        buffer.write(PCI_CAP_ID_VENDOR);
        buffer.write(0u8);
        PciCapability { config, buffer }
    }

    pub fn write<V: Writeable>(&mut self, val: V) {
        self.buffer.write(val);
    }

    pub fn store(&mut self) {
        let offset = self.config.next_capability_offset;
        self.config.update_capability_chain(self.buffer.len());
        self.config.write_bytes(offset, self.buffer.as_ref());
    }

}

pub struct PciConfiguration {
    address: PciAddress,
    irq: u8,
    bytes: [u8; PCI_CONFIG_SPACE_SIZE],
    bar_write_masks: [u32; 6],
    next_capability_offset: usize,
}

impl PciConfiguration {
    pub fn new(irq: u8, vendor: u16, device: u16, class_id: u16) -> Self {
        let mut config = PciConfiguration {
            address: PciAddress::empty(),
            irq,
            bytes: [0; PCI_CONFIG_SPACE_SIZE],
            bar_write_masks: [0; 6],
            next_capability_offset: PCI_CAP_BASE_OFFSET,
        };

        config.buffer()
            .write_at(PCI_VENDOR_ID, vendor)
            .write_at(PCI_DEVICE_ID, device)
            .write_at(PCI_COMMAND, PCI_COMMAND_IO | PCI_COMMAND_MEMORY)
            .write_at(PCI_CLASS_REVISION, u8::from(1))
            .write_at(PCI_CLASS_DEVICE, class_id)
            .write_at(PCI_INTERRUPT_PIN, u8::from(1))
            .write_at(PCI_INTERRUPT_LINE, irq)
            .write_at(PCI_SUBSYSTEM_ID, 0x40u16);

        config
    }

    pub fn address(&self) -> PciAddress {
        self.address
    }

    pub fn set_address(&mut self, address: PciAddress) {
        self.address = address;
    }

    pub fn irq(&self) -> u8 {
        self.irq
    }

    fn buffer(&mut self) -> ByteBuffer<&mut[u8]> {
        ByteBuffer::from_bytes_mut(&mut self.bytes).little_endian()
    }

    fn write_bytes(&mut self, offset: usize, bytes: &[u8]) {
        (&mut self.bytes[offset..offset+bytes.len()])
            .copy_from_slice(bytes)
    }

    fn read_bytes(&self, offset: usize, bytes: &mut [u8]) {
        bytes.copy_from_slice(&self.bytes[offset..offset+bytes.len()]);
    }


    fn bar_mask(&self, offset: usize) -> Option<u32> {

        fn is_bar_offset(offset: usize) -> bool {
            offset >= PCI_BAR0 && offset < (PCI_BAR5 + 4)
        }

        fn bar_idx(offset: usize) -> usize {
            (offset - PCI_BAR0) / 4
        }

        if is_bar_offset(offset) {
            Some(self.bar_write_masks[bar_idx(offset)])
        } else {
            None
        }
    }

    fn write_masked_byte(&mut self, offset: usize, mask: u8, new_byte: u8) {
        let orig = self.bytes[offset];

        self.bytes[offset] = (orig & !mask) | (new_byte & mask);
    }

    fn write_bar(&mut self, offset: usize, data: &[u8]) {
        let mask_bytes = match self.bar_mask(offset) {
            Some(mask) if mask != 0 => mask.to_le_bytes(),
            _ => return,
        };
        let mod4 = offset % 4;
        let mask_bytes = &mask_bytes[mod4..];
        assert!(mask_bytes.len() >= data.len());


        for idx in 0..data.len() {
            self.write_masked_byte(offset + idx, mask_bytes[idx], data[idx])
        }
    }

    fn write_config(&mut self, offset: usize, data: &[u8]) {
        let size = data.len();
        match offset {
            PCI_COMMAND | PCI_STATUS if size == 2 => {
                self.write_bytes(offset, data)
            },
            PCI_CACHE_LINE_SIZE if size == 1 => {
                self.write_bytes(offset, data)
            },
            PCI_BAR0..=0x27 => {
                self.write_bar(offset, data)
            }, // bars
            _ => {},

        }
    }

    fn is_valid_access(offset: u64, size: usize) -> bool {
        fn check_aligned_range(offset: u64, size: usize) -> bool {
            let offset = offset as usize;
            offset + size <= PCI_CONFIG_SPACE_SIZE && offset % size == 0
        }

        match size {
            4 => check_aligned_range(offset, 4),
            2 => check_aligned_range(offset, 2),
            1 => check_aligned_range(offset, 1),
            _ => false,
        }

    }

    fn next_capability(&self, offset: usize) -> Option<usize> {
        fn is_valid_cap_offset(offset: usize) -> bool {
            offset < 254 && offset >= PCI_CAP_BASE_OFFSET
        }

        if is_valid_cap_offset(offset) {
            Some(self.bytes[offset + 1] as usize)
        } else {
            None
        }
    }

    fn update_next_capability_offset(&mut self, caplen: usize) {
        let aligned = (caplen + 3) & !3;
        self.next_capability_offset += aligned;
        assert!(self.next_capability_offset < PCI_CONFIG_SPACE_SIZE);
    }

    fn update_capability_chain(&mut self, caplen: usize)  {

        let next_offset = self.next_capability_offset as u8;
        self.update_next_capability_offset(caplen);

        let mut cap_ptr = self.bytes[PCI_CAPABILITY_LIST] as usize;

        if cap_ptr == 0 {
            self.bytes[PCI_CAPABILITY_LIST] = next_offset;
            self.bytes[PCI_STATUS] |= PCI_STATUS_CAP_LIST as u8;
            return;
        }

        for _ in 0..MAX_CAPABILITY_COUNT {
            if let Some(next) = self.next_capability(cap_ptr) {
                if next == 0 {
                    self.bytes[cap_ptr + 1] = next_offset;
                    return;
                }
                cap_ptr = next;
            }
        }
    }

    pub fn new_capability(&mut self) -> PciCapability {
        PciCapability::new_vendor_capability(self)
    }

    pub fn set_mmio_bar(&mut self, bar: PciBar, range: AddressRange) {
        assert!(range.is_naturally_aligned(), "cannot set_mmio_bar() because mmio range is not naturally aligned");
        self.bar_write_masks[bar.idx()] = !((range.size() as u32) - 1);
        let offset = PCI_BAR0 + (bar.idx() * 4);
        let address = (range.base() as u32).to_le_bytes();
        self.write_bytes(offset, &address);
    }

    pub fn read(&self, offset: u64, data: &mut [u8]) {
        if Self::is_valid_access(offset, data.len()) {
            self.read_bytes(offset as usize, data)
        } else {
            data.fill(0xff)
        }
    }

    pub fn write(&mut self, offset: u64, data: &[u8]) {
        if Self::is_valid_access(offset, data.len()) {
            self.write_config(offset as usize, data);
        }
    }
}