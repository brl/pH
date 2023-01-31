use kvm_bindings::CpuId;
use kvm_ioctls::VcpuFd;
use crate::vm::arch::{Error, Result};

const EBX_CLFLUSH_CACHELINE: u32 = 8; // Flush a cache line size.
const EBX_CLFLUSH_SIZE_SHIFT: u32 = 8; // Bytes flushed when executing CLFLUSH.
const _EBX_CPU_COUNT_SHIFT: u32 = 16; // Index of this CPU.
const EBX_CPUID_SHIFT: u32 = 24; // Index of this CPU.
const _ECX_EPB_SHIFT: u32 = 3; // "Energy Performance Bias" bit.
const _ECX_HYPERVISOR_SHIFT: u32 = 31; // Flag to be set when the cpu is running on a hypervisor.
const _EDX_HTT_SHIFT: u32 = 28; // Hyper Threading Enabled.

const INTEL_EBX: u32 = u32::from_le_bytes([b'G', b'e', b'n', b'u']);
const INTEL_EDX: u32 = u32::from_le_bytes([b'i', b'n', b'e', b'I']);
const INTEL_ECX: u32 = u32::from_le_bytes([b'n', b't', b'e', b'l']);

pub fn setup_cpuid(vcpu: &VcpuFd, cpuid: CpuId) -> Result<()> {
    let mut cpuid = cpuid;

    let cpu_id = 0u32; // first vcpu

    for e in cpuid.as_mut_slice() {
        match e.function {
            0 => {
                e.ebx = INTEL_EBX;
                e.ecx = INTEL_ECX;
                e.edx = INTEL_EDX;
            }
            1 => {
                if e.index == 0 {
                    e.ecx |= 1<<31;
                }
                e.ebx = (cpu_id << EBX_CPUID_SHIFT) as u32 |
                    (EBX_CLFLUSH_CACHELINE << EBX_CLFLUSH_SIZE_SHIFT);
                /*
                if cpu_count > 1 {
                    entry.ebx |= (cpu_count as u32) << EBX_CPU_COUNT_SHIFT;
                    entry.edx |= 1 << EDX_HTT_SHIFT;
                }
                */
            }
            6 => {
                e.ecx &= !(1<<3);

            }
            10 => {
                if e.eax > 0 {
                    let version = e.eax & 0xFF;
                    let ncounters = (e.eax >> 8) & 0xFF;
                    if version != 2 || ncounters == 0 {
                        e.eax = 0;
                    }
                }

            }
            _ => {}
        }
    }
    vcpu.set_cpuid2(&cpuid)
        .map_err(Error::SetupError)?;
    Ok(())
}