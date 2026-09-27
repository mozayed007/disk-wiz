//! Color system: sequential palettes, categorical palettes, file-type
//! categories.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::config::{ColorModeName, ScaleName};
use crate::tree::NodeKind;

/// An RGB color.
pub type Rgb = (u8, u8, u8);

/// Sequential and categorical palettes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Palette {
    Viridis,
    Magma,
    Inferno,
    Plasma,
    Cividis,
    Turbo,
    Spectral,
    OkabeIto,
    Tableau10,
    Dim,
}

impl Palette {
    pub const SEQUENTIAL: [Palette; 7] = [
        Palette::Viridis,
        Palette::Magma,
        Palette::Inferno,
        Palette::Plasma,
        Palette::Cividis,
        Palette::Turbo,
        Palette::Spectral,
    ];
    pub const CATEGORICAL: [Palette; 3] = [Palette::OkabeIto, Palette::Tableau10, Palette::Dim];

    pub fn from_name(name: &str) -> Option<Palette> {
        let n = name.trim().to_ascii_lowercase().replace(['-', '_'], "");
        Some(match n.as_str() {
            "viridis" => Palette::Viridis,
            "magma" => Palette::Magma,
            "inferno" => Palette::Inferno,
            "plasma" => Palette::Plasma,
            "cividis" => Palette::Cividis,
            "turbo" => Palette::Turbo,
            "spectral" => Palette::Spectral,
            "okabeito" | "okabe" => Palette::OkabeIto,
            "tableau10" | "tableau" => Palette::Tableau10,
            "dim" | "muted" | "disktree" => Palette::Dim,
            _ => return None,
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Palette::Viridis => "viridis",
            Palette::Magma => "magma",
            Palette::Inferno => "inferno",
            Palette::Plasma => "plasma",
            Palette::Cividis => "cividis",
            Palette::Turbo => "turbo",
            Palette::Spectral => "spectral",
            Palette::OkabeIto => "okabe-ito",
            Palette::Tableau10 => "tableau10",
            Palette::Dim => "dim",
        }
    }

    pub fn is_categorical(&self) -> bool {
        Self::CATEGORICAL.contains(self)
    }

    pub fn all_names() -> Vec<&'static str> {
        Self::SEQUENTIAL
            .iter()
            .chain(Self::CATEGORICAL.iter())
            .map(|p| p.name())
            .collect()
    }

    fn gradient(&self) -> Option<colorous::Gradient> {
        Some(match self {
            Palette::Viridis => colorous::VIRIDIS,
            Palette::Magma => colorous::MAGMA,
            Palette::Inferno => colorous::INFERNO,
            Palette::Plasma => colorous::PLASMA,
            Palette::Cividis => colorous::CIVIDIS,
            Palette::Turbo => colorous::TURBO,
            _ => return None,
        })
    }
}

/// Evaluate a sequential palette at `t` in `0.0..=1.0`.
pub fn sequential(palette: Palette, t: f64) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    if let Some(g) = palette.gradient() {
        let c = g.eval_continuous(t);
        return (c.r, c.g, c.b);
    }
    if palette == Palette::Spectral {
        return spectral(t);
    }
    // Categorical palettes used sequentially: pick by index.
    let n = categorical_len(palette).max(1);
    let idx = ((t * (n as f64 - 1.0)).round() as usize).min(n - 1);
    categorical(palette, idx)
}

/// Pick color `i` from a categorical palette (wrapping).
pub fn categorical(palette: Palette, i: usize) -> Rgb {
    let table: &[Rgb] = match palette {
        Palette::OkabeIto => &OKABE_ITO,
        Palette::Tableau10 => &TABLEAU10,
        Palette::Dim => &DIM,
        _ => &OKABE_ITO,
    };
    table[i % table.len()]
}

