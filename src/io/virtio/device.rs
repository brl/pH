use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard};
use byteorder::{ByteOrder, LittleEndian};
use vm_memory::GuestMemoryMmap;
use crate::io::address::AddressRange;

use crate::io::busdata::{ReadableInt, WriteableInt};
use crate::io::pci::{PciBar, PciBarAllocation, PciConfiguration, PciDevice};
use crate::io::virtio::consts::*;
use crate::io::virtio::features::FeatureBits;
use crate::io::virtio::queues::Queues;
use crate::io::virtio::Result;
use crate::io::PCI_VENDOR_ID_REDHAT;
use crate::vm::KvmVm;

pub trait VirtioDevice: Send {

    fn features(&self) -> &FeatureBits;
    fn features_ok(&self) -> bool { true }

    fn queue_sizes(&self) -> &[u16];
    fn device_type(&self) -> VirtioDeviceType;

    fn config_size(&self) -> usize { 0 }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        let (_,_) = (offset, data);
    }

    fn write_config(&mut self, offset: u64, data: &[u8]) {
        let (_,_) = (offset, data);
    }

    fn start(&mut self, queues: &Queues);
}

pub struct VirtioDeviceState {
    pci_config: PciConfiguration,
    device: Arc<Mutex<dyn VirtioDevice>>,
    status: u8,
    queues: Queues,
}

impl VirtioDeviceState {

    pub fn new<T: VirtioDevice+'static>(device: T, kvm_vm: KvmVm, guest_memory: GuestMemoryMmap, irq: u8) -> Result<Self> {
        let devtype = device.device_type();
        let config_size = device.config_size();

        let device = Arc::new(Mutex::new(device));
        let queues = Queues::new(kvm_vm, guest_memory, irq)?;
        let mut pci_config = PciConfiguration::new(queues.irq(), PCI_VENDOR_ID_REDHAT, devtype.device_id(), devtype.class_id());
        Self::add_pci_capabilities::<T>(&mut pci_config, config_size);

