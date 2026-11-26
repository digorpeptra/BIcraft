//! The phone canvas of a page's mobile layout (`visuals/<v>/mobile.json` positions), as Power BI
//! Desktop 2.159.1231.0 draws it in View › Mobile layout (black-box observation, 2026-11-23:
//! input events and the page's DOM read through the DevTools port; docs/gaps.md G72): 324 CSS
//! pixels wide, scrolling to 3600 high, its grid cells 18 wide by 20 high. Positions are 1:1 with
//! those pixels. The engine (`visual.mobile {snap: true}`) and the app's phone canvas
//! (`crates/ui/src/mobile.rs`) snap with [`snap_rect`]; the app draws the canvas this wide.
//!
//! Not observed, so not used: how Desktop rounds a position onto the grid (BIcraft takes the
//! nearest line), whether its canvas grows past 3600 (BIcraft's grows with its visuals) and
//! whether it keeps a visual inside the width (BIcraft does when snapping).

/// The phone canvas's width.
pub const WIDTH: f64 = 324.0;
/// The phone canvas's height while it is edited: how far Desktop's canvas scrolls.
pub const HEIGHT: f64 = 3600.0;
/// The grid's column width.
pub const GRID_X: f64 = 18.0;
/// The grid's row height.
pub const GRID_Y: f64 = 20.0;

/// `v` on the nearest multiple of `step`.
fn snap(v: f64, step: f64) -> f64 {
    (v / step).round() * step
}

/// `(x, y, width, height)` on the phone grid: `x` and `width` on its columns, `y` and `height` on
/// its rows, at least one cell, at most the phone's width, kept inside it (`x` from 0 to
/// `WIDTH - width`, `y` from 0). Non-finite values give 0 (a position) or one cell (a size).
pub fn snap_rect(x: f64, y: f64, width: f64, height: f64) -> (f64, f64, f64, f64) {
    let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
    let w = snap(finite(width), GRID_X).clamp(GRID_X, WIDTH);
    let h = snap(finite(height), GRID_Y).max(GRID_Y);
    let x = snap(finite(x), GRID_X).clamp(0.0, WIDTH - w);
    let y = snap(finite(y), GRID_Y).max(0.0);
    (x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_go_to_the_nearest_grid_line_inside_the_phone() {
        assert_eq!(snap_rect(10.0, 31.0, 300.0, 107.0), (18.0, 40.0, 306.0, 100.0));
        // kept inside the width; the grid divides it, so the last column is on the grid too
        assert_eq!(snap_rect(250.0, 0.0, 100.0, 20.0), (216.0, 0.0, 108.0, 20.0));
        assert_eq!(WIDTH % GRID_X, 0.0);
        // at least one cell, at most the phone's width, never above or left of it
        assert_eq!(snap_rect(-40.0, -9.0, 400.0, 3.0), (0.0, 0.0, 324.0, 20.0));
        assert_eq!(snap_rect(5.0, 5.0, 2.0, 2.0), (0.0, 0.0, 18.0, 20.0));
        assert_eq!(
            snap_rect(f64::NAN, f64::INFINITY, f64::NAN, f64::NEG_INFINITY),
            (0.0, 0.0, 18.0, 20.0)
        );
    }
}
