//! TUI rendering.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::color::Rgb;
use crate::tree::SizeMode;
use crate::util::{
    display_width, format_age, format_count, format_duration, format_size, truncate_to_width,
};

use super::app::App;
use super::theme::{Theme, rgb};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 24 || area.height < 6 {
        frame.render_widget(
            Paragraph::new("dw: terminal too small")
                .style(Style::default().fg(app.theme.fg).bg(app.theme.bg)),
            area,
        );
        return;
    }
    let theme = app.theme;
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.bg).fg(theme.fg)),
        area,
    );

    let filter_h: u16 = if app.filter_active || !app.filter.is_empty() {
        1
    } else {
        0
    };
    let header_h: u16 = 1;
    let footer_h: u16 = 1;
    let body_y = area.y + header_h;
    let body_h = area.height.saturating_sub(header_h + footer_h + filter_h);

    let sidebar_visible = app.sidebar_visible(area.width);
    let sidebar_w: u16 = if sidebar_visible {
        (area.width / 4).clamp(28, 42)
    } else {
        0
    };
    let treemap_w = area.width.saturating_sub(sidebar_w);
    let treemap_area = Rect::new(area.x, body_y, treemap_w, body_h);
    let sidebar_area = Rect::new(area.x + treemap_w, body_y, sidebar_w, body_h);

    draw_header(frame, app, Rect::new(area.x, area.y, area.width, header_h));
    if body_h > 0 && treemap_w > 0 {
        app.ensure_layout(treemap_area);
        draw_treemap(frame, app, treemap_area);
        if sidebar_visible {
            draw_sidebar(frame, app, sidebar_area);
        }
    }
    let footer_y = body_y + body_h;
    if filter_h > 0 {
        draw_filter(frame, app, Rect::new(area.x, footer_y, area.width, 1));
    }
    draw_footer(
        frame,
        app,
        Rect::new(area.x, footer_y + filter_h, area.width, footer_h),
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
// Header
// ---------------------------------------------------------------------------

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let mut left: Vec<Span> = Vec::new();
    left.push(Span::styled(
        " dw ",
        Style::default()
            .bg(theme.accent)
            .fg(theme.bg)
            .add_modifier(Modifier::BOLD),
    ));
    left.push(Span::raw(" "));
    let crumbs = app.breadcrumb();
    for (i, &id) in crumbs.iter().enumerate() {
        if i > 0 {
            left.push(Span::styled(" › ", Style::default().fg(theme.dim)));
        }
        let name = app.tree.name(id).into_owned();
        let style = if i + 1 == crumbs.len() {
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        };
        left.push(Span::styled(name, style));
    }

    let mut right: Vec<Span> = Vec::new();
    for mode in [SizeMode::Size, SizeMode::Files, SizeMode::Age] {
        let active = app.size_mode == mode;
        let style = if active {
            Style::default()
                .bg(theme.accent)
                .fg(theme.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim)
        };
        right.push(Span::styled(format!(" {} ", mode.label()), style));
        right.push(Span::raw(" "));
    }
    let hidden_style = if app.cfg.spec.hidden {
        Style::default().fg(theme.good)
    } else {
        Style::default().fg(theme.dim)
    };
    right.push(Span::styled(
        if app.cfg.spec.hidden {
            "hidden:on"
        } else {
            "hidden:off"
        },
        hidden_style,
    ));
    right.push(Span::raw("  "));
    let apparent_style = if app.cfg.spec.apparent_size {
        Style::default().fg(theme.good)
    } else {
        Style::default().fg(theme.dim)
    };
    right.push(Span::styled(
        if app.cfg.spec.apparent_size {
            "apparent:on"
        } else {
            "apparent:off"
        },
        apparent_style,
    ));
    right.push(Span::raw("  "));
    right.push(Span::styled(
        format!("depth {}", app.depth),
        Style::default().fg(theme.fg),
    ));

    let right_w: usize = right.iter().map(|s| display_width(&s.content)).sum();
    let avail = (area.width as usize).saturating_sub(right_w + 2);
    truncate_spans(&mut left, avail);

    let mut spans = left;
    let left_w: usize = spans.iter().map(|s| display_width(&s.content)).sum();
    let pad = (area.width as usize).saturating_sub(left_w + right_w);
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
    }
    spans.extend(right);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.header_bg).fg(theme.fg)),
        area,
    );
}

