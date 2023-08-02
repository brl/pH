use std::iter;
use crate::io::PciIrq;

use crate::memory::GuestRam;
use crate::system::Result;
use crate::util::ByteBuffer;

const APIC_DEFAULT_PHYS_BASE: u32 = 0xfee00000;
const IO_APIC_DEFAULT_PHYS_BASE: u32 = 0xfec00000;

const MP_PROCESSOR: u8 = 0;
const MP_BUS: u8 = 1;
const MP_IOAPIC: u8 = 2;
const MP_INTSRC: u8 = 3;
const MP_LINTSRC: u8 = 4;

const MP_IRQ_SRC_INT: u8 = 0;
const MP_IRQ_SRC_NMI: u8 = 1;

const MP_IRQ_DEFAULT: u16 = 0;

const MPC_APIC_USABLE: u8 = 0x01;

const KVM_APIC_VER: u8 = 0x14;

const CPU_ENABLED: u8 = 1;
const CPU_BOOTPROCESSOR: u8 = 2;

const CPU_STEPPING: u32 = 0x600;
const CPU_FEATURE_APIC: u32 = 0x200;
const CPU_FEATURE_FPU: u32 = 0x001;


const PCI_BUSID: u8 = 0;
const PCI_BUSTYPE: &[u8] = b"PCI   ";
const ISA_BUSID: u8 = 1;
const ISA_BUSTYPE: &[u8] = b"ISA   ";

const MPTABLE_START: u64 = 0x9fc00;

const MPC_TABLE_SIZE: usize = 44;
const MPF_INTEL_SIZE: usize = 16;

struct Buffer {
    buffer: ByteBuffer<Vec<u8>>,
    count: usize,
}

impl Buffer {
    fn new() -> Buffer {
        Buffer {
            buffer: ByteBuffer::new_empty().little_endian(),
            count: 0,
        }
    }

    fn write_all_mpc_cpu(&mut self, ncpus: usize) -> &mut Self {
        for i in 0..ncpus {
            self.write_mpc_cpu(i as u8);
        }
        self
    }

    fn write_mpc_cpu(&mut self, cpuid: u8) -> &mut Self {
        self.count += 1;
        let flag = CPU_ENABLED | if cpuid == 0 { CPU_BOOTPROCESSOR } else { 0 };
        let featureflag = CPU_FEATURE_APIC | CPU_FEATURE_FPU;
        self.w8(MP_PROCESSOR)      // type
            .w8(cpuid)             // Local APIC number
            .w8(KVM_APIC_VER)      // APIC version
            .w8(flag)              // cpuflag
            .w32(CPU_STEPPING)    // cpufeature
            .w32(featureflag)     // CPUID feature value
            .w32(0).w32(0)   // reserved[2]
    }

    fn write_mpc_ioapic(&mut self, ioapicid: u8) -> &mut Self {
        self.count += 1;
        self.w8(MP_IOAPIC)                // type
            .w8(ioapicid)                 // Local APIC number
            .w8(KVM_APIC_VER)             // APIC version
            .w8(MPC_APIC_USABLE)          // flags
            .w32(IO_APIC_DEFAULT_PHYS_BASE) // apic addr
    }

    fn write_mpc_bus(&mut self, busid: u8, bustype: &[u8]) -> &mut Self {
        assert_eq!(bustype.len(), 6);
        self.count += 1;
        self.w8(MP_BUS)
            .w8(busid)
            .bytes(bustype)
    }

    fn write_mpc_intsrc(&mut self, ioapicid: u8, srcbusirq: u8, dstirq: u8) -> &mut Self {
        self.count += 1;
        self.w8(MP_INTSRC)
            .w8(MP_IRQ_SRC_INT)    // irq type
            .w16(MP_IRQ_DEFAULT)  // irq flag
            .w8(PCI_BUSID)         // src bus id
            .w8(srcbusirq)         // src bus irq
            .w8(ioapicid)          // dest apic id
            .w8(dstirq)            // dest irq
    }

    fn write_all_mpc_intsrc(&mut self, ioapicid: u8, pci_irqs: &[PciIrq]) -> &mut Self {
        for irq in pci_irqs {
            self.write_mpc_intsrc(ioapicid, irq.src_bus_irq(), irq.irq_line());
        }
        self
    }

