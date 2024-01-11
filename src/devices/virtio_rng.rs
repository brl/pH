
use std::thread;
use std::fs::File;
use crate::io::{FeatureBits, Queues, VirtioDevice, VirtioDeviceType, VirtQueue};

pub struct VirtioRandom {
    features: FeatureBits,
}

impl VirtioRandom {
    pub fn new() -> VirtioRandom {
        VirtioRandom {
            features: FeatureBits::new_default(0),
        }
    }
}

fn run(q: VirtQueue) {
    let mut random = File::open("/dev/urandom").unwrap();

    loop {
        q.on_each_chain(|mut chain| {
            while !chain.is_end_of_chain() {
                let _ = chain.copy_from_reader(&mut random, 256).unwrap();
            }
        });
    }
}

impl VirtioDevice for VirtioRandom {
    fn features(&self) -> &FeatureBits {
        &self.features
    }

    fn queue_sizes(&self) -> &[u16] {
        &[VirtQueue::DEFAULT_QUEUE_SIZE]
    }

    fn device_type(&self) -> VirtioDeviceType {
        VirtioDeviceType::Rng
    }

    fn start(&mut self, queues: &Queues) {
        let vq = queues.get_queue(0);
        thread::spawn(move|| {
            run(vq)
        });
    }
}