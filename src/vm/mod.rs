static KERNEL: &[u8] = include_bytes!("../../kernel/ph_linux");
static PHINIT: &[u8] = include_bytes!("../../ph-init/target/release/ph-init");
static SOMMELIER: &[u8] = include_bytes!("../../sommelier/build/sommelier");

pub mod arch;
mod setup;
mod error;
mod kernel_cmdline;
mod config;
mod kvm_vm;
mod vcpu;

pub use config::VmConfig;
pub use setup::VmSetup;
pub use kvm_vm::KvmVm;

pub use self::error::{Result,Error};
pub use arch::ArchSetup;


