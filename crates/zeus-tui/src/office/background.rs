//! Office background — procedurally generated pixel office floor.
//!
//! Four zones: Engineering (top-left), Comms (top-right),
//! Research (bottom-left), Break Room (bottom-right).
//! Scales to any terminal size — all positions are proportional.
//! S104 #51: Rich furniture — CRT monitors with green screen text, wheeled chairs,
//! bookshelves with colored books, whiteboards, posters/frames, desk props.

use ratatui::style::Color;
use super::renderer::PixelGrid;
use super::palette as P;

/// Reference dimensions the layout was designed for.
pub const REF_W: usize = 80;
pub const REF_H: usize = 40;

/// Scale a reference X coordinate to actual width.
fn sx(x: usize, w: usize) -> usize {
    (x * w) / REF_W
}

/// Scale a reference Y coordinate to actual height.
fn sy(y: usize, h: usize) -> usize {
    (y * h) / REF_H
}

/// Set a single pixel, bounds-checked.
#[inline]
fn px(grid: &mut PixelGrid, x: usize, y: usize, h: usize, w: usize, color: Color) {
    if y < h && x < w {
        grid[y][x] = Some(color);
    }
}

/// Fill a horizontal span [x0..=x1] at row y.
#[inline]
fn hline(grid: &mut PixelGrid, x0: usize, x1: usize, y: usize, h: usize, w: usize, color: Color) {
    if y >= h { return; }
    for x in x0..=x1 {
        if x < w { grid[y][x] = Some(color); }
    }
}

/// Draw a CRT monitor pixel-art block, top-left corner at (mx, my).
/// Monitor is 9 wide × 5 tall (bezel+screen+base).
///   Row my+0:   _ F F F F F F F _    (top bezel)
///   Row my+1:  F S S S S S S S F    (screen row 1)
///   Row my+2:  F S S S S S S S F    (screen row 2)
///   Row my+3:  F S S S S S S S F    (screen row 3)
///   Row my+4:   _ F F F F F F F _    (bottom bezel / stand top)
/// text_colors: up to 6 (x_offset, y_offset, color) pixels for green-screen text
fn draw_crt(
    grid: &mut PixelGrid,
    mx: usize, my: usize,
    h: usize, w: usize,
    screen_color: Color,
    text: &[(usize, usize, Color)],
) {
    // Top bezel
    hline(grid, mx + 1, mx + 7, my,     h, w, P::CRT_FRAME);
    // Screen rows with side bezels
    for row in 0..3 {
        let y = my + 1 + row;
        px(grid, mx,     y, h, w, P::CRT_FRAME);
        hline(grid, mx + 1, mx + 7, y, h, w, screen_color);
        px(grid, mx + 8, y, h, w, P::CRT_FRAME);
    }
    // Bottom bezel
    hline(grid, mx + 1, mx + 7, my + 4, h, w, P::CRT_FRAME);
    // Green-screen text pixels
    for &(tx, ty, col) in text {
        if tx >= 1 && ty >= 1 && ty <= 3 {
            px(grid, mx + tx, my + ty, h, w, col);
        }
    }
}

/// Draw a desk: surface row at desk_y, front row at desk_y+1, legs at desk_y+2.
fn draw_desk(
    grid: &mut PixelGrid,
    dx: usize, desk_y: usize,
    desk_w: usize, h: usize, w: usize,
) {
    hline(grid, dx, dx + desk_w - 1, desk_y,     h, w, P::WOOD1);
    hline(grid, dx, dx + desk_w - 1, desk_y + 1, h, w, P::WOOD2);
    px(grid, dx,               desk_y + 2, h, w, P::WOOD3);
    px(grid, dx + desk_w - 1,  desk_y + 2, h, w, P::WOOD3);
}

