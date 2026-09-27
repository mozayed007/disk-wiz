//! Non-interactive output renderers: flat, tree, json, and ANSI treemap.

use crate::color::{self, Palette, Rgb};
use crate::config::{ColorModeName, ScaleName};
use crate::layout;
use crate::tree::{NodeId, SortKey, Tree};
use crate::util::{format_size, truncate_to_width};
use ratatui::layout::Rect;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OutputFormat {
    #[default]
    Flat,
    Tree,
    Json,
    Ansi,
}

#[derive(Clone, Debug)]
pub struct OutputOptions {
    pub format: OutputFormat,
    pub depth: Option<usize>,
    pub top: Option<usize>,
    pub sort: SortKey,
    pub reverse: bool,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Category ids to include (empty means all).
    pub type_filter: Vec<String>,
    pub raw_bytes: bool,
    pub color: bool,
    pub palette: Palette,
    pub color_mode: ColorModeName,
    pub scale: ScaleName,
    pub saturation: f64,
    pub width: u16,
    pub height: u16,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            format: OutputFormat::Flat,
            depth: None,
            top: None,
            sort: SortKey::Size,
            reverse: false,
            min_size: None,
            max_size: None,
            type_filter: Vec::new(),
            raw_bytes: false,
            color: false,
            palette: Palette::Viridis,
            color_mode: ColorModeName::Category,
            scale: ScaleName::Log,
            saturation: 0.7,
            width: 100,
            height: 30,
        }
    }
}

/// Render a tree in the chosen format.
pub fn write(tree: &Tree, out: &mut dyn Write, opts: &OutputOptions) -> io::Result<()> {
    match opts.format {
        OutputFormat::Flat => write_flat(tree, out, opts),
        OutputFormat::Tree => write_tree(tree, out, opts),
        OutputFormat::Json => write_json(tree, out, opts),
        OutputFormat::Ansi => write_ansi(tree, out, opts),
    }
}

fn size_str(size: u64, opts: &OutputOptions) -> String {
    if opts.raw_bytes {
        size.to_string()
    } else {
        format_size(size)
    }
}

fn passes_filter(tree: &Tree, id: NodeId, opts: &OutputOptions) -> bool {
    let size = tree.node(id).size;
    if let Some(min) = opts.min_size
        && size < min
    {
        return false;
    }
    if let Some(max) = opts.max_size
        && size > max
    {
        return false;
    }
    if !opts.type_filter.is_empty() {
        let cat = tree
            .category_ids
            .get(tree.node(id).category as usize)
            .map(|s| s.as_str())
            .unwrap_or("other");
        if !opts.type_filter.iter().any(|t| t.eq_ignore_ascii_case(cat)) {
            return false;
        }
    }
    true
}

struct FlatRow {
    node: NodeId,
    size: u64,
    value: u64,
    path: String,
}

fn write_flat(tree: &Tree, out: &mut dyn Write, opts: &OutputOptions) -> io::Result<()> {
    let mut rows = Vec::new();
    let mut path = PathBuf::new();
    collect_flat(tree, tree.root(), &mut path, 0, opts, &mut rows);
    rows.sort_by(|a, b| {
        let ord = match opts.sort {
            SortKey::Name => a.path.cmp(&b.path),
            _ => b.value.cmp(&a.value),
        };
        ord.then_with(|| a.path.cmp(&b.path))
    });
    if opts.reverse {
        rows.reverse();
    }
    if let Some(n) = opts.top {
        rows.truncate(n);
    }
    let values: Vec<f64> = rows.iter().map(|r| r.value as f64).collect();
    let scale = color::Scale::new(&values, opts.scale);
    let plan = color_plan(opts);
    for (rank, row) in rows.iter().enumerate() {
        let size = size_str(row.size, opts);
        let size = if opts.color {
            let rgb = plan.node_color(tree, row.node, opts.sort, rank, rows.len(), &scale);
            paint(&size, rgb)
        } else {
            size
        };
        writeln!(out, "{size}\t{}", row.path)?;
    }
    Ok(())
}

