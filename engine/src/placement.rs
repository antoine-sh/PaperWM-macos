//! Turns a [`Workspace`] into window frames on a monitor.

use std::collections::HashMap;

use crate::backend::WindowId;
use crate::geometry::Rect;
use crate::workspace::{Pin, Workspace};

/// How much of a parked window stays on screen. macOS refuses to move a
/// window completely off every display.
pub const PEEK: f64 = 1.0;

/// Frames for every window of `workspace`, scrolled to `scroll`, shown on a
/// monitor whose usable area is `area`. Columns outside the viewport, or that would spill onto
/// one of the `other_monitors`, are parked against an edge of `area`: macOS
/// would show the spill there, even with "Displays have separate Spaces".
/// Pinned columns stay at their edge, and the other columns scrolled under them are
/// parked. With `original_heights`, a window alone in its column keeps its
/// height there, clamped to the area, instead of filling it.
pub fn frames(
    workspace: &Workspace,
    scroll: f64,
    area: Rect,
    gap: f64,
    other_monitors: &[Rect],
    original_heights: Option<&HashMap<WindowId, f64>>,
) -> Vec<(WindowId, Rect)> {
    let height = area.height - 2.0 * gap;
    let mut result = Vec::new();
    let (lead, trail) = workspace.pin_margins(gap);
    let mut strip_x = 0.0;
    for (index, column) in workspace.columns().iter().enumerate() {
        let x = match column.pinned() {
            Some(Pin::Left) => area.x + gap,
            Some(Pin::Right) => area.max_x() - gap - column.width(),
            None => area.x + gap + strip_x - scroll,
        };
        let column_rect = Rect::new(x, area.y + gap, column.width(), height);
        strip_x += column.width() + gap;

        let rows = height - gap * (column.windows().len() as f64 - 1.0);
        // The focused column always shows, even when wider than the room between the pins.
        let under_pin = column.pinned().is_none()
            && index != workspace.focused_column()
            && ((lead > 0.0 && x < area.x + lead) || (trail > 0.0 && column_rect.max_x() > area.max_x() - trail));
        let mut y = column_rect.y;
        let original =
            original_heights.filter(|_| column.windows().len() == 1).and_then(|h| h.get(&column.windows()[0]).copied());
        let mut rects = Vec::new();
        for (i, &id) in column.windows().iter().enumerate() {
            // A tabbed column lays its windows on top of each other.
            let rect = if column.is_tabbed() {
                column_rect
            } else {
                let row_height = (rows * column.height_share(i)).max(1.0);
                let rect = Rect::new(column_rect.x, y, column_rect.width, row_height);
                y += row_height + gap;
                rect
            };
            let rect = match original {
                Some(h) => Rect { height: h.clamp(1.0, rect.height), ..rect },
                None => rect,
            };
            rects.push((id, rect));
        }
        let park = under_pin || rects.iter().any(|&(_, r)| needs_parking(r, area, other_monitors));
        result.extend(rects.into_iter().map(|(id, r)| (id, if park { parked(r, area, other_monitors) } else { r })));
    }
    result
}

/// Where a window of a hidden workspace goes: `frame`'s size, tucked into
/// the bottom-right corner of the monitor reaching furthest right and down,
/// where it is least likely to overlap another monitor.
// shortcut: assumes no monitor lies right of or below that corner; upgrade
// to a per-arrangement search if users report hidden windows showing.
pub fn hidden_frame(frame: Rect, monitors: &[Rect]) -> Rect {
    let corner = monitors.iter().max_by(|a, b| (a.max_x() + a.max_y()).total_cmp(&(b.max_x() + b.max_y())));
    let Some(area) = corner else { return frame };
    Rect { x: area.max_x() - PEEK, y: area.max_y() - PEEK, ..frame }
}

fn needs_parking(rect: Rect, area: Rect, other_monitors: &[Rect]) -> bool {
    let outside = rect.max_x() <= area.x || rect.x >= area.max_x();
    let spills =
        (rect.x < area.x || rect.max_x() > area.max_x()) && other_monitors.iter().any(|&m| overlap(rect, m) > 0.0);
    outside || spills
}

/// Moves `rect` so only [`PEEK`] of it stays in `area`, on the side it is
/// already closest to, trying the other side and then the bottom edge when
/// that would cover another monitor.
fn parked(rect: Rect, area: Rect, other_monitors: &[Rect]) -> Rect {
    let left = Rect { x: area.x - rect.width + PEEK, ..rect };
    let right = Rect { x: area.max_x() - PEEK, ..rect };
    let bottom = Rect { x: area.x, y: area.max_y() - PEEK, ..rect };
    let centre = rect.x + rect.width / 2.0;
    let sides = if centre < area.x + area.width / 2.0 { [left, right, bottom] } else { [right, left, bottom] };
    sides.into_iter().find(|r| !other_monitors.iter().any(|m| overlap(*r, *m) > 0.0)).unwrap_or(sides[0])
}

