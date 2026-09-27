//! Interactive TUI: terminal setup, event loop, state, rendering.

pub mod app;
pub mod draw;
pub mod keys;
pub mod theme;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout};
use std::time::Duration;

use crate::tree::Tree;

pub use app::{App, UiConfig};

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Run the interactive TUI until the user quits.
pub fn run(tree: Tree, cfg: UiConfig) -> Result<()> {
    run_app(App::new(tree, cfg)).map(|_| ())
}

/// Open the TUI immediately and scan in the background.
///
/// Returns the total size and scan error count of the last completed scan.
pub fn run_scanning(cfg: UiConfig) -> Result<(u64, u64)> {
    let mut app = App::new_pending(cfg);
    app.start_scan();
    run_app(app)
}

fn run_app(mut app: App) -> Result<(u64, u64)> {
    install_panic_hook();
    let mut terminal = setup()?;
    let result = event_loop(&mut terminal, &mut app);
    let restored = restore(&mut terminal);
    result.and(restored)?;
    Ok(app.totals())
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original(info);
    }));
}

fn setup() -> Result<Tui> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore(terminal: &mut Tui) -> Result<()> {
    terminal::disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn event_loop(terminal: &mut Tui, app: &mut App) -> Result<()> {
    loop {
        app.tick();
        terminal.draw(|frame| draw::draw(frame, app))?;
        let timeout = if app.scanning() {
            Duration::from_millis(80)
        } else {
            Duration::from_millis(200)
        };
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    app.handle_key(key);
                }
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                Event::Resize(_, _) => app.invalidate_layout(),
                _ => {}
            }
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::CategoryMap;
    use crate::config::{Config, SidebarMode, SizeModeName, ThemeName};
    use crate::scan::ScanSpec;
    use crate::tree::{NodeKind, ScanNode};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use std::ffi::OsString;

    fn leaf(name: &str, size: u64, mtime: u32) -> ScanNode {
        ScanNode::entry(OsString::from(name), NodeKind::File, size, mtime)
    }

    fn dir(name: &str, children: Vec<ScanNode>, mtime: u32) -> ScanNode {
        let mut d = ScanNode::new(OsString::from(name), NodeKind::Dir);
        d.mtime = mtime;
        for c in children {
            d.absorb(c);
        }
        d.finish();
        d
    }

    fn sample_tree() -> Tree {
        // An old mtime so worth-a-look rules match.
        let old = 1_600_000_000u32;
        let root = dir(
            "root",
            vec![
                dir(
                    "src",
                    vec![
                        leaf("main.rs", 2_500_000, old),
                        leaf("lib.rs", 1_500_000, old),
                    ],
                    old,
                ),
                dir(
                    "target",
                    vec![
                        dir("debug", vec![leaf("dw.exe", 3_000_000, old)], old),
                        leaf("build.log", 1_500_000, old),
                    ],
                    old,
                ),
                dir("media", vec![leaf("clip.mp4", 2_500_000, old)], old),
                leaf("readme.md", 1_000_000, old),
            ],
            old,
        );
        Tree::from_scan(
            root,
            &CategoryMap::builtin(),
            Vec::new(),
            0,
            std::time::Duration::ZERO,
        )
    }

    fn test_app() -> App {
        let cfg = UiConfig {
            spec: ScanSpec::default(),
            config: Config::default(),
            categories: CategoryMap::builtin(),
            size_mode: SizeModeName::Size,
            sidebar: SidebarMode::Always,
            theme: ThemeName::Dark,
            keymap: crate::ui::keys::KeyMap::default_map(),
        };
        App::new(sample_tree(), cfg)
    }

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..h {
            for x in 0..w {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn renders_chrome_and_treemap() {
        let mut app = test_app();
        let out = render(&mut app, 120, 40);
        assert!(out.contains("dw"), "header missing:\n{out}");
        assert!(out.contains("SELECTION"), "sidebar missing:\n{out}");
        assert!(out.contains("WORTH A LOOK"), "worth panel missing:\n{out}");
        assert!(out.contains("src"), "treemap labels missing:\n{out}");
        assert!(out.contains("target"), "treemap labels missing:\n{out}");
        assert!(out.contains("DISK"), "disk panel missing:\n{out}");
    }

    #[test]
    fn layout_covers_the_area() {
        let mut app = test_app();
        let area = ratatui::layout::Rect::new(0, 1, 100, 30);
        app.ensure_layout(area);
        for item in &app.layout {
            assert!(item.rect.right() <= area.right());
            assert!(item.rect.bottom() <= area.bottom());
            assert!(item.rect.width > 0 && item.rect.height > 0);
        }
        // Every cell of the area is covered by at least one item.
        let mut covered = vec![false; area.width as usize * area.height as usize];
        for item in &app.layout {
            for y in item.rect.y..item.rect.bottom() {
                for x in item.rect.x..item.rect.right() {
                    let idx = (y - area.y) as usize * area.width as usize + (x - area.x) as usize;
                    covered[idx] = true;
                }
            }
        }
        assert!(covered.iter().all(|&c| c), "uncovered cells in the treemap");
    }

    /// Sibling cells leave a one-cell gap so the parent color separates them.
    #[test]
    fn siblings_are_separated_by_a_gap() {
        let mut app = test_app();
        let area = ratatui::layout::Rect::new(0, 1, 100, 30);
        app.ensure_layout(area);
        let drawn: Vec<ratatui::layout::Rect> = app
            .layout
            .iter()
            .filter(|i| i.node.is_some())
            .map(|i| {
                if i.rect.width >= 4 && i.rect.height >= 3 {
                    ratatui::layout::Rect {
                        x: i.rect.x,
                        y: i.rect.y,
                        width: i.rect.width - 1,
                        height: i.rect.height - 1,
                    }
                } else {
                    i.rect
                }
            })
            .collect();
        // Siblings (same depth) never touch edge-to-edge.
        for (idx, a) in app.layout.iter().enumerate() {
            if a.node.is_none() || a.rect.width < 4 || a.rect.height < 3 {
                continue;
            }
            for (jdx, b) in app.layout.iter().enumerate().skip(idx + 1) {
                if b.node.is_none() || b.depth != a.depth {
                    continue;
                }
                if b.rect.width < 4 || b.rect.height < 3 {
                    continue;
                }
                let da = drawn[idx];
                let db = drawn[jdx];
                assert_eq!(
                    da.intersection(db).area(),
                    0,
                    "siblings overlap: {:?} and {:?}",
                    da,
                    db
                );
            }
        }
    }

    #[test]
    fn selection_moves_and_zooms() {
        let mut app = test_app();
        app.ensure_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        let first = app.selection;
        assert_eq!(app.tree.name(first), "target");
        app.handle_key(key(KeyCode::Down));
        assert_ne!(app.selection, first);
        // Zoom into the largest directory.
        app.selection = first;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.tree.name(app.root), "target");
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.tree.name(app.root), "root");
    }

    #[test]
    fn mode_cycle_and_depth() {
        let mut app = test_app();
        assert_eq!(app.size_mode, crate::tree::SizeMode::Size);
        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.size_mode, crate::tree::SizeMode::Files);
        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.size_mode, crate::tree::SizeMode::Age);
        let depth = app.depth;
        app.handle_key(key(KeyCode::Char(']')));
        assert_eq!(app.depth, depth + 1);
        app.handle_key(key(KeyCode::Char('[')));
        assert_eq!(app.depth, depth);
    }

    #[test]
    fn marks_and_confirm() {
        let mut app = test_app();
        app.ensure_layout(ratatui::layout::Rect::new(0, 0, 100, 30));
        app.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(app.marked.len(), 1);
        app.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(app.marked.len(), 0);
        app.handle_key(key(KeyCode::Char(' ')));
        app.handle_key(key(KeyCode::Char('d')));
        assert!(app.confirm.is_some());
        app.handle_key(key(KeyCode::Esc));
        assert!(app.confirm.is_none());
    }

    #[test]
    fn filter_matches_names() {
        let mut app = test_app();
        app.filter = "src".to_string();
        let src = app
            .tree
            .sorted_children(app.view_root(), crate::tree::SortKey::Size, false)
            .into_iter()
            .find(|&id| app.tree.name(id) == "src")
            .unwrap();
        assert!(app.matches_filter(src));
        let target = app
            .tree
            .sorted_children(app.view_root(), crate::tree::SortKey::Size, false)
            .into_iter()
            .find(|&id| app.tree.name(id) == "target")
            .unwrap();
        assert!(!app.matches_filter(target));
    }

    #[test]
    fn help_overlay_opens_and_closes() {
        let mut app = test_app();
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.help);
        let out = render(&mut app, 120, 40);
        assert!(out.contains("keys"));
        app.handle_key(key(KeyCode::Char('x')));
        assert!(!app.help);
    }

    #[test]
    fn small_terminal_does_not_panic() {
        let mut app = test_app();
        let _ = render(&mut app, 20, 4);
        let _ = render(&mut app, 40, 8);
    }

    #[test]
    fn renders_all_palettes_modes_and_themes() {
        use crate::color::{ColorPlan, Palette};
        use crate::config::{ColorModeName, ScaleName, ThemeName};
        use crate::ui::theme::Theme;
        for palette in Palette::SEQUENTIAL
            .iter()
            .chain(Palette::CATEGORICAL.iter())
        {
            for mode in [
                ColorModeName::Category,
                ColorModeName::Size,
                ColorModeName::Age,
                ColorModeName::Depth,
            ] {
                let mut app = test_app();
                app.plan = ColorPlan {
                    mode,
                    palette: *palette,
                    scale: ScaleName::Log,
                    reverse: true,
                    light: false,
                    saturation: 0.7,
                };
                app.invalidate_layout();
                let out = render(&mut app, 100, 30);
                assert!(out.contains("dw"), "{palette:?} {mode:?}");
            }
        }
        let mut app = test_app();
        app.theme = Theme::from_name(ThemeName::Light);
        app.plan.light = true;
        app.invalidate_layout();
        let out = render(&mut app, 100, 30);
        assert!(out.contains("dw"));
    }

    #[test]
    fn pending_app_renders_while_scanning() {
        let cfg = UiConfig {
            spec: ScanSpec {
                paths: vec![std::path::PathBuf::from(".")],
                ..Default::default()
            },
            config: Config::default(),
            categories: CategoryMap::builtin(),
            size_mode: SizeModeName::Size,
            sidebar: SidebarMode::Always,
            theme: ThemeName::Dark,
            keymap: crate::ui::keys::KeyMap::default_map(),
        };
        let mut app = App::new_pending(cfg);
        assert_eq!(app.tree.len(), 1);
        let out = render(&mut app, 100, 24);
        assert!(out.contains("nothing to show") || out.contains("scanning"));
    }

    /// Drive the real key path and print the resulting state, to check that
    /// shortcuts actually do what the footer promises.
    #[test]
    #[ignore = "diagnostic: DW_PREVIEW_PATH=F:\\projects cargo test simulate_keys -- --ignored --nocapture"]
    fn simulate_keys() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let path = std::env::var("DW_PREVIEW_PATH").unwrap_or_else(|_| ".".to_string());
        let palette = std::env::var("DW_PREVIEW_PALETTE")
            .ok()
            .and_then(|name| crate::color::Palette::from_name(&name))
            .unwrap_or(crate::color::Palette::Viridis);
        let saturation: f64 = std::env::var("DW_PREVIEW_SAT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.7);
        let color_mode = match std::env::var("DW_PREVIEW_MODE").as_deref() {
            Ok("category") => crate::config::ColorModeName::Category,
            Ok("age") => crate::config::ColorModeName::Age,
            Ok("depth") => crate::config::ColorModeName::Depth,
            _ => crate::config::ColorModeName::Size,
        };
        let mut config = Config::default();
        config.color.palette = palette.name().to_string();
        config.color.saturation = saturation;
        config.color.mode = color_mode;
        let spec = ScanSpec {
            paths: vec![std::path::PathBuf::from(&path)],
            ..Default::default()
        };
        let cfg = UiConfig {
            spec: spec.clone(),
            config,
            categories: CategoryMap::builtin(),
            size_mode: SizeModeName::Size,
            sidebar: SidebarMode::Always,
            theme: ThemeName::Auto,
            keymap: crate::ui::keys::KeyMap::default_map(),
        };
        let outcome = crate::scan::scan_all(
            &spec,
            std::sync::Arc::new(crate::scan::ScanProgress::default()),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();
        let tree = Tree::from_scan(
            outcome.root,
            &cfg.categories,
            outcome.errors,
            outcome.error_count,
            outcome.duration,
        );
        let mut app = App::new(tree, cfg);
        app.ensure_layout(ratatui::layout::Rect::new(0, 0, 110, 38));

        let show = |app: &App, label: &str| {
            let sel = app.selection;
            println!(
                "{label:<12} sel={:<28} root={:<16} depth={} mode={:?} colors={:?} filter={:?} marked={} quit={}",
                app.tree.path_of(sel).to_string_lossy(),
                app.tree.name(app.view_root()),
                app.depth,
                app.size_mode,
                app.plan.mode,
                app.filter,
                app.marked.len(),
                app.should_quit,
            );
        };
        let press = |app: &mut App, code: KeyCode| {
            app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
        };

        show(&app, "start");
        for key in [
            KeyCode::Char('l'),
            KeyCode::Char('l'),
            KeyCode::Char('j'),
            KeyCode::Char('j'),
            KeyCode::Char('h'),
            KeyCode::Char('k'),
        ] {
            press(&mut app, key);
            show(&app, &format!("{key:?}"));
        }
        press(&mut app, KeyCode::Enter);
        show(&app, "enter");
        press(&mut app, KeyCode::Backspace);
        show(&app, "backspace");
        press(&mut app, KeyCode::Char('t'));
        show(&app, "t");
        press(&mut app, KeyCode::Char('m'));
        show(&app, "m");
        press(&mut app, KeyCode::Char(']'));
        show(&app, "]");
        press(&mut app, KeyCode::Char('['));
        show(&app, "[");
        press(&mut app, KeyCode::Char(' '));
        show(&app, "space");
        press(&mut app, KeyCode::Char('d'));
        show(&app, "d");
        assert!(app.confirm.is_some(), "d should open the confirm dialog");
        press(&mut app, KeyCode::Esc);
        show(&app, "esc");
        press(&mut app, KeyCode::Char('/'));
        for ch in "fastf1".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        press(&mut app, KeyCode::Enter);
        show(&app, "filter");
        press(&mut app, KeyCode::Char('0'));
        show(&app, "0");
        press(&mut app, KeyCode::Char('q'));
        show(&app, "q");
        assert!(app.should_quit, "q should quit");
    }

    #[test]
    #[ignore = "manual visual check: cargo test dump_frame -- --ignored --nocapture"]
    fn dump_frame() {
        let mut app = test_app();
        let (w, h) = (120u16, 36u16);
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw::draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        for y in 0..h {
            let mut line = String::new();
            for x in 0..w {
                line.push_str(buffer[(x, y)].symbol());
            }
            println!("{}", line.trim_end());
        }
    }

    /// Render the TUI frame to an HTML file so the colors can be inspected
    /// outside a terminal: `DW_PREVIEW_PATH=F:\projects cargo test preview_html -- --ignored`.
    ///
    /// Variants for comparing looks:
    /// `DW_PREVIEW_PALETTE=cividis DW_PREVIEW_SAT=0.6 DW_PREVIEW_MODE=size
    ///  DW_PREVIEW_OUT=target/preview-cividis.html cargo test preview_html -- --ignored`
    #[test]
    #[ignore = "writes target/tui-preview.html"]
    fn preview_html() {
        use ratatui::style::{Color, Modifier};
        fn css(color: Color, default: (u8, u8, u8)) -> String {
            match color {
                Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                Color::Reset => format!("#{:02x}{:02x}{:02x}", default.0, default.1, default.2),
                Color::Black => "#1c1c22".to_string(),
                Color::DarkGray => "#6c6c78".to_string(),
                Color::LightBlue => "#61afef".to_string(),
                Color::LightYellow => "#e5c07b".to_string(),
                Color::LightGreen => "#98c379".to_string(),
                other => format!("{other:?}"),
            }
        }
        fn escape(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        }
        let path = std::env::var("DW_PREVIEW_PATH").unwrap_or_else(|_| ".".to_string());
        let palette = std::env::var("DW_PREVIEW_PALETTE")
            .ok()
            .and_then(|name| crate::color::Palette::from_name(&name))
            .unwrap_or(crate::color::Palette::Viridis);
        let saturation: f64 = std::env::var("DW_PREVIEW_SAT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.7);
        let color_mode = match std::env::var("DW_PREVIEW_MODE").as_deref() {
            Ok("category") => crate::config::ColorModeName::Category,
            Ok("age") => crate::config::ColorModeName::Age,
            Ok("depth") => crate::config::ColorModeName::Depth,
            _ => crate::config::ColorModeName::Size,
        };
        let mut config = Config::default();
        let out_path = std::env::var("DW_PREVIEW_OUT")
            .unwrap_or_else(|_| "target/tui-preview.html".to_string());
        config.color.palette = palette.name().to_string();
        config.color.saturation = saturation;
        config.color.mode = color_mode;
        let spec = ScanSpec {
            paths: vec![std::path::PathBuf::from(&path)],
            ..Default::default()
        };
        let cfg = UiConfig {
            spec: spec.clone(),
            config,
            categories: CategoryMap::builtin(),
            size_mode: SizeModeName::Size,
            sidebar: SidebarMode::Always,
            theme: ThemeName::Auto,
            keymap: crate::ui::keys::KeyMap::default_map(),
        };
        let outcome = crate::scan::scan_all(
            &spec,
            std::sync::Arc::new(crate::scan::ScanProgress::default()),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();
        let tree = Tree::from_scan(
            outcome.root,
            &cfg.categories,
            outcome.errors,
            outcome.error_count,
            outcome.duration,
        );
        let mut app = App::new(tree, cfg);
        let (w, h) = (150u16, 42u16);
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw::draw(frame, &mut app)).unwrap();
        // Layout statistics: label density and tiny-cell share are the two
        // numbers that decide whether a treemap reads as calm or as noise.
        {
            let real: Vec<_> = app.layout.iter().filter(|i| i.node.is_some()).collect();
            let labelled = real.iter().filter(|i| i.rect.width >= 8).count();
            let detailed = real
                .iter()
                .filter(|i| i.rect.width >= 14 && i.rect.height >= 3 && i.rect.area() >= 60)
                .count();
            let tiny = real.iter().filter(|i| i.rect.area() < 12).count();
            let overflow = app.layout.len() - real.len();
            let mut depths = std::collections::BTreeMap::new();
            for i in &real {
                *depths.entry(i.depth).or_insert(0usize) += 1;
            }
            println!(
                "layout: {} cells ({} real, {} aggregate), labelled {}, detailed {}, tiny {}, depths {:?}",
                app.layout.len(),
                real.len(),
                overflow,
                labelled,
                detailed,
                tiny,
                depths
            );
        }
        let buffer = terminal.backend().buffer();
        let mut html = String::from(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>dw tui preview</title>\
             <style>body{background:#0b0b0f;margin:0;padding:16px}\
             pre{font:14px/1.15 'Cascadia Mono',Consolas,monospace;white-space:pre}</style>\
             </head><body><pre>",
        );
        for y in 0..h {
            for x in 0..w {
                let cell = &buffer[(x, y)];
                let fg = css(cell.fg, (230, 230, 240));
                let bg = css(cell.bg, (11, 11, 15));
                let bold = if cell.modifier.contains(Modifier::BOLD) {
                    "font-weight:700;"
                } else {
                    ""
                };
                let sym = cell.symbol();
                let sym = if sym == " " { "&nbsp;" } else { &escape(sym) };
                html.push_str(&format!(
                    "<span style=\"color:{fg};background:{bg};{bold}\">{sym}</span>"
                ));
            }
            html.push('\n');
        }
        html.push_str("</pre></body></html>");
        std::fs::write(&out_path, html).unwrap();
    }
}
