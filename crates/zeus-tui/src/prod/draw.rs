//! Shared buffer-clamped draw primitives for the prod tabs (#281).
//!
//! ## Why this exists
//!
//! ratatui's [`Buffer::set_string`] and `buf[(x, y)]` index panic on
//! **buffer**-out-of-range writes (`buffer.rs`: "index outside of buffer"),
//! *not* panel-out-of-range. Most prod tabs guarded their row loops with a
//! **panel-relative** bound (`y < area.bottom()`), which does NOT protect the
//! real buffer bound: when a parent layout hands a child an `area` whose
//! `.bottom()` reaches `buf.area.bottom()` (Constraint rounding, a +1
//! content-sized height, or a panel flush to the frame edge), a write at
//! `y == area.bottom()` lands one row past the buffer's last valid row and
//! panics — exactly the `index is (38, 36)` on a `98×36` buffer crash.
//!
//! These helpers promote the safe pattern from `pantheon_tab` to a single
//! audited primitive: every write early-returns on `y >= buf.area.bottom()`
//! and clips x to `buf.area.right()`. Routing the prod tabs' raw writes through
//! these turns implicit panel-trust into an explicit buffer clamp.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// Get a mutable reference to the cell at `(x, y)`, clamped to buffer bounds.
///
/// Returns `Some(&mut Cell)` if the coordinates are in bounds, `None` if clipped.
/// Callers chain whatever setters they need: `.set_char()`, `.set_bg()`, `.set_style()`, etc.
#[inline]
pub fn cell_mut_clamped(buf: &mut Buffer, x: u16, y: u16) -> Option<&mut ratatui::buffer::Cell> {
    let area = buf.area;
    if y >= area.bottom() || x >= area.right() {
        return None;
    }
    Some(&mut buf[(x, y)])
}

/// Write `s` starting at `(x, y)`, clamped to the **buffer** bounds.
///
/// - Early-returns if `y` is at or past `buf.area.bottom()` (the panic row).
/// - Stops at `max_x` (caller's panel right edge) **or** `buf.area.right()`,
///   whichever comes first — so a write can never exceed the buffer width.
/// - `x < buf.area.left()` columns are skipped rather than wrapped.
pub fn set_str(x: u16, y: u16, s: &str, style: Style, max_x: u16, buf: &mut Buffer) {
    if y < buf.area.top() || y >= buf.area.bottom() {
        return;
    }
    let right = max_x.min(buf.area.right());
    for (i, ch) in s.chars().enumerate() {
        let cx = x.saturating_add(i as u16);
        if cx < buf.area.left() {
            continue;
        }
        if cx >= right {
            break;
        }
        buf[(cx, y)].set_char(ch).set_style(style);
    }
}

/// Like [`set_str`] but clips to the buffer's right edge (no separate panel max).
pub fn set_line(x: u16, y: u16, s: &str, style: Style, buf: &mut Buffer) {
    set_str(x, y, s, style, buf.area.right(), buf);
}

/// Buffer-clamped drop-in for ratatui's [`Buffer::set_string`].
///
/// Mirrors `set_string`'s exact signature (`T: AsRef<str>`, so `&str`,
/// `String`, and `format!(..)` all work unchanged) but routes through the
/// audited [`set_str`] clamp: a write at/past `buf.area.bottom()` is dropped
/// instead of panicking, and x is clipped to the buffer's right edge. The
/// reroute of the prod tabs' raw `buf.set_string(..)` calls is a pure
/// method-name swap to `set_string_clamped` with zero argument churn (#281p2).
pub trait BufferClampExt {
    fn set_string_clamped<T: AsRef<str>>(&mut self, x: u16, y: u16, s: T, style: Style);
}

impl BufferClampExt for Buffer {
    fn set_string_clamped<T: AsRef<str>>(&mut self, x: u16, y: u16, s: T, style: Style) {
        let right = self.area.right();
        set_str(x, y, s.as_ref(), style, right, self);
    }
}

/// Intersect `area` with the buffer's own area so a child rect handed down by a
/// parent layout can never exceed the frame. Belt-and-suspenders for the
/// per-tab render entry, complementing the per-write clamp above.
pub fn clamp_to_buffer(area: Rect, buf: &Buffer) -> Rect {
    area.intersection(buf.area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn buf(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    #[test]
    fn write_at_buffer_bottom_row_is_dropped_not_panicking() {
        // 98x36 buffer → valid rows 0..=35. A write at y=36 (== bottom) is the
        // exact #281 panic. The clamp must drop it silently.
        let mut b = buf(98, 36);
        set_str(38, 36, "X", Style::default(), 98, &mut b); // must NOT panic
        // nothing written at the (nonexistent) bottom row — buffer untouched.
        assert_eq!(b.area.bottom(), 36);
    }

    #[test]
    fn write_past_buffer_bottom_is_dropped() {
        let mut b = buf(20, 10);
        set_str(0, 10, "hello", Style::default(), 20, &mut b); // y == bottom
        set_str(0, 50, "hello", Style::default(), 20, &mut b); // far past
        // last valid row 9 stays empty (we only wrote out of range).
        assert_eq!(b[(0, 9)].symbol(), " ");
    }

    #[test]
    fn write_clips_to_buffer_right_edge() {
        let mut b = buf(5, 3);
        // "abcdefgh" from x=2 would reach x=9, but buffer right is 5.
        set_str(2, 0, "abcdefgh", Style::default(), 100, &mut b);
        assert_eq!(b[(2, 0)].symbol(), "a");
        assert_eq!(b[(3, 0)].symbol(), "b");
        assert_eq!(b[(4, 0)].symbol(), "c"); // last valid col
        // x=5 doesn't exist — if we'd written it, the index would have panicked.
        assert_eq!(b.area.right(), 5);
    }

    #[test]
    fn max_x_bounds_before_buffer_right() {
        let mut b = buf(20, 3);
        // panel max_x = 4 → only cols 0..4 written even though buffer is wider.
        set_str(0, 0, "abcdef", Style::default(), 4, &mut b);
        assert_eq!(b[(3, 0)].symbol(), "d");
        assert_eq!(b[(4, 0)].symbol(), " "); // clipped at max_x=4
    }

    #[test]
    fn normal_write_inside_bounds_lands() {
        let mut b = buf(20, 5);
        set_str(2, 2, "ok", Style::default(), 20, &mut b);
        assert_eq!(b[(2, 2)].symbol(), "o");
        assert_eq!(b[(3, 2)].symbol(), "k");
    }

    #[test]
    fn clamp_to_buffer_shrinks_overflowing_area() {
        let b = buf(98, 36);
        // a child area whose bottom (40) exceeds the buffer (36)
        let child = Rect::new(0, 0, 98, 40);
        let clamped = clamp_to_buffer(child, &b);
        assert_eq!(clamped.bottom(), 36);
        assert_eq!(clamped.right(), 98);
    }
}
