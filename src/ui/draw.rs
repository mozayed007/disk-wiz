//! TUI rendering.
//!
//! Design rules: the chrome uses the terminal's own palette (theme `auto`),
//! structure comes from box-drawing rules and spacing rather than heavy
//! color, and treemap cells carry the data color. Cells are shaded by depth
//! and separated by darkened edges so nesting is readable; labels only appear
//! when a cell can show them.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::color::Rgb;
use crate::config::{ColorModeName, SidebarPosition};
use crate::tree::SizeMode;
use crate::util::{
    display_width, format_age, format_count, format_duration, format_size, truncate_to_width,
};

use super::app::{App, label_rows};
use super::theme::{Theme, rgb};

/// Minimum cell size that can show a name.
const LABEL_MIN_W: u16 = 8;
/// Minimum cell size that can show a size line.
const DETAIL_MIN_W: u16 = 14;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let theme = app.theme;
    if area.width < 24 || area.height < 6 {
        frame.render_widget(
            Paragraph::new("dw: terminal too small")
                .style(Style::default().fg(theme.fg).bg(theme.bg)),
            area,
        );
        return;
    }
    {
        let mut c = Canvas {
            buf: frame.buffer_mut(),
            area,
        };
        c.style(area, Style::default().bg(theme.bg).fg(theme.fg));
    }

    let filter_h: u16 = if app.filter_active || !app.filter.is_empty() {
        1
    } else {
        0
    };
    let header_h: u16 = 1;
    let legend_h: u16 = if area.height >= 14 { 1 } else { 0 };
    let footer_h: u16 = 1;
    let body_y = area.y + header_h + legend_h;
    let body_h = area
        .height
        .saturating_sub(header_h + legend_h + footer_h + filter_h);

    let sidebar_visible = app.sidebar_visible(area.width) && body_h >= 6;
    let sidebar_w: u16 = if sidebar_visible {
        (area.width / 4).clamp(26, 38)
    } else {
        0
    };
    let sep_w: u16 = if sidebar_visible { 1 } else { 0 };
    let treemap_w = area.width.saturating_sub(sidebar_w + sep_w);

    let (sidebar_area, treemap_area, sep_x) = match app.sidebar_position {
        SidebarPosition::Left => (
            Rect::new(area.x, body_y, sidebar_w, body_h),
            Rect::new(area.x + sidebar_w + sep_w, body_y, treemap_w, body_h),
            area.x + sidebar_w,
        ),
        SidebarPosition::Right => (
            Rect::new(area.right() - sidebar_w, body_y, sidebar_w, body_h),
            Rect::new(area.x, body_y, treemap_w, body_h),
            area.right() - sidebar_w - 1,
        ),
    };

    draw_header(frame, app, Rect::new(area.x, area.y, area.width, header_h));
    if body_h > 0 && treemap_w > 0 {
        // Layout first: the legend swatches are derived from visible cells.
        app.ensure_layout(treemap_area);
    }
    if legend_h > 0 {
        draw_legend(
            frame,
            app,
            Rect::new(area.x, area.y + header_h, area.width, legend_h),
        );
    }
    if body_h > 0 && treemap_w > 0 {
        draw_treemap(frame, app, treemap_area);
        if sidebar_visible {
            {
                let mut c = Canvas {
                    buf: frame.buffer_mut(),
                    area,
                };
                c.vline(
                    sep_x,
                    body_y,
                    body_y + body_h,
                    '│',
                    Style::default().fg(theme.border),
                );
            }
            draw_sidebar(frame, app, sidebar_area);
        }
    }
    let footer_y = area.bottom() - footer_h;
    if filter_h > 0 {
        draw_filter(
            frame,
            app,
            Rect::new(area.x, footer_y - filter_h, area.width, filter_h),
        );
    }
    draw_footer(
        frame,
        app,
        Rect::new(area.x, footer_y, area.width, footer_h),
    );

    if app.help {
        draw_help(frame, app, area);
    }
    if let Some(confirm) = &app.confirm {
        let message = confirm.message.clone();
        draw_confirm(frame, app, area, &message);
    }
}

// ---------------------------------------------------------------------------
// Canvas helpers
// ---------------------------------------------------------------------------

struct Canvas<'a> {
    buf: &'a mut Buffer,
    area: Rect,
}

