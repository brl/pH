// Copyright 2018 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.


use std::collections::VecDeque;
use std::convert::TryInto;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::{cmp, thread};
use std::fmt::{Debug, Formatter};
use std::time::{Duration, Instant};

use thiserror::Error;
use crate::audio::shm_streams::{ShmStream, ShmStreamSource};
use crate::audio::{BoxError,  SampleFormat, StreamControl, StreamDirection};
use crate::devices::ac97::ac97_mixer::Ac97Mixer;
use crate::devices::ac97::ac97_regs::*;
use crate::devices::irq_event::IrqLevelEvent;
use crate::memory::GuestRam;
use crate::system;

const INPUT_SAMPLE_RATE: u32 = 48000;
const DEVICE_INPUT_CHANNEL_COUNT: usize = 2;

pub(crate) type AudioStreamSource = Box<dyn ShmStreamSource>;

// Bus Master registers. Keeps the state of the bus master register values. Used to share the state
// between the main and audio threads.
struct Ac97BusMasterRegs {
    pi_regs: Ac97FunctionRegs,       // Input
    po_regs: Ac97FunctionRegs,       // Output
    po_pointer_update_time: Instant, // Time the picb and civ regs were last updated.
    mc_regs: Ac97FunctionRegs,       // Microphone
    glob_cnt: u32,
    glob_sta: u32,

    // IRQ event - driven by the glob_sta register.
    irq_evt: Option<IrqLevelEvent>,
}

impl Ac97BusMasterRegs {
    fn new() -> Ac97BusMasterRegs {
        Ac97BusMasterRegs {
            pi_regs: Ac97FunctionRegs::new("Input"),
            po_regs: Ac97FunctionRegs::new("Output"),
            po_pointer_update_time: Instant::now(),
            mc_regs: Ac97FunctionRegs::new("Microphone"),
            glob_cnt: 0,
            glob_sta: GLOB_STA_RESET_VAL,
            irq_evt: None,
        }
    }

    fn func_regs(&self, func: Ac97Function) -> &Ac97FunctionRegs {
        match func {
            Ac97Function::Input => &self.pi_regs,
            Ac97Function::Output => &self.po_regs,
            Ac97Function::Microphone => &self.mc_regs,
        }
    }

    fn func_regs_mut(&mut self, func: Ac97Function) -> &mut Ac97FunctionRegs {
        match func {
            Ac97Function::Input => &mut self.pi_regs,
            Ac97Function::Output => &mut self.po_regs,
            Ac97Function::Microphone => &mut self.mc_regs,
        }
    }

    fn tube_count(&self, func: Ac97Function) -> usize {
        fn output_tube_count(glob_cnt: u32) -> usize {
            let val = (glob_cnt & GLOB_CNT_PCM_246_MASK) >> 20;
            match val {
                0 => 2,
                1 => 4,
                2 => 6,
                _ => {
                    warn!("unknown tube_count: 0x{:x}", val);
                    2
                }
            }
        }

        match func {
            Ac97Function::Output => output_tube_count(self.glob_cnt),
            _ => DEVICE_INPUT_CHANNEL_COUNT,
        }
    }

    /// Returns whether the irq is set for any one of the bus master function registers.
    pub fn has_irq(&self) -> bool {
        self.pi_regs.has_irq() || self.po_regs.has_irq() || self.mc_regs.has_irq()
    }
}

// Internal error type used for reporting errors from guest memory reading.
#[derive(Error, Debug)]
pub(crate) enum GuestMemoryError {
    // Failure getting the address of the audio buffer.
    #[error("Failed to get the address of the audio buffer: {0}.")]
    ReadingGuestBufferAddress(system::Error),
}

#[derive(Error, Debug)]
pub(crate) enum AudioError {
    #[error("Failed to create audio stream: {0}.")]
    CreateStream(BoxError),
    #[error("Offset > max usize")]
    InvalidBufferOffset,
    #[error("Failed to read guest memory: {0}.")]
    ReadingGuestError(GuestMemoryError),
    // Failure to respond to the ServerRequest.
    #[error("Failed to respond to the ServerRequest: {0}")]
    RespondRequest(BoxError),
    // Failure to wait for a request from the stream.
    #[error("Failed to wait for a message from the stream: {0}")]
    WaitForAction(BoxError),
}

