use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;
use pulse::sample::Spec;
use pulse::stream::{FlagSet, SeekMode, State, Stream};
use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};
use crate::audio::pulse::{PulseError,Result};
use crate::audio::pulse::context::PulseContext;
use crate::audio::pulse::message::PulseMessageChannel;
use crate::audio::shm_streams::{BufferSet, GenericResult, ServerRequest, ShmStream};
struct Available {
    byte_count: Mutex<usize>,
    cond: Condvar,
}

impl Available {
    fn new() -> Self {
        Available {
            byte_count: Mutex::new(0),
            cond: Condvar::new(),
        }
    }

    fn byte_count_lock(&self) -> MutexGuard<usize> {
        self.byte_count.lock().unwrap()
    }

    fn update(&self, value: usize) {
        let mut byte_count = self.byte_count_lock();
        *byte_count = value;
        self.cond.notify_one();
    }

    fn decrement(&self, amount: usize) {
        let mut byte_count = self.byte_count_lock();
        *byte_count -= amount;
    }

    fn wait_space(&self, timeout: Duration) -> Option<usize> {
        let mut byte_count = self.byte_count_lock();
        while *byte_count == 0 {
            let (new_lock, wt_result) = self.cond.wait_timeout(byte_count, timeout).unwrap();
            if wt_result.timed_out() {
                return None;
            }
            byte_count = new_lock;
        }
        Some(*byte_count)
    }
}

pub struct PulseStream {
    spec: Spec,
    buffer_size: usize,
    guest_memory: GuestMemoryMmap,
    stream: Arc<Mutex<Stream>>,
    avail: Arc<Available>,
    channel: PulseMessageChannel,
}

impl PulseStream {

    fn stream_connected_finish(&mut self, ctx: &PulseContext) {
        self.stream().set_state_callback(None);
        ctx.mainloop_unlock();
    }

    fn wait_stream_connected(&mut self, ctx: &PulseContext) -> Result<()> {
        loop {
            let state = self.stream().get_state();
            if state == State::Ready {
                break;
            } else if !state.is_good() {
                return Err(PulseError::StreamConnectFailed);
            }
            ctx.mainloop_wait();
        }
        Ok(())
    }

    pub fn connect(&mut self, ctx: &PulseContext) -> Result<()> {
        ctx.mainloop_lock();

        self.stream().set_state_callback(Some(Box::new({
            let ml_ref = ctx.mainloop();
            move || unsafe {
                (*ml_ref.as_ptr()).signal(false);
            }
        })));


        if let Err(err) = self.stream().connect_playback(
            None,
            None,
            FlagSet::NOFLAGS,
            None,
            None) {
            self.stream().set_state_callback(None);
            ctx.mainloop_unlock();
            return Err(PulseError::StreamConnect(err))
        }

        let result = self.wait_stream_connected(ctx);
        self.stream_connected_finish(ctx);
        result
    }

    pub fn new_playback(mut stream: Stream, guest_memory: GuestMemoryMmap, spec: Spec, buffer_size: usize, channel: PulseMessageChannel) -> Self {
        let avail = Arc::new(Available::new());

        stream.set_write_callback(Some(Box::new({
            let avail = avail.clone();
            move |writeable_bytes| {
                avail.update(writeable_bytes);
            }
        })));

        let stream = Arc::new(Mutex::new(stream));
        PulseStream {
            spec,
            buffer_size,
            guest_memory,
            avail,
            stream,
            channel,
        }
    }

    fn stream(&self) -> MutexGuard<Stream> {
        self.stream.lock().unwrap()
    }

    fn uncork(&self) -> GenericResult<()> {
        self.channel.send_mainloop_lock()?;
        if self.stream().is_corked().unwrap() {
            self.stream().uncork(None);
        }
        self.channel.send_mainloop_unlock()?;
        Ok(())
    }
}

impl ShmStream for PulseStream {
    fn frame_size(&self) -> usize {
        self.spec.frame_size()
    }

    fn num_channels(&self) -> usize {
        self.spec.channels as usize
    }

    fn frame_rate(&self) -> u32 {
        self.spec.rate
    }

    fn wait_for_next_action_with_timeout(&self, timeout: Duration) -> GenericResult<Option<ServerRequest>> {
        if let Some(bytes) = self.avail.wait_space(timeout) {
            let frames = bytes / self.frame_size();
            let req = frames.min(self.buffer_size);
            return Ok(Some(ServerRequest::new(req, self)))
        }
        Ok(None)
    }
}

impl BufferSet for PulseStream {
    fn callback(&self, address: u64, frames: usize) -> GenericResult<()> {
        self.uncork()?;
        let mut buffer = vec![0u8; frames * self.frame_size()];
        self.guest_memory.read_slice(&mut buffer, GuestAddress(address))?;

        self.channel.send_mainloop_lock()?;
        self.stream().write_copy(&buffer, 0, SeekMode::Relative)?;
        self.channel.send_mainloop_unlock()?;
        self.avail.decrement(buffer.len());
        Ok(())
    }

    fn ignore(&self) -> GenericResult<()> {
        info!("Request ignored...");
        Ok(())
    }
}