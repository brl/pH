// Copyright 2018 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::io;

use thiserror::Error;
use vm_memory::GuestMemoryMmap;
use crate::audio::pulse::{PulseClient, PulseError};
use crate::devices::ac97::ac97_bus_master::{Ac97BusMaster, AudioStreamSource};
use crate::devices::ac97::ac97_mixer::Ac97Mixer;
use crate::devices::ac97::ac97_regs::{MASTER_REGS_SIZE, MIXER_REGS_SIZE};
use crate::devices::irq_event::IrqLevelEvent;
use crate::io::pci::{PciBar, PciBarAllocation, PciConfiguration, PciDevice};
use crate::vm::KvmVm;


// Use 82801AA because it's what qemu does.
const PCI_DEVICE_ID_INTEL_82801AA_5: u16 = 0x2415;

/// AC97 audio device emulation.
/// Provides the PCI interface for the internal Ac97 emulation.
/// Internally the `Ac97BusMaster` and `Ac97Mixer` structs are used to emulated the bus master and
/// mixer registers respectively. `Ac97BusMaster` handles moving samples between guest memory and
/// the audio backend.
/// Errors that are possible from a `Ac97`.

#[derive(Error, Debug)]
pub enum Ac97Error {
    #[error("Error creating IRQ level event: {0}")]
    IrqLevelEventError(io::Error),
    #[error("PulseAudio: {0}")]
    PulseError(PulseError),
}

pub struct Ac97Dev {
    irq: u8,
    pci_config: PciConfiguration,
    bus_master: Ac97BusMaster,
    mixer: Ac97Mixer,
}

const PCI_CLASS_MULTIMEDIA_AUDIO:u16 = 0x0401;
const PCI_VENDOR_ID_INTEL: u16 = 0x8086;

impl Ac97Dev {
    /// Creates an 'Ac97Dev' that uses the given `GuestRam` and starts with all registers at
    /// default values.
    pub fn new(
        irq: u8,
        mem: &GuestMemoryMmap,
        audio_server: AudioStreamSource,
    ) -> Self {
        let pci_config = PciConfiguration::new(irq, PCI_VENDOR_ID_INTEL, PCI_DEVICE_ID_INTEL_82801AA_5, PCI_CLASS_MULTIMEDIA_AUDIO);

        Self {
            irq,
            pci_config,
            bus_master: Ac97BusMaster::new(
                mem.clone(),
                audio_server,
            ),
            mixer: Ac97Mixer::new(),
        }
    }

    /// Creates an `Ac97Dev` with suitable audio server inside based on Ac97Parameters. If it fails
    /// to create `Ac97Dev` with the given back-end, it'll fallback to the null audio device.
    pub fn try_new(
        kvm_vm: &KvmVm,
        irq: u8,
        mem: &GuestMemoryMmap,
    ) -> Result<Self, Ac97Error> {
        let mut ac97 = Self::initialize_pulseaudio(irq, mem)?;
        let irq_event = IrqLevelEvent::register(kvm_vm, irq)
            .map_err(Ac97Error::IrqLevelEventError)?;
        ac97.bus_master.set_irq_event(irq_event);
        Ok(ac97)
    }

    fn initialize_pulseaudio(irq: u8, mem: &GuestMemoryMmap) -> Result<Self, Ac97Error> {
        let server = PulseClient::connect(mem)
            .map_err(Ac97Error::PulseError)?;
        Ok(Self::new(
            irq,
            mem,
            Box::new(server),
        ))
    }

    fn read_mixer(&mut self, offset: u64, data: &mut [u8]) {
        match data.len() {
            // The mixer is only accessed with 16-bit words.
            2 => {
                let val: u16 = self.mixer.readw(offset);
                data[0] = val as u8;
                data[1] = (val >> 8) as u8;
            }
            l => warn!("mixer read length of {}", l),
        }
    }

    fn write_mixer(&mut self, offset: u64, data: &[u8]) {
        match data.len() {
            // The mixer is only accessed with 16-bit words.
            2 => self
                .mixer
                .writew(offset, u16::from(data[0]) | u16::from(data[1]) << 8),
            l => warn!("mixer write length of {}", l),
        }
        // Apply the new mixer settings to the bus master.
        self.bus_master.update_mixer_settings(&self.mixer);
    }

    fn read_bus_master(&mut self, offset: u64, data: &mut [u8]) {
        match data.len() {
            1 => data[0] = self.bus_master.readb(offset),
            2 => {
                let val: u16 = self.bus_master.readw(offset, &self.mixer);
                data[0] = val as u8;
                data[1] = (val >> 8) as u8;
            }
            4 => {
                let val: u32 = self.bus_master.readl(offset);
                data[0] = val as u8;
                data[1] = (val >> 8) as u8;
                data[2] = (val >> 16) as u8;
                data[3] = (val >> 24) as u8;
            }
            l => warn!("read length of {}", l),
        }
    }

    fn write_bus_master(&mut self, offset: u64, data: &[u8]) {
        match data.len() {
            1 => self.bus_master.writeb(offset, data[0], &self.mixer),
            2 => self
                .bus_master
                .writew(offset, u16::from(data[0]) | u16::from(data[1]) << 8),
            4 => self.bus_master.writel(
                offset,
                (u32::from(data[0]))
                    | (u32::from(data[1]) << 8)
                    | (u32::from(data[2]) << 16)
                    | (u32::from(data[3]) << 24),
                &mut self.mixer,
            ),
            l => warn!("write length of {}", l),
        }
    }
}

impl PciDevice for Ac97Dev {
    fn config(&self) -> &PciConfiguration {
        &self.pci_config
    }

    fn config_mut(&mut self) -> &mut PciConfiguration {
        &mut self.pci_config
    }

    fn read_bar(&mut self, bar: PciBar, offset: u64, data: &mut [u8]) {
        match bar {
            PciBar::Bar0 => self.read_mixer(offset, data),
            PciBar::Bar1 => self.read_bus_master(offset, data),
            _ => {},
        }
    }

    fn write_bar(&mut self, bar: PciBar, offset: u64, data: &[u8]) {
        match bar {
            PciBar::Bar0 => self.write_mixer(offset, data),
            PciBar::Bar1 => self.write_bus_master(offset, data),
            _ => {},
        }
    }

    fn irq(&self) -> Option<u8> {
        Some(self.irq)
    }

    fn bar_allocations(&self) -> Vec<PciBarAllocation> {
        vec![
            PciBarAllocation::Mmio(PciBar::Bar0, MIXER_REGS_SIZE as usize),
            PciBarAllocation::Mmio(PciBar::Bar1, MASTER_REGS_SIZE as usize)
        ]
    }
}