impl From<GuestMemoryError> for AudioError {
    fn from(err: GuestMemoryError) -> Self {
        AudioError::ReadingGuestError(err)
    }
}

type GuestMemoryResult<T> = Result<T, GuestMemoryError>;

type AudioResult<T> = Result<T, AudioError>;

// Audio thread book-keeping data
struct AudioThreadInfo {
    thread: Option<thread::JoinHandle<()>>,
    thread_run: Arc<AtomicBool>,
    thread_semaphore: Arc<Condvar>,
    stream_control: Option<Box<dyn StreamControl>>,
}

impl AudioThreadInfo {
    fn new() -> Self {
        Self {
            thread: None,
            thread_run: Arc::new(AtomicBool::new(false)),
            thread_semaphore: Arc::new(Condvar::new()),
            stream_control: None,
        }
    }

    fn is_running(&self) -> bool {
        self.thread_run.load(Ordering::Relaxed)
    }

    fn start(&mut self, mut worker: AudioWorker) {
        self.thread_run.store(true, Ordering::Relaxed);
        self.thread = Some(thread::spawn(move || {

            if let Err(e) = worker.run() {
                warn!("{:?} error: {}", worker.func, e);
            }

            worker.thread_run.store(false, Ordering::Relaxed);
        }));
    }

    fn stop(&mut self) {
        self.thread_run.store(false, Ordering::Relaxed);
        self.thread_semaphore.notify_one();
        if let Some(thread) = self.thread.take() {
            if let Err(e) = thread.join() {
                warn!("Failed to join thread: {:?}.", e);
            }
        }
    }
}

/// `Ac97BusMaster` emulates the bus master portion of AC97. It exposes a register read/write
/// interface compliant with the ICH bus master.
pub struct Ac97BusMaster {
    // Keep guest memory as each function will use it for buffer descriptors.
    mem: GuestRam,
    regs: Arc<Mutex<Ac97BusMasterRegs>>,
    acc_sema: u8,

    // Bookkeeping info for playback and capture stream.
    po_info: AudioThreadInfo,
    pi_info: AudioThreadInfo,
    pmic_info: AudioThreadInfo,

    // Audio server used to create playback or capture streams.
    audio_server: AudioStreamSource,

    // Thread for hadlind IRQ resample events from the guest.
    irq_resample_thread: Option<thread::JoinHandle<()>>,
}

impl Ac97BusMaster {

    /// Creates an Ac97BusMaster` object that plays audio from `mem` to streams provided by
    /// `audio_server`.
    pub fn new(mem: GuestRam, audio_server: AudioStreamSource) -> Self {
        Ac97BusMaster {
            mem,
            regs: Arc::new(Mutex::new(Ac97BusMasterRegs::new())),
            acc_sema: 0,

            po_info: AudioThreadInfo::new(),
            pi_info: AudioThreadInfo::new(),
            pmic_info: AudioThreadInfo::new(),
            audio_server,

            irq_resample_thread: None,
        }
    }

    fn regs(&self) -> MutexGuard<Ac97BusMasterRegs> {
        self.regs.lock().unwrap()
    }

    /// Provides the events needed to raise interrupts in the guest.
    pub fn set_irq_event(&mut self, irq_evt: IrqLevelEvent) {
        let thread_regs = self.regs.clone();
        self.regs().irq_evt = Some(irq_evt.try_clone().expect("cloning irq_evt failed"));

        self.irq_resample_thread = Some(thread::spawn(move || {
            loop {
                if let Err(e) = irq_evt.wait_resample() {
                    warn!(
                        "Failed to read the irq event from the resample thread: {}.",
                        e,
                    );
                    break;
                }
                {
                    // Scope for the lock on thread_regs.
                    let regs = thread_regs.lock().unwrap();
                    if regs.has_irq() {
                        if let Err(e) = irq_evt.trigger() {
                            warn!("Failed to set the irq from the resample thread: {}.", e);
                            break;
                        }
                    }
                }
            }
        }));
    }

    /// Called when `mixer` has been changed and the new values should be applied to currently
    /// active streams.
    pub fn update_mixer_settings(&mut self, mixer: &Ac97Mixer) {
        if let Some(control) = self.po_info.stream_control.as_mut() {
            // The audio server only supports one volume, not separate left and right.
            let (muted, left_volume, _right_volume) = mixer.get_master_volume();
            control.set_volume(left_volume);
            control.set_mute(muted);
        }
    }

