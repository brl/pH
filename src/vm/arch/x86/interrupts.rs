use kvm_bindings::kvm_lapic_state;
use kvm_ioctls::VcpuFd;

use crate::vm::arch::{Error, Result};

const APIC_MODE_EXTINT: u32 = 0x7;
const APIC_MODE_NMI: u32 = 0x4;
const APIC_LVT_LINT0_OFFSET: usize = 0x350;
const APIC_LVT_LINT1_OFFSET: usize = 0x360;

fn get_klapic_reg(klapic: &kvm_lapic_state, offset: usize) -> u32 {
    let mut bytes = [0u8; 4];
    for idx in 0..4 {
        bytes[idx] = klapic.regs[offset + idx] as u8;
    }
    u32::from_le_bytes(bytes)
}

fn set_klapic_reg(klapic: &mut kvm_lapic_state, offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    for idx in 0..4 {
        klapic.regs[offset + idx] = bytes[idx] as i8;
    }
}

fn set_apic_delivery_mode(reg: u32, mode: u32) -> u32 {
    (reg & !0x700) | (mode << 8)
}

pub fn setup_lapic(vcpu: &VcpuFd) -> Result<()> {
    let mut lapic = vcpu.get_lapic()
        .map_err(Error::SetupError)?;

    let lvt_lint0 = get_klapic_reg(&lapic, APIC_LVT_LINT0_OFFSET);
    set_klapic_reg(&mut lapic, APIC_LVT_LINT0_OFFSET, set_apic_delivery_mode(lvt_lint0, APIC_MODE_EXTINT));
    let lvt_lint1 = get_klapic_reg(&lapic, APIC_LVT_LINT1_OFFSET);
    set_klapic_reg(&mut lapic, APIC_LVT_LINT1_OFFSET, set_apic_delivery_mode(lvt_lint1, APIC_MODE_NMI));

    vcpu.set_lapic(&lapic)
        .map_err(Error::SetupError)?;
    Ok(())
}


