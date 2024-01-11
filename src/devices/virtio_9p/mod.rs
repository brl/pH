use std::thread;

use std::path::{PathBuf, Path};
use vm_memory::GuestMemoryMmap;

use crate::devices::virtio_9p::server::Server;
use crate::devices::virtio_9p::filesystem::{FileSystem, FileSystemOps};
use self::pdu::PduParser;

mod pdu;
mod file;
mod directory;
mod filesystem;
mod server;
mod synthetic;

const VIRTIO_9P_MOUNT_TAG: u64 = 0x1;

pub use synthetic::SyntheticFS;
use crate::io::{FeatureBits, Queues, VirtioDevice, VirtioDeviceType, VirtQueue};

pub struct VirtioP9<T: FileSystemOps> {
    filesystem: T,
    root_dir: PathBuf,
    features: FeatureBits,
    debug: bool,
    config: Vec<u8>,
}

impl <T: FileSystemOps+'static> VirtioP9<T> {
    fn create_config(tag_name: &str) -> Vec<u8> {
        let tag_len = tag_name.len() as u16;
        let mut config = Vec::with_capacity(tag_name.len() + 3);
        config.push(tag_len as u8);
        config.push((tag_len >> 8) as u8);
        config.append(&mut tag_name.as_bytes().to_vec());
        config.push(0);
        config
    }

    pub fn new(filesystem: T, tag_name: &str, root_dir: &str, debug: bool) -> Self {
        VirtioP9 {
            filesystem,
            root_dir: PathBuf::from(root_dir),
            features: FeatureBits::new_default(VIRTIO_9P_MOUNT_TAG),
            debug,
            config: VirtioP9::<T>::create_config(tag_name),
        }
    }

}

impl VirtioP9<FileSystem> {
    pub fn new_filesystem(tag_name: &str, root_dir: &str, read_only: bool, debug: bool) -> Self {
        let filesystem = FileSystem::new(PathBuf::from(root_dir), read_only);
        Self::new(filesystem, tag_name, root_dir, debug)
    }
}

impl <T: FileSystemOps+'static> VirtioDevice for VirtioP9<T> {
    fn features(&self) -> &FeatureBits {
        &self.features
    }

    fn queue_sizes(&self) -> &[u16] {
        &[VirtQueue::DEFAULT_QUEUE_SIZE]
    }

    fn device_type(&self) -> VirtioDeviceType {
        VirtioDeviceType::NineP
    }

    fn config_size(&self) -> usize {
        self.config.len()
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        let offset = offset as usize;
        if offset + data.len() <= self.config.len() {
            data.copy_from_slice(&self.config[offset..offset+data.len()])
        }
    }

    fn start(&mut self, queues: &Queues) {
        let vq = queues.get_queue(0);
        let root_dir = self.root_dir.clone();
        let filesystem = self.filesystem.clone();
        let memory = queues.guest_memory().clone();
        let debug = self.debug;
        thread::spawn(move || run_device(memory, vq, &root_dir, filesystem, debug));
    }
}

fn run_device<T: FileSystemOps>(memory: GuestMemoryMmap, vq: VirtQueue, root_dir: &Path, filesystem: T, debug: bool) {
    let mut server = Server::new(&root_dir, filesystem);

    if debug {
        server.enable_debug();
    }

    vq.on_each_chain(|mut chain| {
        let mut pp = PduParser::new(&mut chain, memory.clone());
        server.handle(&mut pp);
    });
}

