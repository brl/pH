use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use pulse::sample::Spec;
use crate::audio::pulse::{PulseError, PulseStream, Result};
use crate::audio::pulse::PulseError::UnexpectedResponse;

pub enum PulseContextRequest {
    MainloopLock,
    MainloopUnlock,
    NewPlaybackStream {
        spec: Spec,
        buffer_size: usize,
        channel: PulseMessageChannel,
    },
}

pub enum PulseContextResponse {
    ResponseOk,
    ResponseError(PulseError),
    ResponseStream(PulseStream),
}

pub struct PulseContextMessage {
    request: PulseContextRequest,
    response_channel: Sender<PulseContextResponse>,
}

impl PulseContextMessage {

    pub fn new(request: PulseContextRequest) -> (Self, Receiver<PulseContextResponse>) {
        let (tx,rx) = mpsc::channel();
        let msg = PulseContextMessage {
            request,
            response_channel: tx,
        };
        (msg, rx)
    }

    pub fn request(&self) -> &PulseContextRequest {
        &self.request
    }

    fn send_response(&self, response: PulseContextResponse) {
        if let Err(err) = self.response_channel.send(response) {
            warn!("PulseAudio: Error sending message response: {}", err);
        }
    }

    pub fn respond_ok(&self) {
        self.send_response(PulseContextResponse::ResponseOk)
    }

    pub fn respond_err(&self, err: PulseError) {
        self.send_response(PulseContextResponse::ResponseError(err))
    }

    pub fn respond_stream(&self, stream: PulseStream) {
        self.send_response(PulseContextResponse::ResponseStream(stream));
    }
}

#[derive(Clone)]
pub struct PulseMessageChannel {
    sender: Sender<PulseContextMessage>
}

impl PulseMessageChannel {
    pub fn new(sender: Sender<PulseContextMessage>) -> Self {
        PulseMessageChannel { sender }
    }

    fn exchange_message(&self, req: PulseContextRequest) -> Result<PulseContextResponse> {
        let (msg, rx) = PulseContextMessage::new(req);
        self.sender.send(msg).map_err(|_| PulseError::SendMessageFailed)?;
        let resp = rx.recv().map_err(|_| PulseError::RecvMessageFailed)?;
        Ok(resp)
    }

    fn send_expect_ok(&self, req: PulseContextRequest) -> Result<()> {
        let (msg, rx) = PulseContextMessage::new(req);
        self.sender.send(msg).map_err(|_| PulseError::SendMessageFailed)?;
        let response = rx.recv().map_err(|_| PulseError::RecvMessageFailed)?;
        if let PulseContextResponse::ResponseError(err) = response {
            return Err(err);
        }
        Ok(())
    }

    pub fn send_mainloop_lock(&self) -> Result<()> {
        self.send_expect_ok(PulseContextRequest::MainloopLock)
    }

    pub fn send_mainloop_unlock(&self) -> Result<()> {
        self.send_expect_ok(PulseContextRequest::MainloopUnlock)
    }

    pub fn send_new_playback_stream(&self, spec: Spec, buffer_size: usize, channel: PulseMessageChannel) -> Result<PulseStream> {
        match self.exchange_message(PulseContextRequest::NewPlaybackStream { spec, buffer_size, channel})? {
            PulseContextResponse::ResponseOk => Err(UnexpectedResponse),
            PulseContextResponse::ResponseError(err) => Err(err),
            PulseContextResponse::ResponseStream(stream) => Ok(stream),
        }
    }
}