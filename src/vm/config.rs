use std::path::{PathBuf, Path};
use crate::vm::{VmSetup, arch};
use std::{env, process};
use crate::devices::SyntheticFS;
use crate::disk::{RawDiskImage, RealmFSImage, OpenType};
use libcitadel::Realms;
use libcitadel::terminal::{TerminalPalette, AnsiTerminal, Base16Scheme};
use crate::vm::arch::X86ArchSetup;

pub struct VmConfig {
    ram_size: usize,
    ncpus: usize,
    verbose: bool,
    rootshell: bool,
    wayland: bool,
    dmabuf: bool,
    network: bool,
    home: String,
    colorscheme: String,
    bridge_name: String,
    kernel_path: Option<PathBuf>,
    init_path: Option<PathBuf>,
    init_cmd: Option<String>,
    raw_disks: Vec<RawDiskImage>,

    realmfs_images: Vec<RealmFSImage>,
    realm_name: Option<String>,
    synthetic: Option<SyntheticFS>,
}

#[allow(dead_code)]
impl VmConfig {
    pub fn new() -> VmConfig {
        let mut config = VmConfig {
            ram_size: 256 * 1024 * 1024,
            ncpus: 1,
            verbose: false,
            rootshell: false,
            wayland: true,
            dmabuf: false,
            network: true,
            bridge_name: "vz-clear".to_string(),
            home: Self::default_homedir(),
            colorscheme: "dracula".to_string(),
            kernel_path: None,
            init_path: None,
            init_cmd: None,
            realm_name: None,
            raw_disks: Vec::new(),
            realmfs_images: Vec::new(),
            synthetic: None,
        };
        config.parse_args();
        config
    }

    fn default_homedir() -> String {
        if let Ok(home) = env::var("HOME") {
            if Path::new(&home).exists() {
                return home;
            }
        }
        String::from("/home/user")
    }

    pub fn ram_size_megs(mut self, megs: usize) -> Self {
        self.ram_size = megs * 1024 * 1024;
        self
    }

    pub fn raw_disk_image<P: Into<PathBuf>>(self, path: P, open_type: OpenType) -> Self {
        self.raw_disk_image_with_offset(path, open_type, 0)
    }

    pub fn raw_disk_image_with_offset<P: Into<PathBuf>>(mut self, path: P, open_type: OpenType, offset: usize) -> Self {
        match RawDiskImage::new_with_offset(path, open_type, offset) {
            Ok(disk) => self.raw_disks.push(disk),
            Err(e) => warn!("Could not add disk: {}", e),
        };
        self
    }

    pub fn realmfs_image<P: Into<PathBuf>>(mut self, path: P) -> Self {
        match RealmFSImage::new(path, OpenType::MemoryOverlay) {
            Ok(disk) => self.realmfs_images.push(disk),
            Err(e) => warn!("Could not add disk: {}", e),
        };
        self
    }

    pub fn num_cpus(mut self, ncpus: usize) -> Self {
        self.ncpus = ncpus;
        self
    }

    pub fn init_cmdline(mut self, val: &str) -> Self {
        self.init_cmd = Some(val.to_owned());
        self
    }

