use std::result;
use pulse::error::PAErr;

mod client;
mod context;
mod message;
mod stream;

pub type Result<T> = result::Result<T, PulseError>;

#[derive(thiserror::Error, Debug)]
pub enum PulseError {
    #[error("connection to pulseaudio failed: {0}")]
    ConnectFailed(PAErr),
    #[error("failed to connect to pulseaudio server")]
    ConnectFailedErr,
    #[error("failed to start pulseaudio mainloop: {0}")]
    StartFailed(PAErr),
    #[error("failed to connect pulseaudio stream: {0}")]
    StreamConnect(PAErr),
    #[error("stream connect failed")]
    StreamConnectFailed,
    #[error("failed to send channel message")]
    SendMessageFailed,
    #[error("failed to receive channel response message")]
    RecvMessageFailed,
    #[error("unexpected response to channel message")]
    UnexpectedResponse,
}

pub use stream::PulseStream;
pub use client::PulseClient;