/// Draw a wheeled chair: seat row at seat_y, back at seat_y+1, wheels at seat_y+2.
fn draw_chair(
    grid: &mut PixelGrid,
    cx: usize, seat_y: usize,
    h: usize, w: usize,
) {
    // Seat (3 wide)
    hline(grid, cx, cx + 2, seat_y,     h, w, P::CHAIR1);
    // Back (3 wide)
    hline(grid, cx, cx + 2, seat_y + 1, h, w, P::CHAIR_BACK);
    // Wheels
    px(grid, cx.saturating_sub(1), seat_y + 2, h, w, P::CHAIR_WHEEL);
    px(grid, cx + 3,               seat_y + 2, h, w, P::CHAIR_WHEEL);
}

/// Generate the static office floor background at the given pixel dimensions.
/// Called when the terminal resizes; the result is cached until the next resize.
pub fn generate(width: usize, height: usize) -> PixelGrid {
    let w = width.max(20);
    let h = height.max(10);

    let mut grid = vec![vec![Some(P::FLOOR1); w]; h];

    // ── Checkerboard floor ────────────────────────────────────────────────────
    for y in 0..h {
        for x in 0..w {
            grid[y][x] = if (x + y) % 2 == 0 { Some(P::FLOOR1) } else { Some(P::FLOOR2) };
        }
    }

    // ── Walls (top ~10% of height) ────────────────────────────────────────────
    let wall_h = sy(4, h).max(2);
    for y in 0..wall_h {
        for x in 0..w {
            grid[y][x] = if y < wall_h / 2 { Some(P::WALL2) } else { Some(P::WALL1) };
        }
    }
    // Baseboard
    let baseboard_y = wall_h;
    if baseboard_y < h {
        for x in 0..w { grid[baseboard_y][x] = Some(P::WOOD2); }
    }

    // ── Ceiling lights ────────────────────────────────────────────────────────
    for &lx_ref in &[15usize, 35, 55, 70] {
        let lx = sx(lx_ref, w);
        if lx + 1 < w {
            if h > 0 { grid[0][lx] = Some(P::LAMP1); if lx + 1 < w { grid[0][lx + 1] = Some(P::LAMP1); } }
            if h > 1 { grid[1][lx] = Some(P::LAMP2); if lx + 1 < w { grid[1][lx + 1] = Some(P::LAMP2); } }
        }
    }

    // ── Zone dividers (dashed) ────────────────────────────────────────────────
    let divider_x = w / 2;
    let divider_y = h / 2;
    for y in (baseboard_y + 1)..h {
        if y % 4 < 2 && divider_x < w { grid[y][divider_x] = Some(P::WOOD2); }
    }
    for x in 0..w {
        if x % 4 < 2 && divider_y < h { grid[divider_y][x] = Some(P::WOOD2); }
    }

    // ── ENGINEERING ZONE (top-left) — 3 desks with CRT monitors ──────────────
    //   JSX ref lines 73-93: dx = 4 + d*14
    let eng_desk_y = sy(12, h);
    let eng_mon_y  = sy(8, h);
    for d in 0..3usize {
        let dx = sx(4 + d * 14, w);
        let dw = sx(10, w).max(6);

        // Desk (surface + front + legs)
        draw_desk(&mut grid, dx, eng_desk_y, dw, h, w);

        // CRT monitor centred on desk (9px wide, 5px tall, top at mon_y)
        let mon_x = dx + dw / 2 - 4;
        let crt_text: &[(usize, usize, Color)] = &[
            (2, 1, P::CRT_TEXT1), (4, 1, P::CRT_TEXT2), (6, 1, P::CRT_TEXT1),
            (2, 2, P::CRT_TEXT2), (3, 2, P::CRT_TEXT3), (5, 2, P::CRT_TEXT1),
            (2, 3, P::CRT_TEXT1), (4, 3, P::CRT_TEXT3), (7, 3, P::LED1),
        ];
        draw_crt(&mut grid, mon_x, eng_mon_y, h, w, P::CRT_SCREEN, crt_text);

        // Wheeled chair below desk
        let chair_x = dx + dw / 2 - 1;
        let chair_y = eng_desk_y + 4;
        draw_chair(&mut grid, chair_x, chair_y, h, w);

        // Desk props: paper left, mug right
        px(&mut grid, dx + 1,       eng_desk_y, h, w, P::PAPER);
        px(&mut grid, dx + dw - 2,  eng_desk_y, h, w, P::MUG1);
    }

    // Whiteboard on engineering wall (JSX lines 94-97)
    let wb_y0 = sy(2, h).max(1);
    let wb_y1 = (wb_y0 + 2).min(wall_h.saturating_sub(1));
    let wb_x0 = sx(10, w);
    let wb_x1 = sx(30, w).min(w.saturating_sub(1));
    // Border rows
    hline(&mut grid, wb_x0, wb_x1, wb_y0, h, w, P::WB_BORDER);
    if wb_y1 < h {
        hline(&mut grid, wb_x0, wb_x1, wb_y1, h, w, P::WB_BORDER);
    }
    // White surface between border rows
    if wb_y0 + 1 < wb_y1 {
        hline(&mut grid, wb_x0 + 1, wb_x1 - 1, wb_y0 + 1, h, w, P::WHITEBOARD);
    }
    // Marker text on whiteboard (dashes every 3px)
    let text_y = wb_y0 + 1;
    if text_y < h {
        let mut tx = wb_x0 + 2;
        while tx + 1 < wb_x1 {
            px(&mut grid, tx,     text_y, h, w, P::WB_TEXT);
            px(&mut grid, tx + 1, text_y, h, w, P::WB_TEXT);
            tx += 3;
        }
    }
    // Accent dots (marker caps at right edge)
    if wb_y1 > 0 && wb_y1 - 1 < h {
        px(&mut grid, wb_x1.saturating_sub(2), wb_y1 - 1, h, w, P::ACCENT);
        px(&mut grid, wb_x1.saturating_sub(1), wb_y1 - 1, h, w, P::BLUE);
    }

    // ── COMMS ZONE (top-right) — 2 desks with dual monitors ──────────────────
    //   JSX ref lines 99-115: dx = 54 + d*18
    for d in 0..2usize {
        let dx = sx(54 + d * 18, w);
        let dw = sx(14, w).max(8);

        draw_desk(&mut grid, dx, eng_desk_y, dw, h, w);

        // Two monitors per desk (JSX: mx = dx+1+m*7)
        for m in 0..2usize {
            let mon_x = dx + 1 + sx(m * 7, w);
            let crt_text: &[(usize, usize, Color)] = &[
                (1, 1, P::BLUE),  (3, 2, P::CYAN),
                (2, 3, P::BLUE),  (5, 3, P::LED1),
            ];
            draw_crt(&mut grid, mon_x, eng_mon_y, h, w, P::CRT2, crt_text);
        }

        // Chair
        let chair_x = dx + dw / 2 - 1;
        draw_chair(&mut grid, chair_x, eng_desk_y + 4, h, w);
    }

    // Posters on comms wall (JSX lines 116-118)
    let poster_y0 = sy(2, h).max(1);
    let poster_y1 = (poster_y0 + 2).min(wall_h.saturating_sub(1));
    // Poster 1 (dark frame)
    let p1x0 = sx(60, w); let p1x1 = sx(64, w).min(w.saturating_sub(1));
    hline(&mut grid, p1x0, p1x1, poster_y0, h, w, P::FRAME1);
    if poster_y0 + 1 < poster_y1 {
        hline(&mut grid, p1x0 + 1, p1x1.saturating_sub(1), poster_y0 + 1, h, w, P::POSTER2);
    }
    if poster_y1 < h { hline(&mut grid, p1x0, p1x1, poster_y1, h, w, P::FRAME1); }
    // Poster 2 (light frame)
    let p2x0 = sx(70, w); let p2x1 = sx(74, w).min(w.saturating_sub(1));
    hline(&mut grid, p2x0, p2x1, poster_y0, h, w, P::FRAME2);
    if poster_y0 + 1 < poster_y1 {
        hline(&mut grid, p2x0 + 1, p2x1.saturating_sub(1), poster_y0 + 1, h, w, P::POSTER1);
    }
    if poster_y1 < h { hline(&mut grid, p2x0, p2x1, poster_y1, h, w, P::FRAME2); }

    // ── RESEARCH ZONE (bottom-left) — bookshelf + L-desk ─────────────────────
    //   JSX ref lines 120-136
    let shelf_x0 = sx(24, w);
    let shelf_x1 = sx(28, w).min(w.saturating_sub(1));
    let shelf_y0 = sy(24, h);
    let shelf_y1 = sy(38, h).min(h);

    // Bookshelf frame with divider every 3rd row
    for y in shelf_y0..shelf_y1 {
        let col = if y % 3 == 0 { P::SHELF2 } else { P::SHELF1 };
        hline(&mut grid, shelf_x0, shelf_x1, y, h, w, col);
    }
    // Colored books on non-divider rows
    let book_palette = [P::BOOK1, P::BOOK2, P::BOOK3, P::BOOK4, P::BOOK5, P::BOOK6, P::BOOK7];
    for y in shelf_y0..shelf_y1 {
        if y % 3 != 0 {
            for x in (shelf_x0 + 1)..shelf_x1 {
                if x < w {
                    let idx = (x.wrapping_mul(3).wrapping_add(y.wrapping_mul(7))) % 7;
                    grid[y][x] = Some(book_palette[idx]);
                }
            }
        }
    }

    // Research L-desk (JSX lines 128-130)
    let res_desk_y = sy(30, h);
    let res_desk_x0 = sx(32, w);
    let res_desk_x1 = sx(50, w).min(w.saturating_sub(1));
    hline(&mut grid, res_desk_x0, res_desk_x1, res_desk_y,     h, w, P::WOOD1);
    hline(&mut grid, res_desk_x0, res_desk_x1, res_desk_y + 1, h, w, P::WOOD2);

    // CRT on research desk (JSX lines 131-136)
    let res_mon_x = sx(35, w);
    let res_mon_y  = sy(27, h);
    let res_crt_text: &[(usize, usize, Color)] = &[
        (1, 1, P::CRT_TEXT3), (3, 1, P::CRT_TEXT1),
        (1, 2, P::CRT_TEXT2), (4, 2, P::CRT_TEXT1),
    ];
    draw_crt(&mut grid, res_mon_x, res_mon_y, h, w, P::CRT_SCREEN, res_crt_text);

    // Paper scraps on research desk
    px(&mut grid, sx(43, w), res_desk_y, h, w, P::PAPER);
    px(&mut grid, sx(47, w), res_desk_y, h, w, P::PAPER);

    // Research chair
    let res_chair_x = sx(37, w);
    let res_chair_y = sy(34, h);
    draw_chair(&mut grid, res_chair_x, res_chair_y, h, w);

    // Plant in research zone
    let plant_x = sx(26, w);
    let plant_y = sy(24, h);
    px(&mut grid, plant_x,     plant_y,     h, w, P::PLANT3);
    px(&mut grid, plant_x - 1, plant_y + 1, h, w, P::PLANT2);
    px(&mut grid, plant_x,     plant_y + 1, h, w, P::PLANT1);
    px(&mut grid, plant_x + 1, plant_y + 1, h, w, P::PLANT2);
    px(&mut grid, plant_x,     plant_y + 2, h, w, P::WOOD2);

    // ── BREAK ROOM (bottom-right) — sofa + coffee table ───────────────────────
    //   JSX ref lines 137-144
    let sofa_y  = sy(32, h);
    let sofa_x0 = sx(62, w);
    let sofa_x1 = sx(82, w).min(w.saturating_sub(1));

    // Three-row sofa (back, seat, front)
    hline(&mut grid, sofa_x0, sofa_x1, sofa_y,     h, w, P::SOFA3);
    hline(&mut grid, sofa_x0, sofa_x1, sofa_y + 1, h, w, P::SOFA1);
    hline(&mut grid, sofa_x0, sofa_x1, sofa_y + 2, h, w, P::SOFA2);
    // Armrests
    if sofa_y + 2 < h {
        hline(&mut grid, sofa_x0, sofa_x0 + 3, sofa_y + 2, h, w, P::SOFA2);
        hline(&mut grid, sofa_x1.saturating_sub(3), sofa_x1, sofa_y + 2, h, w, P::SOFA2);
    }
    // Pillows (JSX: at x+3, x+8, x+13, x+18 from sofa_x0)
    for &offset in &[3usize, 8, 13, 18] {
        let px_x = sofa_x0 + sx(offset, w);
        px(&mut grid, px_x, sofa_y + 1, h, w, P::SOFA_PILLOW);
    }

    // Coffee table + mugs (JSX lines 143-144)
    let table_y = sy(37, h);
    let table_x0 = sx(68, w);
    let table_x1 = sx(76, w).min(w.saturating_sub(1));
    hline(&mut grid, table_x0, table_x1, table_y,     h, w, P::SHELF3);
    hline(&mut grid, table_x0, table_x1, table_y + 1, h, w, P::WOOD2);
    px(&mut grid, sx(70, w), table_y, h, w, P::MUG1);
    px(&mut grid, sx(71, w), table_y, h, w, P::COFFEE);
    px(&mut grid, sx(74, w), table_y, h, w, P::MUG2);

    // Rug under coffee table
    for y in sy(33, h)..sy(39, h).min(h) {
        for x in sx(64, w)..sx(78, w).min(w) {
            if matches!(grid[y][x], Some(c) if c == P::FLOOR1 || c == P::FLOOR2) {
                grid[y][x] = if (x + y) % 2 == 0 { Some(P::RUG1) } else { Some(P::RUG2) };
            }
        }
    }


    // ── KITCHEN ZONE (bottom-left, below engineering) ─────────────────────────
    //   JSX ref lines 141-145: coffee machine + water cooler + counter
    //   Zone center: {x:10, y:38}
    {
        let kit_y = sy(32, h);
        // Coffee machine (JSX: rect(4,32,8,34)) — 5 wide × 3 tall
        let cm_x0 = sx(4, w);
        let cm_x1 = sx(8, w).min(w.saturating_sub(1));
        if kit_y + 2 < h {
            hline(&mut grid, cm_x0, cm_x1, kit_y,     h, w, P::MACHINE1);
            hline(&mut grid, cm_x0, cm_x1, kit_y + 1, h, w, P::MACHINE2);
            hline(&mut grid, cm_x0, cm_x1, kit_y + 2, h, w, P::MACHINE1);
            px(&mut grid, sx(6, w), kit_y + 1, h, w, P::MACHINE_BTN);
            px(&mut grid, sx(7, w), kit_y + 1, h, w, P::LED1);
            px(&mut grid, sx(5, w), kit_y,     h, w, P::MUG1);
        }
        // Water cooler (JSX: rect(12,31,15,33))
        let wc_y = sy(31, h);
        let wc_x0 = sx(12, w);
        let wc_x1 = sx(15, w).min(w.saturating_sub(1));
        if wc_y + 2 < h {
            hline(&mut grid, wc_x0, wc_x1, wc_y,     h, w, P::COOLER1);
            hline(&mut grid, wc_x0, wc_x1, wc_y + 1, h, w, P::COOLER2);
            hline(&mut grid, wc_x0, wc_x1, wc_y + 2, h, w, P::COOLER1);
            px(&mut grid, sx(13, w), wc_y, h, w, P::WATER_BLUE);
            px(&mut grid, sx(14, w), wc_y, h, w, P::WATER_BLUE);
        }
        // Kitchen counter (JSX: rect(2,36,20,37))
        let counter_y = sy(36, h);
        let ctr_x0 = sx(2, w);
        let ctr_x1 = sx(20, w).min(w.saturating_sub(1));
        if counter_y + 1 < h {
            hline(&mut grid, ctr_x0, ctr_x1, counter_y,     h, w, P::WOOD1);
            hline(&mut grid, ctr_x0, ctr_x1, counter_y + 1, h, w, P::WOOD2);
            px(&mut grid, sx(10, w), counter_y, h, w, P::MUG2);
            px(&mut grid, sx(11, w), counter_y, h, w, P::COFFEE);
        }
    }

    // ── Plants along walls ────────────────────────────────────────────────────
    let plant_wall_y = sy(5, h);
    for &px_ref in &[6usize, 36, 44, 74] {
        let ppx = sx(px_ref, w);
        if ppx + 1 < w {
            px(&mut grid, ppx,     plant_wall_y, h, w, P::PLANT3);
            px(&mut grid, ppx + 1, plant_wall_y, h, w, P::PLANT1);
            if plant_wall_y + 1 < h {
                px(&mut grid, ppx,     plant_wall_y + 1, h, w, P::PLANT1);
                px(&mut grid, ppx + 1, plant_wall_y + 1, h, w, P::PLANT2);
            }
            if plant_wall_y + 2 < h {
                px(&mut grid, ppx, plant_wall_y + 2, h, w, P::WOOD2);
            }
        }
    }

    // ── KITCHEN ZONE (bottom-left corner, below Research) ───────────────────
    // JSX ref: tile floor, water cooler, vending machine
    let kitchen_y0 = sy(30, h);
    let kitchen_x1 = sx(22, w);
    // Tile floor (checkerboard)
    let tile1 = Color::Rgb(46, 54, 56);
    let tile2 = Color::Rgb(52, 62, 64);
    for y in kitchen_y0..h {
        for x in sx(2, w)..kitchen_x1.min(w) {
            if y < h && x < w {
                grid[y][x] = if (x + y) % 2 == 0 { Some(tile1) } else { Some(tile2) };
            }
        }
    }
    // Water cooler
    let cooler_x = sx(6, w);
    let cooler_y = sy(32, h);
    if cooler_x + 2 < w && cooler_y + 3 < h {
        for dy in 0..2 { for dx in 0..2 { grid[cooler_y + dy][cooler_x + dx] = Some(P::COOLER1); } }
        grid[cooler_y + 2][cooler_x] = Some(P::COOLER2);
        grid[cooler_y + 2][cooler_x + 1] = Some(P::COOLER2);
        grid[cooler_y][cooler_x + 1] = Some(P::WATER_BLUE);
    }
    // Vending machine
    let vend_x = sx(14, w);
    let vend_y = sy(32, h);
    if vend_x + 3 < w && vend_y + 5 < h {
        for dy in 0..5 { for dx in 0..3 { grid[vend_y + dy][vend_x + dx] = Some(P::MACHINE1); } }
        grid[vend_y][vend_x] = Some(P::MACHINE2);
        grid[vend_y][vend_x + 2] = Some(P::MACHINE2);
        grid[vend_y + 1][vend_x + 1] = Some(P::MACHINE_BTN);
    }

    grid
}

