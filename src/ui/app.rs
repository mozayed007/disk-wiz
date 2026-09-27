//! TUI application state and input handling.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::color::{CategoryMap, ColorPlan, Palette, Scale};
use crate::config::{Config, SidebarMode, SidebarPosition, SizeModeName, ThemeName, WorthConfig};
use crate::layout::{self, OVERFLOW};
use crate::scan::{ScanOutcome, ScanProgress, ScanSpec, scan_all};
use crate::tree::{NodeId, SizeMode, Tree};
use crate::util::{format_count, format_duration, now_epoch};

use super::keys::{Action, KeyMap};
use super::theme::Theme;

/// Everything the TUI needs to run and to rescan.
pub struct UiConfig {
    pub spec: ScanSpec,
    pub config: Config,
    pub categories: CategoryMap,
    pub size_mode: SizeModeName,
    pub sidebar: SidebarMode,
    pub theme: ThemeName,
    pub keymap: KeyMap,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// One visible treemap cell.
#[derive(Clone, Copy, Debug)]
pub struct LayoutItem {
    pub node: Option<NodeId>,
    pub rect: Rect,
    /// Number of entries aggregated into this cell (0 for real nodes).
    pub overflow: usize,
    pub color: (u8, u8, u8),
    /// Nesting depth of this cell below the view root (top level is 1).
    pub depth: u16,
    /// True when children were laid out inside this cell.
    pub has_children: bool,
}

#[derive(PartialEq)]
struct LayoutKey {
    root: NodeId,
    mode: SizeMode,
    depth: usize,
    reverse: bool,
    area: Rect,
}

/// An entry in the "worth a look" list.
pub struct WorthItem {
    pub node: NodeId,
    pub label: String,
    pub size: u64,
}

pub struct DiskInfo {
    pub label: String,
    pub total: u64,
    pub free: u64,
}

struct ScanJob {
    progress: Arc<ScanProgress>,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<ScanOutcome, String>>,
    started: Instant,
}

#[derive(Clone, Copy)]
pub enum ConfirmKind {
    Trash(usize),
}

pub struct Confirm {
    pub message: String,
    pub kind: ConfirmKind,
}

pub struct App {
    pub tree: Tree,
    pub cfg: UiConfig,
    pub root: NodeId,
    pub depth: usize,
    pub size_mode: SizeMode,
    pub reverse: bool,
    pub selection: NodeId,
    pub marked: BTreeSet<NodeId>,
    pub filter: String,
    pub filter_active: bool,
    pub help: bool,
    pub confirm: Option<Confirm>,
    pub status: Option<(String, Instant)>,
    scan: Option<ScanJob>,
    pub last_scan: Option<(u64, Duration)>,
    pub disk: Option<DiskInfo>,
    pub worth: Vec<WorthItem>,
    pub theme: Theme,
    pub palette: Palette,
    pub plan: ColorPlan,
    pub sidebar_mode: SidebarMode,
    pub sidebar_position: SidebarPosition,
    pub mouse: bool,
    pub layout: Vec<LayoutItem>,
    pub scale: Scale,
    layout_key: Option<LayoutKey>,
    pub should_quit: bool,
    last_click: Option<(Instant, u16, u16)>,
}

impl App {
    pub fn new(tree: Tree, cfg: UiConfig) -> App {
        let mut app = Self::with_tree(tree, cfg);
        app.refresh_derived();
        app.select_first();
        app
    }

    /// Start with a placeholder tree and scan in the background.
    pub fn new_pending(cfg: UiConfig) -> App {
        let name = cfg
            .spec
            .paths
            .first()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());
        let placeholder = Tree::from_scan(
            crate::tree::ScanNode::new(std::ffi::OsString::from(name), crate::tree::NodeKind::Dir),
            &cfg.categories,
            Vec::new(),
            0,
            Duration::ZERO,
        );
        Self::with_tree(placeholder, cfg)
    }

