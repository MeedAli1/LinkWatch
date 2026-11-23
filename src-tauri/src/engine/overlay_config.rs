/// engine/overlay_config.rs - Overlay text appearance & position config.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverlayCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Default for OverlayCorner {
    fn default() -> Self { Self::TopRight }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayConfig {
    pub corner:    OverlayCorner,
    /// Font size in px (10–36)
    pub font_size: u32,
    /// CSS color string e.g. "#06b6d4"
    pub color:     String,
    /// Opacity 0.0–1.0
    pub opacity:   f32,
    /// Margin from edge in px
    pub margin:    u32,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            corner:    OverlayCorner::TopRight,
            font_size: 16,
            color:     "#06b6d4".to_string(),
            opacity:   0.95,
            margin:    20,
        }
    }
}

fn config_path() -> PathBuf {
    let mut p = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    p.push("LinkWatch");
    let _ = std::fs::create_dir_all(&p);
    p.push("overlay_config.json");
    p
}

pub fn load_overlay_config() -> OverlayConfig {
    let path = config_path();
    if let Ok(raw) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<OverlayConfig>(&raw) {
            return cfg;
        }
    }
    OverlayConfig::default()
}

pub fn save_overlay_config(cfg: &OverlayConfig) -> Result<(), String> {
    let path = config_path();
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}