fn categorical_len(palette: Palette) -> usize {
    match palette {
        Palette::OkabeIto => OKABE_ITO.len(),
        Palette::Tableau10 => TABLEAU10.len(),
        Palette::Dim => DIM.len(),
        _ => OKABE_ITO.len(),
    }
}

/// Okabe-Ito colorblind-safe palette (black replaced with gray for dark
/// terminals).
const OKABE_ITO: [Rgb; 8] = [
    (230, 159, 0),
    (86, 180, 233),
    (0, 158, 115),
    (240, 228, 66),
    (0, 114, 178),
    (213, 94, 0),
    (204, 121, 167),
    (153, 153, 153),
];

/// Tableau 10.
const TABLEAU10: [Rgb; 10] = [
    (78, 121, 167),
    (242, 142, 43),
    (225, 87, 89),
    (118, 183, 178),
    (89, 161, 79),
    (237, 201, 72),
    (176, 122, 161),
    (255, 157, 167),
    (156, 117, 95),
    (186, 176, 172),
];

/// Muted colors tuned for treemaps on dark terminal backgrounds.
const DIM: [Rgb; 10] = [
    (74, 111, 165),
    (184, 115, 51),
    (78, 122, 78),
    (62, 107, 107),
    (139, 74, 74),
    (107, 74, 139),
    (138, 138, 160),
    (139, 139, 74),
    (90, 120, 150),
    (120, 90, 110),
];

/// Spectral ramp (11 stops, interpolated).
fn spectral(t: f64) -> Rgb {
    const STOPS: [Rgb; 11] = [
        (158, 1, 66),
        (213, 62, 79),
        (244, 109, 67),
        (253, 174, 97),
        (254, 224, 139),
        (255, 255, 191),
        (230, 245, 152),
        (171, 221, 164),
        (102, 194, 165),
        (50, 136, 189),
        (94, 79, 162),
    ];
    let t = t.clamp(0.0, 1.0) * (STOPS.len() - 1) as f64;
    let i = (t.floor() as usize).min(STOPS.len() - 2);
    let f = t - i as f64;
    let a = STOPS[i];
    let b = STOPS[i + 1];
    let lerp = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * f).round() as u8;
    (lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
}

/// Parse `#RRGGBB` or `#RGB` into RGB.
pub fn parse_hex_color(s: &str) -> Result<Rgb, String> {
    let s = s.trim().trim_start_matches('#');
    let parse =
        |h: &str| u8::from_str_radix(h, 16).map_err(|_| format!("invalid hex color: {s:?}"));
    match s.len() {
        6 => Ok((parse(&s[0..2])?, parse(&s[2..4])?, parse(&s[4..6])?)),
        3 => {
            let r = parse(&s[0..1])?;
            let g = parse(&s[1..2])?;
            let b = parse(&s[2..3])?;
            Ok((r * 17, g * 17, b * 17))
        }
        _ => Err(format!("invalid hex color: {s:?}")),
    }
}

/// Normalization of raw values into `0.0..=1.0` ramp positions.
#[derive(Clone, Copy, Debug)]
pub struct Scale {
    pub kind: ScaleName,
    pub min: f64,
    pub max: f64,
}

impl Scale {
    pub fn new(values: &[f64], kind: ScaleName) -> Scale {
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Scale {
            kind,
            min: if min.is_finite() { min } else { 0.0 },
            max: if max.is_finite() { max } else { 1.0 },
        }
    }

    pub fn t(&self, v: f64) -> f64 {
        if self.max <= self.min {
            return 1.0;
        }
        match self.kind {
            ScaleName::Linear | ScaleName::Rank => (v - self.min) / (self.max - self.min),
            ScaleName::Log => {
                let lo = self.min.max(1.0).ln();
                let hi = self.max.max(self.min + 1.0).ln();
                if hi <= lo {
                    1.0
                } else {
                    (v.max(1.0).ln() - lo) / (hi - lo)
                }
            }
        }
    }
}

