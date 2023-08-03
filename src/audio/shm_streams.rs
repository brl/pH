// Copyright 2019 The ChromiumOS Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::os::unix::io::RawFd;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use thiserror::Error;
use crate::audio::{BoxError, SampleFormat, StreamDirection};

pub(crate) type GenericResult<T> = Result<T, BoxError>;

/// `BufferSet` is used as a callback mechanism for `ServerRequest` objects.
/// It is meant to be implemented by the audio stream, allowing arbitrary code
/// to be run after a buffer address and length is set.
pub trait BufferSet {
    /// Called when the client sets a buffer address and length.
    ///
    /// `address` is the guest address of the buffer and `frames`
    /// indicates the number of audio frames that can be read from or written to
    /// the buffer.
    fn callback(&self, address: u64, frames: usize) -> GenericResult<()>;

    /// Called when the client ignores a request from the server.
    fn ignore(&self) -> GenericResult<()>;
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Provided number of frames {0} exceeds requested number of frames {1}")]
    TooManyFrames(usize, usize),
}

/// `ServerRequest` represents an active request from the server for the client
/// to provide a buffer in shared memory to playback from or capture to.
pub struct ServerRequest<'a> {
    requested_frames: usize,
    buffer_set: &'a dyn BufferSet,
}

impl<'a> ServerRequest<'a> {
    /// Create a new ServerRequest object
    ///
    /// Create a ServerRequest object representing a request from the server
    /// for a buffer `requested_frames` in size.
    ///
    /// When the client responds to this request by calling
    /// [`set_buffer_address_and_frames`](ServerRequest::set_buffer_address_and_frames),
    /// BufferSet::callback will be called on `buffer_set`.
    ///
    /// # Arguments
    /// * `requested_frames` - The requested buffer size in frames.
    /// * `buffer_set` - The object implementing the callback for when a buffer is provided.
    //pub fn new<D: BufferSet>(requested_frames: usize, buffer_set: &'a mut D) -> Self {
    pub fn new<D: BufferSet>(requested_frames: usize, buffer_set: &'a D) -> Self {
        Self {
            requested_frames,
            buffer_set,
        }
    }

    /// Get the number of frames of audio data requested by the server.
    ///
    /// The returned value should never be greater than the `buffer_size`
    /// given in [`new_stream`](ShmStreamSource::new_stream).
    pub fn requested_frames(&self) -> usize {
        self.requested_frames
    }

    /// Sets the buffer address and length for the requested buffer.
    ///
    /// Sets the buffer address and length of the buffer that fulfills this
    /// server request to `address` and `length`, respectively. This means that
    /// `length` bytes of audio samples may be read from/written to that
    /// location in `client_shm` for a playback/capture stream, respectively.
    /// This function may only be called once for a `ServerRequest`, at which
    /// point the ServerRequest is dropped and no further calls are possible.
    ///
    /// # Arguments
    ///
    /// * `address` - The value to use as the address for the next buffer.
    /// * `frames` - The length of the next buffer in frames.
    ///
    /// # Errors
    ///
    /// * If `frames` is greater than `requested_frames`.
    pub fn set_buffer_address_and_frames(self, address: u64, frames: usize) -> GenericResult<()> {
        if frames > self.requested_frames {
            return Err(Box::new(Error::TooManyFrames(
                frames,
                self.requested_frames,
            )));
        }

        self.buffer_set.callback(address, frames)
    }

    /// Ignore this request
    ///
    /// If the client does not intend to respond to this ServerRequest with a
    /// buffer, they should call this function. The stream will be notified that
    /// the request has been ignored and will handle it properly.
    pub fn ignore_request(self) -> GenericResult<()> {
        self.buffer_set.ignore()
    }
}

/// `ShmStream` allows a client to interact with an active CRAS stream.
pub trait ShmStream: Send {
    /// Get the size of a frame of audio data for this stream.
    fn frame_size(&self) -> usize;

    /// Get the number of channels of audio data for this stream.
    fn num_channels(&self) -> usize;

    /// Get the frame rate of audio data for this stream.
    fn frame_rate(&self) -> u32;

