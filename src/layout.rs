//! Squarified treemap layout on a character-cell grid.
//!
//! The layout runs the classic squarify algorithm (Bruls, Huizing, van Wijk)
//! over floating-point areas, then snaps rows to integer cells so rectangles
//! always align with the terminal grid and never overlap.

use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    /// Index into the input weight list.
    pub index: usize,
    pub rect: Rect,
}

/// Sentinel index for the aggregate of entries too small to place.
pub const OVERFLOW: usize = usize::MAX;

/// A treemap entry: a real input index, or the overflow aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreemapEntry {
    /// Index into the input weights, or [`OVERFLOW`].
    pub index: usize,
    pub rect: Rect,
}

/// Lay out `weights` (descending) with an overflow aggregate for sub-cell
/// entries.
///
/// Returns the entries and how many input entries were aggregated into the
/// overflow cell (`0` when everything was placed).
pub fn treemap(weights: &[u64], area: Rect) -> (Vec<TreemapEntry>, usize) {
    let cells = area.width as f64 * area.height as f64;
    if weights.is_empty() || cells < 1.0 {
        return (Vec::new(), weights.len());
    }
    let total: f64 = weights.iter().map(|&w| w as f64).sum();
    if total <= 0.0 {
        return (Vec::new(), weights.len());
    }
    let mut kept: Vec<u64> = Vec::with_capacity(weights.len());
    let mut dropped = 0usize;
    let mut dropped_weight = 0u64;
    for &w in weights {
        let fair = w as f64 / total * cells;
        if fair >= 1.0 {
            kept.push(w);
        } else {
            dropped += 1;
            dropped_weight += w;
        }
    }
    let overflow_index = kept.len();
    let mut all = kept;
    if dropped > 0 {
        all.push(dropped_weight.max(1));
    }
    let (placed, _not_placed) = squarify(&all, area);
    let entries: Vec<TreemapEntry> = placed
        .into_iter()
        .map(|p| TreemapEntry {
            index: if p.index == overflow_index {
                OVERFLOW
            } else {
                p.index
            },
            rect: p.rect,
        })
        .collect();
    // Anything not individually placed is reported as aggregated.
    let real_placed = entries.iter().filter(|e| e.index != OVERFLOW).count();
    let dropped_total = weights.len() - real_placed;
    (entries, dropped_total)
}

/// Lay out `weights` (expected in descending order) into `area`.
///
/// Returns the placed rectangles and the number of entries that were dropped
/// because their fair share was under one cell (or because the area ran out).
pub fn squarify(weights: &[u64], area: Rect) -> (Vec<Placed>, usize) {
    let cells = area.width as f64 * area.height as f64;
    if weights.is_empty() || cells < 1.0 {
        return (Vec::new(), weights.len());
    }
    let total: f64 = weights.iter().map(|&w| w as f64).sum();
    if total <= 0.0 {
        return (Vec::new(), weights.len());
    }

    // Keep only entries that can get a few cells: a one-cell sliver is not a
    // shape, it is noise. The rest are aggregated into an overflow cell.
    const MIN_FAIR_CELLS: f64 = 3.0;
    let mut kept: Vec<(usize, f64)> = Vec::new();
    let mut kept_total = 0f64;
    for (i, &w) in weights.iter().enumerate() {
        let fair = w as f64 / total * cells;
        if fair >= MIN_FAIR_CELLS {
            kept.push((i, w as f64));
            kept_total += w as f64;
        }
    }
    if kept.is_empty() {
        // Everything is tiny: let the largest entry fill the area.
        let (i, _) = weights
            .iter()
            .enumerate()
            .max_by_key(|&(_, &w)| w)
            .expect("non-empty weights");
        return (
            vec![Placed {
                index: i,
                rect: area,
            }],
            weights.len() - 1,
        );
    }
    let dropped = weights.len() - kept.len();
    let areas: Vec<(usize, f64)> = kept
        .iter()
        .map(|&(i, w)| (i, w / kept_total * cells))
        .collect();

    let mut placed = Vec::with_capacity(areas.len());
    let mut rect = area;
    let mut rest: &[(usize, f64)] = &areas;
    let mut unplaced = 0usize;
    while !rest.is_empty() && rect.width > 0 && rect.height > 0 {
        let side = rect.width.min(rect.height) as f64;
        let mut row_len = 1;
        let mut best = worst_ratio(&rest[..1], side);
        while row_len < rest.len() {
            let candidate = worst_ratio(&rest[..row_len + 1], side);
            if candidate > best {
                break;
            }
            best = candidate;
            row_len += 1;
        }
        let is_last = row_len == rest.len();
        unplaced += layout_row(&rest[..row_len], &mut rect, &mut placed, is_last);
        rest = &rest[row_len..];
    }
    let unplaced = unplaced + rest.len();
    (placed, dropped + unplaced)
}