    pub fn kernel_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.kernel_path = Some(path.into());
        self
    }

    pub fn init_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.init_path = Some(path.into());
        self
    }

    pub fn synthetic_fs(mut self, sfs: SyntheticFS) -> Self {
        self.synthetic = Some(sfs);
        self
    }

    pub fn boot(self) {

        let _terminal_restore = TerminalRestore::save();

        if let Some(scheme) = Base16Scheme::by_name(&self.colorscheme) {
            let mut term = AnsiTerminal::new().unwrap();
            if let Err(err) = term.apply_base16(scheme) {
                warn!("Failed to set terminal color scheme: {}", err);
            }
        }
        let mut setup = self.setup();
        let mut vm = match setup.create_vm() {
            Ok(vm) => vm,
            Err(err) => {
                warn!("Failed to create VM: {}", err);
                return;
            }
        };

        if let Err(err) = vm.start() {
            warn!("Failed to start VM: {}", err);
        }
    }

    pub fn setup(self) -> VmSetup<X86ArchSetup> {
        let arch_setup = arch::create_setup(&self);
        VmSetup::new(self, arch_setup)
    }

    pub fn ram_size(&self) -> usize {
        self.ram_size
    }

    pub fn ncpus(&self) -> usize {
        self.ncpus
    }

    pub fn verbose(&self) -> bool {
        self.verbose
    }

    pub fn rootshell(&self) -> bool {
        self.rootshell
    }

    pub fn network(&self) -> bool {
        if unsafe { libc::geteuid() } != 0 {
            false
        } else {
            self.network
        }
    }

    pub fn homedir(&self) -> &str {
        &self.home
    }

    pub fn has_block_image(&self) -> bool {
        !(self.realmfs_images.is_empty() && self.raw_disks.is_empty())
    }

    pub fn get_realmfs_images(&mut self) -> Vec<RealmFSImage> {
        self.realmfs_images.drain(..).collect()
    }

    pub fn get_raw_disk_images(&mut self) -> Vec<RawDiskImage> {
        self.raw_disks.drain(..).collect()
    }

    pub fn get_synthetic_fs(&self) -> Option<SyntheticFS> {
        self.synthetic.clone()
    }

    pub fn get_init_cmdline(&self) -> Option<&str> {
        self.init_cmd.as_ref().map(|s| s.as_str())
    }

    pub fn realm_name(&self) -> Option<&str> {
        self.realm_name.as_ref().map(|s| s.as_str())
    }

    pub fn is_realm(&self) -> bool {
        self.realm_name.is_some()
    }

    pub fn is_wayland_enabled(&self) -> bool {
        if !self.wayland {
            return false;
        }
        let display = env::var("WAYLAND_DISPLAY").unwrap_or("wayland-0".to_string());
        let xdg_runtime = env::var("XDG_RUNTIME_DIR").unwrap_or("/run/user/1000".to_string());

        let socket= Path::new(xdg_runtime.as_str()).join(display);
        socket.exists()
    }

    pub fn is_dmabuf_enabled(&self) -> bool {
        self.dmabuf
    }

    pub fn bridge(&self) -> &str {
        &self.bridge_name
    }

    fn add_realmfs_by_name(&mut self, realmfs: &str) {
        let path = Path::new("/realms/realmfs-images")
            .join(format!("{}-realmfs.img", realmfs));
        if !path.exists() {
            eprintln!("Realmfs image does not exist at {}", path.display());
            process::exit(1);
        }
        match RealmFSImage::new(path, OpenType::MemoryOverlay) {
            Ok(disk) => self.realmfs_images.push(disk),
            Err(e) => {
                warn!("Could not add disk: {}", e);
                process::exit(1);
            },
        };
    }

    fn add_realm_by_name(&mut self, realm: &str) {
        let realms = Realms::load().unwrap();
        if let Some(realm) = realms.by_name(realm) {
            let config = realm.config();
            let realmfs = config.realmfs();
            self.add_realmfs_by_name(realmfs);
            self.home = realm.base_path().join("home").display().to_string();
            self.realm_name = Some(realm.name().to_string());
            self.bridge_name = format!("vz-{}", config.network_zone());
            if let Some(scheme) = config.terminal_scheme() {
                self.colorscheme = scheme.to_string();
            }
        }
    }

    fn parse_args(&mut self) {
        let args = ProgramArgs::new();
        if args.has_arg("-v") {
            self.verbose = true;
        }
        if args.has_arg("--root") {
            self.rootshell = true;
        }
        if args.has_arg("--no-wayland") {
            self.wayland = false;
            self.dmabuf = false;
        }
        if args.has_arg("--use-dmabuf") {
            self.dmabuf = true;
        }
        if args.has_arg("--no-network") {
            self.network = false;
        }
        if let Some(home) = args.arg_with_value("--home") {
            self.home = home.to_string();
        }
        if let Some(realmfs) = args.arg_with_value("--realmfs") {
            self.add_realmfs_by_name(realmfs);
        }
        if let Some(realm) = args.arg_with_value("--realm") {
            self.add_realm_by_name(realm);
        }
    }
}

struct ProgramArgs {
    args: Vec<String>,
}

impl ProgramArgs {
    fn new() -> Self {
        ProgramArgs {
            args: env::args().skip(1).collect(),
        }
    }

    fn has_arg(&self, name: &str) -> bool {
        self.args.iter().any(|arg| arg.as_str() == name)
    }

    fn arg_with_value(&self, name: &str) -> Option<&str> {
        let mut iter = self.args.iter();
        while let Some(arg) = iter.next() {
            if arg.as_str() == name {
                match iter.next() {
                    Some(val) => return Some(val.as_str()),
                    None => {
                        eprintln!("Expected value for {} argument", name);
                        process::exit(1);
                    }
                }
            }
        }
        None
    }
}

pub struct TerminalRestore {
    saved: Option<TerminalPalette>,
}

impl TerminalRestore {
    pub fn save() -> Self {
        let mut term = match AnsiTerminal::new() {
            Ok(term) => term,
            Err(e) => {
                warn!("failed to open terminal: {}", e);
                return TerminalRestore { saved: None }
            }
        };

        let mut palette = TerminalPalette::default();
        if let Err(e) = palette.load(&mut term) {
            warn!("failed to load palette: {}", e);
            return TerminalRestore { saved: None }
        }
        if let Err(e) = term.clear_screen() {
            warn!("failed to clear screen: {}", e);
            return TerminalRestore { saved: None }
        }
        TerminalRestore { saved: Some(palette) }
    }

    fn restore(&self) {
        if let Some(p) = self.saved.as_ref() {
            let mut term = match AnsiTerminal::new() {
                Ok(term) => term,
                _ => return,
            };
            let _ = p.apply(&mut term);
        }
    }
}

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        self.restore();
    }
}
