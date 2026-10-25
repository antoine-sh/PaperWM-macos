//! The PaperWM layout model: an endless horizontal strip of columns, each
//! holding one or more windows stacked vertically, seen through a viewport
//! the width of the monitor.
//!
//! Strip coordinates start at 0 at the left edge of the first column. All
//! widths here exclude the outer gaps around the viewport.

use crate::backend::WindowId;

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    windows: Vec<WindowId>,
    width: f64,
    focused_row: usize,
    /// Width to go back to when leaving full width.
    width_before_full: Option<f64>,
}

impl Column {
    fn new(window: WindowId, width: f64) -> Self {
        Self { windows: vec![window], width, focused_row: 0, width_before_full: None }
    }

    pub fn windows(&self) -> &[WindowId] {
        &self.windows
    }

    pub fn width(&self) -> f64 {
        self.width
    }

    pub fn focused_window(&self) -> WindowId {
        self.windows[self.focused_row]
    }

    pub fn focused_row(&self) -> usize {
        self.focused_row
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workspace {
    columns: Vec<Column>,
    focused: usize,
    /// Strip x shown at the left edge of the viewport.
    scroll: f64,
}

/// Narrowest column the layout produces.
pub const MIN_COLUMN_WIDTH: f64 = 100.0;

impl Workspace {
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn scroll(&self) -> f64 {
        self.scroll
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    pub fn contains(&self, id: WindowId) -> bool {
        self.position(id).is_some()
    }

    pub fn windows(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.columns.iter().flat_map(|c| c.windows.iter().copied())
    }

    pub fn focused_column(&self) -> usize {
        self.focused
    }

    pub fn focused_window(&self) -> Option<WindowId> {
        self.columns.get(self.focused).map(Column::focused_window)
    }

    fn position(&self, id: WindowId) -> Option<(usize, usize)> {
        self.columns.iter().enumerate().find_map(|(c, col)| col.windows.iter().position(|&w| w == id).map(|r| (c, r)))
    }

    /// Strip x of the left edge of column `index`.
    pub fn column_x(&self, index: usize, gap: f64) -> f64 {
        self.columns[..index].iter().map(|c| c.width + gap).sum()
    }

    fn strip_width(&self, gap: f64) -> f64 {
        let n = self.columns.len() as f64;
        self.columns.iter().map(|c| c.width).sum::<f64>() + gap * (n - 1.0).max(0.0)
    }

    /// Adds `id` as a new column right of the focused one and focuses it.
    pub fn insert_window(&mut self, id: WindowId, width: f64, viewport: f64) {
        let index = if self.columns.is_empty() { 0 } else { self.focused + 1 };
        self.columns.insert(index, Column::new(id, clamp_width(width, viewport)));
        self.focused = index;
    }

    /// Removes `id`, keeping focus on the nearest remaining window.
    pub fn remove_window(&mut self, id: WindowId) -> bool {
        let Some((c, r)) = self.position(id) else { return false };
        let column = &mut self.columns[c];
        column.windows.remove(r);
        if column.windows.is_empty() {
            self.columns.remove(c);
            if c < self.focused || (c == self.focused && self.focused == self.columns.len()) {
                self.focused = self.focused.saturating_sub(1);
            }
        } else if column.focused_row >= column.windows.len() || r < column.focused_row {
            column.focused_row = column.focused_row.saturating_sub(1);
        }
        true
    }

    pub fn focus_window(&mut self, id: WindowId) -> bool {
        let Some((c, r)) = self.position(id) else { return false };
        self.focused = c;
        self.columns[c].focused_row = r;
        true
    }

    pub fn focus_column(&mut self, delta: isize) -> bool {
        match self.offset(self.focused, delta, self.columns.len()) {
            Some(target) => {
                self.focused = target;
                true
            }
            None => false,
        }
    }

    pub fn focus_row(&mut self, delta: isize) -> bool {
        let Some(column) = self.columns.get_mut(self.focused) else { return false };
        match offset(column.focused_row, delta, column.windows.len()) {
            Some(target) => {
                column.focused_row = target;
                true
            }
            None => false,
        }
    }

    /// Swaps the focused column with its neighbour `delta` columns away.
    pub fn move_column(&mut self, delta: isize) -> bool {
        let Some(target) = self.offset(self.focused, delta, self.columns.len()) else { return false };
        let column = self.columns.remove(self.focused);
        self.columns.insert(target, column);
        self.focused = target;
        true
    }

    /// Moves the focused window up or down within its column.
    pub fn move_row(&mut self, delta: isize) -> bool {
        let Some(column) = self.columns.get_mut(self.focused) else { return false };
        let Some(target) = offset(column.focused_row, delta, column.windows.len()) else { return false };
        column.windows.swap(column.focused_row, target);
        column.focused_row = target;
        true
    }

    /// Pulls the top window of the column to the right into the bottom of the
    /// focused column (PaperWM's "slurp").
    pub fn consume(&mut self) -> bool {
        let next = self.focused + 1;
        if next >= self.columns.len() {
            return false;
        }
        let window = self.columns[next].windows.remove(0);
        if self.columns[next].windows.is_empty() {
            self.columns.remove(next);
        } else {
            let column = &mut self.columns[next];
            column.focused_row = column.focused_row.saturating_sub(1);
        }
        self.columns[self.focused].windows.push(window);
        true
    }

    /// Moves the focused window out of its column into a new column on the
    /// right (PaperWM's "barf"), keeping it focused.
    pub fn expel(&mut self) -> bool {
        let Some(column) = self.columns.get_mut(self.focused) else { return false };
        if column.windows.len() < 2 {
            return false;
        }
        let window = column.windows.remove(column.focused_row);
        column.focused_row = column.focused_row.min(column.windows.len() - 1);
        let width = column.width;
        self.columns.insert(self.focused + 1, Column::new(window, width));
        self.focused += 1;
        true
    }

    /// Steps the focused column through `fractions` of the viewport.
    pub fn cycle_width(&mut self, fractions: &[f64], viewport: f64, gap: f64) -> bool {
        let Some(column) = self.columns.get_mut(self.focused) else { return false };
        let widths: Vec<f64> = fractions.iter().map(|f| clamp_width(f * (viewport + gap) - gap, viewport)).collect();
        let Some(&first) = widths.first() else { return false };
        column.width = widths.iter().copied().find(|&w| w > column.width + 1.0).unwrap_or(first);
        column.width_before_full = None;
        true
    }

    pub fn toggle_full_width(&mut self, viewport: f64) -> bool {
        let Some(column) = self.columns.get_mut(self.focused) else { return false };
        match column.width_before_full.take() {
            Some(width) => column.width = width,
            None => {
                column.width_before_full = Some(column.width);
                column.width = viewport;
            }
        }
        true
    }

    /// Scrolls the least amount that shows the whole focused column. When
    /// everything fits, the strip is kept flush left.
    pub fn ensure_focused_visible(&mut self, viewport: f64, gap: f64) {
        if self.columns.is_empty() {
            self.scroll = 0.0;
            return;
        }
        if self.strip_width(gap) <= viewport {
            self.scroll = 0.0;
            return;
        }
        let x = self.column_x(self.focused, gap);
        let width = self.columns[self.focused].width;
        if width >= viewport || x < self.scroll {
            self.scroll = x;
        } else if x + width > self.scroll + viewport {
            self.scroll = x + width - viewport;
        }
        // Never leave empty space before the first column, e.g. after a pan.
        self.scroll = self.scroll.max(0.0);
    }

    pub fn center_focused(&mut self, viewport: f64, gap: f64) {
        if let Some(column) = self.columns.get(self.focused) {
            self.scroll = self.column_x(self.focused, gap) + column.width / 2.0 - viewport / 2.0;
        }
    }

    /// Pans the strip without changing focus, stopping half a viewport past
    /// either end.
    pub fn scroll_by(&mut self, dx: f64, viewport: f64, gap: f64) {
        let max = (self.strip_width(gap) - viewport / 2.0).max(-viewport / 2.0);
        self.scroll = (self.scroll + dx).clamp(-viewport / 2.0, max);
    }

    /// Focuses the column whose centre is closest to the viewport's.
    pub fn focus_nearest_centre(&mut self, viewport: f64, gap: f64) -> bool {
        let middle = self.scroll + viewport / 2.0;
        let distance = |i: usize| (self.column_x(i, gap) + self.columns[i].width / 2.0 - middle).abs();
        let Some(nearest) = (0..self.columns.len()).min_by(|&a, &b| distance(a).total_cmp(&distance(b))) else {
            return false;
        };
        let changed = nearest != self.focused;
        self.focused = nearest;
        changed
    }

    /// Focuses the first column, or the last with `last`.
    pub fn focus_edge(&mut self, last: bool) -> bool {
        let Some(end) = self.columns.len().checked_sub(1) else { return false };
        let target = if last { end } else { 0 };
        let changed = target != self.focused;
        self.focused = target;
        changed
    }

    fn offset(&self, from: usize, delta: isize, len: usize) -> Option<usize> {
        if self.columns.is_empty() {
            return None;
        }
        offset(from, delta, len)
    }
}

fn offset(from: usize, delta: isize, len: usize) -> Option<usize> {
    let target = from.checked_add_signed(delta)?;
    (target < len && target != from).then_some(target)
}

fn clamp_width(width: f64, viewport: f64) -> f64 {
    width.min(viewport).max(MIN_COLUMN_WIDTH.min(viewport))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: f64 = 1000.0;
    const GAP: f64 = 10.0;

    fn workspace(widths: &[f64]) -> Workspace {
        let mut ws = Workspace::default();
        for (i, &w) in widths.iter().enumerate() {
            ws.insert_window(i as WindowId + 1, w, VIEW);
        }
        ws
    }

    fn layout(ws: &Workspace) -> Vec<Vec<WindowId>> {
        ws.columns().iter().map(|c| c.windows().to_vec()).collect()
    }

    #[test]
    fn inserts_right_of_focus_and_focuses() {
        let mut ws = workspace(&[300.0, 300.0]);
        ws.focus_window(1);
        ws.insert_window(3, 300.0, VIEW);
        assert_eq!(layout(&ws), vec![vec![1], vec![3], vec![2]]);
        assert_eq!(ws.focused_window(), Some(3));
    }

    #[test]
    fn insert_clamps_width_to_viewport() {
        let ws = workspace(&[5000.0, 10.0]);
        assert_eq!(ws.columns()[0].width(), VIEW);
        assert_eq!(ws.columns()[1].width(), MIN_COLUMN_WIDTH);
    }

    #[test]
    fn removing_focused_last_column_focuses_left_neighbour() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        assert!(ws.remove_window(3));
        assert_eq!(ws.focused_window(), Some(2));
        assert!(!ws.remove_window(3));
    }

    #[test]
    fn removing_focused_middle_column_focuses_right_neighbour() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        ws.focus_window(2);
        ws.remove_window(2);
        assert_eq!(ws.focused_window(), Some(3));
    }

    #[test]
    fn removing_a_column_left_of_focus_keeps_focus() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        ws.remove_window(1);
        assert_eq!(ws.focused_window(), Some(3));
    }