        Ok(VirtioDeviceState {
            pci_config,
            device,
            status: 0,
            queues,
        })
    }

    fn add_pci_capabilities<T: VirtioDevice>(pci_config: &mut PciConfiguration, config_size: usize) {
        VirtioPciCapability::new(VIRTIO_PCI_CAP_COMMON_CFG)
            .set_mmio_range(VIRTIO_MMIO_OFFSET_COMMON_CFG, VIRTIO_MMIO_COMMON_CFG_SIZE)
            .store(pci_config);

        VirtioPciCapability::new(VIRTIO_PCI_CAP_ISR_CFG)
            .set_mmio_range(VIRTIO_MMIO_OFFSET_ISR, VIRTIO_MMIO_ISR_SIZE)
            .store(pci_config);

        VirtioPciCapability::new(VIRTIO_PCI_CAP_NOTIFY_CFG)
            .set_mmio_range(VIRTIO_MMIO_OFFSET_NOTIFY, VIRTIO_MMIO_NOTIFY_SIZE)
            .set_extra_word(4)
            .store(pci_config);

        if config_size > 0 {
            VirtioPciCapability::new(VIRTIO_PCI_CAP_DEVICE_CFG)
                .set_mmio_range(VIRTIO_MMIO_OFFSET_DEV_CFG, config_size as u64)
                .store(pci_config);
        }
    }

    fn device(&self) -> MutexGuard<dyn VirtioDevice + 'static> {
        self.device.lock().unwrap()
    }

    fn reset(&mut self) {
        self.queues.reset();
        self.device().features().reset();
        self.status = 0;
    }

    fn status_write(&mut self, val: u8) {
        let new_bits = val & !self.status;

        let has_new_bit = |bit| -> bool {
            new_bits & bit != 0
        };

        self.status |= new_bits;

        if val == 0 {
            self.reset();
        } else if has_new_bit(VIRTIO_CONFIG_S_FEATURES_OK) {
            // 2.2.2: The device SHOULD accept any valid subset of features the driver accepts,
            // otherwise it MUST fail to set the FEATURES_OK device status bit when the driver
            // writes it.
            if !self.device().features_ok() {
                self.status &= VIRTIO_CONFIG_S_FEATURES_OK;
            }
        } else if has_new_bit(VIRTIO_CONFIG_S_DRIVER_OK) {
            let features = self.device().features().guest_value();
            if let Err(err) = self.queues.configure_queues(features) {
                warn!("Error configuring virtqueue: {}", err);
            } else {
                self.device().start(&self.queues)
            }
        } else if has_new_bit(VIRTIO_CONFIG_S_FAILED) {
            // XXX print a warning
        }
    }

    fn common_config_write(&mut self, offset: u64, val: WriteableInt) {
        match val {
            WriteableInt::Byte(n) => match offset {
                /* device_status */
                20 => self.status_write(n),
                _ => warn!("VirtioDeviceState: common_config_write: unhandled byte offset {}", offset),
            },
            WriteableInt::Word(n) => match offset {
                /* queue_select */
                22 => self.queues.select(n),
                /* queue_size */
                24 => self.queues.set_size(n),
                /* queue_enable */
                28 => self.queues.enable_current(),
                _ => warn!("VirtioDeviceState: common_config_write: unhandled word offset {}", offset),
            }
            WriteableInt::DWord(n) => match offset {
                /* device_feature_select */
                0 => self.device().features().set_device_selected(n),
                /* guest_feature_select */
                8 => self.device().features().set_guest_selected(n),
                /* guest_feature */
                12 => self.device().features().write_guest_word(n),
                /* queue_desc_lo */
                32 => self.queues.set_current_descriptor_area(n, false),
                /* queue_desc_hi */
                36 => self.queues.set_current_descriptor_area(n, true),
                /* queue_avail_lo */
                40 => self.queues.set_avail_area(n, false),
                /* queue_avail_hi */
                44 => self.queues.set_avail_area(n, true),
                /* queue_used_lo */
                48 => self.queues.set_used_area(n, false),
                /* queue_used_hi */
                52 => self.queues.set_used_area(n, true),
                _ => warn!("VirtioDeviceState: common_config_write: unhandled dword offset {}", offset),
            },
            WriteableInt::QWord(_) => warn!("VirtioDeviceState: common_config_write: unhandled qword offset {}", offset),
            WriteableInt::Data(bs) => warn!("VirtioDeviceState: common_config_write: unhandled raw bytes offset {}, len {}", offset, bs.len()),
        }
    }

    fn common_config_read(&self, offset: u64) -> ReadableInt {
        match offset {
            /* device_feature_select */
            0 => self.device().features().device_selected().into(),
            /* device_feature */
            4 => self.device().features().read_device_word().into(),
            /* guest_feature_select */
            8 => self.device().features().guest_selected().into(),
            /* guest_feature */
            12 => self.device().features().read_guest_word().into(),
            /* msix_config */
            16 => VIRTIO_NO_MSI_VECTOR.into(),
            /* num_queues */
            18 => self.queues.num_queues().into(),
            /* device_status */
            20 => self.status.into(),
            /* config_generation */
            21 => (0u8).into(),
            /* queue_select */
            22 => self.queues.selected_queue().into(),
            /* queue_size */
            24 => self.queues.queue_size().into(),
            /* queue_msix_vector */
            26 => VIRTIO_NO_MSI_VECTOR.into(),
            /* queue_enable */
            28 => if self.queues.is_current_enabled() { 1u16.into() } else { 0u16.into() },
            /* queue_notify_off */
            30 => self.queues.selected_queue().into(),
            /* queue_desc_lo */
            32 => self.queues.get_current_descriptor_area(false).into(),
            /* queue_desc_hi */
            36 => self.queues.get_current_descriptor_area(true).into(),
            /* queue_avail_lo */
            40 => self.queues.get_avail_area(false).into(),
            /* queue_avail_hi */
            44 => self.queues.get_avail_area(true).into(),
            /* queue_used_lo */
            48 => self.queues.get_used_area(false).into(),
            /* queue_used_hi */
            52 => self.queues.get_used_area(true).into(),
            _ => ReadableInt::new_dword(0),
        }
    }

    fn isr_read(&self) -> u8 {
        self.queues.isr_read() as u8
    }

    fn is_device_config_range(&self, offset: u64, len: usize) -> bool {
        let dev = self.device();
        if dev.config_size() > 0 {
            let range = AddressRange::new(VIRTIO_MMIO_OFFSET_DEV_CFG, dev.config_size());
            range.contains(offset, len)
        } else {
            false
        }
    }

    fn is_common_cfg_range(&self, offset: u64, len: usize) -> bool {
        AddressRange::new(VIRTIO_MMIO_OFFSET_COMMON_CFG, VIRTIO_MMIO_COMMON_CFG_SIZE as usize)
            .contains(offset, len)
    }
}