/// Scale a reference coordinate to actual dimensions (public for state.rs).
pub fn scale_x(ref_x: i32, width: usize) -> i32 {
    (ref_x as usize * width / REF_W) as i32
}

/// Scale a reference Y coordinate to actual dimensions (public for state.rs).
pub fn scale_y(ref_y: i32, height: usize) -> i32 {
    (ref_y as usize * height / REF_H) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_dimensions() {
        let grid = generate(80, 40);
        assert_eq!(grid.len(), 40);
        assert_eq!(grid[0].len(), 80);
    }

    #[test]
    fn test_generate_larger() {
        let grid = generate(160, 80);
        assert_eq!(grid.len(), 80);
        assert_eq!(grid[0].len(), 160);
    }

    #[test]
    fn test_generate_smaller() {
        let grid = generate(40, 20);
        assert_eq!(grid.len(), 20);
        assert_eq!(grid[0].len(), 40);
    }

    #[test]
    fn test_generate_minimum() {
        let grid = generate(10, 5);
        assert_eq!(grid.len(), 10);  // clamped to max(10)
        assert_eq!(grid[0].len(), 20); // clamped to max(20)
    }

    #[test]
    fn test_walls_exist() {
        let grid = generate(80, 40);
        // Wall region exists — top rows should be wall colors (WALL1 or WALL2)
        let cell = grid[0][20];
        assert!(cell == Some(P::WALL1) || cell == Some(P::WALL2),
            "Expected wall color at [0][20], got {:?}", cell);
    }

    #[test]
    fn test_baseboard() {
        let grid = generate(80, 40);
        assert_eq!(grid[4][30], Some(P::WOOD2));
    }

    #[test]
    fn test_engineering_desk() {
        // Desk surface at ref (10, 12) → pixel (10, 12) at 80×40
        let grid = generate(80, 40);
        assert_eq!(grid[12][4], Some(P::WOOD1));
    }

    #[test]
    fn test_crt_frame_present() {
        // CRT top bezel should be CRT_FRAME at mon_y for first desk
        let grid = generate(80, 40);
        // mon_y = sy(8, 40) = 8; first desk dx = sx(4, 80) = 4; dw = sx(10,80)=10; mon_x = 4+5-4=5
        // bezel row is grid[8][6..=12]
        let has_crt = grid[8].iter().any(|c| *c == Some(P::CRT_FRAME));
        assert!(has_crt, "CRT frame should appear in engineering zone");
    }

    #[test]
    fn test_crt_green_text() {
        let grid = generate(80, 40);
        let has_green = grid[9].iter().chain(grid[10].iter()).chain(grid[11].iter())
            .any(|c| *c == Some(P::CRT_TEXT1) || *c == Some(P::CRT_TEXT2) || *c == Some(P::CRT_TEXT3));
        assert!(has_green, "CRT green screen text should be present");
    }

    #[test]
    fn test_bookshelf_books() {
        let grid = generate(80, 40);
        let book_colors = [P::BOOK1, P::BOOK2, P::BOOK3, P::BOOK4, P::BOOK5, P::BOOK6, P::BOOK7];
        // Shelf rows in research zone: sy(25..37) on non-divider rows
        let has_books = (sy(25, 40)..sy(37, 40))
            .filter(|y| y % 3 != 0)
            .any(|y| {
                let x0 = sx(25, 80); let x1 = sx(27, 80).min(79);
                (x0..=x1).any(|x| book_colors.contains(&grid[y][x].unwrap_or(P::FLOOR1)))
            });
        assert!(has_books, "Bookshelf should have colored books");
    }

    #[test]
    fn test_whiteboard_present() {
        let grid = generate(80, 40);
        // wb_y0 = sy(2, 40) = 2; border row
        let has_wb = grid[2].iter().any(|c| *c == Some(P::WB_BORDER));
        assert!(has_wb, "Whiteboard border should be present on wall");
    }

    #[test]
    fn test_chair_wheels() {
        let grid = generate(80, 40);
        let has_wheels = grid.iter().any(|row| row.iter().any(|c| *c == Some(P::CHAIR_WHEEL)));
        assert!(has_wheels, "Wheeled chairs should be present");
    }

    #[test]
    fn test_posters() {
        let grid = generate(80, 40);
        let has_frame = grid[2].iter().any(|c| *c == Some(P::FRAME1) || *c == Some(P::FRAME2));
        assert!(has_frame, "Poster frames should be on the comms wall");
    }

    #[test]
    fn test_sofa_pillows() {
        let grid = generate(80, 40);
        let has_pillow = grid.iter().any(|row| row.iter().any(|c| *c == Some(P::SOFA_PILLOW)));
        assert!(has_pillow, "Sofa pillows should be present in break room");
    }

    #[test]
    fn test_mugs_on_desk() {
        let grid = generate(80, 40);
        let has_mug = grid.iter().any(|row| row.iter().any(|c| *c == Some(P::MUG1)));
        assert!(has_mug, "Mugs should appear on engineering desks");
    }

    #[test]
    fn test_scale_functions() {
        assert_eq!(scale_x(40, 160), 80);
        assert_eq!(scale_y(20, 80), 40);
        assert_eq!(scale_x(80, 80), 80);
    }
}