    fn with_tree(tree: Tree, cfg: UiConfig) -> App {
        let palette = Palette::from_name(&cfg.config.color.palette).unwrap_or(Palette::Viridis);
        let theme = Theme::from_name(cfg.theme);
        let plan = ColorPlan {
            mode: cfg.config.color.mode,
            palette,
            scale: cfg.config.color.scale,
            reverse: cfg.config.color.reverse,
            light: theme.is_light(),
            saturation: cfg.config.color.saturation,
        };
        let depth = cfg.config.tui.depth.clamp(1, 12);
        let size_mode = match cfg.size_mode {
            SizeModeName::Size => SizeMode::Size,
            SizeModeName::Files => SizeMode::Files,
            SizeModeName::Age => SizeMode::Age,
        };
        let mouse = cfg.config.tui.mouse;
        let sidebar_mode = cfg.sidebar;
        let sidebar_position = cfg.config.tui.sidebar_position;
        App {
            root: tree.root(),
            selection: tree.root(),
            tree,
            cfg,
            depth,
            size_mode,
            reverse: false,
            marked: BTreeSet::new(),
            filter: String::new(),
            filter_active: false,
            help: false,
            confirm: None,
            status: None,
            scan: None,
            last_scan: None,
            disk: None,
            worth: Vec::new(),
            theme,
            palette,
            plan,
            sidebar_mode,
            sidebar_position,
            mouse,
            layout: Vec::new(),
            scale: Scale::new(&[], plan.scale),
            layout_key: None,
            should_quit: false,
            last_click: None,
        }
    }

    /// Total size of the current tree, and its scan error count.
    pub fn totals(&self) -> (u64, u64) {
        (self.tree.node(self.tree.root()).size, self.tree.error_count)
    }

    fn select_first(&mut self) {
        if let Some(&first) = self
            .tree
            .sorted_children(self.root, self.size_mode.sort_key(), false)
            .first()
        {
            self.selection = first;
        } else {
            self.selection = self.root;
        }
    }

    /// Recompute data derived from the tree (worth a look, disk, marks).
    fn refresh_derived(&mut self) {
        self.worth = compute_worth(&self.tree, &self.cfg.config.worth_a_look);
        self.disk = compute_disk(self.cfg.spec.paths.first());
        self.marked.clear();
        self.layout_key = None;
        if self.root as usize >= self.tree.len() {
            self.root = self.tree.root();
        }
        if self.selection as usize >= self.tree.len() {
            self.selection = self.root;
        }
    }

    // -- scanning -----------------------------------------------------------