    #[test]
    fn removing_last_window_empties_workspace() {
        let mut ws = workspace(&[300.0]);
        ws.remove_window(1);
        assert!(ws.is_empty());
        assert_eq!(ws.focused_window(), None);
    }

    #[test]
    fn focus_moves_between_columns_and_stops_at_ends() {
        let mut ws = workspace(&[300.0, 300.0]);
        assert!(!ws.focus_column(1));
        assert!(ws.focus_column(-1));
        assert_eq!(ws.focused_window(), Some(1));
        assert!(!ws.focus_column(-1));
        assert!(!Workspace::default().focus_column(1));
    }

    #[test]
    fn move_column_swaps_with_neighbour() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        assert!(ws.move_column(-1));
        assert_eq!(layout(&ws), vec![vec![1], vec![3], vec![2]]);
        assert_eq!(ws.focused_window(), Some(3));
        assert!(ws.move_column(-1));
        assert!(!ws.move_column(-1));
        assert_eq!(layout(&ws), vec![vec![3], vec![1], vec![2]]);
    }

    #[test]
    fn consume_and_expel_round_trip() {
        let mut ws = workspace(&[300.0, 400.0, 500.0]);
        ws.focus_window(1);
        assert!(ws.consume());
        assert_eq!(layout(&ws), vec![vec![1, 2], vec![3]]);
        assert_eq!(ws.focused_window(), Some(1));

        assert!(ws.focus_row(1));
        assert_eq!(ws.focused_window(), Some(2));
        assert!(ws.move_row(-1));
        assert_eq!(layout(&ws), vec![vec![2, 1], vec![3]]);

        assert!(ws.expel());
        assert_eq!(layout(&ws), vec![vec![1], vec![2], vec![3]]);
        assert_eq!(ws.focused_window(), Some(2));
        assert_eq!(ws.columns()[1].width(), 300.0);
        assert!(!ws.expel());
    }

    #[test]
    fn consume_needs_a_column_on_the_right() {
        let mut ws = workspace(&[300.0]);
        assert!(!ws.consume());
    }

    #[test]
    fn removing_from_a_stack_keeps_a_valid_focus() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        ws.focus_window(1);
        ws.consume();
        ws.consume();
        ws.focus_window(3);
        ws.remove_window(3);
        assert_eq!(ws.focused_window(), Some(2));
        ws.remove_window(1);
        assert_eq!(ws.focused_window(), Some(2));
    }

    #[test]
    fn cycle_width_steps_through_presets_and_wraps() {
        let mut ws = workspace(&[300.0]);
        let presets = [1.0 / 3.0, 0.5, 2.0 / 3.0];
        let expect = |f: f64| f * (VIEW + GAP) - GAP;
        ws.cycle_width(&presets, VIEW, GAP);
        assert!((ws.columns()[0].width() - expect(1.0 / 3.0)).abs() < 1e-9);
        ws.cycle_width(&presets, VIEW, GAP);
        assert_eq!(ws.columns()[0].width(), expect(0.5));
        ws.cycle_width(&presets, VIEW, GAP);
        ws.cycle_width(&presets, VIEW, GAP);
        assert!((ws.columns()[0].width() - expect(1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn two_half_columns_fill_the_viewport() {
        let mut ws = workspace(&[300.0, 300.0]);
        ws.cycle_width(&[0.5], VIEW, GAP);
        ws.focus_column(-1);
        ws.cycle_width(&[0.5], VIEW, GAP);
        assert_eq!(ws.strip_width(GAP), VIEW);
    }

    #[test]
    fn toggle_full_width_restores_previous_width() {
        let mut ws = workspace(&[300.0]);
        ws.toggle_full_width(VIEW);
        assert_eq!(ws.columns()[0].width(), VIEW);
        ws.toggle_full_width(VIEW);
        assert_eq!(ws.columns()[0].width(), 300.0);
    }

    #[test]
    fn scrolls_minimally_to_show_focused_column() {
        let mut ws = workspace(&[600.0, 600.0, 600.0]);
        ws.ensure_focused_visible(VIEW, GAP);
        // Column 3 spans 1220..1820; its right edge meets the viewport's.
        assert_eq!(ws.scroll(), 820.0);

        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        assert_eq!(ws.scroll(), 610.0);

        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        assert_eq!(ws.scroll(), 0.0);
    }

    #[test]
    fn keeps_scroll_when_focused_column_is_visible() {
        let mut ws = workspace(&[400.0, 400.0, 400.0]);
        ws.ensure_focused_visible(VIEW, GAP);
        let scroll = ws.scroll();
        ws.focus_column(-1);
        ws.ensure_focused_visible(VIEW, GAP);
        assert_eq!(ws.scroll(), scroll);
    }

    #[test]
    fn strip_that_fits_stays_flush_left() {
        let mut ws = workspace(&[300.0, 300.0]);
        ws.center_focused(VIEW, GAP);
        assert_ne!(ws.scroll(), 0.0);
        ws.ensure_focused_visible(VIEW, GAP);
        assert_eq!(ws.scroll(), 0.0);
    }

    #[test]
    fn scroll_by_clamps_to_half_a_viewport_past_either_end() {
        let mut ws = workspace(&[600.0, 600.0, 600.0]);
        ws.scroll_by(5000.0, VIEW, GAP);
        assert_eq!(ws.scroll(), 1820.0 - VIEW / 2.0);
        ws.scroll_by(-10000.0, VIEW, GAP);
        assert_eq!(ws.scroll(), -VIEW / 2.0);
    }

    #[test]
    fn focus_nearest_centre_picks_column_under_middle() {
        let mut ws = workspace(&[600.0, 600.0, 600.0]);
        ws.focus_column(-2);
        ws.ensure_focused_visible(VIEW, GAP);
        ws.focus_column(2);
        assert!(ws.focus_nearest_centre(VIEW, GAP));
        assert_eq!(ws.focused_window(), Some(1));
        ws.scroll_by(700.0, VIEW, GAP);
        assert!(ws.focus_nearest_centre(VIEW, GAP));
        assert_eq!(ws.focused_window(), Some(2));
        assert!(!ws.focus_nearest_centre(VIEW, GAP));
        assert!(!Workspace::default().focus_nearest_centre(VIEW, GAP));
    }

    #[test]
    fn focus_edge_jumps_to_first_and_last_column() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        assert!(ws.focus_edge(false));
        assert_eq!(ws.focused_window(), Some(1));
        assert!(!ws.focus_edge(false));
        assert!(ws.focus_edge(true));
        assert_eq!(ws.focused_window(), Some(3));
        assert!(!Workspace::default().focus_edge(true));
    }

    #[test]
    fn center_puts_focused_column_mid_viewport() {
        let mut ws = workspace(&[600.0, 400.0, 600.0]);
        ws.focus_column(-1);
        ws.center_focused(VIEW, GAP);
        // Column 2 spans 610..1010, centre 810.
        assert_eq!(ws.scroll(), 810.0 - VIEW / 2.0);
    }
}