/// Wrap text in a 24-bit foreground color.
fn paint(text: &str, rgb: Rgb) -> String {
    format!("\x1b[38;2;{};{};{}m{text}\x1b[0m", rgb.0, rgb.1, rgb.2)
}

fn color_plan(opts: &OutputOptions) -> color::ColorPlan {
    color::ColorPlan {
        mode: opts.color_mode,
        palette: opts.palette,
        scale: opts.scale,
        reverse: opts.reverse,
        light: false,
        saturation: opts.saturation,
    }
}

fn collect_flat(
    tree: &Tree,
    id: NodeId,
    path: &mut PathBuf,
    depth: usize,
    opts: &OutputOptions,
    rows: &mut Vec<FlatRow>,
) {
    path.push(tree.name_os(id));
    let node = tree.node(id);
    if passes_filter(tree, id, opts) {
        rows.push(FlatRow {
            node: id,
            size: node.size,
            value: tree.value_for(id, opts.sort),
            path: path.to_string_lossy().into_owned(),
        });
    }
    let limit = opts.depth.unwrap_or(usize::MAX);
    if depth < limit {
        for &child in tree.children_of(id) {
            // A child is never larger than its parent, so below-min subtrees
            // can be pruned entirely.
            if let Some(min) = opts.min_size
                && tree.node(child).size < min
            {
                continue;
            }
            collect_flat(tree, child, path, depth + 1, opts, rows);
        }
    }
    path.pop();
}

fn write_tree(tree: &Tree, out: &mut dyn Write, opts: &OutputOptions) -> io::Result<()> {
    let mut values = Vec::new();
    collect_tree_values(tree, tree.root(), 0, opts, &mut values);
    let scale = color::Scale::new(&values, opts.scale);
    let count = values.len();
    let mut writer = TreeWriter {
        tree,
        opts,
        out,
        scale,
        plan: color_plan(opts),
        rank: 0,
        count,
    };
    let root = tree.root();
    let size = writer.size_text(root, 0);
    writeln!(writer.out, "{size}  {}", tree.name(root))?;
    let mut prefix = String::new();
    writer.write_children(root, &mut prefix, 0)
}

fn collect_tree_values(
    tree: &Tree,
    id: NodeId,
    depth: usize,
    opts: &OutputOptions,
    values: &mut Vec<f64>,
) {
    let limit = opts.depth.unwrap_or(usize::MAX);
    if depth >= limit {
        return;
    }
    let mut kids: Vec<NodeId> = tree
        .sorted_children(id, opts.sort, opts.reverse)
        .into_iter()
        .filter(|&c| passes_filter(tree, c, opts))
        .collect();
    if let Some(n) = opts.top {
        kids.truncate(n);
    }
    for &child in &kids {
        values.push(tree.value_for(child, opts.sort) as f64);
        collect_tree_values(tree, child, depth + 1, opts, values);
    }
}

struct TreeWriter<'a> {
    tree: &'a Tree,
    opts: &'a OutputOptions,
    out: &'a mut dyn Write,
    scale: color::Scale,
    plan: color::ColorPlan,
    rank: usize,
    count: usize,
}

impl TreeWriter<'_> {
    fn size_text(&mut self, id: NodeId, width: usize) -> String {
        let plain = size_str(self.tree.node(id).size, self.opts);
        let padded = format!("{plain:>width$}");
        if !self.opts.color {
            return padded;
        }
        let rgb = self.plan.node_color(
            self.tree,
            id,
            self.opts.sort,
            self.rank,
            self.count.max(1),
            &self.scale,
        );
        self.rank += 1;
        paint(&padded, rgb)
    }

    fn write_children(&mut self, id: NodeId, prefix: &mut String, depth: usize) -> io::Result<()> {
        let limit = self.opts.depth.unwrap_or(usize::MAX);
        if depth >= limit {
            return Ok(());
        }
        let mut kids: Vec<NodeId> = self
            .tree
            .sorted_children(id, self.opts.sort, self.opts.reverse)
            .into_iter()
            .filter(|&c| passes_filter(self.tree, c, self.opts))
            .collect();
        if let Some(n) = self.opts.top {
            kids.truncate(n);
        }
        let count = kids.len();
        for (i, &child) in kids.iter().enumerate() {
            let last = i + 1 == count;
            let branch = if last { "└── " } else { "├── " };
            let size = self.size_text(child, 10);
            writeln!(
                self.out,
                "{prefix}{branch}{size}  {}",
                self.tree.name(child)
            )?;
            let saved = prefix.len();
            prefix.push_str(if last { "    " } else { "│   " });
            self.write_children(child, prefix, depth + 1)?;
            prefix.truncate(saved);
        }
        Ok(())
    }
}

