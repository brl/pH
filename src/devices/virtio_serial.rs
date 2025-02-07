use std::io::{self,Write,Read};
use std::thread::spawn;
use termios::*;

use crate::io::{VirtioDevice, VirtioDeviceType, FeatureBits, VirtQueue, ReadableInt, Queues};

const VIRTIO_CONSOLE_F_SIZE: u64 = 0x1;
const VIRTIO_CONSOLE_F_MULTIPORT: u64 = 0x2;

const VIRTIO_CONSOLE_DEVICE_READY: u16  = 0;
const VIRTIO_CONSOLE_DEVICE_ADD: u16    = 1;
const _VIRTIO_CONSOLE_DEVICE_REMOVE: u16 = 2;
const VIRTIO_CONSOLE_PORT_READY: u16    = 3;
const VIRTIO_CONSOLE_CONSOLE_PORT: u16  = 4;
const VIRTIO_CONSOLE_RESIZE: u16        = 5;
const VIRTIO_CONSOLE_PORT_OPEN: u16     = 6;
const _VIRTIO_CONSOLE_PORT_NAME: u16     = 7;

pub struct VirtioSerial {
    features: FeatureBits,
}

impl VirtioSerial {
    pub fn new() -> VirtioSerial {
        let features = FeatureBits::new_default(VIRTIO_CONSOLE_F_MULTIPORT|VIRTIO_CONSOLE_F_SIZE);
        VirtioSerial{
            features,
        }
    }

    fn start_console(&self, q: VirtQueue) {
        spawn(move || {
            loop {
                q.wait_ready().unwrap();
                for mut chain in q.iter() {
                    io::copy(&mut chain, &mut io::stdout()).unwrap();
                    io::stdout().flush().unwrap();
                }
            }
        });
    }

    fn multiport(&self) -> bool {
        self.features.has_guest_bit(VIRTIO_CONSOLE_F_MULTIPORT)
    }
}

use crate::system::ioctl;

#[repr(C)]
#[derive(Default)]
struct WinSz {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

const TIOCGWINSZ: u64 = 0x5413;

impl VirtioDevice for VirtioSerial {
    fn features(&self) -> &FeatureBits {
        &self.features
    }


    fn queue_sizes(&self) -> &[u16] {
        &[
            VirtQueue::DEFAULT_QUEUE_SIZE,
            VirtQueue::DEFAULT_QUEUE_SIZE,
            VirtQueue::DEFAULT_QUEUE_SIZE,
            VirtQueue::DEFAULT_QUEUE_SIZE,
        ]
    }

    fn device_type(&self) -> VirtioDeviceType {
        VirtioDeviceType::Console
    }

    fn config_size(&self) -> usize {
        12
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        if offset == 4 && data.len() == 4 {
            ReadableInt::new_dword(1).read(data);
        } else {
            data.fill(0);
        }
    }

    fn start(&mut self, queues: &Queues) {
        let mut term = Terminal::create(queues.get_queue(0));
        self.start_console(queues.get_queue(1));
        spawn( move || {
            term.read_loop();
        });
        if self.multiport() {
            let mut control = Control::new(queues.get_queue(2), queues.get_queue(3));
            spawn(move || {
                control.run();
            });
        }
    }
}

struct Control {
    rx_vq: VirtQueue,
    tx_vq: VirtQueue,
}

impl Control {
    fn new(rx: VirtQueue, tx: VirtQueue) -> Control {
        Control { rx_vq: rx, tx_vq: tx }
    }

    fn run(&mut self) {
        let mut rx = self.rx_vq.clone();
        self.tx_vq.on_each_chain(|mut chain| {
            let _id = chain.r32().unwrap();
            let event = chain.r16().unwrap();
            let _value = chain.r16().unwrap();
            if event == VIRTIO_CONSOLE_DEVICE_READY {
                Control::send_msg(&mut rx,0, VIRTIO_CONSOLE_DEVICE_ADD, 1).unwrap();
            }
            if event == VIRTIO_CONSOLE_PORT_READY {
                Control::send_msg(&mut rx,0, VIRTIO_CONSOLE_CONSOLE_PORT, 1).unwrap();
                Control::send_msg(&mut rx,0, VIRTIO_CONSOLE_PORT_OPEN, 1).unwrap();
                Control::send_resize(&mut rx, 0).unwrap();
            }
            chain.flush_chain();
        });

    }

    fn send_msg(vq: &mut VirtQueue, id: u32, event: u16, val: u16) -> io::Result<()> {
        let mut chain = vq.wait_next_chain().unwrap();
        chain.w32(id)?;
        chain.w16(event)?;
        chain.w16(val)?;
        chain.flush_chain();
        Ok(())
    }

    fn send_resize(vq: &mut VirtQueue, id: u32) -> io::Result<()> {
        let (cols, rows) = Control::stdin_terminal_size()?;
        let mut chain = vq.wait_next_chain().unwrap();
        chain.w32(id)?;
        chain.w16(VIRTIO_CONSOLE_RESIZE)?;
        chain.w16(0)?;
        chain.w16(rows)?;
        chain.w16(cols)?;
        chain.flush_chain();
        Ok(())
    }

    fn stdin_terminal_size() -> io::Result<(u16, u16)> {
        let mut wsz = WinSz{..Default::default()};
        unsafe {
            if let Err(err) = ioctl::ioctl_with_mut_ref(0, TIOCGWINSZ, &mut wsz) {
                println!("Got error calling TIOCGWINSZ on stdin: {:?}", err);
                return Err(io::Error::last_os_error());
            }
        }
        Ok((wsz.ws_col, wsz.ws_row))
    }

}

struct Terminal {
    saved: Option<Termios>,
    vq: VirtQueue,
}

impl Terminal {
    fn create(vq: VirtQueue) -> Terminal {
        let termios = Termios::from_fd(0).unwrap();
        Terminal {
            saved: Some(termios),
            vq,
        }
    }

    fn setup_term(&self) {
        if let Some(mut termios) = self.saved {
            termios.c_iflag &= !(ICRNL);
            termios.c_lflag &= !(ISIG | ICANON | ECHO);
            let _ = tcsetattr(0, TCSANOW, &termios);
        }
    }
    fn restore_term(&mut self) {
        if let Some(termios) = self.saved.take() {
            let _ = tcsetattr(0, TCSANOW, &termios);
        }
    }

    fn read_loop(&mut self) {
        self.setup_term();
        let mut abort_cnt = 0;
        let mut buf = vec![0u8; 32];
        loop {
            let n = io::stdin().read(&mut buf).unwrap();

            if n > 0 {
                // XXX write_all
                let mut chain = self.vq.wait_next_chain().unwrap();
                chain.write_all(&mut buf[..n]).unwrap();
                chain.flush_chain();
                if n > 1 || buf[0] != 3 {
                    abort_cnt = 0;
                } else {
                    abort_cnt += 1;
                }
            } else {
                println!("n = {}", n);
            }

            if abort_cnt == 3 {
                self.restore_term();
            }

        }

    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.restore_term();
    }
}