/// How a node maps to a color.
#[derive(Clone, Copy, Debug)]
pub struct ColorPlan {
    pub mode: ColorModeName,
    pub palette: Palette,
    pub scale: ScaleName,
    pub reverse: bool,
    pub light: bool,
}

impl Default for ColorPlan {
    fn default() -> Self {
        Self {
            mode: ColorModeName::Category,
            palette: Palette::Viridis,
            scale: ScaleName::Log,
            reverse: false,
            light: false,
        }
    }
}

impl ColorPlan {
    /// Color for a node. `rank` is its position among visible nodes sorted by
    /// the active key (used by rank scaling).
    pub fn node_color(
        &self,
        tree: &crate::tree::Tree,
        id: crate::tree::NodeId,
        sort: crate::tree::SortKey,
        rank: usize,
        count: usize,
        scale: &Scale,
    ) -> Rgb {
        let mut t = match self.mode {
            ColorModeName::Category => {
                return tree
                    .category_colors
                    .get(tree.node(id).category as usize)
                    .copied()
                    .unwrap_or((128, 128, 128));
            }
            ColorModeName::Size => {
                if self.scale == ScaleName::Rank && count > 1 {
                    1.0 - rank as f64 / (count - 1) as f64
                } else {
                    scale.t(tree.value_for(id, sort) as f64)
                }
            }
            ColorModeName::Age => scale.t(tree.node(id).mtime as f64),
            ColorModeName::Depth => {
                return categorical(self.palette, tree.depth_of(id));
            }
        };
        if self.reverse ^ self.light {
            t = 1.0 - t;
        }
        sequential(self.palette, t)
    }
}

/// A file-type category.
#[derive(Clone, Debug)]
pub struct Category {
    pub id: String,
    pub label: String,
    pub rgb: Rgb,
}

/// What a rule matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RuleTarget {
    #[default]
    Any,
    Dir,
    File,
}

impl RuleTarget {
    fn matches(self, kind: NodeKind) -> bool {
        match self {
            RuleTarget::Any => true,
            RuleTarget::Dir => kind == NodeKind::Dir,
            RuleTarget::File => kind != NodeKind::Dir,
        }
    }
}

#[derive(Clone, Debug)]
struct Rule {
    category: u16,
    recursive: bool,
    target: RuleTarget,
}

/// Classification result for one entry.
#[derive(Clone, Copy, Debug)]
pub struct Classified {
    pub category: u16,
    pub propagate: Option<u16>,
}

/// Maps entry names to categories using ordered glob rules.
#[derive(Clone, Debug)]
pub struct CategoryMap {
    pub categories: Vec<Category>,
    rules: Vec<Rule>,
    globset: GlobSet,
}

/// User-provided category override (config file).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CategoryConfig {
    /// Hex color, e.g. `#4C8BF5`.
    pub color: String,
    /// Glob patterns matched against the entry name.
    #[serde(default)]
    pub globs: Vec<String>,
    /// Display label (defaults to the category id).
    #[serde(default)]
    pub label: Option<String>,
    /// Whether descendants inherit the category (default true).
    #[serde(default)]
    pub recursive: Option<bool>,
    /// `any`, `dir`, or `file` (default `any`).
    #[serde(default)]
    pub target: Option<String>,
}

struct RuleSpec {
    id: String,
    label: String,
    rgb: Rgb,
    globs: Vec<String>,
    recursive: bool,
    target: RuleTarget,
}

impl CategoryMap {
    /// Built-in categories and rules.
    pub fn builtin() -> CategoryMap {
        Self::from_specs(Self::builtin_specs()).expect("builtin category globs are valid")
    }