fn write_json(tree: &Tree, out: &mut dyn Write, opts: &OutputOptions) -> io::Result<()> {
    write_json_node(tree, tree.root(), out, 0, opts, true)?;
    writeln!(out)
}

fn write_json_node(
    tree: &Tree,
    id: NodeId,
    out: &mut dyn Write,
    depth: usize,
    opts: &OutputOptions,
    is_root: bool,
) -> io::Result<()> {
    let node = tree.node(id);
    write!(out, "{{\"name\":")?;
    write_json_str(out, &tree.name(id))?;
    if !is_root {
        write!(out, ",\"path\":")?;
        write_json_str(out, &tree.path_of(id).to_string_lossy())?;
    }
    write!(
        out,
        ",\"size\":{},\"files\":{},\"dirs\":{},\"mtime\":{},\"kind\":\"{}\",\"category\":",
        node.size,
        node.files,
        node.dirs,
        node.mtime,
        node.kind.as_str()
    )?;
    let cat = tree
        .category_ids
        .get(node.category as usize)
        .map(|s| s.as_str())
        .unwrap_or("other");
    write_json_str(out, cat)?;
    let limit = opts.depth.unwrap_or(usize::MAX);
    if node.child_count > 0 && depth < limit {
        write!(out, ",\"children\":[")?;
        for (i, child) in tree
            .sorted_children(id, opts.sort, opts.reverse)
            .into_iter()
            .enumerate()
        {
            if i > 0 {
                write!(out, ",")?;
            }
            write_json_node(tree, child, out, depth + 1, opts, false)?;
        }
        write!(out, "]")?;
    }
    write!(out, "}}")
}

fn write_json_str(out: &mut dyn Write, s: &str) -> io::Result<()> {
    serde_json::to_writer(&mut *out, s).map_err(io::Error::from)
}

// ---------------------------------------------------------------------------
// ANSI treemap
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    ch: char,
    fg: Rgb,
    bg: Rgb,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            fg: (255, 255, 255),
            bg: (0, 0, 0),
        }
    }
}

fn write_ansi(tree: &Tree, out: &mut dyn Write, opts: &OutputOptions) -> io::Result<()> {
    let root = tree.root();
    let ids = tree.sorted_children(root, opts.sort, opts.reverse);
    if ids.is_empty() {
        writeln!(out, "{}", tree.name(root))?;
        return Ok(());
    }
    let area = Rect::new(0, 0, opts.width.max(20), opts.height.max(5));
    let weights: Vec<u64> = ids
        .iter()
        .map(|&id| tree.value_for(id, opts.sort).max(1))
        .collect();
    let (placed, _dropped) = layout::squarify(&weights, area);
    let mut buf = vec![Cell::default(); area.width as usize * area.height as usize];
    let values: Vec<f64> = placed
        .iter()
        .map(|p| tree.value_for(ids[p.index], opts.sort) as f64)
        .collect();
    let scale = color::Scale::new(&values, opts.scale);
    let plan = color::ColorPlan {
        mode: opts.color_mode,
        palette: opts.palette,
        scale: opts.scale,
        reverse: opts.reverse,
        light: false,
        saturation: opts.saturation,
    };
    for (rank, p) in placed.iter().enumerate() {
        let id = ids[p.index];
        let bg = plan.node_color(tree, id, opts.sort, rank, placed.len(), &scale);
        fill_rect(&mut buf, area, p.rect, bg);
        let fg = contrast_fg(bg);
        let name = tree.name(id);
        draw_text(
            &mut buf,
            area,
            p.rect.x + 1,
            p.rect.y,
            p.rect.width.saturating_sub(2),
            &name,
            fg,
        );
        if p.rect.height >= 3 && p.rect.width >= 10 {
            let size = size_str(tree.node(id).size, opts);
            let dim = blend(fg, bg, 0.35);
            draw_text(
                &mut buf,
                area,
                p.rect.x + 1,
                p.rect.y + 1,
                p.rect.width.saturating_sub(2),
                &size,
                dim,
            );
        }
    }
    emit(&buf, area, out, opts.color)
}