impl Canvas<'_> {
    fn style(&mut self, rect: Rect, style: Style) {
        let rect = rect.intersection(self.area);
        if rect.width > 0 && rect.height > 0 {
            self.buf.set_style(rect, style);
        }
    }

    fn fill(&mut self, rect: Rect, color: Rgb) {
        let fg = contrast(color);
        self.style(rect, Style::default().bg(rgb(color)).fg(rgb(fg)));
    }

    /// Draw text; returns the x position after it.
    fn text(&mut self, x: u16, y: u16, text: &str, style: Style, max_w: usize) -> u16 {
        if y >= self.area.bottom() || x >= self.area.right() || max_w == 0 {
            return x;
        }
        let avail = (self.area.right() - x) as usize;
        let max_w = max_w.min(avail);
        let t = truncate_to_width(text, max_w);
        self.buf.set_stringn(x, y, t.as_ref(), max_w, style);
        x + display_width(t.as_ref()) as u16
    }

    fn text_right(&mut self, x0: u16, width: usize, y: u16, text: &str, style: Style) -> u16 {
        let w = display_width(text);
        if w > width {
            return self.text(x0, y, text, style, width);
        }
        self.text(x0 + (width - w) as u16, y, text, style, w)
    }

    fn put_char(&mut self, x: u16, y: u16, ch: char, style: Style) {
        if x >= self.area.right() || y >= self.area.bottom() {
            return;
        }
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        if let Some(cell) = self.buf.cell_mut((x, y)) {
            cell.set_symbol(s).set_style(style);
        }
    }

    fn hline(&mut self, y: u16, x0: u16, x1: u16, ch: char, style: Style) {
        for x in x0..x1.min(self.area.right()) {
            self.put_char(x, y, ch, style);
        }
    }

    fn vline(&mut self, x: u16, y0: u16, y1: u16, ch: char, style: Style) {
        for y in y0..y1.min(self.area.bottom()) {
            self.put_char(x, y, ch, style);
        }
    }
}

fn segments_width(segs: &[(String, Style)]) -> usize {
    segs.iter().map(|(s, _)| display_width(s)).sum()
}

fn draw_segments(c: &mut Canvas, mut x: u16, y: u16, segs: &[(String, Style)]) -> u16 {
    for (text, style) in segs {
        x = c.text(x, y, text, *style, display_width(text));
    }
    x
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let right = header_right(app);
    let right_w = segments_width(&right);
    let left_max = (area.width as usize).saturating_sub(right_w + 2);
    let mut c = Canvas {
        buf: frame.buffer_mut(),
        area,
    };
    c.style(area, Style::default().bg(theme.header_bg));

    let mut x = c.text(
        area.x,
        area.y,
        " dw ",
        Style::default()
            .bg(theme.accent)
            .fg(theme.chip_fg)
            .add_modifier(Modifier::BOLD),
        4,
    );
    x += 1;
    let crumbs = app.breadcrumb();
    for (i, &id) in crumbs.iter().enumerate() {
        let used = (x - area.x) as usize;
        if used >= left_max {
            break;
        }
        if i > 0 {
            x = c.text(x, area.y, " › ", Style::default().fg(theme.dim), 3);
        }
        let name = app.tree.name(id).into_owned();
        let style = if i + 1 == crumbs.len() {
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        };
        let remaining = left_max.saturating_sub((x - area.x) as usize);
        x = c.text(x, area.y, &name, style, remaining);
    }

    let rx = area.right().saturating_sub(right_w as u16);
    draw_segments(&mut c, rx, area.y, &right);
}