    /// Checks if the bus master is in the cold reset state.
    pub fn is_cold_reset(&self) -> bool {
        self.regs().glob_cnt & GLOB_CNT_COLD_RESET == 0
    }

    /// Reads a byte from the given `offset`.
    pub fn readb(&mut self, offset: u64) -> u8 {
        fn readb_func_regs(func_regs: &Ac97FunctionRegs, offset: u64) -> u8 {
            let result = match offset {
                CIV_OFFSET => func_regs.civ,
                LVI_OFFSET => func_regs.lvi,
                SR_OFFSET => func_regs.sr as u8,
                PIV_OFFSET => func_regs.piv,
                CR_OFFSET => func_regs.cr,
                _ => 0,
            };
            result
        }

        let regs = self.regs();
        match offset {
            PI_BASE_00..=PI_CR_0B => readb_func_regs(&regs.pi_regs, offset - PI_BASE_00),
            PO_BASE_10..=PO_CR_1B => readb_func_regs(&regs.po_regs, offset - PO_BASE_10),
            MC_BASE_20..=MC_CR_2B => readb_func_regs(&regs.mc_regs, offset - MC_BASE_20),
            ACC_SEMA_34 => self.acc_sema,
            _ => 0,
        }
    }

    /// Reads a word from the given `offset`.
    pub fn readw(&mut self, offset: u64, mixer: &Ac97Mixer) -> u16 {
        let regs = self.regs();
        match offset {
            PI_SR_06 => regs.pi_regs.sr,
            PI_PICB_08 => regs.pi_regs.picb,
            PO_SR_16 => regs.po_regs.sr,
            PO_PICB_18 => {
                // PO PICB
                if !self.thread_info(Ac97Function::Output).is_running() {
                    // Not running, no need to estimate what has been consumed.
                    regs.po_regs.picb
                } else {
                    // Estimate how many samples have been played since the last audio callback.
                    let num_channels = regs.tube_count(Ac97Function::Output) as u64;
                    let micros = regs.po_pointer_update_time.elapsed().subsec_micros();
                    // Round down to the next 10 millisecond boundary. The linux driver often
                    // assumes that two rapid reads from picb will return the same value.
                    let millis = micros / 1000 / 10 * 10;
                    let sample_rate = self.current_sample_rate(Ac97Function::Output, mixer);
                    let frames_consumed = sample_rate as u64 * u64::from(millis) / 1000;

                    regs.po_regs
                        .picb
                        .saturating_sub((num_channels * frames_consumed) as u16)
                }
            }
            MC_SR_26 => regs.mc_regs.sr,
            MC_PICB_28 => regs.mc_regs.picb,
            _ => 0,
        }
    }

    /// Reads a 32-bit word from the given `offset`.
    pub fn readl(&mut self, offset: u64) -> u32 {
        let regs = self.regs();
        let result = match offset {
            PI_BDBAR_00 => regs.pi_regs.bdbar,
            PI_CIV_04 => regs.pi_regs.atomic_status_regs(),
            PO_BDBAR_10 => regs.po_regs.bdbar,
            PO_CIV_14 => regs.po_regs.atomic_status_regs(),
            MC_BDBAR_20 => regs.mc_regs.bdbar,
            MC_CIV_24 => regs.mc_regs.atomic_status_regs(),
            GLOB_CNT_2C => regs.glob_cnt,
            GLOB_STA_30 => regs.glob_sta,
            _ => 0,
        };
        result
    }

    /// Writes the byte `val` to the register specified by `offset`.
    pub fn writeb(&mut self, offset: u64, val: u8, mixer: &Ac97Mixer) {
        // Only process writes to the control register when cold reset is set.
        if self.is_cold_reset() {
            info!("Ignoring control register write at offset {:02x} due to cold reset status");
            return;
        }

        match offset {
            PI_CIV_04 => (), // RO
            PI_LVI_05 => self.set_lvi(Ac97Function::Input, val),
            PI_SR_06 => self.set_sr(Ac97Function::Input, u16::from(val)),
            PI_PIV_0A => (), // RO
            PI_CR_0B => self.set_cr(Ac97Function::Input, val, mixer),
            PO_CIV_14 => (), // RO
            PO_LVI_15 => self.set_lvi(Ac97Function::Output, val),
            PO_SR_16 => self.set_sr(Ac97Function::Output, u16::from(val)),
            PO_PIV_1A => (), // RO
            PO_CR_1B => self.set_cr(Ac97Function::Output, val, mixer),
            MC_CIV_24 => (), // RO
            MC_LVI_25 => self.set_lvi(Ac97Function::Microphone, val),
            MC_SR_26 => self.set_sr(Ac97Function::Microphone, u16::from(val)),
            MC_PIV_2A => (), // RO
            MC_CR_2B => self.set_cr(Ac97Function::Microphone, val, mixer),
            ACC_SEMA_34 => self.acc_sema = val,
            o => warn!("AC97: write byte to 0x{:x}", o),
        }
    }

