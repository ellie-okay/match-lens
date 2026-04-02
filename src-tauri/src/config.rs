use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub resolution: String, // "1080p" or "native"
    pub audio_mode: String, // "system" or "off"
    pub output_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub max_gb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub autostart: bool,
    pub theme: String,
    #[serde(default)]
    pub event_filters: EventFiltersConfig,
    #[serde(default)]
    pub review_window: ReviewWindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFiltersConfig {
    pub kill: bool,
    pub death: bool,
    pub assist: bool,
    pub dragon: bool,
    pub baron: bool,
    pub herald: bool,
    pub turret: bool,
    pub inhibitor: bool,
}

impl Default for EventFiltersConfig {
    fn default() -> Self {
        Self {
            kill: true,
            death: true,
            assist: true,
            dragon: true,
            baron: true,
            herald: true,
            turret: false,
            inhibitor: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReviewWindowConfig {
    #[serde(default)]
    pub monitor_name: String,
    #[serde(default)]
    pub monitor_x: Option<i32>,
    #[serde(default)]
    pub monitor_y: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub recording: RecordingConfig,
    pub storage: StorageConfig,
    pub app: AppConfig,
}

impl Default for Config {
    fn default() -> Self {
        let default_output = dirs::video_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
            .join("Match Lens Recordings")
            .to_string_lossy()
            .to_string();

        Config {
            recording: RecordingConfig {
                resolution: "1080p".into(),
                audio_mode: "system".into(),
                output_dir: default_output,
            },
            storage: StorageConfig { max_gb: 50 },
            app: AppConfig {
                autostart: true,
                theme: "dark".into(),
                event_filters: EventFiltersConfig::default(),
                review_window: ReviewWindowConfig::default(),
            },
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
    base.join("match-lens").join("config.toml")
}

pub fn load() -> Config {
    let path = config_path();
    if path.exists() {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&text).unwrap_or_default()
    } else {
        let cfg = Config::default();
        let _ = save(&cfg);
        cfg
    }
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, text)?;
    Ok(())
}