/// The area `a` and `b` share.
fn overlap(a: Rect, b: Rect) -> f64 {
    let width = a.max_x().min(b.max_x()) - a.x.max(b.x);
    let height = a.max_y().min(b.max_y()) - a.y.max(b.y);
    width.max(0.0) * height.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0.0, 25.0, 1020.0, 520.0);
    const GAP: f64 = 10.0;
    const VIEW: f64 = 1000.0;

    fn workspace(widths: &[f64]) -> Workspace {
        let mut ws = Workspace::default();
        for (i, &w) in widths.iter().enumerate() {
            ws.insert_window(i as WindowId + 1, w, VIEW);
        }
        ws
    }

    #[test]
    fn hidden_frame_uses_the_far_bottom_right_monitor() {
        let lower_right = Rect::new(1020.0, 100.0, 800.0, 600.0);
        let frame = Rect::new(5.0, 5.0, 300.0, 200.0);
        assert_eq!(hidden_frame(frame, &[AREA]), Rect::new(AREA.max_x() - PEEK, AREA.max_y() - PEEK, 300.0, 200.0));
        assert_eq!(hidden_frame(frame, &[AREA, lower_right]), Rect::new(1820.0 - PEEK, 700.0 - PEEK, 300.0, 200.0));
    }

    #[test]
    fn lays_columns_side_by_side() {
        let mut ws = workspace(&[400.0, 300.0]);
        ws.ensure_focused_visible(VIEW, GAP);
        assert_eq!(
            frames(&ws, ws.scroll(), AREA, GAP, &[], None),
            vec![(1, Rect::new(10.0, 35.0, 400.0, 500.0)), (2, Rect::new(420.0, 35.0, 300.0, 500.0))]
        );
    }

    #[test]
    fn stacks_windows_within_a_column() {
        let mut ws = workspace(&[400.0, 400.0]);
        ws.focus_column(-1);
        ws.consume();
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[], None);
        assert_eq!(frames[0], (1, Rect::new(10.0, 35.0, 400.0, 245.0)));
        assert_eq!(frames[1], (2, Rect::new(10.0, 290.0, 400.0, 245.0)));
    }

    #[test]
    fn stacked_windows_take_their_share_of_the_height() {
        let mut ws = workspace(&[400.0, 400.0]);
        ws.focus_column(-1);
        ws.consume();
        // Rows share 490 points: 0.6 and 0.4 of it.
        ws.resize_row(0.1, 500.0, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[], None);
        assert_eq!(frames[0], (1, Rect::new(10.0, 35.0, 400.0, 294.0)));
        assert_eq!(frames[1], (2, Rect::new(10.0, 339.0, 400.0, 196.0)));
    }

    #[test]
    fn tabbed_column_gives_every_window_the_full_column() {
        let mut ws = workspace(&[400.0, 400.0]);
        ws.focus_column(-1);
        ws.consume();
        ws.toggle_tabbed();
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[], None);
        assert_eq!(frames[0], (1, Rect::new(10.0, 35.0, 400.0, 500.0)));
        assert_eq!(frames[1], (2, Rect::new(10.0, 35.0, 400.0, 500.0)));
    }

    #[test]
    fn partially_visible_column_peeks_in_from_the_edge() {
        let mut ws = workspace(&[600.0, 600.0]);
        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[], None);
        assert_eq!(frames[1], (2, Rect::new(620.0, 35.0, 600.0, 500.0)));
    }

    #[test]
    fn offscreen_columns_are_parked_on_their_side() {
        let mut ws = workspace(&[900.0, 1000.0, 900.0]);
        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[], None);
        assert_eq!(frames[0].1.x, AREA.x - 900.0 + PEEK);
        assert_eq!(frames[1].1.x, 10.0);
        assert_eq!(frames[2].1.x, AREA.max_x() - PEEK);
    }

    #[test]
    fn parks_away_from_neighbouring_monitors() {
        let right_monitor = Rect::new(1020.0, 0.0, 800.0, 600.0);
        let mut ws = workspace(&[600.0, 600.0]);
        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[right_monitor], None);
        // Column 2 would spill onto the right monitor, so it goes left instead.
        assert_eq!(frames[1].1.x, AREA.x - 600.0 + PEEK);
    }

    #[test]
    fn pinned_columns_stay_at_their_edge_over_the_scrolled_strip() {
        let mut ws = workspace(&[300.0, 600.0, 600.0]);
        ws.focus_edge(false);
        ws.toggle_pin(Pin::Left, VIEW, GAP);
        ws.focus_edge(true);
        ws.ensure_focused_visible(VIEW, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[], None);
        assert_eq!(frames[0], (1, Rect::new(10.0, 35.0, 300.0, 500.0)));
        // Column 2 slid under the pin, so it is parked rather than covering it.
        assert_eq!(frames[1].1.x, AREA.x - 600.0 + PEEK);
        assert_eq!(frames[2].1.x, 410.0);

        let mut ws = workspace(&[600.0, 600.0, 300.0]);
        ws.toggle_pin(Pin::Right, VIEW, GAP);
        ws.focus_edge(false);
        ws.ensure_focused_visible(VIEW, GAP);
        let shown = super::frames(&ws, ws.scroll(), AREA, GAP, &[], None);
        assert_eq!(shown[0].1.x, 10.0);
        assert_eq!(shown[1].1.x, AREA.max_x() - PEEK);
        assert_eq!(shown[2], (3, Rect::new(710.0, 35.0, 300.0, 500.0)));
    }

    #[test]
    fn focused_column_wider_than_the_room_between_pins_stays_on_screen() {
        let mut ws = workspace(&[300.0, 500.0, 300.0]);
        ws.toggle_pin(Pin::Right, VIEW, GAP);
        ws.focus_edge(false);
        ws.toggle_pin(Pin::Left, VIEW, GAP);
        ws.focus_column(1);
        ws.ensure_focused_visible(VIEW, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[], None);
        assert_eq!(frames[1], (2, Rect::new(320.0, 35.0, 500.0, 500.0)));
    }

    #[test]
    fn falls_back_to_bottom_edge_between_monitors() {
        let others = [Rect::new(-800.0, 0.0, 800.0, 600.0), Rect::new(1020.0, 0.0, 800.0, 600.0)];
        let mut ws = workspace(&[900.0, 900.0]);
        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &others, None);
        assert_eq!((frames[1].1.x, frames[1].1.y), (AREA.x, AREA.max_y() - PEEK));
    }
}