fn truncate_spans(spans: &mut Vec<Span<'static>>, width: usize) {
    let mut used = 0usize;
    let mut keep = 0usize;
    for span in spans.iter() {
        let w = display_width(&span.content);
        if used + w > width {
            break;
        }
        used += w;
        keep += 1;
    }
    spans.truncate(keep);
}

// ---------------------------------------------------------------------------
// Treemap
// ---------------------------------------------------------------------------

fn draw_treemap(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(theme.bg));
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
            put_text(
                buf,
                area,
                x,
                y,
                &text,
                area.width as usize,
                Style::default().fg(theme.warn),
            );
        }
        return;
    }
    let total = app.tree.node(app.root()).size.max(1);
    for item in &app.layout {
        let Some(node) = item.node else {
            // Overflow aggregate.
            fill_cell(buf, area, item.rect, item.color);
            if item.rect.width >= 6 && item.rect.height >= 1 {
                let text = format!("{} more", item.overflow);
                put_text(
                    buf,
                    area,
                    item.rect.x + 1,
                    item.rect.y,
                    &text,
                    item.rect.width.saturating_sub(2) as usize,
                    Style::default().fg(rgb((200, 200, 210))),
                );
            }
            continue;
        };
        let selected = node == app.selection;
        let marked = app.marked.contains(&node);
        let dimmed = !app.matches_filter(node);
        let mut color = item.color;
        if dimmed {
            color = desaturate(color, theme.is_light());
        }
        if selected {
            color = emphasize(color, theme.is_light(), 0.35);
        }
        fill_cell(buf, area, item.rect, color);
        let name = app.tree.name(node);
        let size = format_size(app.tree.node(node).size);
        let pct = app.tree.node(node).size as f64 / total as f64 * 100.0;
        draw_cell_labels(
            buf,
            area,
            item.rect,
            name.as_ref(),
            &size,
            pct,
            selected,
            marked,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_cell_labels(
    buf: &mut Buffer,
    area: Rect,
    rect: Rect,
    name: &str,
    size: &str,
    pct: f64,
    selected: bool,
    marked: bool,
) {
    if rect.width < 4 || rect.height < 1 {
        return;
    }
    let pad: u16 = 1;
    let inner_x = rect.x + pad;
    let max_w = rect.width.saturating_sub(pad * 2) as usize;
    if max_w == 0 {
        return;
    }
    let mut label = String::new();
    if marked {
        label.push('●');
        label.push(' ');
    } else if selected {
        label.push('▸');
        label.push(' ');
    }
    label.push_str(name);
    let style = if selected {
        Style::default()
            .fg(rgb((20, 20, 24)))
            .bg(rgb((255, 255, 255)))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(rgb((245, 245, 250)))
    };
    put_text(buf, area, inner_x, rect.y, &label, max_w, style);
    if rect.height >= 3 && rect.width >= 12 {
        let detail = if rect.width >= 22 {
            format!("{size}  {pct:.1}%")
        } else {
            size.to_string()
        };
        let detail_style = if selected {
            Style::default().fg(rgb((230, 230, 235)))
        } else {
            Style::default().fg(rgb((210, 212, 220)))
        };
        put_text(buf, area, inner_x, rect.y + 1, &detail, max_w, detail_style);
    }
}

fn put_text(buf: &mut Buffer, area: Rect, x: u16, y: u16, text: &str, max_w: usize, style: Style) {
    if y >= area.bottom() || max_w == 0 {
        return;
    }
    let available = (area.right().saturating_sub(x)) as usize;
    let max_w = max_w.min(available);
    if max_w == 0 {
        return;
    }
    let text = truncate_to_width(text, max_w);
    buf.set_stringn(x, y, text.as_ref(), max_w, style);
}

fn fill_cell(buf: &mut Buffer, area: Rect, rect: Rect, color: Rgb) {
    let rect = rect.intersection(area);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let fg = contrast(color);
    buf.set_style(rect, Style::default().bg(rgb(color)).fg(rgb(fg)));
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

struct Panel<'a> {
    buf: &'a mut Buffer,
    x: u16,
    y: u16,
    w: usize,
    bottom: u16,
    theme: Theme,
}

impl Panel<'_> {
    fn put(&mut self, text: &str, style: Style) {
        if self.y >= self.bottom {
            return;
        }
        let t = truncate_to_width(text, self.w);
        self.buf
            .set_stringn(self.x, self.y, t.as_ref(), self.w, style);
        self.y += 1;
    }

    fn row(&mut self, left: &str, right: &str, style: Style) {
        if self.y >= self.bottom {
            return;
        }
        let right_w = display_width(right);
        let left_w = self.w.saturating_sub(right_w + 1);
        let left_t = truncate_to_width(left, left_w);
        self.buf
            .set_stringn(self.x, self.y, left_t.as_ref(), left_w, style);
        if right_w <= self.w {
            let rx = self.x + (self.w - right_w) as u16;
            let right_t = truncate_to_width(right, self.w);
            self.buf
                .set_stringn(rx, self.y, right_t.as_ref(), right_w, style);
        }
        self.y += 1;
    }

    fn blank(&mut self) {
        self.y += 1;
    }

    fn section(&mut self, title: &str, right: &str) {
        self.row(
            title,
            right,
            Style::default()
                .fg(self.theme.dim)
                .add_modifier(Modifier::BOLD),
        );
    }
}

fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(theme.panel_bg));
    let mut p = Panel {
        buf,
        x: area.x + 2,
        y: area.y + 1,
        w: area.width.saturating_sub(4) as usize,
        bottom: area.bottom(),
        theme,
    };
    if p.w < 8 {
        return;
    }

    p.section("SELECTION", "");
    p.blank();
    let sel = app.selection;
    let name = app.tree.name(sel).into_owned();
    p.put(
        &name,
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    );
    let path = app.tree.path_of(sel).to_string_lossy().into_owned();
    for line in wrap_text(&path, p.w).into_iter().take(3) {
        p.put(&line, Style::default().fg(theme.dim));
    }
    p.blank();
    let node = app.tree.node(sel);
    let big = format_size(node.size);
    p.put(
        &big,
        Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    p.blank();
    let total = app.tree.node(app.root()).size.max(1);
    let pct = node.size as f64 / total as f64 * 100.0;
    p.row(
        "of view",
        &format!("{pct:.1}%"),
        Style::default().fg(theme.fg),
    );
    p.row(
        "files",
        &format_count(node.files as u64),
        Style::default().fg(theme.fg),
    );
    if app.tree.is_dir(sel) {
        p.row(
            "dirs",
            &format_count(node.dirs as u64),
            Style::default().fg(theme.fg),
        );
    }
    let age = if node.mtime > 0 {
        format_age(node.mtime as u64)
    } else {
        "unknown".to_string()
    };
    p.row("last write", &age, Style::default().fg(theme.fg));
    let category = app
        .tree
        .category_ids
        .get(node.category as usize)
        .map(|s| s.as_str())
        .unwrap_or("other");
    p.row(
        "kind",
        &format!("{} · {category}", node.kind.as_str()),
        Style::default().fg(theme.fg),
    );
    if !app.marked.is_empty() {
        let marked_sel = app.marked.contains(&sel);
        p.row(
            "marked",
            &format!(
                "{}{}",
                app.marked.len(),
                if marked_sel { " (this)" } else { "" }
            ),
            Style::default().fg(theme.warn),
        );
        p.row(
            "marked size",
            &format_size(app.marked_size()),
            Style::default().fg(theme.warn),
        );
    }
    p.blank();

    if !app.worth.is_empty() && p.y + 3 < p.bottom {
        let total_worth: u64 = app.worth.iter().map(|w| w.size).sum();
        p.section("WORTH A LOOK", &format_size(total_worth));
        p.blank();
        let max = app.worth.iter().map(|w| w.size).max().unwrap_or(1).max(1);
        for item in &app.worth {
            let label = app.tree.name(item.node).into_owned();
            let size = format_size(item.size);
            p.row(&label, &size, Style::default().fg(theme.fg));
            if p.y >= p.bottom {
                break;
            }
            let bar_w = p.w.saturating_sub(2);
            let filled = ((item.size as f64 / max as f64) * bar_w as f64).round() as usize;
            let bar = format!(
                "{}{}",
                "█".repeat(filled.min(bar_w)),
                " ".repeat(bar_w.saturating_sub(filled))
            );
            let color = if app.theme.is_light() {
                theme.accent
            } else {
                theme.warn
            };
            p.put(&bar, Style::default().fg(color));
        }
        p.blank();
    }

    if let Some(disk) = &app.disk
        && p.y + 5 < p.bottom
    {
        p.section("DISK", &disk.label);
        p.blank();
        let used = disk.total.saturating_sub(disk.free);
        p.row(
            "free",
            &format_size(disk.free),
            Style::default().fg(theme.good),
        );
        p.row("used", &format_size(used), Style::default().fg(theme.fg));
        p.row(
            "total",
            &format_size(disk.total),
            Style::default().fg(theme.dim),
        );
        if disk.total > 0 {
            let bar_w = p.w.saturating_sub(2);
            let filled = ((used as f64 / disk.total as f64) * bar_w as f64).round() as usize;
            let bar = format!(
                "{}{}",
                "█".repeat(filled.min(bar_w)),
                " ".repeat(bar_w.saturating_sub(filled))
            );
            p.put(&bar, Style::default().fg(theme.accent));
        }
    }

    if app.tree.error_count > 0 && p.y + 2 < p.bottom {
        p.blank();
        p.put(
            &format!("{} entries unreadable", app.tree.error_count),
            Style::default().fg(theme.warn),
        );
    }
}

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
    let text = format!("/{}", app.filter);
    let style = if app.filter_active {
        Style::default().fg(theme.fg).bg(theme.header_bg)
    } else {
        Style::default().fg(theme.dim).bg(theme.header_bg)
    };
    let mut spans = vec![
        Span::styled(" filter ", Style::default().fg(theme.bg).bg(theme.warn)),
        Span::styled(format!(" {text}"), style),
    ];
    if !app.filter.is_empty() {
        let matching = app
            .layout
            .iter()
            .filter(|i| i.node.is_some_and(|n| app.matches_filter(n)))
            .count();
        spans.push(Span::styled(
            format!("  ({matching} visible)"),
            Style::default().fg(theme.dim),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.header_bg)),
        area,
    );
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let keys =
        "space mark  enter open  hjkl move  / filter  [ ] depth  t mode  r rescan  ? help  q quit";
    let mut spans = vec![Span::styled(
        format!(" {keys}"),
        Style::default().fg(theme.dim),
    )];
    if let Some((entries, bytes, secs)) = app.scan_progress() {
        spans.push(Span::styled(
            format!(
                "scanning: {} entries · {} · {secs:.1}s ",
                format_count(entries),
                format_size(bytes)
            ),
            Style::default().fg(theme.warn),
        ));
    } else if let Some((message, _)) = &app.status {
        spans.push(Span::styled(
            format!("{message} "),
            Style::default().fg(theme.good),
        ));
    } else if let Some((entries, duration)) = app.last_scan {
        spans.push(Span::styled(
            format!(
                "{} entries · {} ",
                format_count(entries),
                format_duration(duration)
            ),
            Style::default().fg(theme.fg),
        ));
    }
    let total_w: usize = spans.iter().map(|s| display_width(&s.content)).sum();
    let avail = area.width as usize;
    if total_w > avail {
        // Truncate the key hints first.
        let mut out = Vec::new();
        let mut used = 0usize;
        for span in &spans {
            let w = display_width(&span.content);
            if used + w > avail {
                break;
            }
            used += w;
            out.push(span.clone());
        }
        spans = out;
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.header_bg)),
        area,
    );
}

fn draw_help(frame: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let w = 64.min(area.width.saturating_sub(4));
    let h = 22.min(area.height.saturating_sub(4));
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
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        " disk-wiz ",
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for (key, desc) in rows {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{key:<18}"),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(desc.to_string(), Style::default().fg(theme.fg)),
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
    let w = 52.min(area.width.saturating_sub(4));
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
        Line::from(""),
        Line::from(Span::styled(
            format!("  {message}"),
            Style::default().fg(theme.fg),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  y",
                Style::default().fg(theme.warn).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" confirm    ", Style::default().fg(theme.dim)),
            Span::styled("n / esc", Style::default().fg(theme.accent)),
            Span::styled(" cancel", Style::default().fg(theme.dim)),
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

fn contrast(bg: Rgb) -> Rgb {
    let l = 0.2126 * bg.0 as f64 + 0.7152 * bg.1 as f64 + 0.0722 * bg.2 as f64;
    if l > 140.0 {
        (15, 15, 20)
    } else {
        (245, 245, 250)
    }
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
    let gray = (0.2126 * c.0 as f64 + 0.7152 * c.1 as f64 + 0.0722 * c.2 as f64) as u8;
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