    /// Writes the word `val` to the register specified by `offset`.
    pub fn writew(&mut self, offset: u64, val: u16) {
        // Only process writes to the control register when cold reset is set.
        if self.is_cold_reset() {
            return;
        }
        match offset {
            PI_SR_06 => self.set_sr(Ac97Function::Input, val),
            PI_PICB_08 => (), // RO
            PO_SR_16 => self.set_sr(Ac97Function::Output, val),
            PO_PICB_18 => (), // RO
            MC_SR_26 => self.set_sr(Ac97Function::Microphone, val),
            MC_PICB_28 => (), // RO
            o => warn!("AC97: write word to 0x{:x}", o),
        }
    }

    /// Writes the 32-bit `val` to the register specified by `offset`.
    pub fn writel(&mut self, offset: u64, val: u32, mixer: &mut Ac97Mixer) {
        // Only process writes to the control register when cold reset is set.
        if self.is_cold_reset() && offset != 0x2c {
            return;
        }
        match offset {
            PI_BDBAR_00 => self.set_bdbar(Ac97Function::Input, val),
            PO_BDBAR_10 => self.set_bdbar(Ac97Function::Output, val),
            MC_BDBAR_20 => self.set_bdbar(Ac97Function::Microphone, val),
            GLOB_CNT_2C => self.set_glob_cnt(val, mixer),
            GLOB_STA_30 => (), // RO
            o => warn!("AC97: write long to 0x{:x}", o),
        }
    }

    fn set_bdbar(&mut self, func: Ac97Function, val: u32) {
        self.regs().func_regs_mut(func).bdbar = val & !0x07;
    }

    fn set_lvi(&mut self, func: Ac97Function, val: u8) {
        let mut regs = self.regs();
        let func_regs = regs.func_regs_mut(func);
        func_regs.lvi = val % 32; // LVI wraps at 32.

        // If running and stalled waiting for more valid buffers, restart by clearing the "DMA
        // stopped" bit.
        if func_regs.cr & CR_RPBM == CR_RPBM
            && func_regs.sr & SR_DCH == SR_DCH
            && func_regs.civ != func_regs.lvi
        {
            Ac97BusMaster::check_and_move_to_next_buffer(func_regs);

            func_regs.sr &= !(SR_DCH | SR_CELV);

            self.thread_semaphore_notify(func);
        }
    }

    fn set_sr(&mut self, func: Ac97Function, val: u16) {
        let mut sr = self.regs().func_regs(func).sr;
        if val & SR_FIFOE != 0 {
            sr &= !SR_FIFOE;
        }
        if val & SR_LVBCI != 0 {
            sr &= !SR_LVBCI;
        }
        if val & SR_BCIS != 0 {
            sr &= !SR_BCIS;
        }
        let mut regs = self.regs();
        update_sr(&mut regs, func, sr);
    }

    fn set_cr(&mut self, func: Ac97Function, val: u8, mixer: &Ac97Mixer) {
        if val & CR_RR != 0 {
            let mut regs = self.regs();
            Self::reset_func_regs(&mut regs, func);
        } else {
            let cr = self.regs().func_regs(func).cr;
            if val & CR_RPBM == 0 {
                // Run/Pause set to pause.
                self.thread_info_mut(func).stop();
                let mut regs = self.regs();
                regs.func_regs_mut(func).sr |= SR_DCH;
            } else if cr & CR_RPBM == 0 {
                // Not already running.
                // Run/Pause set to run.
                {
                    let mut regs = self.regs();
                    let func_regs = regs.func_regs_mut(func);
                    func_regs.piv = 1;
                    func_regs.civ = 0;
                    func_regs.sr &= !SR_DCH;
                }
                if let Err(e) = self.start_audio(func, mixer) {
                    warn!("Failed to start audio: {}", e);
                }
            }
            let mut regs = self.regs();
            regs.func_regs_mut(func).cr = val & CR_VALID_MASK;
        }
    }

