use std::path::Path;
use std::sync::OnceLock;

use eyre::{Result, WrapErr};
use serde::Deserialize;

pub const DEFAULT_CONFIG_PATH: &str = "hardware-config.json";
static GLOBAL_CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    /// Front-Left max wheel speed in range [-1.0, 1.0]
    ///
    /// Negative values invert the direction of the wheel so that negative values are used for
    /// forward movement.
    #[serde(default = "default_forward_max_speed")]
    pub fl_max: f32,

    /// Back-Left max wheel speed in range [-1.0, 1.0]
    ///
    /// Negative values invert the direction of the wheel so that negative values are used for
    /// forward movement.
    #[serde(default = "default_forward_max_speed")]
    pub bl_max: f32,

    /// Front-Right max wheel speed in range [-1.0, 1.0]
    ///
    /// Negative values invert the direction of the wheel so that negative values are used for
    /// forward movement.
    #[serde(default = "default_forward_max_speed")]
    pub fr_max: f32,

    /// Back-Right max wheel speed in range [-1.0, 1.0]
    ///
    /// Negative values invert the direction of the wheel so that negative values are used for
    /// forward movement.
    #[serde(default = "default_forward_max_speed")]
    pub br_max: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            fl_max: default_forward_max_speed(),
            bl_max: default_forward_max_speed(),
            fr_max: default_forward_max_speed(),
            br_max: default_forward_max_speed(),
        }
    }
}

impl Config {
    pub fn get_global() -> &'static Self {
        Self::load_file(DEFAULT_CONFIG_PATH.as_ref())
    }

    /// Load the global config file from the provided path if it hasn't already been loaded (or
    /// default if that fails).
    pub fn load_file(path: &Path) -> &'static Self {
        GLOBAL_CONFIG.get_or_init(|| {
            log::info!("Attempting to load hardware config from '{}'", path.display());
            match Self::load_inner(path.as_ref()) {
                Ok(c) => return c,
                Err(e) => {
                    log::warn!("Failed to load hardware config '{}': {e}", path.display());
                }
            }

            log::info!("Using default hardware config values");
            Config::default()
        })
    }

    fn load_inner(path: &Path) -> Result<Self> {
        let f = std::fs::OpenOptions::new().read(true).open(path)
            .wrap_err_with(|| format!("Failed to open file '{}'", path.display()))?;
        Ok(serde_json::from_reader::<_, Config>(f)
            .wrap_err_with(|| format!("Failed to deserialize JSON from '{}'", path.display()))?)
    }
}

#[inline(always)]
const fn default_forward_max_speed() -> f32 { 1.0 }
