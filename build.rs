use std::path::{Path, PathBuf};
use std::env;
use std::io::Result;
use std::process::Command;

fn main() -> Result<()> {
    build_phinit()?;
    build_kernel()?;
    build_sommelier()?;
    // Rerun build.rs upon making or pulling in new commits
    println!("cargo:rerun-if-changed=.git/refs/heads/master");
    println!("cargo:rerun-if-changed=ph-init/src");
    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}

fn build_phinit() -> Result<()> {
    let _dir = ChdirTo::path("ph-init");

    Command::new("cargo")
        .arg("build")
        .arg("--release")
        .status()?;

    Command::new("strip")
        .arg("target/release/ph-init")
        .status()?;

    Ok(())
}

fn build_kernel() -> Result<()> {
    Command::new("make")
        .arg("-C")
        .arg("kernel")
        .status()?;

    Ok(())
}

fn build_sommelier() -> Result<()> {
    Command::new("meson")
        .arg("setup")
        .arg("-Dxwayland_path=/usr/bin/Xwayland")
        .arg("-Dxwayland_gl_driver_path=")
        .arg("-Dwith_tests=false")
        .arg("sommelier/build")
        .arg("sommelier")
        .status()?;

    Command::new("meson")
        .arg("compile")
        .arg("-C")
        .arg("sommelier/build")
        .status()?;
    Ok(())
}

struct ChdirTo {
    saved: PathBuf,
}

impl ChdirTo {
    fn path<P: AsRef<Path>>(directory: P) -> ChdirTo {
        let saved = env::current_dir()
            .expect("current_dir()");
        env::set_current_dir(directory.as_ref())
            .expect("set_current_dir()");
        ChdirTo { saved }
    }
}

impl Drop for ChdirTo {
    fn drop(&mut self) {
        env::set_current_dir(&self.saved)
            .expect("restore current dir");
    }
}