impl PciDevice for VirtioDeviceState {

    fn config(&self) -> &PciConfiguration {
        &self.pci_config
    }

    fn config_mut(&mut self) -> &mut PciConfiguration {
        &mut self.pci_config
    }

    fn read_bar(&mut self, bar: PciBar, offset: u64, data: &mut [u8]) {
        if bar != PciBar::Bar0 {
            warn!("Virtio PciDevice: read_bar() expected bar0!");
            return;

        }

        if self.is_common_cfg_range(offset, data.len()) {
            let v = self.common_config_read(offset);
            v.read(data);
        } else if offset == VIRTIO_MMIO_OFFSET_ISR && data.len() == 1 {
            data[0] = self.isr_read();
        } else if self.is_device_config_range(offset, data.len()) {
            let dev = self.device();
            dev.read_config(offset - VIRTIO_MMIO_OFFSET_DEV_CFG, data);
        }
    }

    fn write_bar(&mut self, bar: PciBar, offset: u64, data: &[u8]) {
        if bar != PciBar::Bar0 {
            warn!("Virtio PciDevice: write_bar() expected bar0!");
            return;
        }
        if self.is_common_cfg_range(offset, data.len()) {
            let data = WriteableInt::from(data);
            self.common_config_write(offset, data);
        } else if self.is_device_config_range(offset, data.len()) {
            let mut dev = self.device();
            dev.write_config(offset - VIRTIO_MMIO_OFFSET_DEV_CFG, data);
        }
    }

    fn irq(&self) -> Option<u8> {
        Some(self.queues.irq())
    }

    fn bar_allocations(&self) -> Vec<PciBarAllocation> {
        vec![PciBarAllocation::Mmio(PciBar::Bar0, VIRTIO_MMIO_AREA_SIZE)]
    }

    fn configure_bars(&mut self, allocations: Vec<(PciBar, u64)>) {
        for (bar,base) in allocations {
            if bar == PciBar::Bar0 {
                let queue_sizes = self.device().queue_sizes().to_vec();
                if let Err(e) = self.queues.create_queues(base, &queue_sizes) {
                    warn!("Error creating queues: {}", e);
                }
            } else {
                warn!("Virtio PciDevice: Cannot configure unexpected PCI bar: {}", bar.idx());
            }
        }
    }
}

struct VirtioPciCapability {
    vtype: u8,
    size: u8,
    mmio_offset: u32,
    mmio_len: u32,
    extra_word: Option<u32>,
}

impl VirtioPciCapability {
    fn new(vtype: u8) -> VirtioPciCapability{
        VirtioPciCapability {
            vtype,
            size: 16,
            mmio_offset: 0,
            mmio_len: 0,
            extra_word: None
        }
    }
    fn set_mmio_range(&mut self, offset: u64, len: u64) -> &mut VirtioPciCapability {
        self.mmio_offset = offset as u32;
        self.mmio_len = len as u32;
        self
    }