    fn set_glob_cnt(&mut self, new_glob_cnt: u32, mixer: &mut Ac97Mixer) {
        // Only the reset bits are emulated, the GPI and PCM formatting are not supported.
        if new_glob_cnt & GLOB_CNT_COLD_RESET == 0 {
            self.reset_audio_regs();
            mixer.reset();
            self.regs().glob_cnt = new_glob_cnt & GLOB_CNT_STABLE_BITS;
            self.acc_sema = 0;
            return;
        }
        if new_glob_cnt & GLOB_CNT_WARM_RESET != 0 {
            // Check if running and if so, ignore. Warm reset is specified to no-op when the device
            // is playing or recording audio.
            if !self.is_audio_running() {
                self.stop_all_audio();
                let mut regs = self.regs();
                regs.glob_cnt = new_glob_cnt & !GLOB_CNT_WARM_RESET; // Auto-cleared reset bit.
                return;
            }
        }
        self.regs().glob_cnt = new_glob_cnt;
    }

    fn current_sample_rate(&self, func: Ac97Function, mixer: &Ac97Mixer) -> u32 {
        match func {
            Ac97Function::Output => mixer.get_sample_rate().into(),
            _ => INPUT_SAMPLE_RATE,
        }
    }

    fn thread_info(&self, func: Ac97Function) -> &AudioThreadInfo {
        match func {
            Ac97Function::Microphone => &self.pmic_info,
            Ac97Function::Input => &self.pi_info,
            Ac97Function::Output => &self.po_info,
        }
    }

    fn thread_info_mut(&mut self, func: Ac97Function) -> &mut AudioThreadInfo {
        match func {
            Ac97Function::Microphone => &mut self.pmic_info,
            Ac97Function::Input => &mut self.pi_info,
            Ac97Function::Output => &mut self.po_info,
        }
    }

    fn is_audio_running(&self) -> bool {
        self.thread_info(Ac97Function::Output).is_running()
            || self.thread_info(Ac97Function::Input).is_running()
            || self.thread_info(Ac97Function::Microphone).is_running()
    }

    fn stop_all_audio(&mut self) {
        self.thread_info_mut(Ac97Function::Input).stop();
        self.thread_info_mut(Ac97Function::Output).stop();
        self.thread_info_mut(Ac97Function::Microphone).stop();
    }

    // Helper function for resetting function registers.
    fn reset_func_regs(regs: &mut Ac97BusMasterRegs, func: Ac97Function) {
        regs.func_regs_mut(func).do_reset();
        update_sr(regs, func, SR_DCH);
    }

    fn reset_audio_regs(&mut self) {
        self.stop_all_audio();
        let mut regs = self.regs();
        Self::reset_func_regs(&mut regs, Ac97Function::Input);
        Self::reset_func_regs(&mut regs, Ac97Function::Output);
        Self::reset_func_regs(&mut regs, Ac97Function::Microphone);
    }

    fn check_and_move_to_next_buffer(func_regs: &mut Ac97FunctionRegs) {
        if func_regs.sr & SR_CELV != 0 {
            // CELV means we'd already processed the buffer at CIV.
            // Move CIV to the next buffer now that LVI has moved.
            func_regs.move_to_next_buffer();
        }
    }

    fn thread_semaphore_notify(&self, func: Ac97Function) {
        match func {
            Ac97Function::Input => self.pi_info.thread_semaphore.notify_one(),
            Ac97Function::Output => self.po_info.thread_semaphore.notify_one(),
            Ac97Function::Microphone => self.pmic_info.thread_semaphore.notify_one(),
        }
    }

