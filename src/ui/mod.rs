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
    install_panic_hook();
    let mut app = App::new(tree, cfg);
    let mut terminal = setup()?;
    let result = event_loop(&mut terminal, &mut app);
    let restored = restore(&mut terminal);
    result.and(restored)
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
            .sorted_children(app.root(), crate::tree::SortKey::Size, false)
            .into_iter()
            .find(|&id| app.tree.name(id) == "src")
            .unwrap();
        assert!(app.matches_filter(src));
        let target = app
            .tree
            .sorted_children(app.root(), crate::tree::SortKey::Size, false)
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
}
