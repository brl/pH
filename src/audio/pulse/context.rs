use std::cell::RefCell;
use std::ops::DerefMut;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use pulse::context::{Context, FlagSet, State};
use pulse::mainloop::threaded::Mainloop;
use pulse::proplist::{properties, Proplist};
use pulse::sample::Spec;
use pulse::stream::Stream;
use crate::memory::GuestRam;
use crate::audio::pulse::{Result, PulseError, PulseStream};
use crate::audio::pulse::message::{PulseContextMessage, PulseContextRequest, PulseMessageChannel};

pub struct PulseContext {
    guest_ram: GuestRam,
    mainloop: Rc<RefCell<Mainloop>>,
    context: Rc<RefCell<Context>>,
}

impl PulseContext {
    pub fn mainloop_lock(&self) {
        self.mainloop.borrow_mut().lock();
    }

    pub fn mainloop_unlock(&self) {
        self.mainloop.borrow_mut().unlock();
    }

    pub fn mainloop_wait(&self) {
        self.mainloop.borrow_mut().wait();
    }

    pub fn mainloop(&self) -> Rc<RefCell<Mainloop>> {
        self.mainloop.clone()
    }

    pub fn new(guest_ram: GuestRam) -> Self {
        let mainloop = Mainloop::new()
            .expect("Failed to create a pulseaudio mainloop");

        let mut proplist = Proplist::new()
            .expect("Failed to create pulseaudio proplist");

        proplist.set_str(properties::APPLICATION_NAME, "pH")
            .expect("Failed to set pulseaudio property");

        let context = Context::new_with_proplist(
            &mainloop,
            "pHContext",
            &proplist
        ).expect("Failed to create a pulseaudio context");

        PulseContext {
            guest_ram,
            mainloop: Rc::new(RefCell::new(mainloop)),
            context: Rc::new(RefCell::new(context)),
        }
    }

    fn start_and_connect(&self) -> Result<()> {
        self.mainloop_lock();

        self.context.borrow_mut().set_state_callback(Some(Box::new({
            let ml_ref = self.mainloop.clone();
            move || unsafe {
                (*ml_ref.as_ptr()).signal(false);
            }
        })));

        self.context.borrow_mut().connect(None, FlagSet::NOFLAGS, None)
            .map_err(PulseError::ConnectFailed)?;

        self.mainloop.borrow_mut().start()
            .map_err(PulseError::StartFailed)?;

        Ok(())
    }

    fn wait_context_connected(&self) -> Result<()> {
        loop {
            let st = self.context.borrow().get_state();
            if st == State::Ready {
                break;
            } else if !st.is_good() {
                return Err(PulseError::ConnectFailedErr)
            }
            self.mainloop.borrow_mut().wait();
        }
        Ok(())
    }

    fn context_connect_finish(&self) {
        self.context.borrow_mut().set_state_callback(None);
        self.mainloop_unlock();
    }

    pub fn connect(&self) -> Result<()> {
        let result = self.start_and_connect().and_then(|()| {
            self.wait_context_connected()
        });
        self.context_connect_finish();
        result
    }

    fn new_playback_stream(&self, spec: Spec, buffer_size: usize, channel: PulseMessageChannel) -> PulseStream {
        self.mainloop_lock();

        let stream = Stream::new(self.context.borrow_mut().deref_mut(),
                                                   "ph-pa-playback",
                                                   &spec,
                                                   None)
                .expect("Failed to create pulseaudio stream");

        let ps = PulseStream::new_playback(stream, self.guest_ram.clone(), spec, buffer_size, channel);
        self.mainloop_unlock();
        ps
    }

    pub fn run(&mut self, receiver: Receiver<PulseContextMessage>) {
        loop {
            match receiver.recv() {
                Ok(msg) => self.dispatch_message(msg),
                Err(_) => break,
            }
        }
    }

    fn dispatch_message(&mut self, msg: PulseContextMessage) {
        match msg.request() {
            PulseContextRequest::MainloopLock => {
                self.mainloop_lock();
                msg.respond_ok();
            }
            PulseContextRequest::MainloopUnlock => {
                self.mainloop_unlock();
                msg.respond_ok();
            }
            PulseContextRequest::NewPlaybackStream {spec, buffer_size, channel} => {
                let mut ps = self.new_playback_stream(*spec, *buffer_size, channel.clone());
                match ps.connect(self) {
                    Ok(()) => msg.respond_stream(ps),
                    Err(err) => msg.respond_err(err),
                }
            }
        }
    }
}