/// Worst aspect ratio of a candidate row placed along `side`.
fn worst_ratio(row: &[(usize, f64)], side: f64) -> f64 {
    let sum: f64 = row.iter().map(|&(_, a)| a).sum();
    if sum <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }
    let thickness = sum / side;
    row.iter()
        .map(|&(_, a)| {
            let len = (a / thickness).max(f64::MIN_POSITIVE);
            (thickness / len).max(len / thickness)
        })
        .fold(1.0, f64::max)
}

/// Place one row as a strip along the shorter side of `rect`, then shrink
/// `rect` by the strip. When `fill` is set (the final row), the strip takes
/// the whole remaining perpendicular extent so the area is tiled exactly.
///
/// Returns how many row entries could not be given a cell.
fn layout_row(
    row: &[(usize, f64)],
    rect: &mut Rect,
    placed: &mut Vec<Placed>,
    fill: bool,
) -> usize {
    let total: f64 = row.iter().map(|&(_, a)| a).sum();
    if total <= 0.0 {
        return row.len();
    }
    let mut skipped = 0usize;
    if rect.width >= rect.height {
        // Vertical strip on the left.
        let thickness = if fill {
            rect.width
        } else {
            ((total / rect.height as f64).round() as u16).clamp(1, rect.width)
        };
        let mut y = rect.y;
        let mut remaining = rect.height;
        for (k, &(idx, a)) in row.iter().enumerate() {
            let height = if k + 1 == row.len() {
                remaining
            } else {
                let h = ((a / total) * rect.height as f64).round() as u16;
                h.clamp(1, remaining.saturating_sub(1).max(1))
                    .min(remaining)
            };
            if height > 0 {
                placed.push(Placed {
                    index: idx,
                    rect: Rect {
                        x: rect.x,
                        y,
                        width: thickness,
                        height,
                    },
                });
            } else {
                skipped += 1;
            }
            y += height;
            remaining = remaining.saturating_sub(height);
        }
        rect.x += thickness;
        rect.width -= thickness;
    } else {
        // Horizontal strip on top.
        let thickness = if fill {
            rect.height
        } else {
            ((total / rect.width as f64).round() as u16).clamp(1, rect.height)
        };
        let mut x = rect.x;
        let mut remaining = rect.width;
        for (k, &(idx, a)) in row.iter().enumerate() {
            let width = if k + 1 == row.len() {
                remaining
            } else {
                let w = ((a / total) * rect.width as f64).round() as u16;
                w.clamp(1, remaining.saturating_sub(1).max(1))
                    .min(remaining)
            };
            if width > 0 {
                placed.push(Placed {
                    index: idx,
                    rect: Rect {
                        x,
                        y: rect.y,
                        width,
                        height: thickness,
                    },
                });
            } else {
                skipped += 1;
            }
            x += width;
            remaining = remaining.saturating_sub(width);
        }
        rect.y += thickness;
        rect.height -= thickness;
    }
    skipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn assert_tiles(weights: &[u64], area: Rect, placed: &[Placed]) {
        for p in placed {
            assert!(
                p.rect.x >= area.x
                    && p.rect.y >= area.y
                    && p.rect.right() <= area.right()
                    && p.rect.bottom() <= area.bottom(),
                "rect {:?} escapes area {:?}",
                p.rect,
                area
            );
            assert!(p.rect.width > 0 && p.rect.height > 0);
        }
        for (i, a) in placed.iter().enumerate() {
            for b in &placed[i + 1..] {
                assert!(
                    a.rect.intersection(b.rect).area() == 0,
                    "overlap between {:?} and {:?}",
                    a.rect,
                    b.rect
                );
            }
        }
        let mut indices: Vec<usize> = placed.iter().map(|p| p.index).collect();
        indices.sort_unstable();
        indices.dedup();
        assert_eq!(indices.len(), placed.len(), "duplicate placements");
        let _ = weights;
    }

    #[test]
    fn single_weight_fills_area() {
        let area = Rect::new(0, 0, 80, 24);
        let (placed, dropped) = squarify(&[100], area);
        assert_eq!(dropped, 0);
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].rect, area);
    }

    #[test]
    fn splits_two_weights() {
        let area = Rect::new(0, 0, 80, 24);
        let (placed, dropped) = squarify(&[50, 50], area);
        assert_eq!(dropped, 0);
        assert_eq!(placed.len(), 2);
        assert_tiles(&[50, 50], area, &placed);
        let total: u32 = placed
            .iter()
            .map(|p| p.rect.width as u32 * p.rect.height as u32)
            .sum();
        assert_eq!(total, 80 * 24);
    }

    #[test]
    fn drops_subcell_entries() {
        let area = Rect::new(0, 0, 10, 10);
        let weights = [10_000u64, 1, 1, 1];
        let (placed, dropped) = squarify(&weights, area);
        assert_eq!(placed.len(), 1);
        assert_eq!(dropped, 3);
        assert_eq!(placed[0].rect, area);
    }

    #[test]
    fn empty_and_zero_weights() {
        let area = Rect::new(0, 0, 10, 10);
        assert_eq!(squarify(&[], area), (Vec::new(), 0));
        let (placed, dropped) = squarify(&[0, 0], area);
        assert!(placed.is_empty());
        assert_eq!(dropped, 2);
    }

    proptest! {
        #[test]
        fn tiling_invariants(
            mut weights in prop::collection::vec(1u64..1_000_000, 1..60),
            w in 1u16..200,
            h in 1u16..60,
        ) {
            weights.sort_unstable_by(|a, b| b.cmp(a));
            let area = Rect::new(0, 0, w, h);
            let (placed, dropped) = squarify(&weights, area);
            prop_assert_eq!(placed.len() + dropped, weights.len());
            assert_tiles(&weights, area, &placed);
        }

        #[test]
        fn treemap_covers_area(
            mut weights in prop::collection::vec(1u64..1_000_000, 1..200),
            w in 1u16..120,
            h in 1u16..40,
        ) {
            weights.sort_unstable_by(|a, b| b.cmp(a));
            let area = Rect::new(0, 0, w, h);
            let (entries, dropped) = treemap(&weights, area);
            let placed: Vec<Placed> = entries
                .iter()
                .filter(|e| e.index != OVERFLOW)
                .map(|e| Placed { index: e.index, rect: e.rect })
                .collect();
            prop_assert_eq!(placed.len() + dropped, weights.len());
            let total_cells: u32 = entries.iter().map(|e| e.rect.area()).sum();
            prop_assert_eq!(total_cells, area.area());
            // No overlaps.
            for (i, a) in entries.iter().enumerate() {
                for b in &entries[i + 1..] {
                    prop_assert_eq!(a.rect.intersection(b.rect).area(), 0);
                }
            }
        }

        #[test]
        fn deterministic(
            mut weights in prop::collection::vec(1u64..1_000_000, 1..40),
            w in 1u16..120,
            h in 1u16..40,
        ) {
            weights.sort_unstable_by(|a, b| b.cmp(a));
            let area = Rect::new(0, 0, w, h);
            let a = squarify(&weights, area);
            let b = squarify(&weights, area);
            prop_assert_eq!(a, b);
        }
    }
}
