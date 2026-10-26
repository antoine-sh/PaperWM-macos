//! Turns a [`Workspace`] into window frames on a monitor.

use crate::backend::WindowId;
use crate::geometry::Rect;
use crate::workspace::Workspace;

/// How much of a parked window stays on screen. macOS refuses to move a
/// window completely off every display.
pub const PEEK: f64 = 1.0;

/// Frames for every window of `workspace`, scrolled to `scroll`, shown on a
/// monitor whose usable area is `area`. Columns outside the viewport, or that would spill onto
/// one of the `other_monitors`, are parked against an edge of `area`.
pub fn frames(
    workspace: &Workspace,
    scroll: f64,
    area: Rect,
    gap: f64,
    other_monitors: &[Rect],
) -> Vec<(WindowId, Rect)> {
    let height = area.height - 2.0 * gap;
    let mut result = Vec::new();
    let mut strip_x = 0.0;
    for column in workspace.columns() {
        let x = area.x + gap + strip_x - scroll;
        let column_rect = Rect::new(x, area.y + gap, column.width(), height);
        strip_x += column.width() + gap;

        let rows = height - gap * (column.windows().len() as f64 - 1.0);
        let park = needs_parking(column_rect, area, other_monitors);
        let mut y = column_rect.y;
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
            result.push((id, if park { parked(rect, area, other_monitors) } else { rect }));
        }
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
        (rect.x < area.x || rect.max_x() > area.max_x()) && other_monitors.iter().any(|m| intersects(rect, *m));
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
    sides.into_iter().find(|r| !other_monitors.iter().any(|m| intersects(*r, *m))).unwrap_or(sides[0])
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.max_x() && b.x < a.max_x() && a.y < b.max_y() && b.y < a.max_y()
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
            frames(&ws, ws.scroll(), AREA, GAP, &[]),
            vec![(1, Rect::new(10.0, 35.0, 400.0, 500.0)), (2, Rect::new(420.0, 35.0, 300.0, 500.0))]
        );
    }

    #[test]
    fn stacks_windows_within_a_column() {
        let mut ws = workspace(&[400.0, 400.0]);
        ws.focus_column(-1);
        ws.consume();
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[]);
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
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[]);
        assert_eq!(frames[0], (1, Rect::new(10.0, 35.0, 400.0, 294.0)));
        assert_eq!(frames[1], (2, Rect::new(10.0, 339.0, 400.0, 196.0)));
    }

    #[test]
    fn tabbed_column_gives_every_window_the_full_column() {
        let mut ws = workspace(&[400.0, 400.0]);
        ws.focus_column(-1);
        ws.consume();
        ws.toggle_tabbed();
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[]);
        assert_eq!(frames[0], (1, Rect::new(10.0, 35.0, 400.0, 500.0)));
        assert_eq!(frames[1], (2, Rect::new(10.0, 35.0, 400.0, 500.0)));
    }

    #[test]
    fn partially_visible_column_peeks_in_from_the_edge() {
        let mut ws = workspace(&[600.0, 600.0]);
        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[]);
        assert_eq!(frames[1], (2, Rect::new(620.0, 35.0, 600.0, 500.0)));
    }

    #[test]
    fn offscreen_columns_are_parked_on_their_side() {
        let mut ws = workspace(&[900.0, 1000.0, 900.0]);
        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[]);
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
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &[right_monitor]);
        // Column 2 would spill onto the right monitor, so it goes left instead.
        assert_eq!(frames[1].1.x, AREA.x - 600.0 + PEEK);
    }

    #[test]
    fn falls_back_to_bottom_edge_between_monitors() {
        let others = [Rect::new(-800.0, 0.0, 800.0, 600.0), Rect::new(1020.0, 0.0, 800.0, 600.0)];
        let mut ws = workspace(&[900.0, 900.0]);
        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        let frames = frames(&ws, ws.scroll(), AREA, GAP, &others);
        assert_eq!((frames[1].1.x, frames[1].1.y), (AREA.x, AREA.max_y() - PEEK));
    }
}
