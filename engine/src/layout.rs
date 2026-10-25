//! Pure layout calculations.

use crate::geometry::Rect;

/// Splits `area` into `count` equal-width columns separated by `gap`, with a
/// `gap` margin on every side.
pub fn columns(area: Rect, count: usize, gap: f64) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let gap = gap.max(0.0);
    let inner_width = area.width - gap * (count as f64 + 1.0);
    let width = (inner_width / count as f64).max(1.0);
    let height = (area.height - 2.0 * gap).max(1.0);
    (0..count).map(|i| Rect::new(area.x + gap + i as f64 * (width + gap), area.y + gap, width, height)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_columns() {
        assert!(columns(Rect::new(0.0, 0.0, 100.0, 100.0), 0, 10.0).is_empty());
    }

    #[test]
    fn single_column_fills_area_minus_gaps() {
        let rects = columns(Rect::new(0.0, 25.0, 1000.0, 800.0), 1, 10.0);
        assert_eq!(rects, vec![Rect::new(10.0, 35.0, 980.0, 780.0)]);
    }

    #[test]
    fn columns_are_evenly_spaced() {
        let rects = columns(Rect::new(0.0, 0.0, 1030.0, 500.0), 3, 10.0);
        assert_eq!(
            rects,
            vec![
                Rect::new(10.0, 10.0, 330.0, 480.0),
                Rect::new(350.0, 10.0, 330.0, 480.0),
                Rect::new(690.0, 10.0, 330.0, 480.0),
            ]
        );
        assert_eq!(rects[2].max_x() + 10.0, 1030.0);
    }

    #[test]
    fn negative_gap_is_treated_as_zero() {
        let rects = columns(Rect::new(0.0, 0.0, 200.0, 100.0), 2, -5.0);
        assert_eq!(rects, vec![Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(100.0, 0.0, 100.0, 100.0)]);
    }
}
