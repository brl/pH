use std::sync::mpsc;
use std::thread;
use pulse::sample::{Format, Spec};
use crate::audio::pulse::context::PulseContext;
use crate::audio::pulse::message::PulseMessageChannel;
use crate::audio::pulse::Result;
use crate::audio::{SampleFormat, StreamDirection};
use crate::audio::shm_streams::{GenericResult, NullShmStream, ShmStream, ShmStreamSource};
use crate::memory::GuestRam;

pub struct PulseClient {
    channel: PulseMessageChannel,
}

impl PulseClient {
    pub fn connect(guest_ram: GuestRam) -> Result<Self> {
        let (tx,rx) = mpsc::channel();

        let _ = thread::spawn(move || {
            let mut ctx = PulseContext::new(guest_ram);
            if let Err(err) = ctx.connect() {
                warn!("PulseAudio Error: {}", err);
            } else {
                ctx.run(rx);
            }
        });
        Ok(PulseClient {
            channel: PulseMessageChannel::new(tx),
        })
    }



    fn create_spec(num_channels: usize, format: SampleFormat, frame_rate: u32) -> Spec {
        let format = match format {
            SampleFormat::U8 => Format::U8,
            SampleFormat::S16LE => Format::S16le,
            SampleFormat::S24LE => Format::S24le,
            SampleFormat::S32LE => Format::S32le,
        };

        Spec {
            format,
            rate: frame_rate,
            channels: num_channels as u8,
        }
    }
}

impl ShmStreamSource for PulseClient {
    fn new_stream(&mut self,
                  direction: StreamDirection,
                  num_channels: usize,
                  format: SampleFormat,
                  frame_rate: u32,
                  buffer_size: usize)-> GenericResult<Box<dyn ShmStream>> {

        if direction != StreamDirection::Playback {
            let stream = NullShmStream::new(buffer_size, num_channels, format, frame_rate);
            return Ok(Box::new(stream))
        }
        let spec = PulseClient::create_spec(num_channels, format, frame_rate);
        let stream = self.channel.send_new_playback_stream(spec,  buffer_size, self.channel.clone())?;
        Ok(Box::new(stream))
    }
}