    fn set_extra_word(&mut self, val: u32) -> &mut VirtioPciCapability {
        self.size += 4;
        self.extra_word = Some(val);
        self
    }

    fn store(&self, pci_config: &mut PciConfiguration) {
        /*
         * struct virtio_pci_cap {
         *     u8 cap_vndr; /* Generic PCI field: PCI_CAP_ID_VNDR */
         *     u8 cap_next; /* Generic PCI field: next ptr. */
         *     u8 cap_len; /* Generic PCI field: capability length */
         *     u8 cfg_type; /* Identifies the structure. */
         *     u8 bar; /* Where to find it. */
         *     u8 padding[3]; /* Pad to full dword. */
         *     le32 offset; /* Offset within bar. */
         *     le32 length; /* Length of the structure, in bytes. */
         * };
         */
        let mut cap = pci_config.new_capability();
        cap.write(self.size);
        cap.write(self.vtype);
        // Also fills the padding bytes
        cap.write(VIRTIO_MMIO_BAR as u32);
        cap.write(self.mmio_offset);
        cap.write(self.mmio_len);
        if let Some(word) = self.extra_word {
            cap.write(word);
        }
        cap.store();
    }
}

pub struct DeviceConfigArea {
    buffer: Vec<u8>,
    write_filter: DeviceConfigWriteFilter,
}


#[allow(dead_code)]
impl DeviceConfigArea {
    pub fn new(size: usize) -> Self {
        DeviceConfigArea{
            buffer: vec![0u8; size],
            write_filter: DeviceConfigWriteFilter::new(size),
        }
    }

    pub fn read_config(&self, offset: u64, data: &mut [u8]) {
        let offset = offset as usize;
        if offset + data.len() <= self.buffer.len() {
            data.copy_from_slice(&self.buffer[offset..offset+data.len()]);
        }
    }
    pub fn write_config(&mut self, offset: u64, data: &[u8]) {
        let offset = offset as usize;
        if self.write_filter.is_writeable(offset, data.len()) {
            self.buffer[offset..offset+data.len()].copy_from_slice(data);
        }
    }

    pub fn set_writeable(&mut self, offset: usize, size: usize) {
        self.write_filter.set_writable(offset, size)
    }

    pub fn write_u8(&mut self, offset: usize, val: u8) {
        assert!(offset + 1 <= self.buffer.len());
        self.buffer[offset] = val;
    }

    pub fn write_u16(&mut self, offset: usize, val: u16) {
        assert!(offset + 2 <= self.buffer.len());
        LittleEndian::write_u16(&mut self.buffer[offset..], val);
    }

    pub fn write_u32(&mut self, offset: usize, val: u32) {
        assert!(offset + 4 <= self.buffer.len());
        LittleEndian::write_u32(&mut self.buffer[offset..], val);
    }

    pub fn write_u64(&mut self, offset: usize, val: u64) {
        assert!(offset + 8 <= self.buffer.len());
        LittleEndian::write_u64(&mut self.buffer[offset..], val);
    }

    pub fn write_bytes(&mut self, offset: usize, bytes: &[u8]) {
        assert!(offset + bytes.len() <= self.buffer.len());
        self.buffer[offset..offset + bytes.len()].copy_from_slice(bytes);
    }
}

struct DeviceConfigWriteFilter {
    size: usize,
    ranges: Vec<Range<usize>>,
}

impl DeviceConfigWriteFilter {
    fn new(size: usize) -> Self {
        DeviceConfigWriteFilter { size, ranges: Vec::new() }
    }

    fn set_writable(&mut self, offset: usize, size: usize) {
        let end = offset + size;
        self.ranges.push(offset..end);
    }

    fn is_writeable(&self, offset: usize, size: usize) -> bool {
        if offset + size > self.size {
            false
        } else {
            let last = offset + size - 1;
            self.ranges.iter().any(|r| r.contains(&offset) && r.contains(&last))
        }
    }
}
