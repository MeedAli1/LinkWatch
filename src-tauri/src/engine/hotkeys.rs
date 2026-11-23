/// engine/hotkeys.rs - Persistent global hotkey configuration for LinkWatch.
///
/// Stores the user's chosen shortcut strings in a JSON file under %LOCALAPPDATA%\LinkWatch\.
/// The Tauri app reads these on startup and registers them via tauri-plugin-global-shortcut.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Each action that can be bound to a global hotkey.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    /// Show / hide the main LinkWatch window
    ToggleWindow,
    /// Toggle the floating mini-HUD (PiP widget)
    ToggleMiniHud,
    /// Trigger a 1-click network repair
    NetworkRepair,
    /// Open the diagnostics panel directly
    OpenDiagnostics,
}

/// A single hotkey binding: an action + an optional accelerator string.
/// `None` means no key is bound for this action.
///
/// Accelerator strings follow the Tauri / global-hotkey format:
///   e.g. "CmdOrCtrl+Shift+L", "Alt+F9", "Ctrl+Alt+D"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub action: HotkeyAction,
    pub accelerator: Option<String>,
    pub enabled: bool,
}

/// Full set of user-configured hotkey bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub bindings: Vec<HotkeyBinding>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            bindings: vec![
                HotkeyBinding {
                    action: HotkeyAction::ToggleWindow,
                    accelerator: Some("CmdOrCtrl+Shift+L".to_string()),
                    enabled: true,
                },
                HotkeyBinding {
                    action: HotkeyAction::ToggleMiniHud,
                    accelerator: Some("CmdOrCtrl+Shift+M".to_string()),
                    enabled: true,
                },
                HotkeyBinding {
                    action: HotkeyAction::NetworkRepair,
                    accelerator: None,
                    enabled: false,
                },
                HotkeyBinding {
                    action: HotkeyAction::OpenDiagnostics,
                    accelerator: None,
                    enabled: false,
                },
            ],
        }
    }
}

fn config_path() -> PathBuf {
    let mut p = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    p.push("LinkWatch");
    let _ = std::fs::create_dir_all(&p);
    p.push("hotkeys.json");
    p
}

/// Load config from disk; falls back to defaults on any error.
pub fn load_hotkey_config() -> HotkeyConfig {
    let path = config_path();
    if let Ok(raw) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<HotkeyConfig>(&raw) {
            return cfg;
        }
    }
    HotkeyConfig::default()
}

/// Persist config to disk.
pub fn save_hotkey_config(cfg: &HotkeyConfig) -> Result<(), String> {
    let path = config_path();
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}