fn header_right(app: &App) -> Vec<(String, Style)> {
    let theme = app.theme;
    let mut segs: Vec<(String, Style)> = Vec::new();
    for mode in [SizeMode::Size, SizeMode::Files, SizeMode::Age] {
        let active = app.size_mode == mode;
        let style = if active {
            Style::default()
                .bg(theme.accent)
                .fg(theme.chip_fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        };
        segs.push((format!(" {} ", mode.label()), style));
        segs.push((" ".to_string(), Style::default()));
    }
    segs.push(("  ".to_string(), Style::default()));
    let hidden = app.cfg.spec.hidden;
    segs.push((
        if hidden { "✓ hidden" } else { "· hidden" }.to_string(),
        Style::default().fg(if hidden { theme.good } else { theme.dim }),
    ));
    segs.push(("   ".to_string(), Style::default()));
    let apparent = app.cfg.spec.apparent_size;
    segs.push((
        if apparent {
            "✓ apparent"
        } else {
            "· apparent"
        }
        .to_string(),
        Style::default().fg(if apparent { theme.good } else { theme.dim }),
    ));
    segs.push(("    ".to_string(), Style::default()));
    segs.push(("depth ".to_string(), Style::default().fg(theme.dim)));
    segs.push((
        app.depth.to_string(),
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
    ));
    segs.push((" ".to_string(), Style::default()));
    segs
}

// ---------------------------------------------------------------------------
// Legend row
// ---------------------------------------------------------------------------

fn draw_legend(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let mut c = Canvas {
        buf: frame.buffer_mut(),
        area,
    };
    c.style(area, Style::default().bg(theme.header_bg));

    let root = app.tree_root();
    let node = app.tree.node(root);
    let mut x = area.x + 1;
    let totals = format!(
        "{} · {} files · {} dirs",
        format_size(node.size),
        format_count(node.files as u64),
        format_count(node.dirs as u64)
    );
    x = c.text(x, area.y, &totals, Style::default().fg(theme.fg), 44);
    x = c.text(x, area.y, "   │   ", Style::default().fg(theme.border), 7);

    let right = legend_right(app);
    let right_w = segments_width(&right);
    let swatch_max = (area.right() as usize).saturating_sub(right_w + 2 + x as usize);
    match app.plan.mode {
        ColorModeName::Category => category_swatches(&mut c, app, x, area.y, swatch_max, theme),
        mode => ramp_swatch(&mut c, app, x, area.y, swatch_max, mode, theme),
    }

    let rx = area.right().saturating_sub(right_w as u16);
    draw_segments(&mut c, rx, area.y, &right);
}

fn legend_right(app: &App) -> Vec<(String, Style)> {
    let theme = app.theme;
    if let Some((entries, bytes, secs)) = app.scan_progress() {
        return vec![(
            format!(
                "scanning: {} entries · {} · {secs:.1}s",
                format_count(entries),
                format_size(bytes)
            ),
            Style::default().fg(theme.warn),
        )];
    }
    if let Some((entries, duration)) = app.last_scan {
        return vec![(
            format!(
                "{} entries · {}",
                format_count(entries),
                format_duration(duration)
            ),
            Style::default().fg(theme.dim),
        )];
    }
    Vec::new()
}

fn category_swatches(c: &mut Canvas, app: &App, x: u16, y: u16, max_w: usize, theme: Theme) {
    // Category totals across the whole tree (each byte counted once).
    let mut totals: Vec<(u16, u64)> = app
        .tree
        .category_totals
        .iter()
        .enumerate()
        .filter(|&(_, &size)| size > 0)
        .map(|(idx, &size)| (idx as u16, size))
        .collect();
    totals.sort_by_key(|&(_, size)| std::cmp::Reverse(size));
    let mut x = x;
    for (cat, size) in totals {
        let Some(name) = app.tree.category_ids.get(cat as usize) else {
            continue;
        };
        let color = app
            .tree
            .category_colors
            .get(cat as usize)
            .copied()
            .unwrap_or((128, 128, 128));
        let size_text = format_size(size);
        let seg_w = 2 + display_width(name) + 1 + display_width(&size_text) + 3;
        if (x as usize).saturating_sub(c.area.x as usize) + seg_w > max_w {
            break;
        }
        x = c.text(x, y, "●", Style::default().fg(rgb(color)), 2);
        x = c.text(x, y, " ", Style::default(), 1);
        x = c.text(
            x,
            y,
            name,
            Style::default().fg(theme.dim),
            display_width(name),
        );
        x = c.text(x, y, " ", Style::default(), 1);
        x = c.text(
            x,
            y,
            &size_text,
            Style::default().fg(theme.fg),
            display_width(&size_text),
        );
        x = c.text(x, y, "   ", Style::default(), 3);
    }
}

fn ramp_swatch(
    c: &mut Canvas,
    app: &App,
    x: u16,
    y: u16,
    max_w: usize,
    mode: ColorModeName,
    theme: Theme,
) {
    if max_w < 24 {
        return;
    }
    let (lo, hi) = match mode {
        ColorModeName::Size => ("small", "large"),
        ColorModeName::Age => ("old", "recent"),
        ColorModeName::Depth => ("shallow", "deep"),
        ColorModeName::Category => return,
    };
    let bar_w = (max_w - 16).clamp(8, 36);
    let palette_name = app.palette.name();
    let mut x = c.text(x, y, palette_name, Style::default().fg(theme.dim), 10);
    x = c.text(x, y, "  ", Style::default(), 2);
    x = c.text(x, y, lo, Style::default().fg(theme.dim), 8);
    x = c.text(x, y, " ", Style::default(), 1);
    for i in 0..bar_w {
        let t = i as f64 / (bar_w - 1).max(1) as f64;
        let t = if app.plan.reverse { 1.0 - t } else { t };
        let color = crate::color::sequential(app.plan.palette, t);
        c.put_char(x, y, ' ', Style::default().bg(rgb(color)));
        x += 1;
    }
    x = c.text(x, y, " ", Style::default(), 1);
    c.text(x, y, hi, Style::default().fg(theme.dim), 8);
}

// ---------------------------------------------------------------------------
// Treemap
// ---------------------------------------------------------------------------

fn draw_treemap(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let mut c = Canvas {
        buf: frame.buffer_mut(),
        area,
    };
    c.style(area, Style::default().bg(theme.bg));

    if app.layout.is_empty() {
        let text = if let Some((entries, bytes, secs)) = app.scan_progress() {
            format!(
                "scanning: {} entries · {} · {secs:.1}s",
                format_count(entries),
                format_size(bytes)
            )
        } else if app.tree.len() <= 1 {
            "nothing to show".to_string()
        } else {
            String::new()
        };
        if !text.is_empty() && area.height >= 3 {
            let y = area.y + area.height / 2;
            let x = area.x + area.width.saturating_sub(display_width(&text) as u16) / 2;
            c.text(
                x,
                y,
                &text,
                Style::default().fg(theme.warn),
                area.width as usize,
            );
        }
        return;
    }

    let total = app.tree.node(app.view_root()).size.max(1);
    let category_mode = app.plan.mode == ColorModeName::Category;
    for item in &app.layout {
        let rect = item.rect;
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        if item.node.is_none() {
            // Aggregate cell: neutral and quiet, no bevel or band.
            let base = if theme.is_light() {
                (216, 218, 224)
            } else {
                (52, 55, 66)
            };
            c.fill(rect, base);
            if rect.width >= 12 && rect.height >= 1 {
                let text = format!("{} more", item.overflow);
                let w = display_width(&text);
                if w + 2 <= rect.width as usize {
                    let x = rect.x + (rect.width - w as u16) / 2;
                    c.text(
                        x,
                        rect.y + rect.height / 2,
                        &text,
                        Style::default().fg(rgb(mix(base, contrast(base), 0.35))),
                        w,
                    );
                }
            }
            continue;
        }
        let node = item.node.expect("checked");
        let selected = node == app.selection;
        let dimmed = !app.matches_filter(node);
        let mut color = item.color;
        if dimmed {
            color = desaturate(color, theme.is_light());
        }
        if selected {
            color = emphasize(color, theme.is_light(), 0.30);
        }
        // In category mode the fill color is constant per category, so depth
        // lightening carries the hierarchy. In ramp modes size already varies
        // the lightness, so the layout gaps do the work instead.
        if category_mode {
            let lift = 1.0 + 0.12 * (item.depth.saturating_sub(1)).min(4) as f64;
            color = shade(color, lift.min(1.55));
        }

        // Draw the cell with a one-cell gap on its right and bottom edges so
        // the parent's color shows through: containment reads without hard
        // grid lines (shaded frames, Bruls et al.).
        let area_cells = rect.area();
        let gap = rect.width >= 4 && rect.height >= 3;
        let drawn = if gap {
            Rect {
                x: rect.x,
                y: rect.y,
                width: rect.width - 1,
                height: rect.height - 1,
            }
        } else {
            rect
        };
        c.fill(drawn, color);

        // Cushion bevel: light top/left, dark bottom/right (van Wijk).
        if area_cells >= 40 {
            bevel(&mut c, drawn, color);
        }
        // Label band above the children; stronger for the top level so
        // top-level groupings stand out (NN/g).
        if item.has_children && area_cells >= 60 && drawn.height > label_rows(drawn) {
            let rows = label_rows(drawn);
            let band = Rect {
                x: drawn.x,
                y: drawn.y,
                width: drawn.width,
                height: rows,
            };
            let factor = if item.depth == 1 { 0.78 } else { 0.90 };
            c.fill(band, shade(color, factor));
        }
        if selected {
            selection_edges(&mut c, drawn, contrast(color));
        }
        labels(&mut c, app, item, drawn, color, selected, total, area_cells);
    }
}

/// A subtle cushion: brighter top/left edge, darker bottom/right edge.
fn bevel(c: &mut Canvas, rect: Rect, color: Rgb) {
    let light = shade(color, 1.14);
    let dark = shade(color, 0.86);
    if rect.width >= 3 {
        c.vline(
            rect.x,
            rect.y,
            rect.bottom(),
            ' ',
            Style::default().bg(rgb(light)),
        );
        c.vline(
            rect.right() - 1,
            rect.y,
            rect.bottom(),
            ' ',
            Style::default().bg(rgb(dark)),
        );
    }
    if rect.height >= 3 {
        c.hline(
            rect.y,
            rect.x,
            rect.right(),
            ' ',
            Style::default().bg(rgb(light)),
        );
        c.hline(
            rect.bottom() - 1,
            rect.x,
            rect.right(),
            ' ',
            Style::default().bg(rgb(dark)),
        );
    }
}

fn selection_edges(c: &mut Canvas, rect: Rect, color: Rgb) {
    let style = Style::default().bg(rgb(color));
    if rect.height >= 2 {
        c.hline(rect.bottom() - 1, rect.x, rect.right(), ' ', style);
    }
    if rect.width >= 2 {
        c.vline(rect.x, rect.y, rect.bottom(), ' ', style);
        c.vline(rect.right() - 1, rect.y, rect.bottom(), ' ', style);
    }
}

#[allow(clippy::too_many_arguments)]
fn labels(
    c: &mut Canvas,
    app: &App,
    item: &super::app::LayoutItem,
    rect: Rect,
    color: Rgb,
    selected: bool,
    total: u64,
    area_cells: u32,
) {
    let node = item.node.expect("real node");
    if rect.width < LABEL_MIN_W || rect.height < 1 {
        return;
    }
    let max_w = rect.width.saturating_sub(2) as usize;
    if max_w == 0 {
        return;
    }
    let name = app.tree.name(node);
    let fg = contrast(color);
    let name_style = if selected {
        Style::default()
            .bg(rgb(fg))
            .fg(rgb(color))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(rgb(fg)).add_modifier(Modifier::BOLD)
    };
    c.text(rect.x + 1, rect.y, &name, name_style, max_w);

    // Detail only where the cell is roomy enough that it is not clutter.
    if rect.width >= DETAIL_MIN_W && rect.height >= 3 && area_cells >= 60 {
        let size = app.tree.node(node).size;
        let pct = size as f64 / total as f64 * 100.0;
        let detail = if rect.width >= 24 {
            format!("{}  {pct:.1}%", format_size(size))
        } else {
            format_size(size)
        };
        let dim_fg = mix(fg, color, 0.30);
        c.text(
            rect.x + 1,
            rect.y + 1,
            &detail,
            Style::default().fg(rgb(dim_fg)),
            max_w,
        );
    }
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let mut c = Canvas {
        buf: frame.buffer_mut(),
        area,
    };
    c.style(area, Style::default().bg(theme.panel_bg));
    let x = area.x + 2;
    let w = area.width.saturating_sub(4) as usize;
    if w < 10 {
        return;
    }
    let panel = Panel {
        x,
        w,
        bottom: area.bottom(),
    };
    let mut y = area.y + 1;

    section(&mut c, &panel, &mut y, "SELECTION", "", theme);
    let sel = app.selection;
    let node = app.tree.node(sel);
    c.text(
        x,
        y,
        &app.tree.name(sel),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        w,
    );
    y += 1;
    let path = app.tree.path_of(sel).to_string_lossy().into_owned();
    for line in wrap_text(&path, w).into_iter().take(3) {
        if y >= area.bottom() {
            break;
        }
        c.text(x, y, &line, Style::default().fg(theme.dim), w);
        y += 1;
    }
    y += 1;

    if y >= area.bottom() {
        return;
    }
    let total = app.tree.node(app.view_root()).size.max(1);
    let pct = node.size as f64 / total as f64 * 100.0;
    let size_text = format_size(node.size);
    c.text(
        x,
        y,
        &size_text,
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        w,
    );
    c.text_right(
        x,
        w,
        y,
        &format!("{pct:.1}%"),
        Style::default().fg(theme.dim),
    );
    y += 1;
    bar(&mut c, &panel, &mut y, pct / 100.0, theme.accent, theme);
    y += 1;

    metric(
        &mut c,
        &panel,
        &mut y,
        "files",
        &format_count(node.files as u64),
        theme,
    );
    if app.tree.is_dir(sel) {
        metric(
            &mut c,
            &panel,
            &mut y,
            "dirs",
            &format_count(node.dirs as u64),
            theme,
        );
    }
    let age = if node.mtime > 0 {
        format_age(node.mtime as u64)
    } else {
        "unknown".to_string()
    };
    metric(&mut c, &panel, &mut y, "last write", &age, theme);
    let category = app
        .tree
        .category_ids
        .get(node.category as usize)
        .map(|s| s.as_str())
        .unwrap_or("other");
    metric(
        &mut c,
        &panel,
        &mut y,
        "kind",
        &format!("{} · {category}", node.kind.as_str()),
        theme,
    );
    if !app.marked.is_empty() {
        let marked_here = app.marked.contains(&sel);
        metric(
            &mut c,
            &panel,
            &mut y,
            "marked",
            &format!(
                "{}{}",
                app.marked.len(),
                if marked_here { " (this)" } else { "" }
            ),
            theme,
        );
        metric(
            &mut c,
            &panel,
            &mut y,
            "marked size",
            &format_size(app.marked_size()),
            theme,
        );
    }
    y += 1;

    if !app.worth.is_empty() && y + 4 < area.bottom() {
        let total_worth: u64 = app.worth.iter().map(|w| w.size).sum();
        section(
            &mut c,
            &panel,
            &mut y,
            "WORTH A LOOK",
            &format_size(total_worth),
            theme,
        );
        let max = app.worth.iter().map(|w| w.size).max().unwrap_or(1).max(1);
        for item in &app.worth {
            if y + 2 >= area.bottom() {
                break;
            }
            let name = app.tree.name(item.node).into_owned();
            c.text(
                x,
                y,
                &name,
                Style::default().fg(theme.fg),
                w.saturating_sub(10),
            );
            c.text_right(
                x,
                w,
                y,
                &format_size(item.size),
                Style::default().fg(theme.dim),
            );
            y += 1;
            let color = app
                .tree
                .category_colors
                .get(app.tree.node(item.node).category as usize)
                .copied()
                .unwrap_or((128, 128, 128));
            bar(
                &mut c,
                &panel,
                &mut y,
                item.size as f64 / max as f64,
                rgb(color),
                theme,
            );
        }
        y += 1;
    }

    if let Some(disk) = &app.disk
        && y + 5 < area.bottom()
    {
        section(&mut c, &panel, &mut y, "DISK", &disk.label, theme);
        let used = disk.total.saturating_sub(disk.free);
        metric(
            &mut c,
            &panel,
            &mut y,
            "free",
            &format_size(disk.free),
            theme,
        );
        metric(&mut c, &panel, &mut y, "used", &format_size(used), theme);
        metric(
            &mut c,
            &panel,
            &mut y,
            "total",
            &format_size(disk.total),
            theme,
        );
        if disk.total > 0 {
            bar(
                &mut c,
                &panel,
                &mut y,
                used as f64 / disk.total as f64,
                theme.accent,
                theme,
            );
        }
    }

    if app.tree.error_count > 0 && y + 1 < area.bottom() {
        y += 1;
        c.text(
            x,
            y,
            &format!("{} entries unreadable", app.tree.error_count),
            Style::default().fg(theme.warn),
            w,
        );
    }
}

/// Fixed geometry for the sidebar helpers.
struct Panel {
    x: u16,
    w: usize,
    bottom: u16,
}

fn section(c: &mut Canvas, p: &Panel, y: &mut u16, title: &str, right: &str, theme: Theme) {
    if *y + 1 >= p.bottom {
        return;
    }
    c.text(
        p.x,
        *y,
        title,
        Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        p.w,
    );
    if !right.is_empty() {
        c.text_right(p.x, p.w, *y, right, Style::default().fg(theme.fg));
    }
    *y += 1;
    for i in 0..p.w as u16 {
        c.put_char(p.x + i, *y, '─', Style::default().fg(theme.border));
    }
    *y += 1;
}

fn metric(c: &mut Canvas, p: &Panel, y: &mut u16, label: &str, value: &str, theme: Theme) {
    if *y >= p.bottom {
        return;
    }
    c.text(p.x, *y, label, Style::default().fg(theme.dim), p.w);
    c.text_right(p.x, p.w, *y, value, Style::default().fg(theme.fg));
    *y += 1;
}

fn bar(
    c: &mut Canvas,
    p: &Panel,
    y: &mut u16,
    frac: f64,
    color: ratatui::style::Color,
    theme: Theme,
) {
    if *y >= p.bottom {
        return;
    }
    let filled = (frac.clamp(0.0, 1.0) * p.w as f64).round() as usize;
    for i in 0..filled.min(p.w) {
        c.put_char(p.x + i as u16, *y, '█', Style::default().fg(color));
    }
    let _ = theme;
    *y += 1;
}

// The sidebar helpers take the resolved theme so rules and values match the
// active palette.

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;
    for ch in text.chars() {
        if ch == '/' || ch == '\\' {
            current.push(ch);
            lines.push(std::mem::take(&mut current));
            current_w = 0;
            continue;
        }
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_w + cw > width {
            lines.push(std::mem::take(&mut current));
            current_w = 0;
        }
        current.push(ch);
        current_w += cw;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

// ---------------------------------------------------------------------------
// Footer, filter, overlays
// ---------------------------------------------------------------------------

fn draw_filter(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let mut c = Canvas {
        buf: frame.buffer_mut(),
        area,
    };
    c.style(area, Style::default().bg(theme.header_bg));
    let mut x = c.text(
        area.x,
        area.y,
        " filter ",
        Style::default().bg(theme.warn).fg(theme.chip_fg),
        8,
    );
    x = c.text(
        x,
        area.y,
        &format!(" /{}", app.filter),
        Style::default().fg(if app.filter_active {
            theme.fg
        } else {
            theme.dim
        }),
        60,
    );
    if !app.filter.is_empty() {
        let matching = app
            .layout
            .iter()
            .filter(|i| i.node.is_some_and(|n| app.matches_filter(n)))
            .count();
        c.text(
            x,
            area.y,
            &format!("   {matching} cells match"),
            Style::default().fg(theme.dim),
            24,
        );
    }
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let right = footer_right(app);
    let right_w = segments_width(&right);
    let mut c = Canvas {
        buf: frame.buffer_mut(),
        area,
    };
    c.style(area, Style::default().bg(theme.header_bg));
    let keys = " space mark · enter open · hjkl move · / filter · [ ] depth · t size · m colors · p palette · r rescan · ? help · q quit";
    let keys_max = (area.width as usize).saturating_sub(right_w + 2);
    c.text(
        area.x,
        area.y,
        keys,
        Style::default().fg(theme.dim),
        keys_max,
    );
    if right_w > 0 {
        let rx = area.right().saturating_sub(right_w as u16);
        draw_segments(&mut c, rx, area.y, &right);
    }
}

fn footer_right(app: &App) -> Vec<(String, Style)> {
    let theme = app.theme;
    if !app.marked.is_empty() {
        return vec![(
            format!(
                "{} marked · {} · d trash ",
                app.marked.len(),
                format_size(app.marked_size())
            ),
            Style::default().fg(theme.warn),
        )];
    }
    if let Some((message, _)) = &app.status {
        return vec![(format!("{message} "), Style::default().fg(theme.good))];
    }
    Vec::new()
}

fn draw_help(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let w = 66.min(area.width.saturating_sub(4));
    let h = 24.min(area.height.saturating_sub(4));
    let rect = centered(area, w, h);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.panel_bg).fg(theme.fg))
        .title(" keys ");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let rows = [
        ("enter / o", "open (zoom into the selected directory)"),
        ("backspace / u", "up one level"),
        ("h j k l / arrows", "move the selection"),
        ("[ ]", "decrease / increase depth"),
        ("t", "cycle size / files / age"),
        ("m", "cycle colors: category / size / age / depth"),
        ("p", "cycle palette (viridis, cividis, slate, ...)"),
        ("H", "toggle hidden entries (rescans)"),
        ("a", "toggle apparent size (rescans)"),
        ("/", "filter by name (esc clears)"),
        ("space", "mark / unmark the selection"),
        ("d", "move marked entries to the trash"),
        ("c", "clear marks"),
        ("r", "rescan"),
        ("0", "reset the view"),
        ("mouse", "click select, double click open, wheel move"),
        ("?", "close this help"),
        ("q / ctrl-c", "quit"),
    ];
    let mut lines: Vec<ratatui::text::Line> = Vec::new();
    lines.push(ratatui::text::Line::from(ratatui::text::Span::styled(
        " disk-wiz ",
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(ratatui::text::Line::from(""));
    for (key, desc) in rows {
        lines.push(ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(
                format!("{key:<18}"),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            ratatui::text::Span::styled(desc.to_string(), Style::default().fg(theme.fg)),
        ]));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(theme.panel_bg).fg(theme.fg))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_confirm(frame: &mut Frame, app: &App, area: Rect, message: &str) {
    let theme = app.theme;
    let w = 54.min(area.width.saturating_sub(4));
    let h = 6.min(area.height.saturating_sub(2));
    let rect = centered(area, w, h);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.warn))
        .style(Style::default().bg(theme.panel_bg).fg(theme.fg))
        .title(" confirm ");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines = vec![
        ratatui::text::Line::from(""),
        ratatui::text::Line::from(ratatui::text::Span::styled(
            format!("  {message}"),
            Style::default().fg(theme.fg),
        )),
        ratatui::text::Line::from(""),
        ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(
                "  y",
                Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
            ),
            ratatui::text::Span::styled(" confirm    ", Style::default().fg(theme.dim)),
            ratatui::text::Span::styled("n / esc", Style::default().fg(theme.accent)),
            ratatui::text::Span::styled(" cancel", Style::default().fg(theme.dim)),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel_bg)),
        inner,
    );
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