    /// Waits until the next server message indicating action is required.
    ///
    /// For playback streams, this will be `AUDIO_MESSAGE_REQUEST_DATA`, meaning
    /// that we must set the buffer address to the next location where playback
    /// data can be found.
    /// For capture streams, this will be `AUDIO_MESSAGE_DATA_READY`, meaning
    /// that we must set the buffer address to the next location where captured
    /// data can be written to.
    /// Will return early if `timeout` elapses before a message is received.
    ///
    /// # Arguments
    ///
    /// * `timeout` - The amount of time to wait until a message is received.
    ///
    /// # Return value
    ///
    /// Returns `Some(request)` where `request` is an object that implements the
    /// [`ServerRequest`](ServerRequest) trait and which can be used to get the
    /// number of bytes requested for playback streams or that have already been
    /// written to shm for capture streams.
    ///
    /// If the timeout occurs before a message is received, returns `None`.
    ///
    /// # Errors
    ///
    /// * If an invalid message type is received for the stream.
    fn wait_for_next_action_with_timeout(
        &self,
        timeout: Duration,
    ) -> GenericResult<Option<ServerRequest>>;
}

/// `SharedMemory` specifies features of shared memory areas passed on to `ShmStreamSource`.
pub trait SharedMemory {
    type Error: std::error::Error;

    /// Creates a new shared memory file descriptor without specifying a name.
    fn anon(size: u64) -> Result<Self, Self::Error>
    where
        Self: Sized;

    /// Gets the size in bytes of the shared memory.
    ///
    /// The size returned here does not reflect changes by other interfaces or users of the shared
    /// memory file descriptor..
    fn size(&self) -> u64;

    /// Returns the underlying raw fd.
    #[cfg(unix)]
    fn as_raw_fd(&self) -> RawFd;
}

/// `ShmStreamSource` creates streams for playback or capture of audio.
pub trait ShmStreamSource: Send {
    /// Creates a new [`ShmStream`](ShmStream)
    ///
    /// Creates a new `ShmStream` object, which allows:
    /// * Waiting until the server has communicated that data is ready or
    ///   requested that we make more data available.
    /// * Setting the location and length of buffers for reading/writing audio data.
    ///
    /// # Arguments
    ///
    /// * `direction` - The direction of the stream, either `Playback` or `Capture`.
    /// * `num_channels` - The number of audio channels for the stream.
    /// * `format` - The audio format to use for audio samples.
    /// * `frame_rate` - The stream's frame rate in Hz.
    /// * `buffer_size` - The maximum size of an audio buffer. This will be the
    ///                   size used for transfers of audio data between client
    ///                   and server.
    ///
    /// # Errors
    ///
    /// * If sending the connect stream message to the server fails.
    fn new_stream(
        &mut self,
        direction: StreamDirection,
        num_channels: usize,
        format: SampleFormat,
        frame_rate: u32,
        buffer_size: usize,
    ) -> GenericResult<Box<dyn ShmStream>>;
}

/// Class that implements ShmStream trait but does nothing with the samples
pub struct NullShmStream {
    num_channels: usize,
    frame_rate: u32,
    buffer_size: usize,
    frame_size: usize,
    interval: Duration,
    next_frame: Mutex<Duration>,
    start_time: Instant,
}

impl NullShmStream {
    /// Attempt to create a new NullShmStream with the given number of channels,
    /// format, frame_rate, and buffer_size.
    pub fn new(
        buffer_size: usize,
        num_channels: usize,
        format: SampleFormat,
        frame_rate: u32,
    ) -> Self {
        let interval = Duration::from_millis(buffer_size as u64 * 1000 / frame_rate as u64);
        Self {
            num_channels,
            frame_rate,
            buffer_size,
            frame_size: format.sample_bytes() * num_channels,
            interval,
            next_frame: Mutex::new(interval),
            start_time: Instant::now(),
        }
    }
}

impl BufferSet for NullShmStream {
    fn callback(&self, _address: u64, _frames: usize) -> GenericResult<()> {
        Ok(())
    }

    fn ignore(&self) -> GenericResult<()> {
        Ok(())
    }
}

impl ShmStream for NullShmStream {
    fn frame_size(&self) -> usize {
        self.frame_size
    }

    fn num_channels(&self) -> usize {
        self.num_channels
    }

    fn frame_rate(&self) -> u32 {
        self.frame_rate
    }

    fn wait_for_next_action_with_timeout(
        &self,
        timeout: Duration,
    ) -> GenericResult<Option<ServerRequest>> {
        let elapsed = self.start_time.elapsed();
        let mut next_frame = self.next_frame.lock().unwrap();
        if elapsed < *next_frame {
            if timeout < *next_frame - elapsed {
                std::thread::sleep(timeout);
                return Ok(None);
            } else {
                std::thread::sleep(*next_frame - elapsed);
            }
        }
        *next_frame += self.interval;
        Ok(Some(ServerRequest::new(self.buffer_size, self)))
    }
}