    fn create_audio_worker(&mut self, mixer: &Ac97Mixer, func: Ac97Function) -> AudioResult<AudioWorker> {
        info!("AC97: create_audio_worker({:?})", func);
        let direction = match func {
            Ac97Function::Microphone => StreamDirection::Capture,
            Ac97Function::Input => StreamDirection::Capture,
            Ac97Function::Output => StreamDirection::Playback,
        };

        let locked_regs = self.regs.lock().unwrap();
        let sample_rate = self.current_sample_rate(func, mixer);
        let buffer_samples = current_buffer_size(locked_regs.func_regs(func), &self.mem)?;
        let num_channels = locked_regs.tube_count(func);
        let buffer_frames = buffer_samples / num_channels;

        let pending_buffers = VecDeque::with_capacity(2);

        let stream = self
            .audio_server
            .new_stream(
                direction,
                num_channels,
                SampleFormat::S16LE,
                sample_rate,
                buffer_frames)
            .map_err(AudioError::CreateStream)?;

        let params = AudioWorkerParams {
            func,
            stream,
            pending_buffers,
            message_interval: Duration::from_secs_f64(buffer_frames as f64 / sample_rate as f64),
        };
        Ok(AudioWorker::new(self, params))
    }

    fn start_audio(&mut self, func: Ac97Function, mixer: &Ac97Mixer) -> AudioResult<()> {
        let audio_worker = self.create_audio_worker(mixer, func)?;
        self.thread_info_mut(func).start(audio_worker);
        self.update_mixer_settings(mixer);
        Ok(())
    }
}

fn get_buffer_samples(
    func_regs: &Ac97FunctionRegs,
    mem: &GuestRam,
    index: u8,
) -> GuestMemoryResult<usize> {
    let descriptor_addr = func_regs.bdbar + u32::from(index) * DESCRIPTOR_LENGTH as u32;
    let control_reg: u32 = mem
        .read_int(u64::from(descriptor_addr) + 4)
        .map_err(GuestMemoryError::ReadingGuestBufferAddress)?;
    let buffer_samples = control_reg as usize & 0x0000_ffff;
    Ok(buffer_samples)
}

// Marks the current buffer completed and moves to the next buffer for the given
// function and registers.
fn buffer_completed(
    regs: &mut Ac97BusMasterRegs,
    mem: &GuestRam,
    func: Ac97Function,
) -> AudioResult<()> {
    // check if the completed descriptor wanted an interrupt on completion.
    let civ = regs.func_regs(func).civ;
    let descriptor_addr = regs.func_regs(func).bdbar + u32::from(civ) * DESCRIPTOR_LENGTH as u32;
    let control_reg: u32 = mem
        .read_int(u64::from(descriptor_addr) + 4)
        .map_err(GuestMemoryError::ReadingGuestBufferAddress)?;

    let mut new_sr = regs.func_regs(func).sr & !SR_CELV;
    if control_reg & BD_IOC != 0 {
        new_sr |= SR_BCIS;
    }

    let lvi = regs.func_regs(func).lvi;
    // if the current buffer was the last valid buffer, then update the status register to
    // indicate that the end of audio was hit and possibly raise an interrupt.
    if civ == lvi {
        new_sr |= SR_DCH | SR_CELV | SR_LVBCI;
    } else {
        regs.func_regs_mut(func).move_to_next_buffer();
    }

    update_sr(regs, func, new_sr);

    regs.func_regs_mut(func).picb = current_buffer_size(regs.func_regs(func), mem)? as u16;
    if func == Ac97Function::Output {
        regs.po_pointer_update_time = Instant::now();
    }

    Ok(())
}

// Update the status register and if any interrupts need to fire, raise them.
fn update_sr(regs: &mut Ac97BusMasterRegs, func: Ac97Function, val: u16) {
    let int_mask = match func {
        Ac97Function::Input => GS_PIINT,
        Ac97Function::Output => GS_POINT,
        Ac97Function::Microphone => GS_MINT,
    };

    let mut interrupt_high = false;

    {
        let func_regs = regs.func_regs_mut(func);
        let old_sr = func_regs.sr;
        func_regs.sr = val;
        if (old_sr ^ val) & SR_INT_MASK != 0 {
            if (val & SR_LVBCI) != 0 && (func_regs.cr & CR_LVBIE) != 0 {
                interrupt_high = true;
            }
            if (val & SR_BCIS) != 0 && (func_regs.cr & CR_IOCE) != 0 {
                interrupt_high = true;
            }
        } else {
            return;
        }
    }

    if interrupt_high {
        regs.glob_sta |= int_mask;
        if let Some(ref irq_evt) = regs.irq_evt {
            // Ignore write failure, nothing can be done about it from here.
            let _ = irq_evt.trigger();
        } else {
            info!("AC97: No interrupt! uh oh");
        }
    } else {
        regs.glob_sta &= !int_mask;
    }
}

