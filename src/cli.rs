//! Command-line interface.

use clap::{Parser, ValueEnum};
use clap_complete::Shell;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "dw",
    version,
    about = "Disk usage treemap: interactive TUI and pipe-friendly CLI",
    long_about = "Disk usage treemap: interactive TUI and pipe-friendly CLI.\n\n\
        With no arguments and a terminal attached, dw opens an interactive\n\
        treemap. When piped, or with --print, it writes a flat size-sorted\n\
        listing like du. Use --format for tree, json, or an ANSI treemap.",
    after_help = "Examples:\n  \
        dw                          TUI on the current directory\n  \
        dw ~/src --depth 3          TUI with a preset depth\n  \
        dw -p --format flat -n 20   top 20 entries by size\n  \
        dw --json | jq .children    machine-readable output\n  \
        dw --format ansi --palette magma --color-mode size\n  \
        dw --min-size 1G --type cache\n\n\
        Config: ~/.config/disk-wiz/config.toml (or %APPDATA%\\disk-wiz\\config.toml)"
)]
pub struct Cli {
    /// Paths to scan (defaults to the current directory)
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// Force non-interactive output
    #[arg(short = 'p', long)]
    pub print: bool,

    /// Output format
    #[arg(short = 'f', long, value_enum, default_value_t = FormatArg::Auto)]
    pub format: FormatArg,

    /// Maximum depth to show
    #[arg(short = 'd', long)]
    pub depth: Option<usize>,

    /// Limit results (rows for flat, entries per directory for tree)
    #[arg(short = 'n', long)]
    pub top: Option<usize>,

    /// Sort key
    #[arg(long, value_enum, default_value_t = SortArg::Size)]
    pub sort: SortArg,

    /// Reverse the sort order
    #[arg(long)]
    pub reverse: bool,

    /// Skip entries smaller than SIZE (e.g. 10M, 1.5GiB)
    #[arg(long, value_parser = parse_size_arg, value_name = "SIZE")]
    pub min_size: Option<u64>,

    /// Skip entries larger than SIZE
    #[arg(long, value_parser = parse_size_arg, value_name = "SIZE")]
    pub max_size: Option<u64>,

    /// Only include these categories (comma separated, e.g. code,media)
    #[arg(long = "type", value_delimiter = ',', value_name = "CAT")]
    pub type_filter: Vec<String>,

    /// Exclude entries matching a glob (repeatable)
    #[arg(long, value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Read exclude globs from a file (one per line)
    #[arg(long, value_name = "PATH")]
    pub exclude_file: Option<PathBuf>,

    /// Include hidden entries
    #[arg(long, overrides_with = "no_hidden")]
    pub hidden: bool,

    /// Hide hidden entries
    #[arg(long = "no-hidden", overrides_with = "hidden")]
    pub no_hidden: bool,

    /// Count logical bytes instead of allocated blocks
    #[arg(long)]
    pub apparent_size: bool,

    /// Follow symlinks (cycle-safe)
    #[arg(long)]
    pub follow: bool,

    /// Do not cross filesystem boundaries
    #[arg(long)]
    pub one_file_system: bool,

    /// Count hardlinked files once per link
    #[arg(long)]
    pub count_links: bool,

    /// Scanner thread count (default: available parallelism)
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,

    /// Color output
    #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
    pub color: ColorArg,

    /// Sequential palette (viridis, magma, inferno, plasma, cividis, turbo,
    /// spectral, okabe-ito, tableau10, dim)
    #[arg(long, value_name = "NAME")]
    pub palette: Option<String>,

    /// Color mapping mode
    #[arg(long, value_enum, value_name = "MODE")]
    pub color_mode: Option<ColorModeArg>,

    /// Theme variant for the TUI
    #[arg(long, value_enum, value_name = "THEME")]
    pub theme: Option<ThemeArg>,

    /// Width for --format ansi (defaults to the terminal width)
    #[arg(long, value_name = "COLS")]
    pub width: Option<u16>,

    /// Height for --format ansi (defaults to the terminal height)
    #[arg(long, value_name = "ROWS")]
    pub height: Option<u16>,

    /// Shorthand for --format json
    #[arg(long)]
    pub json: bool,

    /// Print raw byte counts instead of human sizes
    #[arg(long)]
    pub bytes: bool,

    /// Suppress progress output
    #[arg(long)]
    pub quiet: bool,

    /// Show scan error samples
    #[arg(long)]
    pub verbose: bool,

    /// Exit with code 3 when the scan had errors
    #[arg(long)]
    pub strict: bool,

    /// Exit with code 4 when the total exceeds SIZE
    #[arg(long, value_parser = parse_size_arg, value_name = "SIZE")]
    pub threshold: Option<u64>,

    /// Config file path
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Ignore the config file
    #[arg(long)]
    pub no_config: bool,

    /// List available palettes and exit
    #[arg(long)]
    pub list_palettes: bool,

    /// List categories and exit
    #[arg(long)]
    pub list_categories: bool,

    /// Generate shell completions and exit
    #[arg(long, value_name = "SHELL")]
    pub completions: Option<Shell>,

    /// Initial size mode for the TUI
    #[arg(long, value_enum, value_name = "MODE")]
    pub size_mode: Option<SizeModeArg>,

    /// Sidebar visibility in the TUI
    #[arg(long, value_enum, value_name = "WHEN")]
    pub sidebar: Option<SidebarArg>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum FormatArg {
    /// TUI when a terminal is attached, flat output otherwise
    Auto,
    /// Interactive treemap
    Tui,
    /// Size-sorted list, one entry per line
    Flat,
    /// Indented tree
    Tree,
    /// Nested JSON
    Json,
    /// Treemap rendered with ANSI colors
    Ansi,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum SortArg {
    Size,
    Files,
    Mtime,
    Name,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum ColorArg {
    Auto,
    Always,
    Never,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum ColorModeArg {
    /// Color by file-type category
    Category,
    /// Sequential ramp over size
    Size,
    /// Sequential ramp over modification age
    Age,
    /// Categorical cycle by depth
    Depth,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum SizeModeArg {
    Size,
    Files,
    Age,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum ThemeArg {
    Dark,
    Light,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum SidebarArg {
    Auto,
    Always,
    Never,
}

fn parse_size_arg(s: &str) -> Result<u64, String> {
    crate::util::parse_size(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_sizes_and_lists() {
        let cli = Cli::try_parse_from([
            "dw",
            "-p",
            "--min-size",
            "10M",
            "--type",
            "code,media",
            "--exclude",
            "*.log",
            "--exclude",
            "tmp",
            "some/path",
        ])
        .unwrap();
        assert!(cli.print);
        assert_eq!(cli.min_size, Some(10 * 1024 * 1024));
        assert_eq!(cli.type_filter, vec!["code", "media"]);
        assert_eq!(cli.exclude.len(), 2);
        assert_eq!(cli.paths.len(), 1);
    }

    #[test]
    fn rejects_bad_size() {
        assert!(Cli::try_parse_from(["dw", "--min-size", "nope"]).is_err());
    }
}
