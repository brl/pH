use std::{error, fmt};
use std::fmt::Display;
use std::str::FromStr;
use thiserror::Error;

pub mod shm_streams;
pub mod pulse;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    U8,
    S16LE,
    S24LE,
    S32LE,
}

impl SampleFormat {
    pub fn sample_bytes(self) -> usize {
        use SampleFormat::*;
        match self {
            U8 => 1,
            S16LE => 2,
            S24LE => 4, // Not a typo, S24_LE samples are stored in 4 byte chunks.
            S32LE => 4,
        }
    }
}

impl Display for SampleFormat {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use SampleFormat::*;
        match self {
            U8 => write!(f, "Unsigned 8 bit"),
            S16LE => write!(f, "Signed 16 bit Little Endian"),
            S24LE => write!(f, "Signed 24 bit Little Endian"),
            S32LE => write!(f, "Signed 32 bit Little Endian"),
        }
    }
}

impl FromStr for SampleFormat {
    type Err = SampleFormatError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "U8" => Ok(SampleFormat::U8),
            "S16_LE" => Ok(SampleFormat::S16LE),
            "S24_LE" => Ok(SampleFormat::S24LE),
            "S32_LE" => Ok(SampleFormat::S32LE),
            _ => Err(SampleFormatError::InvalidSampleFormat),
        }
    }
}

/// Errors that are possible from a `SampleFormat`.
#[derive(Error, Debug)]
pub enum SampleFormatError {
    #[error("Must be in [U8, S16_LE, S24_LE, S32_LE]")]
    InvalidSampleFormat,
}
/// Valid directions of an audio stream.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StreamDirection {
    Playback,
    Capture,
}

/// Valid effects for an audio stream.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StreamEffect {
    NoEffect,
    EchoCancellation,
}

impl Default for StreamEffect {
    fn default() -> Self {
        StreamEffect::NoEffect
    }
}

/// Errors that can pass across threads.
pub type BoxError = Box<dyn error::Error + Send + Sync>;

/// Errors that are possible from a `StreamEffect`.
#[derive(Error, Debug)]
pub enum StreamEffectError {
    #[error("Must be in [EchoCancellation, aec]")]
    InvalidEffect,
}

impl FromStr for StreamEffect {
    type Err = StreamEffectError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "EchoCancellation" | "aec" => Ok(StreamEffect::EchoCancellation),
            _ => Err(StreamEffectError::InvalidEffect),
        }
    }
}

/// `StreamControl` provides a way to set the volume and mute states of a stream. `StreamControl`
/// is separate from the stream so it can be owned by a different thread if needed.
pub trait StreamControl: Send + Sync {
    fn set_volume(&mut self, _scaler: f64) {}
    fn set_mute(&mut self, _mute: bool) {}
}

/// `BufferCommit` is a cleanup funcion that must be called before dropping the buffer,
/// allowing arbitrary code to be run after the buffer is filled or read by the user.
pub trait BufferCommit {
    /// `write_playback_buffer` or `read_capture_buffer` would trigger this automatically. `nframes`
    /// indicates the number of audio frames that were read or written to the device.
    fn commit(&mut self, nframes: usize);
}