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
    /// Each window's share of the column height, summing to 1. Empty means
    /// equal shares, which is what any change to the stack goes back to.
    heights: Vec<f64>,
    /// Shows only the focused window, at the column's full height, with the
    /// others behind it.
    tabbed: bool,
    pinned: Option<Pin>,
}

/// The screen edge a pinned column stays at while the rest of the strip
/// scrolls beside it. Pinned columns are kept at that end of the strip, so
/// focusing left and right reaches them in screen order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pin {
    Left,
    Right,
}

impl Column {
    fn new(window: WindowId, width: f64) -> Self {
        Self {
            windows: vec![window],
            width,
            focused_row: 0,
            width_before_full: None,
            heights: Vec::new(),
            tabbed: false,
            pinned: None,
        }
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

    /// The share of the column height that window `row` gets.
    pub fn height_share(&self, row: usize) -> f64 {
        self.heights.get(row).copied().unwrap_or(1.0 / self.windows.len() as f64)
    }

    pub fn is_tabbed(&self) -> bool {
        self.tabbed
    }

    pub fn pinned(&self) -> Option<Pin> {
        self.pinned
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workspace {
    columns: Vec<Column>,
    focused: usize,
    /// Strip x shown at the left edge of the viewport.
    scroll: f64,
    /// Whether `scroll` was set by centering, which the minimal scrolling
    /// of `ensure_focused_visible` must not undo.
    centered: bool,
}

/// When the strip scrolls to keep the focused column centred rather than
/// just visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CenterFocus {
    #[default]
    Never,
    /// Only while the strip is wider than the viewport.
    OnOverflow,
    Always,
}

/// Narrowest column the layout produces.
pub const MIN_COLUMN_WIDTH: f64 = 100.0;

/// Shortest a stacked window gets from resizing its neighbours.
pub const MIN_ROW_HEIGHT: f64 = 100.0;

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

    /// Width the column pinned at `pin` takes from the strip, gap included.
    pub fn pin_margin(&self, pin: Pin, gap: f64) -> f64 {
        self.columns.iter().find(|c| c.pinned == Some(pin)).map_or(0.0, |c| c.width + gap)
    }

    /// The left and right pin margins.
    pub fn pin_margins(&self, gap: f64) -> (f64, f64) {
        (self.pin_margin(Pin::Left, gap), self.pin_margin(Pin::Right, gap))
    }

    /// Shrinks the pins so they leave the strip at least a minimum-width
    /// column, after a pin was widened or moved to a narrower monitor.
    fn fit_pins(&mut self, viewport: f64, gap: f64) {
        let (lead, trail) = self.pin_margins(gap);
        let room = viewport - MIN_COLUMN_WIDTH;
        if lead + trail <= room {
            return;
        }
        for column in self.columns.iter_mut().filter(|c| c.pinned.is_some()) {
            column.width = ((column.width + gap) / (lead + trail) * room - gap).max(MIN_COLUMN_WIDTH.min(viewport));
        }
    }

    fn strip_width(&self, gap: f64) -> f64 {
        let n = self.columns.len() as f64;
        self.columns.iter().map(|c| c.width).sum::<f64>() + gap * (n - 1.0).max(0.0)
    }

    /// Adds `id` as a new column right of the focused one and focuses it.
    pub fn insert_window(&mut self, id: WindowId, width: f64, viewport: f64) {
        let index = if self.columns.is_empty() { 0 } else { self.focused + 1 };
        self.insert_window_at(id, index, width, viewport);
    }

    /// Adds `id` as a new column at `index`, or at the end if there are
    /// fewer columns, pushing the columns from there right, and focuses it.
    pub fn insert_window_at(&mut self, id: WindowId, index: usize, width: f64, viewport: f64) {
        let index = index.min(self.columns.len());
        self.columns.insert(index, Column::new(id, clamp_width(width, viewport)));
        self.focused = index;
        self.keep_pins_at_edges();
    }

    /// Puts `id` in place of column `target`, the columns in between
    /// shifting over to make room, and focuses it. A stacked window leaves
    /// its column for a column of its own.
    pub fn move_window_to(&mut self, id: WindowId, target: usize) -> bool {
        let Some((c, r)) = self.position(id) else { return false };
        let target = target.min(self.columns.len() - 1);
        if target == c {
            return false;
        }
        if self.columns[c].windows.len() == 1 {
            let column = self.columns.remove(c);
            self.columns.insert(target, column);
            self.focused = target;
            self.keep_pins_at_edges();
            return true;
        }
        let column = &mut self.columns[c];
        column.windows.remove(r);
        column.heights.clear();
        if column.focused_row >= column.windows.len() || r < column.focused_row {
            column.focused_row = column.focused_row.saturating_sub(1);
        }
        let width = column.width;
        // Column `c` stays, so moving right lands just past the target.
        let index = if target > c { target + 1 } else { target };
        self.columns.insert(index, Column::new(id, width));
        self.focused = index;
        self.keep_pins_at_edges();
        true
    }

    /// Removes `id`, keeping focus on the nearest remaining window.
    pub fn remove_window(&mut self, id: WindowId) -> bool {
        let Some((c, r)) = self.position(id) else { return false };
        let column = &mut self.columns[c];
        column.windows.remove(r);
        column.heights.clear();
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

    /// Puts `new` in the place of `old`.
    pub fn replace_window(&mut self, old: WindowId, new: WindowId) {
        if let Some((c, r)) = self.position(old) {
            self.columns[c].windows[r] = new;
        }
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
        if self.columns[target].pinned != self.columns[self.focused].pinned {
            return false;
        }
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
        if !column.heights.is_empty() {
            column.heights.swap(column.focused_row, target);
        }
        column.focused_row = target;
        true
    }

    /// Pulls the top window of the column to the right into the bottom of the
    /// focused column (PaperWM's "slurp").
    pub fn consume(&mut self) -> bool {
        let next = self.focused + 1;
        // A pin only goes away when the user unpins it or closes its windows.
        if next >= self.columns.len() || self.columns[next].pinned.is_some() {
            return false;
        }
        let window = self.columns[next].windows.remove(0);
        self.columns[next].heights.clear();
        if self.columns[next].windows.is_empty() {
            self.columns.remove(next);
        } else {
            let column = &mut self.columns[next];
            column.focused_row = column.focused_row.saturating_sub(1);
        }
        self.columns[self.focused].windows.push(window);
        self.columns[self.focused].heights.clear();
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
        column.heights.clear();
        column.focused_row = column.focused_row.min(column.windows.len() - 1);
        let width = column.width;
        self.columns.insert(self.focused + 1, Column::new(window, width));
        self.focused += 1;
        self.keep_pins_at_edges();
        true
    }

    /// Pins the focused column to `pin`'s edge in place of any column pinned
    /// there, or unpins it if it is already there. Refused when the pins
    /// would leave the strip less than a minimum-width column.
    pub fn toggle_pin(&mut self, pin: Pin, viewport: f64, gap: f64) -> bool {
        let Some(column) = self.columns.get(self.focused) else { return false };
        if column.pinned == Some(pin) {
            // Already at its end of the strip, where it stays.
            self.columns[self.focused].pinned = None;
            return true;
        }
        let other = if pin == Pin::Left { Pin::Right } else { Pin::Left };
        let other_margin = if column.pinned == Some(other) { 0.0 } else { self.pin_margin(other, gap) };
        if column.width + gap + other_margin > viewport - MIN_COLUMN_WIDTH {
            return false;
        }
        for column in &mut self.columns {
            if column.pinned == Some(pin) {
                column.pinned = None;
            }
        }
        self.columns[self.focused].pinned = Some(pin);
        self.keep_pins_at_edges();
        true
    }

    /// Moves pinned columns back to their ends of the strip, keeping focus
    /// on the same column.
    fn keep_pins_at_edges(&mut self) {
        let focused = self.focused;
        let mut tagged: Vec<(bool, Column)> =
            std::mem::take(&mut self.columns).into_iter().enumerate().map(|(i, c)| (i == focused, c)).collect();
        tagged.sort_by_key(|(_, c)| match c.pinned {
            Some(Pin::Left) => 0,
            None => 1,
            Some(Pin::Right) => 2,
        });
        self.focused = tagged.iter().position(|&(f, _)| f).unwrap_or(0);
        self.columns = tagged.into_iter().map(|(_, c)| c).collect();
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

    /// Grows (or, with a negative `delta`, shrinks) the focused column by
    /// `delta` points, between the minimum width and the viewport.
    pub fn nudge_width(&mut self, delta: f64, viewport: f64) -> bool {
        let Some(column) = self.columns.get_mut(self.focused) else { return false };
        let width = clamp_width(column.width + delta, viewport);
        if width == column.width {
            return false;
        }
        column.width = width;
        column.width_before_full = None;
        true
    }

    /// Grows the focused window by `step` of its column's `height`, or
    /// shrinks it for a negative step, taking from or giving to the other
    /// windows in the column. No window goes below [`MIN_ROW_HEIGHT`].
    /// Tabbed columns keep their heights for when they stack again.
    pub fn resize_row(&mut self, step: f64, height: f64, gap: f64) -> bool {
        let Some(column) = self.columns.get_mut(self.focused) else { return false };
        let n = column.windows.len();
        // A tabbed column shows each window at full height.
        if n < 2 || column.tabbed {
            return false;
        }
        let rows = (height - gap * (n - 1) as f64).max(1.0);
        let min = (MIN_ROW_HEIGHT / rows).min(1.0 / n as f64);
        let mut shares: Vec<f64> = (0..n).map(|i| column.height_share(i)).collect();
        let row = column.focused_row;
        let others = 1.0 - shares[row];
        let spare = others - min * (n - 1) as f64;
        // Shares set on a taller monitor can sit below `min`; never resize the wrong way.
        let delta = if step > 0.0 { step.min(spare).max(0.0) } else { step.max(min - shares[row]).min(0.0) };
        if delta.abs() < 1e-9 {
            return false;
        }
        for (i, share) in shares.iter_mut().enumerate() {
            if i == row {
                *share += delta;
            } else if delta > 0.0 {
                // In proportion to what each has above the minimum, so none goes below it.
                *share -= delta * (*share - min) / spare;
            } else {
                *share -= delta * *share / others;
            }
        }
        column.heights = shares;
        true
    }

    /// Gives every window in the focused column the same height, unless it
    /// is tabbed.
    pub fn equalize_rows(&mut self) -> bool {
        self.columns.get_mut(self.focused).is_some_and(|c| !c.tabbed && !std::mem::take(&mut c.heights).is_empty())
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

    pub fn toggle_tabbed(&mut self) -> bool {
        let Some(column) = self.columns.get_mut(self.focused) else { return false };
        column.tabbed = !column.tabbed;
        true
    }

    /// Scrolls the least amount that shows the whole focused column in the
    /// width the pins leave. When everything fits, the strip is kept flush
    /// left.
    pub fn ensure_focused_visible(&mut self, viewport: f64, gap: f64) {
        if self.columns.is_empty() {
            self.scroll = 0.0;
            self.centered = false;
            return;
        }
        // Pins are always in view; the strip beside them just stays in range.
        if self.columns[self.focused].pinned.is_some() {
            self.scroll = self.scroll.min(self.strip_width(gap) - viewport).max(0.0);
            return;
        }
        let (lead, trail) = self.pin_margins(gap);
        let x = self.column_x(self.focused, gap) - lead;
        let width = self.columns[self.focused].width;
        let room = viewport - lead - trail;
        if self.centered && x >= self.scroll && x + width <= self.scroll + room {
            return;
        }
        self.centered = false;
        if self.strip_width(gap) <= viewport {
            self.scroll = 0.0;
            return;
        }
        if width >= room || x < self.scroll {
            self.scroll = x;
        } else if x + width > self.scroll + room {
            self.scroll = x + width - room;
        }
        // Never leave empty space before the first column, e.g. after a pan.
        self.scroll = self.scroll.max(0.0);
    }

    /// Brings the focused column into view the way `mode` asks.
    pub fn follow_focus(&mut self, mode: CenterFocus, viewport: f64, gap: f64) {
        self.fit_pins(viewport, gap);
        let center = match mode {
            CenterFocus::Never => false,
            CenterFocus::OnOverflow => self.strip_width(gap) > viewport,
            CenterFocus::Always => !self.columns.is_empty(),
        };
        // A pin is always in view, so only the strip's range is kept.
        let center = center && self.columns[self.focused].pinned.is_none();
        if center {
            self.center_focused(viewport, gap);
            // Centred by the mode, not by request, so it ends with the mode.
            self.centered = false;
        } else {
            self.ensure_focused_visible(viewport, gap);
        }
    }

    /// Centres the focused column in the width the pins leave; pinned
    /// columns stay where they are.
    pub fn center_focused(&mut self, viewport: f64, gap: f64) {
        if let Some(column) = self.columns.get(self.focused).filter(|c| c.pinned.is_none()) {
            self.scroll = self.column_x(self.focused, gap) + column.width / 2.0 - self.strip_middle(viewport, gap);
            self.centered = true;
        }
    }

    /// How far right of the viewport's left edge the middle of the width
    /// the pins leave is.
    fn strip_middle(&self, viewport: f64, gap: f64) -> f64 {
        let (lead, trail) = self.pin_margins(gap);
        lead + (viewport - lead - trail) / 2.0
    }

    /// Pans the strip without changing focus, stopping half a viewport past
    /// either end.
    pub fn scroll_by(&mut self, dx: f64, viewport: f64, gap: f64) {
        let max = (self.strip_width(gap) - viewport / 2.0).max(-viewport / 2.0);
        self.scroll = (self.scroll + dx).clamp(-viewport / 2.0, max);
        self.centered = false;
    }

    /// Strip x of the left edge of every column, pinned ones where they
    /// are shown at the current scroll.
    pub fn column_xs(&self, viewport: f64, gap: f64) -> Vec<f64> {
        let mut x = 0.0;
        self.columns
            .iter()
            .map(|c| {
                let left = x;
                x += c.width + gap;
                match c.pinned {
                    Some(Pin::Left) => self.scroll,
                    Some(Pin::Right) => self.scroll + viewport - c.width,
                    None => left,
                }
            })
            .collect()
    }

    /// The column under strip x `x`, or the nearest one; `None` for an
    /// empty strip.
    pub fn column_at(&self, x: f64, viewport: f64, gap: f64) -> Option<usize> {
        let xs = self.column_xs(viewport, gap);
        let distance = |i: usize| (xs[i] - x).max(x - xs[i] - self.columns[i].width).max(0.0);
        let all = 0..self.columns.len();
        // A pin covers the columns scrolled behind it.
        let pin = all.clone().find(|&i| self.columns[i].pinned.is_some() && distance(i) == 0.0);
        pin.or_else(|| all.min_by(|&a, &b| distance(a).total_cmp(&distance(b))))
    }

    /// Focuses the unpinned column whose centre is closest to the middle of
    /// the width the pins leave.
    pub fn focus_nearest_centre(&mut self, viewport: f64, gap: f64) -> bool {
        let middle = self.scroll + self.strip_middle(viewport, gap);
        let xs = self.column_xs(viewport, gap);
        let distance = |i: usize| (xs[i] + self.columns[i].width / 2.0 - middle).abs();
        let unpinned = (0..self.columns.len()).filter(|&i| self.columns[i].pinned.is_none());
        let Some(nearest) = unpinned.min_by(|&a, &b| distance(a).total_cmp(&distance(b))) else {
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
    fn moving_a_window_onto_a_column_takes_its_place() {
        let mut ws = workspace(&[300.0, 300.0, 300.0, 300.0]);
        assert!(ws.move_window_to(1, 2));
        assert_eq!(layout(&ws), vec![vec![2], vec![3], vec![1], vec![4]]);
        assert_eq!(ws.focused_window(), Some(1));
        assert!(ws.move_window_to(4, 0));
        assert_eq!(layout(&ws), vec![vec![4], vec![2], vec![3], vec![1]]);
        assert!(!ws.move_window_to(4, 0));
    }

    #[test]
    fn moving_a_stacked_window_gives_it_its_own_column() {
        let mut ws = workspace(&[300.0, 200.0, 300.0]);
        ws.focus_column(-1);
        ws.consume();
        assert_eq!(layout(&ws), vec![vec![1], vec![2, 3]]);
        assert!(ws.move_window_to(3, 0));
        assert_eq!(layout(&ws), vec![vec![3], vec![1], vec![2]]);
        assert_eq!(ws.columns()[0].width(), 200.0);
        ws.focus_window(3);
        ws.consume();
        assert_eq!(layout(&ws), vec![vec![3, 1], vec![2]]);
        assert!(ws.move_window_to(3, 1));
        assert_eq!(layout(&ws), vec![vec![1], vec![2], vec![3]]);
        assert_eq!(ws.focused_window(), Some(3));
    }

    #[test]
    fn column_at_finds_the_column_under_or_nearest_a_point() {
        let ws = workspace(&[300.0, 300.0]);
        assert_eq!(ws.column_at(-50.0, VIEW, GAP), Some(0));
        assert_eq!(ws.column_at(150.0, VIEW, GAP), Some(0));
        assert_eq!(ws.column_at(312.0, VIEW, GAP), Some(1));
        assert_eq!(ws.column_at(5000.0, VIEW, GAP), Some(1));
        assert_eq!(Workspace::default().column_at(0.0, VIEW, GAP), None);
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
    fn inserts_at_a_column_clamped_to_the_end() {
        let mut ws = workspace(&[300.0, 300.0]);
        ws.insert_window_at(3, 0, 300.0, VIEW);
        assert_eq!(layout(&ws), vec![vec![3], vec![1], vec![2]]);
        assert_eq!(ws.focused_window(), Some(3));
        ws.insert_window_at(4, 9, 300.0, VIEW);
        assert_eq!(layout(&ws), vec![vec![3], vec![1], vec![2], vec![4]]);
        assert_eq!(ws.focused_window(), Some(4));
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
    fn toggle_tabbed_flips_the_focused_column_and_survives_stacking() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        ws.focus_window(1);
        assert!(ws.toggle_tabbed());
        assert!(ws.columns()[0].is_tabbed());
        assert!(!ws.columns()[1].is_tabbed());
        ws.consume();
        ws.expel();
        assert!(ws.columns()[0].is_tabbed());
        assert!(!ws.columns()[1].is_tabbed());
        ws.focus_window(2);
        assert!(ws.toggle_tabbed());
        assert!(!ws.columns()[0].is_tabbed());
        assert!(!Workspace::default().toggle_tabbed());
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
    fn nudge_width_steps_and_clamps() {
        let mut ws = workspace(&[300.0]);
        assert!(ws.nudge_width(100.0, VIEW));
        assert_eq!(ws.columns()[0].width(), 400.0);
        assert!(ws.nudge_width(1000.0, VIEW));
        assert_eq!(ws.columns()[0].width(), VIEW);
        assert!(!ws.nudge_width(100.0, VIEW));
        assert!(ws.nudge_width(-2000.0, VIEW));
        assert_eq!(ws.columns()[0].width(), MIN_COLUMN_WIDTH);
        assert!(!ws.nudge_width(-100.0, VIEW));
        // A nudge after full width keeps the new width; toggling goes full again.
        ws.toggle_full_width(VIEW);
        ws.nudge_width(-100.0, VIEW);
        ws.toggle_full_width(VIEW);
        assert_eq!(ws.columns()[0].width(), VIEW);
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
        ws.scroll_by(100.0, VIEW, GAP);
        ws.ensure_focused_visible(VIEW, GAP);
        assert_eq!(ws.scroll(), 0.0);
    }

    #[test]
    fn centering_survives_until_the_focused_column_leaves_the_viewport() {
        let mut ws = workspace(&[300.0, 300.0]);
        ws.focus_column(-1);
        ws.center_focused(VIEW, GAP);
        let centred = ws.scroll();
        assert!(centred < 0.0);
        ws.ensure_focused_visible(VIEW, GAP);
        assert_eq!(ws.scroll(), centred);

        let mut ws = workspace(&[600.0, 600.0, 600.0]);
        ws.center_focused(VIEW, GAP);
        ws.focus_column(-2);
        ws.ensure_focused_visible(VIEW, GAP);
        assert_eq!(ws.scroll(), 0.0);
    }

    #[test]
    fn follow_focus_centres_as_configured() {
        let mut ws = workspace(&[300.0, 300.0]);
        ws.follow_focus(CenterFocus::Never, VIEW, GAP);
        assert_eq!(ws.scroll(), 0.0);
        ws.follow_focus(CenterFocus::OnOverflow, VIEW, GAP);
        assert_eq!(ws.scroll(), 0.0);
        ws.follow_focus(CenterFocus::Always, VIEW, GAP);
        // Column 2 spans 310..610; its middle sits at the viewport's.
        assert_eq!(ws.scroll(), 460.0 - VIEW / 2.0);

        let mut ws = workspace(&[600.0, 600.0, 600.0]);
        ws.focus_column(-1);
        ws.follow_focus(CenterFocus::OnOverflow, VIEW, GAP);
        assert_eq!(ws.scroll(), 910.0 - VIEW / 2.0);
        // Once centring is off again, minimal scrolling takes over.
        ws.focus_column(-1);
        ws.follow_focus(CenterFocus::Never, VIEW, GAP);
        assert_eq!(ws.scroll(), 0.0);
    }

    #[test]
    fn centring_on_overflow_stops_once_the_strip_fits() {
        let mut ws = workspace(&[300.0, 300.0, 300.0, 300.0]);
        ws.focus_column(-1);
        ws.follow_focus(CenterFocus::OnOverflow, VIEW, GAP);
        assert!(ws.scroll() > 0.0);
        ws.remove_window(4);
        ws.follow_focus(CenterFocus::OnOverflow, VIEW, GAP);
        assert_eq!(ws.scroll(), 0.0);
    }

    #[test]
    fn column_xs_lists_left_edges() {
        let ws = workspace(&[300.0, 400.0, 500.0]);
        assert_eq!(ws.column_xs(VIEW, GAP), vec![0.0, 310.0, 720.0]);
        assert_eq!(ws.column_xs(VIEW, GAP)[2], ws.column_x(2, GAP));
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

    fn pins(ws: &Workspace) -> Vec<Option<Pin>> {
        ws.columns().iter().map(Column::pinned).collect()
    }

    #[test]
    fn pinning_moves_the_column_to_its_edge_and_pinning_again_unpins() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        ws.focus_column(-1);
        assert!(ws.toggle_pin(Pin::Left, VIEW, GAP));
        assert_eq!(layout(&ws), vec![vec![2], vec![1], vec![3]]);
        assert_eq!(pins(&ws), vec![Some(Pin::Left), None, None]);
        assert_eq!(ws.focused_window(), Some(2));
        assert!(ws.toggle_pin(Pin::Left, VIEW, GAP));
        assert_eq!(pins(&ws), vec![None, None, None]);
        assert!(!Workspace::default().toggle_pin(Pin::Right, VIEW, GAP));
    }

    #[test]
    fn pinning_another_column_replaces_the_pin_on_that_side() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        ws.focus_window(2);
        ws.toggle_pin(Pin::Left, VIEW, GAP);
        ws.focus_window(3);
        ws.toggle_pin(Pin::Left, VIEW, GAP);
        assert_eq!(layout(&ws), vec![vec![3], vec![2], vec![1]]);
        assert_eq!(pins(&ws), vec![Some(Pin::Left), None, None]);
        ws.toggle_pin(Pin::Right, VIEW, GAP);
        assert_eq!(layout(&ws), vec![vec![2], vec![1], vec![3]]);
        assert_eq!(pins(&ws), vec![None, None, Some(Pin::Right)]);
    }

    #[test]
    fn columns_stay_between_the_pins() {
        let mut ws = workspace(&[300.0, 300.0]);
        ws.toggle_pin(Pin::Right, VIEW, GAP);
        ws.insert_window(3, 300.0, VIEW);
        assert_eq!(layout(&ws), vec![vec![1], vec![3], vec![2]]);
        assert_eq!(ws.focused_window(), Some(3));
        assert!(!ws.move_column(1));
        ws.focus_window(1);
        assert!(ws.move_window_to(1, 2));
        assert_eq!(layout(&ws), vec![vec![3], vec![1], vec![2]]);
        // Focus still walks onto the pin.
        assert!(ws.focus_column(1));
        assert_eq!(ws.focused_window(), Some(2));
    }

    #[test]
    fn pins_leave_room_for_the_strip() {
        let mut ws = workspace(&[300.0, 600.0, 300.0]);
        ws.focus_column(-1);
        ws.toggle_full_width(VIEW);
        assert!(!ws.toggle_pin(Pin::Left, VIEW, GAP));
        let mut ws = workspace(&[500.0, 300.0, 500.0]);
        assert!(ws.toggle_pin(Pin::Right, VIEW, GAP));
        ws.focus_edge(false);
        assert!(!ws.toggle_pin(Pin::Left, VIEW, GAP));
        assert_eq!(pins(&ws), vec![None, None, Some(Pin::Right)]);
    }

    #[test]
    fn pins_grown_past_the_room_are_shrunk_to_fit() {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        ws.focus_edge(false);
        ws.toggle_pin(Pin::Left, VIEW, GAP);
        ws.toggle_full_width(VIEW);
        ws.follow_focus(CenterFocus::Never, VIEW, GAP);
        assert_eq!(ws.pin_margins(GAP), (VIEW - MIN_COLUMN_WIDTH, 0.0));
    }

    #[test]
    fn centring_keeps_the_strip_in_range_while_a_pin_is_focused() {
        let mut ws = workspace(&[300.0, 300.0, 300.0, 300.0, 300.0]);
        ws.follow_focus(CenterFocus::Always, VIEW, GAP);
        ws.toggle_pin(Pin::Left, VIEW, GAP);
        ws.follow_focus(CenterFocus::Always, VIEW, GAP);
        assert_eq!(ws.scroll(), 1540.0 - VIEW);
    }

    #[test]
    fn consume_leaves_a_pin_alone() {
        let mut ws = workspace(&[300.0, 300.0]);
        ws.toggle_pin(Pin::Right, VIEW, GAP);
        ws.focus_column(-1);
        assert!(!ws.consume());
        assert_eq!(layout(&ws), vec![vec![1], vec![2]]);
    }

    #[test]
    fn the_strip_scrolls_in_the_width_beside_the_pins() {
        let mut ws = workspace(&[300.0, 600.0, 600.0, 600.0]);
        ws.focus_edge(false);
        ws.toggle_pin(Pin::Left, VIEW, GAP);
        ws.focus_edge(true);
        ws.ensure_focused_visible(VIEW, GAP);
        // Column 4 spans 1530..2130 and ends at the right of the viewport.
        assert_eq!(ws.scroll(), 1130.0);
        ws.focus_column(-2);
        ws.ensure_focused_visible(VIEW, GAP);
        // Column 2 starts at 310, just right of the 300 wide pin.
        assert_eq!(ws.scroll(), 0.0);
        ws.focus_column(1);
        ws.center_focused(VIEW, GAP);
        // Column 3's middle (1220) sits mid-way across the 690 left for the strip.
        assert_eq!(ws.scroll(), 1220.0 - (310.0 + 345.0));
        ws.focus_edge(false);
        assert!(ws.focus_nearest_centre(VIEW, GAP));
        assert_eq!(ws.focused_window(), Some(3));
        ws.focus_edge(false);
        let scroll = ws.scroll();
        ws.ensure_focused_visible(VIEW, GAP);
        assert_eq!(ws.scroll(), scroll);
    }

    #[test]
    fn pins_report_where_they_are_shown_and_cover_the_strip_behind_them() {
        let mut ws = workspace(&[300.0, 600.0, 600.0, 300.0]);
        ws.toggle_pin(Pin::Right, VIEW, GAP);
        ws.focus_edge(false);
        ws.toggle_pin(Pin::Left, VIEW, GAP);
        ws.scroll_by(200.0, VIEW, GAP);
        assert_eq!(ws.column_xs(VIEW, GAP), vec![200.0, 310.0, 920.0, 200.0 + VIEW - 300.0]);
        assert_eq!(ws.column_at(250.0, VIEW, GAP), Some(0));
        assert_eq!(ws.column_at(1100.0, VIEW, GAP), Some(3));
        assert_eq!(ws.column_at(600.0, VIEW, GAP), Some(1));
    }

    #[test]
    fn center_puts_focused_column_mid_viewport() {
        let mut ws = workspace(&[600.0, 400.0, 600.0]);
        ws.focus_column(-1);
        ws.center_focused(VIEW, GAP);
        // Column 2 spans 610..1010, centre 810.
        assert_eq!(ws.scroll(), 810.0 - VIEW / 2.0);
    }

    fn shares(ws: &Workspace) -> Vec<f64> {
        let column = &ws.columns()[ws.focused_column()];
        (0..column.windows().len()).map(|i| (column.height_share(i) * 1000.0).round() / 1000.0).collect()
    }

    /// Workspace with windows 1, 2 and 3 stacked in one column, 1 focused.
    fn stack_of_three() -> Workspace {
        let mut ws = workspace(&[300.0, 300.0, 300.0]);
        ws.focus_column(-2);
        ws.consume();
        ws.consume();
        ws
    }

    #[test]
    fn resizing_a_row_trades_height_with_the_others() {
        let mut ws = stack_of_three();
        assert_eq!(shares(&ws), vec![0.333, 0.333, 0.333]);
        assert!(ws.resize_row(0.1, 900.0, GAP));
        assert_eq!(shares(&ws), vec![0.433, 0.283, 0.283]);
        assert!(ws.resize_row(-0.2, 900.0, GAP));
        assert_eq!(shares(&ws), vec![0.233, 0.383, 0.383]);
    }

    #[test]
    fn resizing_a_row_stops_at_the_minimum_height() {
        let mut ws = stack_of_three();
        // 880 points of rows, so the 100 point minimum is 0.114 of the column.
        for _ in 0..10 {
            ws.resize_row(0.1, 900.0, GAP);
        }
        assert_eq!(shares(&ws), vec![0.773, 0.114, 0.114]);
        assert!(!ws.resize_row(0.1, 900.0, GAP));
        for _ in 0..10 {
            ws.resize_row(-0.1, 900.0, GAP);
        }
        assert_eq!(shares(&ws)[0], 0.114);
        assert!(!ws.resize_row(-0.1, 900.0, GAP));
    }

    #[test]
    fn heights_follow_a_moved_window_and_reset_when_stacking_changes() {
        let mut ws = stack_of_three();
        ws.resize_row(0.1, 900.0, GAP);
        ws.move_row(1);
        assert_eq!(shares(&ws), vec![0.283, 0.433, 0.283]);
        assert!(ws.equalize_rows());
        assert_eq!(shares(&ws), vec![0.333, 0.333, 0.333]);
        assert!(!ws.equalize_rows());
        ws.resize_row(0.1, 900.0, GAP);
        ws.expel();
        ws.focus_column(-1);
        assert_eq!(shares(&ws), vec![0.5, 0.5]);
        ws.resize_row(0.1, 900.0, GAP);
        ws.remove_window(3);
        assert_eq!(shares(&ws), vec![1.0]);
    }

    #[test]
    fn resizing_never_goes_the_wrong_way_below_the_minimum() {
        let mut ws = workspace(&[300.0, 300.0]);
        ws.focus_column(-1);
        ws.consume();
        for _ in 0..10 {
            ws.resize_row(0.1, 900.0, GAP);
        }
        assert_eq!(shares(&ws), vec![0.888, 0.112]);
        // On a shorter monitor the 100 point minimum is 0.345 of the column,
        // more than window 2 has, so neither may move towards it.
        assert!(!ws.resize_row(0.1, 300.0, GAP));
        ws.focus_row(1);
        assert!(!ws.resize_row(-0.1, 300.0, GAP));
        assert_eq!(shares(&ws), vec![0.888, 0.112]);
    }

    #[test]
    fn resizing_and_equalizing_do_nothing_in_a_tabbed_column() {
        let mut ws = stack_of_three();
        ws.resize_row(0.1, 900.0, GAP);
        ws.toggle_tabbed();
        assert!(!ws.resize_row(0.1, 900.0, GAP));
        assert!(!ws.equalize_rows());
        ws.toggle_tabbed();
        assert_eq!(shares(&ws), vec![0.433, 0.283, 0.283]);
    }

    #[test]
    fn resizing_needs_a_stack() {
        let mut ws = workspace(&[300.0]);
        assert!(!ws.resize_row(0.1, 900.0, GAP));
        assert!(!ws.equalize_rows());
        assert!(!Workspace::default().resize_row(0.1, 900.0, GAP));
    }
}
