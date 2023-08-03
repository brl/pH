use std::fs;
use crate::{Error, sys, warn};
use crate::error::Result;
use std::path::Path;

const DAEMON_CONF: &str = r#"
log-target = file:/tmp/pulseaudio.log
log-level = debug
"#;

// If extra-arguments is not set, pulseaudio will be launched with
// '--log-target=syslog' which overrides the log settings in daemon.conf
const CLIENT_CONF: &str = r#"
autospawn = yes
daemon-binary = /usr/bin/pulseaudio
extra-arguments = --
"#;

const DEFAULT_PA: &str = r#"
load-module module-device-restore
load-module module-stream-restore
load-module module-card-restore

load-module module-alsa-sink device=hw:0,0
load-module module-native-protocol-unix
load-module module-always-sink
"#;

const SOUND_DEVICES_PATH: &str = "/dev/snd";
const PULSE_RUN_PATH: &str = "/run/ph/pulse";

pub struct AudioSupport;

impl AudioSupport {
    pub fn setup() -> Result<()> {
        if Path::new(SOUND_DEVICES_PATH).exists() {
            Self::setup_sound_devices()?;
            Self::setup_pulse_audio_config()?;
        }
        Ok(())
    }

    fn setup_sound_devices() -> Result<()> {
        for entry in fs::read_dir(SOUND_DEVICES_PATH)
            .map_err(Error::DevSndReadDir)? {
            let entry = entry.map_err(Error::DevSndReadDir)?;
            let path = entry.path();
            if let Some(path_str) = path.as_os_str().to_str() {
                sys::chmod(path_str, 0o666)?;
            }
        }
        Ok(())
    }

    fn write_config_file(name: &str, content: &str) -> Result<()> {
        let pulse_run_path = Path::new(PULSE_RUN_PATH);
        fs::write(pulse_run_path.join(name), content)
            .map_err(Error::PulseAudioConfigWrite)
    }

    fn setup_pulse_audio_config() -> Result<()> {
        fs::create_dir_all(PULSE_RUN_PATH)
            .map_err(Error::PulseAudioConfigWrite)?;
        Self::write_config_file("daemon.conf", DAEMON_CONF)?;
        Self::write_config_file("client.conf", CLIENT_CONF)?;
        Self::write_config_file("default.pa", DEFAULT_PA)?;
        sys::bind_mount(PULSE_RUN_PATH, "/etc/pulse")?;
        Ok(())
    }
}

