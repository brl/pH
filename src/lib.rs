#[macro_use]
extern crate lazy_static;
#[macro_use]
mod system;
#[macro_use]
pub mod util;
mod vm;
mod devices;
mod disk;
mod io;
mod audio;

pub use util::{Logger,LogLevel};
pub use vm::VmConfig;