    fn write_mpc_lintsrc(&mut self, irqtype: u8, dstirq: u8) -> &mut Self {
        self.count += 1;
        self.w8(MP_LINTSRC)
            .w8(irqtype)          // irq type
            .w16(MP_IRQ_DEFAULT) // irq flag
            .w8(ISA_BUSID)        // src bus id
            .w8(0)                // src bus irq
            .w8(0)                // dest apic id
            .w8(dstirq)           // dest apid lint
    }

    fn write_mpf_intel(&mut self) -> &mut Self {
        let start = self.buffer.len();
        let config_address = (MPTABLE_START + MPF_INTEL_SIZE as u64) as u32;
        self.align(16)
            .bytes(b"_MP_") // Signature
            .w32(config_address)   // Configuration table address
            .w8(1)           // Our length (paragraphs)
            .w8(4)           // Specification version
            .w8(0)           // checksum (offset 10)
            .pad(5)        // feature1 - feature5
            .checksum(start, MPF_INTEL_SIZE, 10)
    }

    fn write_mpc_table(&mut self, offset: usize) -> &mut Self {
        let old = self.buffer.len();
        let len = old - offset;

        self.buffer.set_offset(offset);
        self.bytes(b"PCMP")          // 0 Signature
            .w16(len as u16)         // 4 length
            .w8(4)                    // 6 Specification version
            .w8(0)                    // 7 checksum
            .bytes(b"SUBGRAPH")      // 8 oem[8]
            .bytes(b"0.1         ")  // 16 productid[12]
            .w32(0)                  // 28 oem ptr (0 if not present)
            .w16(0)  // 32 oem size
            .w16(0)              // 34 oem count
            .w32(APIC_DEFAULT_PHYS_BASE) // 36 APIC address
            .w32(0)                  // 40 reserved
            .checksum(offset, len, 7);
        self.buffer.set_offset(old);
        self
    }

    fn w8(&mut self, val: u8) -> &mut Self {
        self.buffer.write(val);
        self
    }
    fn w16(&mut self, data: u16) -> &mut Self {
        self.buffer.write(data);
        self
    }
    fn w32(&mut self, data: u32) -> &mut Self {
        self.buffer.write(data);
        self
    }

    fn bytes(&mut self, data: &[u8]) -> &mut Self {
        self.buffer.write(data);
        self
    }

    fn pad(&mut self, count: usize) -> &mut Self {
        if count > 0 {
            let zeros = iter::repeat(0).take(count).collect::<Vec<u8>>();
            self.buffer.write(zeros.as_slice());
        }
        self
    }

    fn align(&mut self, n: usize) -> &mut Self {
        let aligned = align(self.buffer.len(), n);
        let padlen = aligned - self.buffer.len();
        self.pad(padlen)
    }

    fn checksum(&mut self, start: usize, len: usize, csum_off: usize) -> &mut Self {
        {
            let slice = self.buffer.mut_at(start, len);
            let csum = slice.iter().fold(0i32, |acc, &x| acc.wrapping_add(x as i32));
            let b = (-csum & 0xFF) as u8;
            slice[csum_off] = b;
        }
        self
    }
}

fn align(sz: usize, n: usize) -> usize {
    (sz + (n - 1)) & !(n - 1)
}

pub fn setup_mptable(memory: &GuestRam, ncpus: usize, pci_irqs: &[PciIrq]) -> Result<()> {
    let ioapicid = (ncpus + 1) as u8;
    let mut buffer = Buffer::new();
    let address = MPTABLE_START;

    buffer.write_mpf_intel()
        .pad(MPC_TABLE_SIZE)
        .write_all_mpc_cpu(ncpus)
        .write_mpc_bus(PCI_BUSID, PCI_BUSTYPE)
        .write_mpc_bus(ISA_BUSID, ISA_BUSTYPE)
        .write_mpc_ioapic(ioapicid)
        .write_all_mpc_intsrc(ioapicid, &pci_irqs)
        .write_mpc_lintsrc(MP_IRQ_SRC_INT, 0)
        .write_mpc_lintsrc(MP_IRQ_SRC_NMI, 1)
        .write_mpc_table(MPF_INTEL_SIZE);

    memory.write_bytes(address, buffer.buffer.as_ref())
}