// ---------------------------------------------------------------------------
// Color helpers
// ---------------------------------------------------------------------------

fn luminance(c: Rgb) -> f64 {
    0.2126 * c.0 as f64 + 0.7152 * c.1 as f64 + 0.0722 * c.2 as f64
}

fn contrast(bg: Rgb) -> Rgb {
    if luminance(bg) > 140.0 {
        (16, 16, 20)
    } else {
        (244, 244, 248)
    }
}

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let f = |x: u8, y: u8| {
        (x as f64 + (y as f64 - x as f64) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

fn shade(c: Rgb, factor: f64) -> Rgb {
    let f = |x: u8| (x as f64 * factor).round().clamp(0.0, 255.0) as u8;
    (f(c.0), f(c.1), f(c.2))
}

fn emphasize(c: Rgb, light: bool, amount: f64) -> Rgb {
    let f = |x: u8| {
        let v = x as f64;
        let out = if light {
            v * (1.0 - amount)
        } else {
            v + (255.0 - v) * amount
        };
        out.round().clamp(0.0, 255.0) as u8
    };
    (f(c.0), f(c.1), f(c.2))
}

fn desaturate(c: Rgb, light: bool) -> Rgb {
    let gray = luminance(c) as u8;
    let t = 0.75;
    let f = |x: u8| {
        let v = x as f64 * (1.0 - t) + gray as f64 * t;
        let v = if light {
            v + (255.0 - v) * 0.4
        } else {
            v * 0.55
        };
        v.round().clamp(0.0, 255.0) as u8
    };
    (f(c.0), f(c.1), f(c.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_is_readable_on_light_and_dark() {
        let dark = (40, 40, 60);
        let light = (240, 230, 140);
        assert!(luminance(contrast(dark)) > luminance(dark));
        assert!(luminance(contrast(light)) < luminance(light));
    }

    #[test]
    fn shades_separate_cells() {
        let base = (120, 120, 60);
        assert!(luminance(shade(base, 0.70)) < luminance(base));
        assert!(luminance(shade(base, 1.2)) > luminance(base));
    }
}