    /// Build a category map from builtins merged with config overrides.
    ///
    /// Overrides replace the builtin entry with the same id (keeping its rule
    /// position); new ids append after the builtins.
    pub fn from_config(
        overrides: &BTreeMap<String, CategoryConfig>,
    ) -> Result<CategoryMap, String> {
        let mut specs = Self::builtin_specs();
        for (id, cc) in overrides {
            let rgb = parse_hex_color(&cc.color)?;
            let recursive = cc.recursive.unwrap_or(true);
            let target = match cc.target.as_deref() {
                None | Some("any") => RuleTarget::Any,
                Some("dir") => RuleTarget::Dir,
                Some("file") => RuleTarget::File,
                Some(other) => return Err(format!("invalid target {other:?} for category {id:?}")),
            };
            if let Some(spec) = specs.iter_mut().find(|s| &s.id == id) {
                if !cc.globs.is_empty() {
                    spec.globs = cc.globs.clone();
                }
                spec.rgb = rgb;
                spec.recursive = recursive;
                spec.target = target;
                if let Some(label) = &cc.label {
                    spec.label = label.clone();
                }
            } else {
                specs.push(RuleSpec {
                    id: id.clone(),
                    label: cc.label.clone().unwrap_or_else(|| id.clone()),
                    rgb,
                    globs: cc.globs.clone(),
                    recursive,
                    target,
                });
            }
        }
        Self::from_specs(specs)
    }

    fn builtin_specs() -> Vec<RuleSpec> {
        BUILTIN_SPECS
            .iter()
            .map(|s| RuleSpec {
                id: s.0.to_string(),
                label: s.1.to_string(),
                rgb: s.2,
                globs: s.3.iter().map(|g| g.to_string()).collect(),
                recursive: s.4,
                target: s.5,
            })
            .collect()
    }

    fn from_specs(specs: Vec<RuleSpec>) -> Result<CategoryMap, String> {
        let mut categories = vec![Category {
            id: "other".into(),
            label: "Other".into(),
            rgb: (128, 128, 128),
        }];
        let mut rules = Vec::new();
        let mut builder = GlobSetBuilder::new();
        for spec in specs {
            let idx = categories.len() as u16;
            categories.push(Category {
                id: spec.id,
                label: spec.label,
                rgb: spec.rgb,
            });
            for g in &spec.globs {
                let glob = GlobBuilder::new(g)
                    .case_insensitive(true)
                    .literal_separator(false)
                    .build()
                    .map_err(|e| format!("invalid category glob {g:?}: {e}"))?;
                builder.add(glob);
                rules.push(Rule {
                    category: idx,
                    recursive: spec.recursive,
                    target: spec.target,
                });
            }
        }
        let globset = builder
            .build()
            .map_err(|e| format!("invalid category globs: {e}"))?;
        Ok(CategoryMap {
            categories,
            rules,
            globset,
        })
    }

    /// Category id for an index.
    pub fn id_of(&self, idx: u16) -> &str {
        self.categories
            .get(idx as usize)
            .map(|c| c.id.as_str())
            .unwrap_or("other")
    }

    /// RGB color for a category index.
    pub fn color_of(&self, idx: u16) -> Rgb {
        self.categories
            .get(idx as usize)
            .map(|c| c.rgb)
            .unwrap_or((128, 128, 128))
    }

    /// Classify one entry, given the category inherited from its parent.
    pub fn classify(&self, name: &str, kind: NodeKind, inherited: Option<u16>) -> Classified {
        if let Some(c) = inherited {
            return Classified {
                category: c,
                propagate: Some(c),
            };
        }
        for &idx in self.globset.matches(name).iter() {
            let rule = &self.rules[idx];
            if !rule.target.matches(kind) {
                continue;
            }
            let propagate = if rule.recursive && kind == NodeKind::Dir {
                Some(rule.category)
            } else {
                None
            };
            return Classified {
                category: rule.category,
                propagate,
            };
        }
        Classified {
            category: 0,
            propagate: None,
        }
    }
}

/// `(id, label, rgb, globs, recursive, target)` for the builtin categories.
type BuiltinSpec = (
    &'static str,
    &'static str,
    Rgb,
    &'static [&'static str],
    bool,
    RuleTarget,
);

