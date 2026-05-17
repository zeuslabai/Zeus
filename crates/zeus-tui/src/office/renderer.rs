//! Half-block pixel art renderer for terminal.
// S92: renderer verified by zeus107
//!
//! Each terminal cell displays TWO vertical pixels using the `▀` (upper half block)
//! character: foreground color = top pixel, background color = bottom pixel.
//! An 80x40 pixel grid becomes 80x20 terminal cells.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

/// A 2D grid of pixels. `None` = transparent (shows terminal background).
pub type PixelGrid = Vec<Vec<Option<Color>>>;

/// Half-block character used to encode two vertical pixels per cell.
const HALF_BLOCK: &str = "\u{2580}"; // ▀

/// Create a new pixel grid filled with a single color.
pub fn new_grid(width: usize, height: usize, fill: Option<Color>) -> PixelGrid {
    vec![vec![fill; width]; height]
}

/// Stamp a sprite onto a grid at position (sx, sy).
/// Transparent pixels in the sprite are skipped (no overwrite).
pub fn stamp_sprite(
    grid: &mut PixelGrid,
    sprite: &[Vec<Option<Color>>],
    sx: i32,
    sy: i32,
) {
    let gh = grid.len() as i32;
    let gw = if gh > 0 { grid[0].len() as i32 } else { return };

    for (row, sprite_row) in sprite.iter().enumerate() {
        let py = sy + row as i32;
        if py < 0 || py >= gh { continue; }
        for (col, px) in sprite_row.iter().enumerate() {
            if let Some(color) = px {
                let px_x = sx + col as i32;
                if px_x < 0 || px_x >= gw { continue; }
                grid[py as usize][px_x as usize] = Some(*color);
            }
        }
    }
}

/// Render a pixel grid into a ratatui Buffer using half-block encoding.
///
/// Two pixel rows map to one terminal row:
/// - Row 2n   = top pixel (foreground color)
/// - Row 2n+1 = bottom pixel (background color)
///
/// The grid is rendered starting at `area.x, area.y` and clipped to `area`.
pub fn render_halfblock(grid: &PixelGrid, area: Rect, buf: &mut Buffer) {
    let grid_h = grid.len();
    let grid_w = if grid_h > 0 { grid[0].len() } else { 0 };

    // Number of terminal rows we can fill (each uses 2 pixel rows)
    let term_rows = area.height as usize;
    let term_cols = area.width as usize;

    for term_y in 0..term_rows {
        let pixel_y_top = term_y * 2;
        let pixel_y_bot = pixel_y_top + 1;

        for term_x in 0..term_cols.min(grid_w) {
            let top = if pixel_y_top < grid_h {
                grid[pixel_y_top].get(term_x).copied().flatten()
            } else {
                None
            };
            let bot = if pixel_y_bot < grid_h {
                grid[pixel_y_bot].get(term_x).copied().flatten()
            } else {
                None
            };

            let cell_x = area.x + term_x as u16;
            let cell_y = area.y + term_y as u16;

            if cell_x >= area.x + area.width || cell_y >= area.y + area.height {
                continue;
            }

            let style = match (top, bot) {
                (Some(t), Some(b)) => Style::default().fg(t).bg(b),
                (Some(t), None) => Style::default().fg(t),
                (None, Some(b)) => Style::default().fg(b).bg(b),
                (None, None) => continue,
            };

            if let Some(cell) = buf.cell_mut((cell_x, cell_y)) {
                cell.set_symbol(HALF_BLOCK);
                cell.set_style(style);
            }
        }
    }
}

/// Deep-copy a pixel grid (for compositing sprites onto a background).
pub fn clone_grid(grid: &PixelGrid) -> PixelGrid {
    grid.iter().map(|row| row.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_grid_dimensions() {
        let grid = new_grid(80, 40, None);
        assert_eq!(grid.len(), 40);
        assert_eq!(grid[0].len(), 80);
    }

    #[test]
    fn test_new_grid_fill() {
        let c = Color::Rgb(255, 0, 0);
        let grid = new_grid(10, 10, Some(c));
        assert_eq!(grid[5][5], Some(c));
    }

    #[test]
    fn test_stamp_sprite_within_bounds() {
        let mut grid = new_grid(20, 20, None);
        let sprite = vec![
            vec![Some(Color::Rgb(255, 0, 0)), Some(Color::Rgb(0, 255, 0))],
            vec![Some(Color::Rgb(0, 0, 255)), None],
        ];
        stamp_sprite(&mut grid, &sprite, 5, 5);
        assert_eq!(grid[5][5], Some(Color::Rgb(255, 0, 0)));
        assert_eq!(grid[5][6], Some(Color::Rgb(0, 255, 0)));
        assert_eq!(grid[6][5], Some(Color::Rgb(0, 0, 255)));
        assert_eq!(grid[6][6], None); // transparent pixel not stamped
    }

    #[test]
    fn test_stamp_sprite_clips_negative() {
        let mut grid = new_grid(10, 10, None);
        let sprite = vec![
            vec![Some(Color::Rgb(255, 0, 0)), Some(Color::Rgb(0, 255, 0))],
        ];
        stamp_sprite(&mut grid, &sprite, -1, 0);
        assert_eq!(grid[0][0], Some(Color::Rgb(0, 255, 0)));
    }

    #[test]
    fn test_stamp_sprite_clips_overflow() {
        let mut grid = new_grid(10, 10, None);
        let sprite = vec![
            vec![Some(Color::Rgb(255, 0, 0)), Some(Color::Rgb(0, 255, 0))],
        ];
        stamp_sprite(&mut grid, &sprite, 9, 0);
        assert_eq!(grid[0][9], Some(Color::Rgb(255, 0, 0)));
        // col 10 doesn't exist, should be clipped
    }

    #[test]
    fn test_clone_grid_is_independent() {
        let grid = new_grid(5, 5, Some(Color::Rgb(1, 2, 3)));
        let mut copy = clone_grid(&grid);
        copy[0][0] = None;
        assert_eq!(grid[0][0], Some(Color::Rgb(1, 2, 3)));
        assert_eq!(copy[0][0], None);
    }

    #[test]
    fn test_halfblock_pair_math() {
        // 40 pixel rows -> 20 terminal rows
        let grid = new_grid(10, 40, Some(Color::Rgb(128, 128, 128)));
        let term_rows = grid.len() / 2;
        assert_eq!(term_rows, 20);
    }

    #[test]
    fn test_empty_grid_no_panic() {
        let grid: PixelGrid = vec![];
        let area = Rect::new(0, 0, 10, 10);
        let mut buf = Buffer::empty(area);
        render_halfblock(&grid, area, &mut buf);
    }
}
