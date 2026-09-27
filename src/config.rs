//! Configuration: schema, defaults, and layered loading.
//!
//! Precedence: defaults < config file < environment < CLI flags.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::color::CategoryConfig;

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub scan: ScanConfig,
    pub tui: TuiConfig,
    pub color: ColorConfig,
    pub worth_a_look: WorthConfig,
    pub keys: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// Include hidden entries (dot-prefixed, or the Windows hidden attribute).
    pub hidden: bool,
    /// Count logical bytes instead of allocated blocks.
    pub apparent_size: bool,
    pub follow_symlinks: bool,
    pub one_file_system: bool,
    /// Count hardlinked files once per link.
    pub count_links: bool,
    /// Glob patterns to skip (matched against the entry name and path).
    pub exclude: Vec<String>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            hidden: true,
            apparent_size: false,
            follow_symlinks: false,
            one_file_system: false,
            count_links: false,
            exclude: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    pub depth: usize,
    pub size_mode: SizeModeName,
    pub sidebar: SidebarMode,
    /// Where the details panel sits.
    pub sidebar_position: SidebarPosition,
    pub mouse: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            depth: 4,
            size_mode: SizeModeName::Size,
            sidebar: SidebarMode::Auto,
            sidebar_position: SidebarPosition::Left,
            mouse: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SidebarPosition {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SizeModeName {
    #[default]
    Size,
    Files,
    Age,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SidebarMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorConfig {
    pub mode: ColorModeName,
    /// Sequential palette name.
    pub palette: String,
    pub scale: ScaleName,
    /// Discrete size tiers, e.g. `["10MB", "100MB", "1GB"]`.
    pub tiers: Vec<String>,
    pub reverse: bool,
    /// How strongly palette colors are used: 1.0 = full, lower = muted.
    pub saturation: f64,
    pub theme: ThemeName,
    /// Per-category overrides keyed by category id.
    pub categories: BTreeMap<String, CategoryConfig>,
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            // A quantitative ramp encodes size directly and gives every cell a
            // distinct lightness; category mode is one flag away.
            mode: ColorModeName::Size,
            palette: "viridis".to_string(),
            scale: ScaleName::Log,
            tiers: Vec::new(),
            reverse: false,
            saturation: 0.7,
            theme: ThemeName::Auto,
            categories: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorModeName {
    #[default]
    Category,
    Size,
    Age,
    Depth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScaleName {
    Linear,
    #[default]
    Log,
    Rank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    /// Use the terminal's own colors (default).
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorthConfig {
    pub enabled: bool,
    pub max_items: usize,
    pub rules: Vec<WorthRule>,
}

impl Default for WorthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_items: 7,
            rules: vec![
                WorthRule {
                    name: "build output".to_string(),
                    glob: "**/{target,dist,build,out}".to_string(),
                    older_than: Some("7d".to_string()),
                },
                WorthRule {
                    name: "node_modules".to_string(),
                    glob: "**/node_modules".to_string(),
                    older_than: Some("30d".to_string()),
                },
                WorthRule {
                    name: "cache".to_string(),
                    glob: "**/.cache".to_string(),
                    older_than: Some("30d".to_string()),
                },
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorthRule {
    pub name: String,
    pub glob: String,
    #[serde(default)]
    pub older_than: Option<String>,
}

impl Config {
    /// Load config from the given path, the default path, or builtin defaults.
    pub fn load(explicit: Option<&Path>, no_config: bool) -> Result<Config, String> {
        if no_config {
            return Ok(Config::default());
        }
        let path = match explicit {
            Some(p) => Some(p.to_path_buf()),
            None => default_config_path(),
        };
        let Some(path) = path else {
            return Ok(Config::default());
        };
        if !path.exists() {
            if explicit.is_some() {
                return Err(format!("config file not found: {}", path.display()));
            }
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read config {}: {e}", path.display()))?;
        let cfg: Config =
            toml::from_str(&text).map_err(|e| format!("invalid config {}: {e}", path.display()))?;
        Ok(cfg)
    }
}

/// Default config file location.
pub fn default_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "disk-wiz").map(|d| d.config_dir().join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_partial_config() {
        let text = r##"
[scan]
hidden = false
exclude = ["**/node_modules/**"]

[tui]
depth = 6

[color]
mode = "size"
palette = "magma"

[color.categories]
code = { color = "#123456", globs = ["*.rs"] }
"##;
        let cfg: Config = toml::from_str(text).unwrap();
        assert!(!cfg.scan.hidden);
        assert_eq!(cfg.scan.exclude.len(), 1);
        assert_eq!(cfg.tui.depth, 6);
        assert_eq!(cfg.tui.size_mode, SizeModeName::Size);
        assert_eq!(cfg.color.mode, ColorModeName::Size);
        assert_eq!(cfg.color.palette, "magma");
        assert_eq!(cfg.color.scale, ScaleName::Log);
        assert!(cfg.color.categories.contains_key("code"));
    }

    #[test]
    fn defaults_are_sane() {
        let cfg = Config::default();
        assert!(cfg.scan.hidden);
        assert!(!cfg.scan.apparent_size);
        assert_eq!(cfg.tui.depth, 4);
        assert_eq!(cfg.color.mode, ColorModeName::Size);
        assert!(cfg.worth_a_look.enabled);
    }

    #[test]
    fn rejects_bad_values() {
        let text = "[tui]\ndepth = \"deep\"\n";
        assert!(toml::from_str::<Config>(text).is_err());
    }
}
