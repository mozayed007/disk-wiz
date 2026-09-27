//! disk-wiz library: scanning, tree model, layout, color, output, TUI.

pub mod cli;
pub mod color;
pub mod config;
pub mod layout;
pub mod output;
pub mod scan;
pub mod tree;
pub mod ui;
pub mod util;

use anyhow::{Result, anyhow};
use clap::CommandFactory;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::cli::{Cli, ColorArg, FormatArg, SortArg};
use crate::color::{CategoryMap, Palette};
use crate::config::{ColorModeName, Config, SizeModeName};
use crate::output::{OutputFormat, OutputOptions};
use crate::scan::{ScanProgress, ScanSpec, scan_all};
use crate::tree::{SortKey, Tree};
use crate::util::{format_count, format_size};

/// Interactive TUI or a non-interactive renderer.
enum Mode {
    Tui,
    Cli(OutputFormat),
}

/// Entry point used by `main` and integration tests.
pub fn run(cli: Cli) -> Result<ExitCode> {
    if let Some(shell) = cli.completions {
        let mut cmd = Cli::command();
        clap_complete::generate(shell, &mut cmd, "dw", &mut io::stdout());
        return Ok(ExitCode::SUCCESS);
    }

    let config = Config::load(cli.config.as_deref(), cli.no_config).map_err(|e| anyhow!(e))?;
    let categories = CategoryMap::from_config(&config.color.categories).map_err(|e| anyhow!(e))?;

    if cli.list_palettes {
        for name in Palette::all_names() {
            println!("{name}");
        }
        return Ok(ExitCode::SUCCESS);
    }
    if cli.list_categories {
        for cat in &categories.categories {
            println!(
                "{:<12} {:<16} #{:02X}{:02X}{:02X}",
                cat.id, cat.label, cat.rgb.0, cat.rgb.1, cat.rgb.2
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    let paths: Vec<PathBuf> = if cli.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        cli.paths.clone()
    };
    for p in &paths {
        if !p.exists() {
            return Err(anyhow!("path not found: {}", p.display()));
        }
    }

    // Assemble the scan spec from config and flags.
    let mut excludes = config.scan.exclude.clone();
    excludes.extend(cli.exclude.iter().cloned());
    if let Some(file) = &cli.exclude_file {
        let text = std::fs::read_to_string(file)
            .map_err(|e| anyhow!("cannot read {}: {e}", file.display()))?;
        excludes.extend(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string),
        );
    }
    let hidden = if cli.no_hidden {
        false
    } else if cli.hidden {
        true
    } else {
        config.scan.hidden
    };
    let spec = ScanSpec {
        paths: paths.clone(),
        hidden,
        apparent_size: cli.apparent_size || config.scan.apparent_size,
        follow_symlinks: cli.follow || config.scan.follow_symlinks,
        one_file_system: cli.one_file_system || config.scan.one_file_system,
        count_links: cli.count_links || config.scan.count_links,
        excludes,
        threads: cli.threads,
    };

    let stdout_tty = io::stdout().is_terminal();
    let mode = if cli.json {
        Mode::Cli(OutputFormat::Json)
    } else {
        match cli.format {
            FormatArg::Auto => {
                if cli.print || !stdout_tty {
                    Mode::Cli(OutputFormat::Flat)
                } else {
                    Mode::Tui
                }
            }
            FormatArg::Tui => {
                if stdout_tty {
                    Mode::Tui
                } else {
                    Mode::Cli(OutputFormat::Flat)
                }
            }
            FormatArg::Flat => Mode::Cli(OutputFormat::Flat),
            FormatArg::Tree => Mode::Cli(OutputFormat::Tree),
            FormatArg::Json => Mode::Cli(OutputFormat::Json),
            FormatArg::Ansi => Mode::Cli(OutputFormat::Ansi),
        }
    };

    let color_enabled = match cli.color {
        ColorArg::Always => true,
        ColorArg::Never => false,
        ColorArg::Auto => stdout_tty && std::env::var_os("NO_COLOR").is_none(),
    };

    let progress = Arc::new(ScanProgress::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let show_progress = matches!(mode, Mode::Cli(_)) && !cli.quiet && io::stderr().is_terminal();
    let progress_thread = if show_progress {
        Some(spawn_progress(progress.clone()))
    } else {
        None
    };
    let outcome = scan_all(&spec, progress.clone(), cancel.clone()).map_err(|e| anyhow!(e))?;
    if let Some(t) = progress_thread {
        t.finish();
    }

    let tree = Tree::from_scan(
        outcome.root,
        &categories,
        outcome.errors,
        outcome.error_count,
        outcome.duration,
    );

    let total = tree.node(tree.root()).size;
    let error_count = tree.error_count;
    let error_samples = if cli.verbose {
        tree.errors.clone()
    } else {
        Vec::new()
    };

    match mode {
        Mode::Cli(format) => {
            let opts = output_options(&cli, &config, format, color_enabled)?;
            let stdout = io::stdout();
            let mut lock = stdout.lock();
            crate::output::write(&tree, &mut lock, &opts)?;
            lock.flush()?;
        }
        Mode::Tui => {
            let keymap = ui::keys::KeyMap::with_overrides(&config.keys).map_err(|e| anyhow!(e))?;
            let ui_config = ui::UiConfig {
                spec,
                config,
                categories,
                size_mode: cli.size_mode.map(size_mode_name).unwrap_or_default(),
                sidebar: cli.sidebar.map(sidebar_mode).unwrap_or_default(),
                theme: cli.theme.map(theme_name).unwrap_or_default(),
                keymap,
            };
            ui::run(tree, ui_config)?;
        }
    }

    if let Some(threshold) = cli.threshold
        && total > threshold
    {
        if !cli.quiet {
            eprintln!(
                "dw: total {} exceeds threshold {}",
                format_size(total),
                format_size(threshold)
            );
        }
        return Ok(ExitCode::from(4));
    }
    if error_count > 0 {
        if !cli.quiet {
            eprintln!("dw: warning: {error_count} entries could not be read (see --verbose)");
        }
        if cli.verbose {
            for e in &error_samples {
                eprintln!("  {}: {}", e.path.display(), e.message);
            }
        }
        if cli.strict {
            return Ok(ExitCode::from(3));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn size_mode_name(m: cli::SizeModeArg) -> SizeModeName {
    match m {
        cli::SizeModeArg::Size => SizeModeName::Size,
        cli::SizeModeArg::Files => SizeModeName::Files,
        cli::SizeModeArg::Age => SizeModeName::Age,
    }
}

fn sidebar_mode(m: cli::SidebarArg) -> config::SidebarMode {
    match m {
        cli::SidebarArg::Auto => config::SidebarMode::Auto,
        cli::SidebarArg::Always => config::SidebarMode::Always,
        cli::SidebarArg::Never => config::SidebarMode::Never,
    }
}

fn theme_name(t: cli::ThemeArg) -> config::ThemeName {
    match t {
        cli::ThemeArg::Dark => config::ThemeName::Dark,
        cli::ThemeArg::Light => config::ThemeName::Light,
    }
}

fn output_options(
    cli: &Cli,
    config: &Config,
    format: OutputFormat,
    color: bool,
) -> Result<OutputOptions> {
    let palette_name = cli.palette.as_deref().unwrap_or(&config.color.palette);
    let palette = Palette::from_name(palette_name)
        .ok_or_else(|| anyhow!("unknown palette {palette_name:?}; see --list-palettes"))?;
    let color_mode = match cli.color_mode {
        Some(cli::ColorModeArg::Category) => ColorModeName::Category,
        Some(cli::ColorModeArg::Size) => ColorModeName::Size,
        Some(cli::ColorModeArg::Age) => ColorModeName::Age,
        Some(cli::ColorModeArg::Depth) => ColorModeName::Depth,
        None => config.color.mode,
    };
    let sort = match cli.sort {
        SortArg::Size => SortKey::Size,
        SortArg::Files => SortKey::Files,
        SortArg::Mtime => SortKey::Mtime,
        SortArg::Name => SortKey::Name,
    };
    let depth = cli.depth.or(if format == OutputFormat::Tree {
        Some(2)
    } else {
        None
    });
    let mut width = cli.width;
    let mut height = cli.height;
    if format == OutputFormat::Ansi {
        let (tw, th) = crossterm::terminal::size().unwrap_or((100, 30));
        width = width.or(Some(tw));
        height = height.or(Some(th.saturating_sub(1).max(5)));
    }
    Ok(OutputOptions {
        format,
        depth,
        top: cli.top,
        sort,
        reverse: cli.reverse,
        min_size: cli.min_size,
        max_size: cli.max_size,
        type_filter: cli.type_filter.clone(),
        raw_bytes: cli.bytes,
        color,
        palette,
        color_mode,
        scale: config.color.scale,
        width: width.unwrap_or(100),
        height: height.unwrap_or(30),
    })
}

struct ProgressThread {
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl ProgressThread {
    fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn spawn_progress(progress: Arc<ScanProgress>) -> ProgressThread {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    let join = std::thread::spawn(move || {
        let start = Instant::now();
        while !stop_flag.load(Ordering::Relaxed) {
            let snap = progress.snapshot();
            eprint!(
                "\rscanning: {} entries, {} - {:.1}s",
                format_count(snap.entries),
                format_size(snap.bytes),
                start.elapsed().as_secs_f64()
            );
            let _ = io::stderr().flush();
            std::thread::sleep(Duration::from_millis(100));
        }
        eprint!("\r\x1b[2K");
        let _ = io::stderr().flush();
    });
    ProgressThread {
        stop,
        join: Some(join),
    }
}