fn fill_rect(buf: &mut [Cell], area: Rect, rect: Rect, bg: Rgb) {
    for y in rect.y..rect.bottom().min(area.bottom()) {
        for x in rect.x..rect.right().min(area.right()) {
            let idx = y as usize * area.width as usize + x as usize;
            buf[idx].bg = bg;
            buf[idx].fg = contrast_fg(bg);
        }
    }
}

fn draw_text(buf: &mut [Cell], area: Rect, x: u16, y: u16, max_w: u16, text: &str, fg: Rgb) {
    if max_w == 0 || y >= area.bottom() {
        return;
    }
    let text = truncate_to_width(text, max_w as usize);
    let mut cx = x;
    let mut skip_next = false;
    for ch in text.chars() {
        if cx >= area.right() {
            break;
        }
        let idx = y as usize * area.width as usize + cx as usize;
        if skip_next {
            buf[idx].ch = '\0';
            skip_next = false;
            cx += 1;
            continue;
        }
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if w > 1 {
            skip_next = true;
        }
        buf[idx].ch = ch;
        buf[idx].fg = fg;
        cx += 1;
    }
}

fn emit(buf: &[Cell], area: Rect, out: &mut dyn Write, color: bool) -> io::Result<()> {
    for y in 0..area.height {
        let row = &buf[y as usize * area.width as usize..(y as usize + 1) * area.width as usize];
        if color {
            let mut last: Option<(Rgb, Rgb)> = None;
            for cell in row {
                if cell.ch == '\0' {
                    continue;
                }
                if last != Some((cell.fg, cell.bg)) {
                    write!(
                        out,
                        "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m",
                        cell.fg.0, cell.fg.1, cell.fg.2, cell.bg.0, cell.bg.1, cell.bg.2
                    )?;
                    last = Some((cell.fg, cell.bg));
                }
                write!(out, "{}", cell.ch)?;
            }
            writeln!(out, "\x1b[0m")?;
        } else {
            let mut line = String::with_capacity(row.len());
            for cell in row {
                if cell.ch != '\0' {
                    line.push(cell.ch);
                }
            }
            writeln!(out, "{}", line.trim_end())?;
        }
    }
    Ok(())
}

fn contrast_fg(bg: Rgb) -> Rgb {
    let l = 0.2126 * bg.0 as f64 + 0.7152 * bg.1 as f64 + 0.0722 * bg.2 as f64;
    if l > 140.0 {
        (0, 0, 0)
    } else {
        (255, 255, 255)
    }
}