const BUILTIN_SPECS: &[BuiltinSpec] = &[
    (
        "agent",
        "Agent scratch",
        (184, 115, 51),
        &[".codex", ".claude", ".cursor", ".opencode"],
        true,
        RuleTarget::Dir,
    ),
    (
        "git",
        "Git",
        (139, 74, 74),
        &[".git"],
        true,
        RuleTarget::Dir,
    ),
    (
        "toolchains",
        "Toolchains",
        (78, 122, 78),
        &[".cargo", ".rustup", ".nvm", ".pyenv", ".gradle", ".m2"],
        true,
        RuleTarget::Dir,
    ),
    (
        "cache",
        "Cache",
        (139, 139, 74),
        &[
            "target",
            "node_modules",
            ".cache",
            "__pycache__",
            ".venv",
            "venv",
            ".npm",
            ".yarn",
            ".pytest_cache",
        ],
        true,
        RuleTarget::Dir,
    ),
    (
        "synced",
        "Synced",
        (62, 107, 107),
        &["OneDrive", "Dropbox", "iCloud Drive", "Google Drive"],
        true,
        RuleTarget::Dir,
    ),
    (
        "code",
        "Code",
        (74, 111, 165),
        &[
            "*.rs", "*.py", "*.ts", "*.tsx", "*.js", "*.jsx", "*.go", "*.c", "*.h", "*.cc",
            "*.cpp", "*.hpp", "*.java", "*.kt", "*.swift", "*.rb", "*.sh", "*.ps1", "*.toml",
            "*.json", "*.yaml", "*.yml", "*.html", "*.css", "*.sql",
        ],
        false,
        RuleTarget::File,
    ),
    (
        "media",
        "Media",
        (107, 74, 139),
        &[
            "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.svg", "*.bmp", "*.mp4", "*.mov",
            "*.mkv", "*.avi", "*.webm", "*.mp3", "*.wav", "*.flac", "*.m4a",
        ],
        false,
        RuleTarget::File,
    ),
    (
        "documents",
        "Documents",
        (138, 138, 160),
        &[
            "*.pdf", "*.doc", "*.docx", "*.xls", "*.xlsx", "*.ppt", "*.pptx", "*.txt", "*.md",
            "*.epub",
        ],
        false,
        RuleTarget::File,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_name_and_extension() {
        let map = CategoryMap::builtin();
        let code = map.categories.iter().position(|c| c.id == "code").unwrap() as u16;
        let git = map.categories.iter().position(|c| c.id == "git").unwrap() as u16;
        let c = map.classify("main.rs", NodeKind::File, None);
        assert_eq!(c.category, code);
        assert_eq!(c.propagate, None);
        let c = map.classify(".git", NodeKind::Dir, None);
        assert_eq!(c.category, git);
        assert_eq!(c.propagate, Some(git));
        // Inheritance wins over extension rules.
        let c = map.classify("HEAD", NodeKind::File, Some(git));
        assert_eq!(c.category, git);
    }

    #[test]
    fn parses_hex() {
        assert_eq!(parse_hex_color("#ff8800").unwrap(), (255, 136, 0));
        assert_eq!(parse_hex_color("f80").unwrap(), (255, 136, 0));
        assert!(parse_hex_color("#12").is_err());
    }

    #[test]
    fn parses_palette_names() {
        assert_eq!(Palette::from_name("Viridis"), Some(Palette::Viridis));
        assert_eq!(Palette::from_name("okabe_ito"), Some(Palette::OkabeIto));
        assert_eq!(Palette::from_name("nope"), None);
    }

    #[test]
    fn sequential_endpoints() {
        let lo = sequential(Palette::Viridis, 0.0);
        let hi = sequential(Palette::Viridis, 1.0);
        assert_ne!(lo, hi);
        assert_eq!(sequential(Palette::Spectral, 0.0), (158, 1, 66));
        assert_eq!(sequential(Palette::Spectral, 1.0), (94, 79, 162));
    }
}