// Returns the size in samples of the buffer pointed to by the CIV register.
fn current_buffer_size(
    func_regs: &Ac97FunctionRegs,
    mem: &GuestRam,
) -> GuestMemoryResult<usize> {
    let civ = func_regs.civ;
    get_buffer_samples(func_regs, mem, civ)
}

#[derive(Clone)]
struct GuestBuffer {
    index: u8,
    address: u64,
    samples: usize,
    channels: usize,
    consumed_frames: Arc<AtomicUsize>,
}

impl GuestBuffer {
    fn new(index: u8, address: u64, samples: usize, channels: usize) -> Self {
        GuestBuffer {
            index,
            address,
            samples,
            channels,
            consumed_frames: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn start_address(&self, frame_size: usize) -> u64 {
        self.address + (self.consumed_frames() * frame_size) as u64
    }

    fn frames(&self) -> usize {
        self.samples / self.channels
    }

    fn add_consumed(&self, frames: usize) {
        self.consumed_frames.fetch_add(frames, Ordering::Relaxed);
    }

    fn consumed_frames(&self) -> usize {
        self.consumed_frames.load(Ordering::Relaxed)
    }

    fn remaining_frames(&self) -> usize {
        self.frames() - self.consumed_frames()
    }

    fn is_consumed(&self) -> bool {
        self.consumed_frames() >= self.frames()
    }
}

impl Debug for GuestBuffer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "GuestBuffer([{}]@0x{:08x}, [{} of {} frames remaining])", self.index, self.address, self.remaining_frames(), self.frames())
    }
}

fn get_buffer_address(
    func_regs: &Ac97FunctionRegs,
    mem: &GuestRam,
    index: u8,
) -> GuestMemoryResult<u64> {
    let descriptor_addr = func_regs.bdbar + u32::from(index) * DESCRIPTOR_LENGTH as u32;
    let buffer_addr_reg: u32 = mem
        .read_int(u64::from(descriptor_addr))
        .map_err(GuestMemoryError::ReadingGuestBufferAddress)?;
    let buffer_addr = (buffer_addr_reg & !0x03u32) as u64; // The address must be aligned to four bytes.
    Ok(buffer_addr)
}

// Gets the start address and length of the buffer at `civ + offset` from the
// guest.
// This will return `None` if `civ + offset` is past LVI; if the DMA controlled
// stopped bit is set, such as after an underrun where CIV hits LVI; or if
// `civ + offset == LVI and the CELV flag is set.
fn next_guest_buffer(
    regs: &Ac97BusMasterRegs,
    mem: &GuestRam,
    func: Ac97Function,
    offset: usize,
) -> AudioResult<Option<GuestBuffer>> {
    let func_regs = regs.func_regs(func);
    let offset = (offset % 32) as u8;
    let index = (func_regs.civ + offset) % 32;

    // Check that value is between `low` and `high` modulo some `n`.
    fn check_between(low: u8, high: u8, value: u8) -> bool {
        // If low <= high, value must be in the interval between them:
        // 0     l     h     n
        // ......+++++++......
        (low <= high && (low <= value && value <= high)) ||
            // If low > high, value must not be in the interval between them:
            // 0       h      l  n
            // +++++++++......++++
            (low > high && (low <= value || value <= high))
    }

    // Check if
    //  * we're halted
    //  * `index` is not between CIV and LVI (mod 32)
    //  * `index is LVI and we've already processed LVI (SR_CELV is set)
    //  if any of these are true `index` isn't valid.
    if func_regs.sr & SR_DCH != 0
        || !check_between(func_regs.civ, func_regs.lvi, index)
        || func_regs.sr & SR_CELV != 0
    {
        return Ok(None);
    }

    let address = get_buffer_address(func_regs, mem, index)?
        .try_into()
        .map_err(|_| AudioError::InvalidBufferOffset)?;

    let samples = get_buffer_samples(func_regs, mem, index)?;
    let channels = regs.tube_count(func);

    Ok(Some(GuestBuffer::new(index, address, samples, channels)))
}

// Runs and updates the offset within the stream shm where samples can be
// found/placed for shm playback/capture streams, respectively
struct AudioWorker {
    func: Ac97Function,
    regs: Arc<Mutex<Ac97BusMasterRegs>>,
    mem: GuestRam,
    thread_run: Arc<AtomicBool>,
    lvi_semaphore: Arc<Condvar>,
    message_interval: Duration,
    stream: Box<dyn ShmStream>,
    pending_buffers: Arc<Mutex<VecDeque<Option<GuestBuffer>>>>,
}