fn blend(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let f = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::CategoryMap;
    use crate::tree::{NodeKind, ScanNode, Tree};
    use std::ffi::OsString;
    use std::time::Duration;

    fn leaf(name: &str, size: u64) -> ScanNode {
        ScanNode::entry(OsString::from(name), NodeKind::File, size, 1_700_000_000)
    }

    fn dir(name: &str, children: Vec<ScanNode>) -> ScanNode {
        let mut d = ScanNode::new(OsString::from(name), NodeKind::Dir);
        for c in children {
            d.absorb(c);
        }
        d.finish();
        d
    }

    fn sample() -> Tree {
        let root = dir(
            "root",
            vec![
                dir("a", vec![leaf("x.bin", 300), leaf("y.bin", 100)]),
                dir("b", vec![leaf("z.rs", 200)]),
            ],
        );
        Tree::from_scan(root, &CategoryMap::builtin(), Vec::new(), 0, Duration::ZERO)
    }

    fn render(opts: &OutputOptions) -> String {
        let tree = sample();
        let mut buf = Vec::new();
        write(&tree, &mut buf, opts).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn flat_lists_sorted_by_size() {
        let opts = OutputOptions {
            format: OutputFormat::Flat,
            ..Default::default()
        };
        let out = render(&opts).replace('\\', "/");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 6);
        assert!(lines[0].ends_with("root"));
        assert!(lines[1].ends_with("root/a"));
        assert!(lines[2].ends_with("root/a/x.bin"));
        assert!(lines[3].ends_with("root/b"));
        assert!(lines[4].ends_with("root/b/z.rs"));
        assert!(lines[5].ends_with("root/a/y.bin"));
    }

    #[test]
    fn flat_top_and_depth() {
        let opts = OutputOptions {
            format: OutputFormat::Flat,
            top: Some(2),
            ..Default::default()
        };
        let out = render(&opts);
        assert_eq!(out.lines().count(), 2);

        let opts = OutputOptions {
            format: OutputFormat::Flat,
            depth: Some(1),
            ..Default::default()
        };
        let out = render(&opts);
        assert_eq!(out.lines().count(), 3); // root, a, b
    }

    #[test]
    fn flat_type_filter() {
        let opts = OutputOptions {
            format: OutputFormat::Flat,
            type_filter: vec!["code".into()],
            ..Default::default()
        };
        let out = render(&opts);
        assert_eq!(out.lines().count(), 1);
        assert!(out.contains("z.rs"));
    }

    #[test]
    fn tree_draws_branches() {
        let opts = OutputOptions {
            format: OutputFormat::Tree,
            ..Default::default()
        };
        let out = render(&opts);
        assert!(out.contains("├──"));
        assert!(out.contains("└──"));
        assert!(out.contains("z.rs"));
    }

    #[test]
    fn json_is_valid_and_nested() {
        let opts = OutputOptions {
            format: OutputFormat::Json,
            ..Default::default()
        };
        let out = render(&opts);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["name"], "root");
        assert_eq!(v["size"], 600);
        assert_eq!(v["children"][0]["name"], "a");
        assert_eq!(v["children"][0]["children"][0]["name"], "x.bin");
        assert_eq!(v["children"][1]["children"][0]["category"], "code");
    }

    #[test]
    fn ansi_renders_labels() {
        let opts = OutputOptions {
            format: OutputFormat::Ansi,
            width: 60,
            height: 12,
            color: false,
            ..Default::default()
        };
        let out = render(&opts);
        assert!(out.contains("a"));
        assert!(out.contains("b"));
        assert_eq!(out.lines().count(), 12);
    }

    #[test]
    fn ansi_with_color_has_escape_codes() {
        let opts = OutputOptions {
            format: OutputFormat::Ansi,
            width: 40,
            height: 8,
            color: true,
            ..Default::default()
        };
        let out = render(&opts);
        assert!(out.contains("\x1b[48;2;"));
    }

    #[test]
    fn flat_and_tree_colorize_when_enabled() {
        for format in [OutputFormat::Flat, OutputFormat::Tree] {
            let opts = OutputOptions {
                format,
                color: true,
                ..Default::default()
            };
            let out = render(&opts);
            assert!(out.contains("\x1b[38;2;"), "{format:?} should be colored");
        }
    }

    #[test]
    fn every_palette_and_mode_renders() {
        use crate::color::Palette;
        use crate::config::ColorModeName;
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
                for scale in [
                    crate::config::ScaleName::Linear,
                    crate::config::ScaleName::Log,
                    crate::config::ScaleName::Rank,
                ] {
                    let opts = OutputOptions {
                        format: OutputFormat::Ansi,
                        width: 60,
                        height: 16,
                        color: true,
                        palette: *palette,
                        color_mode: mode,
                        scale,
                        reverse: true,
                        ..Default::default()
                    };
                    let out = render(&opts);
                    assert_eq!(out.lines().count(), 16, "{palette:?} {mode:?} {scale:?}");
                    assert!(out.contains("\x1b[48;2;"), "{palette:?} {mode:?}");
                }
            }
        }
    }
}