    pub fn start_scan(&mut self) {
        if let Some(job) = &self.scan {
            job.cancel.store(true, Ordering::Relaxed);
        }
        let spec = self.cfg.spec.clone();
        let progress = Arc::new(ScanProgress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let progress_thread = progress.clone();
        let cancel_thread = cancel.clone();
        std::thread::spawn(move || {
            let result = scan_all(&spec, progress_thread, cancel_thread).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.scan = Some(ScanJob {
            progress,
            cancel,
            rx,
            started: Instant::now(),
        });
    }

    pub fn poll_scan(&mut self) {
        let Some(job) = &self.scan else {
            return;
        };
        match job.rx.try_recv() {
            Ok(Ok(outcome)) => {
                let synthetic = self.cfg.spec.paths.len() > 1;
                let tree = Tree::from_scan_with(
                    outcome.root,
                    &self.cfg.categories,
                    outcome.errors,
                    outcome.error_count,
                    outcome.duration,
                    synthetic,
                );
                let entries = tree.len() as u64;
                let duration = outcome.duration;
                self.tree = tree;
                self.root = self.tree.root();
                self.selection = self.root;
                self.last_scan = Some((entries, duration));
                self.scan = None;
                self.refresh_derived();
                self.select_first();
                self.status = Some((
                    format!(
                        "scanned {} entries in {}",
                        format_count(entries),
                        format_duration(duration)
                    ),
                    Instant::now(),
                ));
            }
            Ok(Err(e)) => {
                self.scan = None;
                self.status = Some((format!("scan failed: {e}"), Instant::now()));
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.scan = None;
            }
        }
    }

    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    pub fn scan_progress(&self) -> Option<(u64, u64, f64)> {
        let job = self.scan.as_ref()?;
        let snap = job.progress.snapshot();
        Some((
            snap.entries,
            snap.bytes,
            job.started.elapsed().as_secs_f64(),
        ))
    }

    /// Per-frame maintenance.
    pub fn tick(&mut self) {
        self.poll_scan();
        if let Some((_, at)) = &self.status
            && at.elapsed() > Duration::from_secs(8)
        {
            self.status = None;
        }
    }

    // -- layout -------------------------------------------------------------

    /// Compute (and cache) the treemap layout for the given area.
    pub fn ensure_layout(&mut self, area: Rect) {
        let key = LayoutKey {
            root: self.root,
            mode: self.size_mode,
            depth: self.depth,
            reverse: self.reverse,
            area,
        };
        if self.layout_key.as_ref() == Some(&key) {
            return;
        }
        let mut items: Vec<LayoutItem> = Vec::new();
        if area.width > 0 && area.height > 0 {
            layout_node(
                &self.tree,
                self.root,
                area,
                self.depth,
                self.size_mode,
                self.reverse,
                1,
                &mut items,
            );
        }
        // Selection must point at a visible node.
        if !items.iter().any(|i| i.node == Some(self.selection)) {
            self.selection = self
                .tree
                .sorted_children(self.root, self.size_mode.sort_key(), false)
                .first()
                .copied()
                .unwrap_or(self.root);
        }
        // Colors: scale and ranks over visible real nodes.
        let sort = self.size_mode.sort_key();
        let mut real: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.node.is_some())
            .map(|(idx, _)| idx)
            .collect();
        real.sort_by_key(|&i| {
            std::cmp::Reverse(self.tree.value_for(items[i].node.expect("real"), sort))
        });
        let values: Vec<f64> = real
            .iter()
            .map(|&i| self.tree.value_for(items[i].node.expect("real"), sort) as f64)
            .collect();
        self.scale = Scale::new(&values, self.plan.scale);
        let count = real.len();
        for (rank, &idx) in real.iter().enumerate() {
            let node = items[idx].node.expect("real");
            items[idx].color =
                self.plan
                    .node_color(&self.tree, node, sort, rank, count, &self.scale);
        }
        for item in items.iter_mut().filter(|i| i.node.is_none()) {
            item.color = if self.theme.is_light() {
                (198, 200, 208)
            } else {
                (72, 75, 86)
            };
        }
        self.layout = items;
        self.layout_key = Some(key);
    }

    pub fn invalidate_layout(&mut self) {
        self.layout_key = None;
    }

    pub fn selection_item(&self) -> Option<&LayoutItem> {
        self.layout.iter().find(|i| i.node == Some(self.selection))
    }

    pub fn node_at(&self, x: u16, y: u16) -> Option<NodeId> {
        self.layout
            .iter()
            .rev()
            .find(|i| i.node.is_some() && i.rect.contains((x, y).into()))
            .and_then(|i| i.node)
    }

    // -- navigation ---------------------------------------------------------

    /// Move the selection to the nearest cell in a direction.
    ///
    /// Movement is level-by-level: first among siblings, then up one level to
    /// the parent's siblings, and so on. That keeps the selection in a
    /// predictable region instead of jumping into arbitrary nested cells.
    pub fn move_selection(&mut self, dir: Dir) {
        let Some(cur) = self.selection_item().copied() else {
            return;
        };
        let mut anchor = self.selection;
        let mut anchor_rect = cur.rect;
        loop {
            let parent = self.tree.node(anchor).parent;
            if parent == crate::tree::NO_PARENT {
                break;
            }
            if let Some(best) = self.nearest_sibling(parent, anchor, anchor_rect, dir) {
                self.selection = best;
                return;
            }
            anchor = parent;
            match self.layout.iter().find(|i| i.node == Some(anchor)) {
                Some(item) => anchor_rect = item.rect,
                None => break,
            }
        }
    }

    /// Nearest visible sibling of `current` in a direction.
    fn nearest_sibling(
        &self,
        parent: NodeId,
        current: NodeId,
        cur_rect: Rect,
        dir: Dir,
    ) -> Option<NodeId> {
        let cx = cur_rect.x as f64 + cur_rect.width as f64 / 2.0;
        let cy = cur_rect.y as f64 + cur_rect.height as f64 / 2.0;
        let mut best: Option<(f64, NodeId)> = None;
        for &sib in self.tree.children_of(parent) {
            if sib == current {
                continue;
            }
            let Some(item) = self.layout.iter().find(|i| i.node == Some(sib)) else {
                continue;
            };
            let ix = item.rect.x as f64 + item.rect.width as f64 / 2.0;
            let iy = item.rect.y as f64 + item.rect.height as f64 / 2.0;
            let dx = ix - cx;
            let dy = iy - cy;
            let (primary, secondary) = match dir {
                Dir::Right => (dx, dy.abs()),
                Dir::Left => (-dx, dy.abs()),
                Dir::Down => (dy, dx.abs()),
                Dir::Up => (-dy, dx.abs()),
            };
            if primary <= 0.5 {
                continue;
            }
            let score = primary + 2.0 * secondary;
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, sib));
            }
        }
        best.map(|(_, node)| node)
    }

    pub fn zoom_in(&mut self) {
        if !self.tree.is_dir(self.selection) {
            return;
        }
        if self.tree.children_of(self.selection).is_empty() {
            return;
        }
        self.root = self.selection;
        self.select_first();
        self.invalidate_layout();
    }

    pub fn zoom_out(&mut self) {
        if self.root == self.tree.root() {
            return;
        }
        let old = self.root;
        let parent = self.tree.node(old).parent;
        if parent == crate::tree::NO_PARENT {
            return;
        }
        self.root = parent;
        self.selection = old;
        self.invalidate_layout();
    }

    pub fn set_depth(&mut self, depth: usize) {
        self.depth = depth.clamp(1, 12);
        self.invalidate_layout();
    }

    pub fn cycle_mode(&mut self) {
        self.size_mode = self.size_mode.next();
        self.invalidate_layout();
    }

    /// Cycle the color mapping: category, size, age, depth.
    pub fn cycle_color_mode(&mut self) {
        use crate::config::ColorModeName;
        self.plan.mode = match self.plan.mode {
            ColorModeName::Category => ColorModeName::Size,
            ColorModeName::Size => ColorModeName::Age,
            ColorModeName::Age => ColorModeName::Depth,
            ColorModeName::Depth => ColorModeName::Category,
        };
        self.status = Some((
            format!("colors: {:?}", self.plan.mode).to_lowercase(),
            Instant::now(),
        ));
        self.invalidate_layout();
    }

    /// Cycle the palette so the look can be judged live.
    pub fn cycle_palette(&mut self) {
        self.palette = self.palette.next();
        self.plan.palette = self.palette;
        self.status = Some((format!("palette: {}", self.palette.name()), Instant::now()));
        self.invalidate_layout();
    }

    pub fn reset_view(&mut self) {
        self.root = self.tree.root();
        self.depth = self.cfg.config.tui.depth.clamp(1, 12);
        self.reverse = false;
        self.filter.clear();
        self.filter_active = false;
        self.select_first();
        self.invalidate_layout();
    }

    pub fn toggle_hidden(&mut self) {
        self.cfg.spec.hidden = !self.cfg.spec.hidden;
        self.rescan("rescanning");
    }

    pub fn toggle_apparent(&mut self) {
        self.cfg.spec.apparent_size = !self.cfg.spec.apparent_size;
        self.rescan("rescanning");
    }

    fn rescan(&mut self, message: &str) {
        self.status = Some((message.to_string(), Instant::now()));
        self.start_scan();
    }

    // -- marks and trash ----------------------------------------------------

    pub fn toggle_mark(&mut self) {
        if self.selection == self.view_root() {
            return;
        }
        if !self.marked.insert(self.selection) {
            self.marked.remove(&self.selection);
        }
    }

    pub fn clear_marks(&mut self) {
        self.marked.clear();
    }

    pub fn request_trash(&mut self) {
        let count = self.marked.len();
        if count == 0 {
            self.status = Some((
                "nothing marked (space marks entries)".into(),
                Instant::now(),
            ));
            return;
        }
        self.confirm = Some(Confirm {
            message: format!(
                "Move {count} marked {} to the trash?",
                if count == 1 { "entry" } else { "entries" }
            ),
            kind: ConfirmKind::Trash(count),
        });
    }

    pub fn execute_trash(&mut self, count: usize) {
        let paths: Vec<PathBuf> = self
            .marked
            .iter()
            .map(|&id| self.tree.path_of(id))
            .collect();
        let mut errors = 0usize;
        let mut first_error: Option<String> = None;
        for path in &paths {
            if path.as_os_str().is_empty() || !path.exists() {
                continue;
            }
            if let Err(e) = trash::delete(path) {
                errors += 1;
                if first_error.is_none() {
                    first_error = Some(format!("{}: {e}", path.display()));
                }
            }
        }
        self.marked.clear();
        let message = if errors == 0 {
            format!("moved {count} to trash")
        } else if let Some(err) = first_error {
            format!("{errors} of {count} failed: {err}")
        } else {
            format!("{errors} of {count} failed")
        };
        self.status = Some((message, Instant::now()));
        self.start_scan();
    }

    pub fn marked_size(&self) -> u64 {
        self.marked.iter().map(|&id| self.tree.node(id).size).sum()
    }

    // -- input --------------------------------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) {
        if let Some(confirm) = self.confirm.take() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => match confirm.kind {
                    ConfirmKind::Trash(count) => self.execute_trash(count),
                },
                _ => {}
            }
            return;
        }
        if self.help {
            self.help = false;
            return;
        }
        if self.filter_active {
            match key.code {
                KeyCode::Enter => self.filter_active = false,
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filter_active = false;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Char('c') && ctrl {
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::Esc {
            if !self.filter.is_empty() {
                self.filter.clear();
            } else {
                self.zoom_out();
            }
            return;
        }
        match self.cfg.keymap.action(key) {
            Some(Action::Quit) => self.should_quit = true,
            Some(Action::Help) => self.help = true,
            Some(Action::Filter) => self.filter_active = true,
            Some(Action::Mark) => self.toggle_mark(),
            Some(Action::Trash) => self.request_trash(),
            Some(Action::ClearMarks) => self.clear_marks(),
            Some(Action::Rescan) => self.rescan("rescanning"),
            Some(Action::Mode) => self.cycle_mode(),
            Some(Action::ColorMode) => self.cycle_color_mode(),
            Some(Action::Palette) => self.cycle_palette(),
            Some(Action::Hidden) => self.toggle_hidden(),
            Some(Action::Apparent) => self.toggle_apparent(),
            Some(Action::Reset) => self.reset_view(),
            Some(Action::DepthDec) => self.set_depth(self.depth.saturating_sub(1)),
            Some(Action::DepthInc) => self.set_depth(self.depth + 1),
            Some(Action::ZoomIn) => self.zoom_in(),
            Some(Action::ZoomOut) => self.zoom_out(),
            Some(Action::Up) => self.move_selection(Dir::Up),
            Some(Action::Down) => self.move_selection(Dir::Down),
            Some(Action::Left) => self.move_selection(Dir::Left),
            Some(Action::Right) => self.move_selection(Dir::Right),
            None => {}
        }
    }

    pub fn handle_mouse(&mut self, m: MouseEvent) {
        if !self.mouse {
            return;
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let pos = (m.column, m.row);
                let double = self.last_click.is_some_and(|(at, x, y)| {
                    at.elapsed() < Duration::from_millis(400) && x == pos.0 && y == pos.1
                });
                self.last_click = Some((Instant::now(), pos.0, pos.1));
                if let Some(node) = self.node_at(pos.0, pos.1) {
                    self.selection = node;
                    if double {
                        self.zoom_in();
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Right) => self.zoom_out(),
            MouseEventKind::ScrollDown => self.move_selection(Dir::Down),
            MouseEventKind::ScrollUp => self.move_selection(Dir::Up),
            _ => {}
        }
    }

    /// The scan root of the whole tree.
    pub fn tree_root(&self) -> NodeId {
        self.tree.root()
    }

    /// The root of the current view (zoom target).
    pub fn view_root(&self) -> NodeId {
        self.root
    }

    pub fn sidebar_visible(&self, width: u16) -> bool {
        match self.sidebar_mode {
            SidebarMode::Always => true,
            SidebarMode::Never => false,
            SidebarMode::Auto => width >= 100,
        }
    }

    /// Breadcrumb from the scan root to the current zoom root.
    pub fn breadcrumb(&self) -> Vec<NodeId> {
        self.tree.ancestors(self.root)
    }

    /// Does a node match the current filter (case-insensitive substring)?
    pub fn matches_filter(&self, id: NodeId) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let needle = self.filter.to_lowercase();
        self.tree.name(id).to_lowercase().contains(needle.as_str())
    }
}

/// Maximum children drawn per cell; the rest aggregate into one cell.
///
/// Bounding the fan-out guarantees every drawn child gets a readable share of
/// its parent instead of a one-cell sliver (the "N more" pattern).
const MAX_CHILDREN: usize = 24;

/// Recursively lay out visible cells for a node's children.
#[allow(clippy::too_many_arguments)]
fn layout_node(
    tree: &Tree,
    id: NodeId,
    rect: Rect,
    depth_left: usize,
    mode: SizeMode,
    reverse: bool,
    depth: u16,
    out: &mut Vec<LayoutItem>,
) {
    if depth_left == 0 {
        return;
    }
    let sort = mode.sort_key();
    let kids = tree.sorted_children(id, sort, reverse);
    let keep = if kids.len() > MAX_CHILDREN {
        MAX_CHILDREN - 1
    } else {
        kids.len()
    };
    let aggregated = kids.len() - keep;
    let mut weights: Vec<u64> = kids[..keep]
        .iter()
        .map(|&k| tree.value_for(k, sort).max(1))
        .collect();
    let overflow_index = if aggregated > 0 {
        let sum: u64 = kids[keep..]
            .iter()
            .map(|&k| tree.value_for(k, sort).max(1))
            .sum();
        weights.push(sum.max(1));
        Some(weights.len() - 1)
    } else {
        None
    };
    let (entries, dropped) = layout::treemap(&weights, rect);
    for entry in entries {
        let is_overflow = entry.index == OVERFLOW
            || (overflow_index.is_some() && Some(entry.index) == overflow_index);
        if is_overflow {
            let count = if entry.index == OVERFLOW {
                dropped
            } else {
                aggregated + dropped
            };
            out.push(LayoutItem {
                node: None,
                rect: entry.rect,
                overflow: count,
                color: (72, 75, 86),
                depth,
                has_children: false,
            });
            continue;
        }
        let child = kids[entry.index];
        let inner = if depth_left > 1 && tree.is_dir(child) {
            inner_rect(entry.rect)
        } else {
            None
        };
        out.push(LayoutItem {
            node: Some(child),
            rect: entry.rect,
            overflow: 0,
            color: (72, 75, 86),
            depth,
            has_children: inner.is_some(),
        });
        if let Some(inner) = inner {
            layout_node(
                tree,
                child,
                inner,
                depth_left - 1,
                mode,
                reverse,
                depth + 1,
                out,
            );
        }
    }
}

/// Rows reserved at the top of a cell for its label.
pub fn label_rows(rect: Rect) -> u16 {
    if rect.width >= 12 && rect.height >= 3 {
        2
    } else {
        1
    }
}

/// The area of a rect left for children, below its label rows.
///
/// Recursion stops early for small cells: a three-by-three mosaic of tiny
/// rectangles is noise, not information, so a cell needs enough room to show
/// its children as real shapes before they are laid out.
fn inner_rect(rect: Rect) -> Option<Rect> {
    if rect.width < 9 || rect.height < 6 {
        return None;
    }
    let rows = label_rows(rect);
    let width = rect.width.saturating_sub(2);
    let height = rect.height.saturating_sub(rows + 1);
    if width < 7 || height < 3 {
        return None;
    }
    Some(Rect {
        x: rect.x + 1,
        y: rect.y + rows,
        width,
        height,
    })
}

/// Match worth-a-look rules against the tree.
fn compute_worth(tree: &Tree, cfg: &WorthConfig) -> Vec<WorthItem> {
    if !cfg.enabled || cfg.max_items == 0 || tree.is_empty() {
        return Vec::new();
    }
    let now = now_epoch();
    let mut rules = Vec::new();
    for rule in &cfg.rules {
        if let Ok(glob) = globset::GlobBuilder::new(&rule.glob)
            .literal_separator(false)
            .build()
        {
            let older = rule.older_than.as_deref().and_then(parse_age_spec);
            rules.push((glob.compile_matcher(), older, rule.name.clone()));
        }
    }
    if rules.is_empty() {
        return Vec::new();
    }
    const MIN_SIZE: u64 = 1 << 20; // 1 MiB
    let mut matches: Vec<WorthItem> = Vec::new();
    let mut large: Vec<(u64, NodeId)> = Vec::new();
    let mut stack: Vec<NodeId> = vec![tree.root()];
    while let Some(id) = stack.pop() {
        for &child in tree.children_of(id) {
            if !tree.is_dir(child) {
                continue;
            }
            let size = tree.node(child).size;
            if size < MIN_SIZE {
                continue;
            }
            stack.push(child);
            let rel = rel_path_string(tree, child);
            let mut matched = false;
            for (matcher, older, name) in &rules {
                if !matcher.is_match(rel.as_str()) {
                    continue;
                }
                let age_ok = match older {
                    None => true,
                    Some(secs) => {
                        let mtime = tree.node(child).mtime as u64;
                        mtime > 0 && now.saturating_sub(mtime) >= *secs
                    }
                };
                if age_ok {
                    matches.push(WorthItem {
                        node: child,
                        label: name.clone(),
                        size,
                    });
                    matched = true;
                    break;
                }
            }
            if !matched {
                large.push((size, child));
            }
        }
    }
    let mut seen: BTreeSet<NodeId> = BTreeSet::new();
    matches.retain(|m| seen.insert(m.node));
    matches.sort_by_key(|m| std::cmp::Reverse(m.size));
    if matches.len() < cfg.max_items {
        large.sort_by_key(|&(size, _)| std::cmp::Reverse(size));
        for (size, node) in large {
            if matches.len() >= cfg.max_items {
                break;
            }
            if seen.insert(node) {
                matches.push(WorthItem {
                    node,
                    label: "large".to_string(),
                    size,
                });
            }
        }
    }
    matches.truncate(cfg.max_items);
    matches
}

fn rel_path_string(tree: &Tree, id: NodeId) -> String {
    let mut parts = Vec::new();
    let mut cur = id;
    while cur != tree.root() {
        parts.push(tree.name(cur).into_owned());
        cur = tree.node(cur).parent;
        if cur == crate::tree::NO_PARENT {
            break;
        }
    }
    let mut out = String::new();
    for part in parts.iter().rev() {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(part);
    }
    out
}

/// Parse `7d`, `12h`, `30m`, `2w` into seconds.
fn parse_age_spec(spec: &str) -> Option<u64> {
    let spec = spec.trim();
    let (num, unit) = spec.split_at(spec.find(|c: char| c.is_ascii_alphabetic())?);
    let value: u64 = num.trim().parse().ok()?;
    let secs = match unit.trim().to_ascii_lowercase().as_str() {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return None,
    };
    Some(value * secs)
}

fn compute_disk(path: Option<&PathBuf>) -> Option<DiskInfo> {
    let path = path?;
    let total = fs4::total_space(path).ok()?;
    let free = fs4::available_space(path).ok()?;
    let label = mount_label(path);
    Some(DiskInfo { label, total, free })
}

fn mount_label(path: &std::path::Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        let s = canonical.to_string_lossy();
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        if s.len() >= 2 && s.as_bytes()[1] == b':' {
            return s[..2].to_string();
        }
    }
    let mut cur = canonical.as_path();
    loop {
        match cur.parent() {
            Some(p) if p.as_os_str().is_empty() => break,
            Some(p) => cur = p,
            None => break,
        }
    }
    cur.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_age_specs() {
        assert_eq!(parse_age_spec("7d"), Some(7 * 86_400));
        assert_eq!(parse_age_spec("12h"), Some(12 * 3600));
        assert_eq!(parse_age_spec("2w"), Some(2 * 604_800));
        assert_eq!(parse_age_spec("nope"), None);
    }
}