struct AudioWorkerParams {
    func: Ac97Function,
    stream: Box<dyn ShmStream>,
    pending_buffers: VecDeque<Option<GuestBuffer>>,
    message_interval: Duration,
}

impl AudioWorker {
    fn new(bus_master: &Ac97BusMaster, args: AudioWorkerParams) -> Self {
        Self {
            func: args.func,
            regs: bus_master.regs.clone(),
            mem: bus_master.mem.clone(),
            thread_run: bus_master.thread_info(args.func).thread_run.clone(),
            lvi_semaphore: bus_master.thread_info(args.func).thread_semaphore.clone(),
            message_interval: args.message_interval,
            stream: args.stream,
            pending_buffers: Arc::new( Mutex::new(args.pending_buffers)),
        }
    }

    fn next_guest_buffer(&self) -> AudioResult<Option<GuestBuffer>> {
        let mut pending = self.pending_buffers.lock().unwrap();
        if let Some(Some(front_buffer)) = pending.front() {
            if !front_buffer.is_consumed() {
                return Ok(Some(front_buffer.clone()))
            }
        }

        let start = Instant::now();
        let mut locked_regs = self.regs.lock().unwrap();
        if pending.len() == 2 {
            // When we have two pending buffers and receive a request for
            // another, we know that oldest buffer has been completed.
            // However, if that old buffer was an empty buffer we sent
            // because the guest driver had no available buffers, we don't
            // want to mark a buffer complete.
            if let Some(Some(_)) = pending.pop_front() {
                buffer_completed(&mut locked_regs, &self.mem, self.func)?;
                if let Some(Some(front_buffer)) = pending.front() {
                    if !front_buffer.is_consumed() {
                        return Ok(Some(front_buffer.clone()))
                    }
                }
            }
        }

        // We count the number of pending, real buffers at the server, and
        // then use that as our offset from CIV.
        let offset = pending.iter().filter(|e| e.is_some()).count();

        // Get a buffer to respond to our request. If there's no buffer
        // available, we'll wait one buffer interval and check again.
        let buffer = loop {
            if let Some(buffer) = next_guest_buffer(&locked_regs, &self.mem, self.func, offset)?
            {
                break Some(buffer);
            }
            let elapsed = start.elapsed();
            if elapsed > self.message_interval {
                break None;
            }
            locked_regs = self
                .lvi_semaphore
                .wait_timeout(locked_regs, self.message_interval - elapsed)
                .unwrap()
                .0;
        };
        pending.push_back(buffer.clone());
        Ok(buffer)
    }

    // Runs and updates the offset within the stream shm where samples can be
    // found/placed for shm playback/capture streams, respectively
    fn run(&mut self) -> AudioResult<()> {
        let func = self.func;
        // Set up picb.
        {
            let mut locked_regs = self.regs.lock().unwrap();
            locked_regs.func_regs_mut(func).picb =
                current_buffer_size(locked_regs.func_regs(func), &self.mem)? as u16;
        }

        'audio_loop: while self.thread_run.load(Ordering::Relaxed) {
            {
                let mut locked_regs = self.regs.lock().unwrap();
                while locked_regs.func_regs(func).sr & SR_DCH != 0 {
                    locked_regs = self.lvi_semaphore.wait(locked_regs).unwrap();
                    if !self.thread_run.load(Ordering::Relaxed) {
                        break 'audio_loop;
                    }
                }
            }

            let timeout = Duration::from_secs(1);
            let action = self
                .stream
                .wait_for_next_action_with_timeout(timeout)
                .map_err(AudioError::WaitForAction)?;

            let request = match action {
                None => {
                    warn!("No audio message received within timeout of {:?}", timeout);
                    continue;
                }
                Some(request) => request,
            };

            match self.next_guest_buffer()? {
                None => request.ignore_request().map_err(AudioError::RespondRequest)?,
                Some(buffer) => {

                    let addr = buffer.start_address(self.stream.frame_size());

                    let nframes = cmp::min(request.requested_frames(), buffer.remaining_frames());

                    buffer.add_consumed(nframes);
                    request.set_buffer_address_and_frames(addr, nframes)
                        .map_err(AudioError::RespondRequest)?;
                }
            }
        }
        Ok(())
    }
}
