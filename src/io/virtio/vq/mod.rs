use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub mod chain;
mod descriptor;
mod splitqueue;
pub mod virtqueue;

///
/// A convenience wrapper around `AtomicUsize`
///
#[derive(Clone)]
pub struct SharedIndex(Arc<AtomicUsize>);

impl SharedIndex {
    fn new() -> SharedIndex {
        SharedIndex(Arc::new(AtomicUsize::new(0)))
    }
    fn get(&self) -> u16 {
        self.0.load(Ordering::SeqCst) as u16
    }
    fn inc(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn set(&self, v: u16) {
        self.0.store(v as usize, Ordering::SeqCst);
    }
}
