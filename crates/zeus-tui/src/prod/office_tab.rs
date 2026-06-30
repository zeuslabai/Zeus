//! Office Tab — retro pixel-art fleet dashboard.
//!
//! Tracks `docs/zeus-the-office-tui.jsx`: a 96×48 pixel office rendered as
//! 24 terminal rows of half-block (`▀`) cells, with live fleet agents layered
//! over the static scene.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::prod::draw::{BufferClampExt, cell_mut_clamped};
use crate::theme;

const OFFICE_W: usize = 96;
const OFFICE_H: usize = 48;
const OFFICE_ROWS: u16 = (OFFICE_H / 2) as u16;
const SIDEBAR_W: u16 = 28;
const STATUS_H: u16 = 1;
const SLOT_COUNT: u8 = 4;
const OFFICE_MOTION_LEG_TICKS: u64 = 16;

type PixelGrid = [[Color; OFFICE_W]; OFFICE_H];

// ── Retro RPG palette from docs/zeus-the-office-tui.jsx ─────────────────────
const PLANK1: Color = Color::Rgb(0x3d, 0x2b, 0x1a);
const PLANK2: Color = Color::Rgb(0x4a, 0x34, 0x22);
const PLANK3: Color = Color::Rgb(0x35, 0x24, 0x18);
const PLANK4: Color = Color::Rgb(0x48, 0x30, 0x20);
const PLANK_HI: Color = Color::Rgb(0x5a, 0x3e, 0x2a);
const PLANK_LINE: Color = Color::Rgb(0x2a, 0x1e, 0x12);
const TILE1: Color = Color::Rgb(0x2e, 0x36, 0x38);
const TILE2: Color = Color::Rgb(0x34, 0x3e, 0x40);
const CARPET1: Color = Color::Rgb(0x4a, 0x20, 0x28);
const CARPET2: Color = Color::Rgb(0x3e, 0x1a, 0x22);
const CARPET3: Color = Color::Rgb(0x56, 0x28, 0x30);
const WALL1: Color = Color::Rgb(0x2a, 0x24, 0x20);
const WALL3: Color = Color::Rgb(0x3a, 0x34, 0x30);
const WALL_TRIM: Color = Color::Rgb(0x4a, 0x3a, 0x2e);
const WALLPAPER1: Color = Color::Rgb(0x2e, 0x28, 0x22);
const WALLPAPER2: Color = Color::Rgb(0x34, 0x2e, 0x28);
const DESK_TOP: Color = Color::Rgb(0x6a, 0x52, 0x38);
const DESK_FRONT: Color = Color::Rgb(0x5a, 0x44, 0x30);
const DESK_SIDE: Color = Color::Rgb(0x4a, 0x38, 0x28);
const DESK_LEG: Color = Color::Rgb(0x3a, 0x2a, 0x1a);
const SHELF1: Color = Color::Rgb(0x5a, 0x42, 0x30);
const SHELF2: Color = Color::Rgb(0x4a, 0x38, 0x28);
const SHELF3: Color = Color::Rgb(0x6a, 0x54, 0x40);
const CRT_SCREEN: Color = Color::Rgb(0x1a, 0x30, 0x48);
const CRT2: Color = Color::Rgb(0x0e, 0x18, 0x24);
const CRT_TEXT1: Color = Color::Rgb(0x40, 0xc0, 0x80);
const CRT_TEXT2: Color = Color::Rgb(0x30, 0xa0, 0x60);
const CRT_TEXT3: Color = Color::Rgb(0x60, 0xe0, 0xa0);
const CRT_FRAME: Color = Color::Rgb(0x4a, 0x4a, 0x5a);
const LED1: Color = Color::Rgb(0x40, 0xff, 0x80);
const CHAIR_SEAT: Color = Color::Rgb(0x6a, 0x3a, 0x2a);
const CHAIR_BACK: Color = Color::Rgb(0x5a, 0x30, 0x20);
const CHAIR_WHEEL: Color = Color::Rgb(0x2a, 0x2a, 0x2a);
const SOFA1: Color = Color::Rgb(0x7a, 0x4a, 0x32);
const SOFA2: Color = Color::Rgb(0x6a, 0x3e, 0x2a);
const SOFA3: Color = Color::Rgb(0x8a, 0x5a, 0x3e);
const SOFA_PILLOW: Color = Color::Rgb(0x9a, 0x6a, 0x48);
const LEAF1: Color = Color::Rgb(0x2d, 0x68, 0x28);
const LEAF2: Color = Color::Rgb(0x3a, 0x7a, 0x30);
const LEAF3: Color = Color::Rgb(0x1e, 0x5a, 0x1a);
const LEAF4: Color = Color::Rgb(0x4a, 0x8a, 0x38);
const POT1: Color = Color::Rgb(0x7a, 0x4a, 0x2a);
const POT2: Color = Color::Rgb(0x6a, 0x3e, 0x22);
const LAMP_SHADE: Color = Color::Rgb(0xd4, 0xa0, 0x40);
const LAMP_GLOW: Color = Color::Rgb(0xff, 0xd8, 0x80);
const LAMP_POST: Color = Color::Rgb(0x5a, 0x4a, 0x3a);
const WARM_GLOW1: Color = Color::Rgb(0x3a, 0x2a, 0x18);
const BOOK1: Color = Color::Rgb(0xa0, 0x30, 0x20);
const BOOK2: Color = Color::Rgb(0x20, 0x50, 0xa0);
const BOOK3: Color = Color::Rgb(0x20, 0xa0, 0x50);
const BOOK4: Color = Color::Rgb(0xa0, 0xa0, 0x20);
const BOOK5: Color = Color::Rgb(0x80, 0x30, 0xa0);
const BOOK6: Color = Color::Rgb(0xa0, 0x60, 0x20);
const BOOK7: Color = Color::Rgb(0x20, 0x60, 0x80);
const MUG1: Color = Color::Rgb(0xe0, 0xd0, 0xc0);
const MUG2: Color = Color::Rgb(0xc0, 0xb0, 0xa0);
const COFFEE: Color = Color::Rgb(0x3a, 0x1e, 0x0e);
const PAPER: Color = Color::Rgb(0xd8, 0xd0, 0xc0);
const FRAME1: Color = Color::Rgb(0x5a, 0x4a, 0x3a);
const FRAME2: Color = Color::Rgb(0x4a, 0x3a, 0x2a);
const WHITEBOARD: Color = Color::Rgb(0xd0, 0xd8, 0xe0);
const WB_BORDER: Color = Color::Rgb(0x8a, 0x8a, 0x9a);
const WB_TEXT: Color = Color::Rgb(0x3a, 0x4a, 0x5a);
const CLOCK: Color = Color::Rgb(0xd0, 0xc8, 0xb8);
const CLOCK_HAND: Color = Color::Rgb(0x2a, 0x2a, 0x2a);
const COOLER1: Color = Color::Rgb(0xa0, 0xb8, 0xc8);
const COOLER2: Color = Color::Rgb(0x8a, 0xa0, 0xb0);
const WATER_BLUE: Color = Color::Rgb(0x60, 0xa0, 0xd0);
const MACHINE1: Color = Color::Rgb(0x4a, 0x4a, 0x4a);
const MACHINE2: Color = Color::Rgb(0x3a, 0x3a, 0x3a);
const MACHINE_BTN: Color = Color::Rgb(0xff, 0x3c, 0x14);
const POSTER1: Color = Color::Rgb(0xa0, 0x40, 0x40);
const POSTER2: Color = Color::Rgb(0x40, 0x40, 0xa0);

/// Live data supplied by the production app. `agents` is populated from
/// `/v1/network/agents`; this widget must not fabricate an old mock roster.
pub struct OfficeLive<'a> {
    pub agents: Option<&'a [crate::api::AgentResponse]>,
    pub status: Option<&'a crate::api::StatusResponse>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AgentState {
    Idle,
    Writing,
    Executing,
    Researching,
    Syncing,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OfficeZone {
    Engineering,
    Comms,
    Research,
    Breakroom,
    Kitchen,
}

#[derive(Clone, Debug)]
struct AgentView {
    name: String,
    state: AgentState,
    zone: OfficeZone,
    task: String,
    model: String,
    slot: usize,
    x: usize,
    y: usize,
}

/// Office tab — retro half-block scene plus live fleet sprite/sidebar overlay.
pub struct OfficeTab<'a> {
    pub focused: Option<u8>,
    pub live: Option<OfficeLive<'a>>,
    pub tick: u64,
    pub show_memo: bool,
    pub show_help: bool,
}

impl OfficeTab<'_> {
    pub fn new() -> Self {
        Self {
            focused: None,
            live: None,
            tick: 0,
            show_memo: false,
            show_help: false,
        }
    }

    pub fn with_focus(focused: Option<u8>) -> Self {
        Self {
            focused,
            live: None,
            tick: 0,
            show_memo: false,
            show_help: false,
        }
    }

    pub fn with_live<'a>(focused: Option<u8>, live: OfficeLive<'a>) -> OfficeTab<'a> {
        OfficeTab {
            focused,
            live: Some(live),
            tick: 0,
            show_memo: false,
            show_help: false,
        }
    }

    pub fn with_tick(mut self, tick: u64) -> Self {
        self.tick = tick;
        self
    }

    pub fn with_memo(mut self, show_memo: bool) -> Self {
        self.show_memo = show_memo;
        self
    }

    pub fn with_help(mut self, show_help: bool) -> Self {
        self.show_help = show_help;
        self
    }
}

impl Default for OfficeTab<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for OfficeTab<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 80 || area.height < 12 {
            return;
        }

        Clear.render(area, buf);
        fill_rect(area, theme::BG, buf);

        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(STATUS_H)])
            .split(area);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(OFFICE_W as u16),
                Constraint::Length(SIDEBAR_W),
            ])
            .split(vertical[0]);

        let mut agents = collect_agent_views(self.live.as_ref());
        apply_motion(&mut agents, self.tick);
        render_scene(chunks[0], buf, &agents);
        render_office_sidebar(chunks[1], buf, self.focused, &agents, self.live.as_ref());
        render_status_bar(vertical[1], buf, &agents, self.tick);
        if self.show_memo {
            render_memo_overlay(area, buf);
        }
        if self.show_help {
            render_help_overlay(area, buf);
        }
    }
}

fn build_office() -> PixelGrid {
    let mut g = [[PLANK1; OFFICE_W]; OFFICE_H];

    // Floor planks
    for y in 0..OFFICE_H {
        for x in 0..OFFICE_W {
            let pi = ((x + (y % 2) * 3) / 6) % 4;
            g[y][x] = [PLANK1, PLANK2, PLANK3, PLANK4][pi];
            if (x + (y % 2) * 3) % 6 == 0 {
                g[y][x] = PLANK_LINE;
            }
            if (x * 7 + y * 13) % 97 == 0 {
                g[y][x] = PLANK_HI;
            }
        }
    }

    // Walls
    for y in 0..6 {
        for x in 0..OFFICE_W {
            g[y][x] = if y < 2 {
                WALL1
            } else if y < 4 {
                if (x + y) % 8 < 4 {
                    WALLPAPER1
                } else {
                    WALLPAPER2
                }
            } else if y == 4 {
                WALL3
            } else {
                WALL_TRIM
            };
        }
    }

    // Carpet (break room)
    for y in 30..44 {
        for x in 56..90 {
            let edge = x == 56 || x == 89 || y == 30 || y == 43;
            g[y][x] = if edge {
                CARPET3
            } else if (x + y) % 2 == 0 {
                CARPET1
            } else {
                CARPET2
            };
        }
    }

    // Tile (kitchen)
    for y in 30..44 {
        for x in 2..22 {
            g[y][x] = if (x + y) % 2 == 0 { TILE1 } else { TILE2 };
        }
    }

    draw_engineering(&mut g);
    draw_whiteboard(&mut g);
    draw_comms(&mut g);
    draw_research(&mut g);
    draw_break_room(&mut g);
    draw_kitchen(&mut g);
    draw_decor(&mut g);
    draw_dividers(&mut g);

    g
}

fn draw_engineering(g: &mut PixelGrid) {
    for d in 0..3 {
        let dx = 4 + d * 14;
        rect(g, dx, 12, dx + 10, 12, DESK_TOP);
        rect(g, dx, 13, dx + 10, 13, DESK_FRONT);
        rect(g, dx, 14, dx + 10, 14, DESK_SIDE);
        px(g, dx, 15, DESK_LEG);
        px(g, dx + 10, 15, DESK_LEG);

        rect(g, dx + 2, 8, dx + 8, 8, CRT_FRAME);
        px(g, dx + 1, 9, CRT_FRAME);
        rect(g, dx + 2, 9, dx + 8, 9, CRT_SCREEN);
        px(g, dx + 9, 9, CRT_FRAME);
        px(g, dx + 1, 10, CRT_FRAME);
        rect(g, dx + 2, 10, dx + 8, 10, CRT_SCREEN);
        px(g, dx + 9, 10, CRT_FRAME);
        px(g, dx + 1, 11, CRT_FRAME);
        rect(g, dx + 2, 11, dx + 8, 11, CRT_SCREEN);
        px(g, dx + 9, 11, CRT_FRAME);
        rect(g, dx + 2, 12, dx + 8, 12, CRT_FRAME);

        px(g, dx + 3, 9, CRT_TEXT1);
        px(g, dx + 5, 9, CRT_TEXT2);
        px(g, dx + 7, 9, CRT_TEXT1);
        px(g, dx + 3, 10, CRT_TEXT2);
        px(g, dx + 4, 10, CRT_TEXT3);
        px(g, dx + 6, 10, CRT_TEXT1);
        px(g, dx + 3, 11, CRT_TEXT1);
        px(g, dx + 5, 11, CRT_TEXT3);
        px(g, dx + 8, 11, LED1);

        rect(g, dx + 4, 18, dx + 6, 18, CHAIR_SEAT);
        rect(g, dx + 4, 19, dx + 6, 19, CHAIR_BACK);
        px(g, dx + 3, 20, CHAIR_WHEEL);
        px(g, dx + 7, 20, CHAIR_WHEEL);
        px(g, dx + 1, 12, PAPER);
        px(g, dx + 9, 12, MUG1);
    }
}

fn draw_whiteboard(g: &mut PixelGrid) {
    rect(g, 10, 2, 30, 2, WB_BORDER);
    rect(g, 10, 3, 30, 4, WHITEBOARD);
    rect(g, 10, 5, 30, 5, WB_BORDER);
    for x in (12..29).step_by(3) {
        px(g, x, 3, WB_TEXT);
        px(g, x + 1, 3, WB_TEXT);
    }
    px(g, 28, 4, theme::ACCENT);
    px(g, 29, 4, theme::BLUE);
}

fn draw_comms(g: &mut PixelGrid) {
    for d in 0..2 {
        let dx = 54 + d * 18;
        rect(g, dx, 12, dx + 14, 12, DESK_TOP);
        rect(g, dx, 13, dx + 14, 13, DESK_FRONT);
        rect(g, dx, 14, dx + 14, 14, DESK_SIDE);
        for m in 0..2 {
            let mx = dx + 1 + m * 7;
            rect(g, mx, 8, mx + 5, 8, CRT_FRAME);
            px(g, mx - 1, 9, CRT_FRAME);
            rect(g, mx, 9, mx + 5, 9, CRT2);
            px(g, mx + 6, 9, CRT_FRAME);
            px(g, mx - 1, 10, CRT_FRAME);
            rect(g, mx, 10, mx + 5, 10, CRT2);
            px(g, mx + 6, 10, CRT_FRAME);
            px(g, mx - 1, 11, CRT_FRAME);
            rect(g, mx, 11, mx + 5, 11, CRT2);
            px(g, mx + 6, 11, CRT_FRAME);
            rect(g, mx, 12, mx + 5, 12, CRT_FRAME);
            px(g, mx + 1, 9, theme::BLUE);
            px(g, mx + 3, 10, theme::CYAN);
            px(g, mx + 2, 11, theme::BLUE);
            px(g, mx + 5, 11, LED1);
        }
        rect(g, dx + 5, 18, dx + 9, 18, CHAIR_SEAT);
        rect(g, dx + 5, 19, dx + 9, 19, CHAIR_BACK);
    }
    rect(g, 60, 2, 64, 4, FRAME1);
    rect(g, 61, 3, 63, 3, POSTER2);
    rect(g, 70, 2, 74, 4, FRAME2);
    rect(g, 71, 3, 73, 3, POSTER1);
}

fn draw_research(g: &mut PixelGrid) {
    for y in 24..38 {
        rect(g, 24, y, 28, y, SHELF1);
        if y % 3 == 0 {
            rect(g, 24, y, 28, y, SHELF2);
        }
    }
    let books = [BOOK1, BOOK2, BOOK3, BOOK4, BOOK5, BOOK6, BOOK7];
    for y in 25..37 {
        if y % 3 != 0 {
            for x in 25..28 {
                px(g, x, y, books[(x * 3 + y * 7) % books.len()]);
            }
        }
    }

    rect(g, 32, 30, 46, 30, DESK_TOP);
    rect(g, 32, 31, 46, 31, DESK_FRONT);
    rect(g, 46, 30, 50, 30, DESK_TOP);
    rect(g, 46, 31, 50, 31, DESK_FRONT);
    rect(g, 35, 27, 41, 27, CRT_FRAME);
    rect(g, 34, 28, 42, 28, CRT_FRAME);
    rect(g, 35, 28, 41, 28, CRT_SCREEN);
    rect(g, 34, 29, 42, 29, CRT_FRAME);
    rect(g, 35, 29, 41, 29, CRT_SCREEN);
    rect(g, 35, 30, 41, 30, CRT_FRAME);
    px(g, 36, 28, CRT_TEXT3);
    px(g, 38, 28, CRT_TEXT1);
    px(g, 36, 29, CRT_TEXT2);
    px(g, 39, 29, CRT_TEXT1);
    px(g, 43, 30, PAPER);
    px(g, 47, 30, PAPER);
    rect(g, 37, 34, 39, 34, CHAIR_SEAT);
    rect(g, 37, 35, 39, 35, CHAIR_BACK);
}

fn draw_break_room(g: &mut PixelGrid) {
    rect(g, 62, 32, 82, 32, SOFA3);
    rect(g, 62, 33, 82, 33, SOFA1);
    rect(g, 62, 34, 82, 34, SOFA2);
    rect(g, 62, 35, 66, 35, SOFA2);
    rect(g, 78, 35, 82, 35, SOFA2);
    for x in [65, 70, 75, 80] {
        px(g, x, 33, SOFA_PILLOW);
    }
    rect(g, 68, 37, 76, 37, SHELF3);
    rect(g, 68, 38, 76, 38, DESK_FRONT);
    px(g, 70, 37, MUG1);
    px(g, 71, 37, COFFEE);
    px(g, 74, 37, MUG2);
}

fn draw_kitchen(g: &mut PixelGrid) {
    rect(g, 4, 32, 8, 32, MACHINE1);
    rect(g, 4, 33, 8, 33, MACHINE2);
    rect(g, 4, 34, 8, 34, MACHINE1);
    px(g, 6, 33, MACHINE_BTN);
    px(g, 7, 33, LED1);
    px(g, 5, 32, MUG1);
    rect(g, 12, 31, 15, 31, COOLER1);
    rect(g, 12, 32, 15, 32, COOLER2);
    rect(g, 12, 33, 15, 33, COOLER1);
    px(g, 13, 31, WATER_BLUE);
    px(g, 14, 31, WATER_BLUE);
    rect(g, 2, 36, 20, 36, DESK_TOP);
    rect(g, 2, 37, 20, 37, DESK_FRONT);
}

fn draw_decor(g: &mut PixelGrid) {
    for (x, y) in [(2, 9), (42, 9), (50, 9), (88, 9), (22, 28), (52, 40)] {
        plant(g, x, y);
    }
    for (x, y) in [(3, 8), (40, 8), (88, 8)] {
        lamp(g, x, y);
    }
    for lx in [10, 26, 42, 58, 74] {
        for x in lx..lx + 8 {
            px(g, x, 0, LAMP_GLOW);
            px(g, x, 1, Color::Rgb(0xe0, 0xd8, 0xc0));
        }
    }
    rect(g, 44, 2, 48, 2, FRAME1);
    rect(g, 44, 3, 48, 4, CLOCK);
    px(g, 46, 3, CLOCK_HAND);
    px(g, 47, 3, CLOCK_HAND);
    px(g, 46, 4, CLOCK_HAND);
}

fn draw_dividers(g: &mut PixelGrid) {
    for y in 6..OFFICE_H {
        if y % 2 == 0 {
            px(g, 48, y, WALL_TRIM);
            px(g, 49, y, WALL_TRIM);
        }
    }
    for x in 0..OFFICE_W {
        if x % 2 == 0 {
            px(g, x, 22, WALL_TRIM);
            px(g, x, 23, WALL_TRIM);
        }
    }
}

fn plant(g: &mut PixelGrid, x: usize, y: usize) {
    px(g, x, y - 3, LEAF2);
    px(g, x + 1, y - 3, LEAF1);
    px(g, x - 1, y - 2, LEAF3);
    px(g, x, y - 2, LEAF1);
    px(g, x + 1, y - 2, LEAF4);
    px(g, x + 2, y - 2, LEAF2);
    px(g, x - 1, y - 1, LEAF4);
    px(g, x, y - 1, LEAF2);
    px(g, x + 1, y - 1, LEAF1);
    px(g, x + 2, y - 1, LEAF3);
    px(g, x, y, POT1);
    px(g, x + 1, y, POT1);
    px(g, x, y + 1, POT2);
    px(g, x + 1, y + 1, POT2);
}

fn lamp(g: &mut PixelGrid, x: usize, y: usize) {
    px(g, x, y - 1, LAMP_SHADE);
    px(g, x + 1, y - 1, LAMP_SHADE);
    px(g, x, y, LAMP_GLOW);
    px(g, x + 1, y, LAMP_GLOW);
    px(g, x, y + 1, LAMP_POST);
    for dy in 0..3 {
        for dx in -1..3 {
            let gx = x as isize + dx;
            let gy = y + 2 + dy;
            if gx >= 0 {
                px(g, gx as usize, gy, WARM_GLOW1);
            }
        }
    }
}

fn px(g: &mut PixelGrid, x: usize, y: usize, c: Color) {
    if x < OFFICE_W && y < OFFICE_H {
        g[y][x] = c;
    }
}

fn rect(g: &mut PixelGrid, x1: usize, y1: usize, x2: usize, y2: usize, c: Color) {
    for y in y1..=y2 {
        for x in x1..=x2 {
            px(g, x, y, c);
        }
    }
}

fn render_scene(area: Rect, buf: &mut Buffer, agents: &[AgentView]) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    fill_rect(area, theme::BG, buf);
    let mut grid = build_office();
    for agent in agents {
        draw_agent_sprite(&mut grid, agent);
    }

    let scene_w = area.width.min(OFFICE_W as u16);
    let scene_rows = area.height.min(OFFICE_ROWS);

    for row in 0..scene_rows {
        let top = (row as usize) * 2;
        let bottom = top + 1;
        for col in 0..scene_w {
            let x = area.x + col;
            let y = area.y + row;
            if let Some(cell) = cell_mut_clamped(buf, x, y) {
                cell.set_symbol("▀").set_style(
                    Style::default()
                        .fg(grid[top][col as usize])
                        .bg(grid[bottom][col as usize]),
                );
            }
        }
    }

    // Low-opacity prototype zone labels are represented as muted overlay text in
    // ratatui, above the pixel-grid furniture and below speech bubbles.
    overlay_label(area, 1, 7, "ENGINEERING", theme::ACCENT, buf);
    overlay_label(area, 58, 7, "COMMS", theme::BLUE, buf);
    overlay_label(area, 26, 13, "RESEARCH", theme::CYAN, buf);
    overlay_label(area, 60, 16, "BREAK ROOM", theme::YELLOW, buf);
    overlay_label(area, 1, 17, "KITCHEN", theme::DIM, buf);
    render_speech_bubbles(area, agents, buf);
}

fn overlay_label(area: Rect, x: u16, y: u16, label: &str, color: Color, buf: &mut Buffer) {
    if x >= area.width || y >= area.height {
        return;
    }
    buf.set_string_clamped(
        area.x + x,
        area.y + y,
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    );
}

fn render_status_bar(area: Rect, buf: &mut Buffer, agents: &[AgentView], tick: u64) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    fill_rect(area, theme::BG_PANEL, buf);
    let active = agents
        .iter()
        .filter(|a| a.state != AgentState::Idle)
        .count();
    let text = format!(
        "● the-office │ {} agents │ {} active │ tick {} │ 8 TPS │ M Memo │ Tab Focus │ ? Help",
        agents.len(),
        active,
        tick
    );
    put(
        area,
        1,
        area.y,
        &truncate(&text, area.width as usize - 1),
        theme::DIM,
        buf,
    );
}

fn render_memo_overlay(area: Rect, buf: &mut Buffer) {
    let width = area.width.saturating_sub(8).clamp(44, 72);
    let height = area.height.saturating_sub(4).clamp(8, 14);
    let overlay = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + 2.min(area.height.saturating_sub(1)),
        width,
        height,
    };
    Clear.render(overlay, buf);
    fill_rect(overlay, theme::BG_PANEL, buf);
    draw_panel_border(overlay, theme::MUTED, buf);
    put(
        overlay,
        2,
        overlay.y + 1,
        "YESTERDAY'S MEMO",
        theme::ACCENT,
        buf,
    );
    put(
        overlay,
        overlay.width.saturating_sub(22),
        overlay.y + 1,
        "Yesterday, March 27",
        theme::DIM,
        buf,
    );
    let lines = [
        ("Zeus Prime:", "Deployed NovaTradeEngine v0.4.2 to staging."),
        ("", "  Binary 12.2MB, 24/24 tests passed."),
        ("Hermes:", "Sent 147 messages across Telegram + Discord."),
        ("", "  Release notes -> #ops-alerts channel."),
        ("Athena:", "Completed LLM benchmark (8 models tested)."),
        ("Prometheus:", "Ran 8 heartbeat checks. All green."),
    ];
    for (idx, (who, msg)) in lines.iter().enumerate() {
        let y = overlay.y + 3 + idx as u16;
        if y >= overlay.bottom().saturating_sub(2) {
            break;
        }
        if who.is_empty() {
            put(overlay, 2, y, msg, theme::DIM, buf);
        } else {
            put(overlay, 2, y, who, theme::ACCENT_DIM, buf);
            put(overlay, 15, y, msg, theme::TEXT, buf);
        }
    }
    if overlay.height > 3 {
        put(
            overlay,
            2,
            overlay.bottom().saturating_sub(2),
            "Generated by Mnemosyne at 00:00 UTC",
            theme::MUTED,
            buf,
        );
    }
}

fn render_help_overlay(area: Rect, buf: &mut Buffer) {
    let width = area.width.saturating_sub(8).clamp(24, 34);
    let height = area.height.saturating_sub(4).clamp(6, 8);
    let overlay = Rect {
        x: area.right().saturating_sub(width + 3),
        y: area.y + 2.min(area.height.saturating_sub(1)),
        width,
        height,
    };
    Clear.render(overlay, buf);
    fill_rect(overlay, theme::BG_PANEL, buf);
    draw_panel_border(overlay, theme::MUTED, buf);
    put(
        overlay,
        2,
        overlay.y + 1,
        "CONTROLS",
        theme::ACCENT_DIM,
        buf,
    );
    put(
        overlay,
        2,
        overlay.y + 2,
        "M    Yesterday's memo",
        theme::DIM,
        buf,
    );
    put(
        overlay,
        2,
        overlay.y + 3,
        "Tab  Cycle agent focus",
        theme::DIM,
        buf,
    );
    put(
        overlay,
        2,
        overlay.y + 4,
        "?    Toggle help",
        theme::DIM,
        buf,
    );
    put(
        overlay,
        2,
        overlay.y + 5,
        "Esc  Close / unfocus",
        theme::DIM,
        buf,
    );
}

fn draw_panel_border(area: Rect, color: Color, buf: &mut Buffer) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let right = area.right().saturating_sub(1);
    let bottom = area.bottom().saturating_sub(1);
    for x in area.x..=right {
        let sym_top = if x == area.x {
            "┌"
        } else if x == right {
            "┐"
        } else {
            "─"
        };
        let sym_bottom = if x == area.x {
            "└"
        } else if x == right {
            "┘"
        } else {
            "─"
        };
        if let Some(cell) = cell_mut_clamped(buf, x, area.y) {
            cell.set_symbol(sym_top)
                .set_style(Style::default().fg(color).bg(theme::BG_PANEL));
        }
        if let Some(cell) = cell_mut_clamped(buf, x, bottom) {
            cell.set_symbol(sym_bottom)
                .set_style(Style::default().fg(color).bg(theme::BG_PANEL));
        }
    }
    for y in area.y + 1..bottom {
        if let Some(cell) = cell_mut_clamped(buf, area.x, y) {
            cell.set_symbol("│")
                .set_style(Style::default().fg(color).bg(theme::BG_PANEL));
        }
        if let Some(cell) = cell_mut_clamped(buf, right, y) {
            cell.set_symbol("│")
                .set_style(Style::default().fg(color).bg(theme::BG_PANEL));
        }
    }
}

fn render_office_sidebar(
    area: Rect,
    buf: &mut Buffer,
    focused: Option<u8>,
    agents: &[AgentView],
    live: Option<&OfficeLive<'_>>,
) {
    fill_rect(area, theme::BG_PANEL, buf);
    if area.width == 0 || area.height == 0 {
        return;
    }

    for y in area.y..area.bottom().min(buf.area.bottom()) {
        if let Some(cell) = cell_mut_clamped(buf, area.x, y) {
            cell.set_symbol("│")
                .set_style(Style::default().fg(theme::MUTED).bg(theme::BG_PANEL));
        }
    }

    let mut y = area.y + 1;
    put(area, 2, y, "FLEET STATUS", theme::ACCENT_DIM, buf);
    y += 1;

    if agents.is_empty() {
        put(area, 2, y + 1, "waiting for", theme::DIM, buf);
        put(area, 2, y + 2, "/v1/network/agents", theme::MUTED, buf);
        y += 4;
    } else {
        for agent in agents.iter().take(5) {
            y += 1;
            let selected = focused == Some(agent.slot as u8);
            let name_style = if selected {
                theme::TEXT_BRIGHT
            } else {
                theme::TEXT
            };
            put(area, 2, y, "●", state_color(agent.state), buf);
            put(
                area,
                4,
                y,
                &format!("{} {}", truncate(&agent.name, 11), state_label(agent.state)),
                name_style,
                buf,
            );
            y += 1;
            put(area, 4, y, &truncate(&agent.task, 20), theme::DIM, buf);
            y += 1;
            put(area, 4, y, &truncate(&agent.model, 20), theme::MUTED, buf);
        }
        y += 1;
    }

    put(area, 2, y, "EVENT LOG", theme::ACCENT_DIM, buf);
    y += 1;
    for agent in agents.iter().take(4) {
        y += 1;
        put(area, 2, y, "→", state_color(agent.state), buf);
        put(
            area,
            4,
            y,
            &truncate(&format!("{}: {}", short_name(&agent.name), agent.task), 21),
            theme::DIM,
            buf,
        );
    }
    if agents.is_empty() {
        y += 1;
        put(area, 2, y, "no live events yet", theme::MUTED, buf);
    }

    y += 2;
    put(area, 2, y, "ZONES", theme::ACCENT_DIM, buf);
    for zone in [
        OfficeZone::Engineering,
        OfficeZone::Comms,
        OfficeZone::Research,
        OfficeZone::Breakroom,
        OfficeZone::Kitchen,
    ] {
        y += 1;
        put(area, 2, y, "●", zone_color(zone), buf);
        put(area, 4, y, zone_label(zone), theme::DIM, buf);
        put(
            area,
            area.width.saturating_sub(4),
            y,
            &zone_count(agents, zone).to_string(),
            zone_color(zone),
            buf,
        );
    }

    if let Some(status) = live.and_then(|l| l.status) {
        let footer_y = area.bottom().saturating_sub(2);
        put(
            area,
            2,
            footer_y,
            &truncate(&format!("{} {}", status.status, status.gateway_url), 23),
            theme::MUTED,
            buf,
        );
    }
}

fn apply_motion(agents: &mut [AgentView], tick: u64) {
    if tick == 0 {
        return;
    }
    for agent in agents {
        let (from_x, from_y) = zone_position(previous_zone(agent.zone), agent.slot);
        let (to_x, to_y) = zone_position(agent.zone, agent.slot);
        let phase = (tick % OFFICE_MOTION_LEG_TICKS) as isize;
        let denom = (OFFICE_MOTION_LEG_TICKS - 1) as isize;
        agent.x = lerp_coord(from_x, to_x, phase, denom).clamp(1, OFFICE_W - 11);
        agent.y = lerp_coord(from_y, to_y, phase, denom).clamp(15, OFFICE_H - 2);
    }
}

fn lerp_coord(from: usize, to: usize, phase: isize, denom: isize) -> usize {
    let from = from as isize;
    let to = to as isize;
    (from + ((to - from) * phase) / denom).max(0) as usize
}

fn previous_zone(zone: OfficeZone) -> OfficeZone {
    match zone {
        OfficeZone::Engineering => OfficeZone::Kitchen,
        OfficeZone::Comms => OfficeZone::Engineering,
        OfficeZone::Research => OfficeZone::Comms,
        OfficeZone::Breakroom => OfficeZone::Research,
        OfficeZone::Kitchen => OfficeZone::Breakroom,
    }
}

fn collect_agent_views(live: Option<&OfficeLive<'_>>) -> Vec<AgentView> {
    let Some(live) = live else {
        return Vec::new();
    };

    if let Some(agents) = live.agents {
        if !agents.is_empty() {
            return agents
                .iter()
                .take(8)
                .enumerate()
                .map(|(slot, agent)| agent_view_from_response(slot, agent, live.status))
                .collect();
        }
    }

    // Real local gateway status is still live data; use it only when the network
    // roster has not returned yet, rather than restoring the old mock const.
    live.status
        .map(|status| {
            let state = classify_state(&status.status, None);
            let zone = zone_for_state(state);
            let (x, y) = zone_position(zone, 0);
            vec![AgentView {
                name: if status.agent_name.is_empty() {
                    "local gateway".into()
                } else {
                    status.agent_name.clone()
                },
                state,
                zone,
                task: if status.status.is_empty() {
                    "gateway status".into()
                } else {
                    status.status.clone()
                },
                model: model_or_provider(status.model.as_str(), status.provider.as_str()),
                slot: 0,
                x,
                y,
            }]
        })
        .unwrap_or_default()
}

fn agent_view_from_response(
    slot: usize,
    agent: &crate::api::AgentResponse,
    status: Option<&crate::api::StatusResponse>,
) -> AgentView {
    let task = agent
        .current_task
        .as_deref()
        .filter(|task| !task.trim().is_empty())
        .unwrap_or_else(|| fallback_task(&agent.status));
    let state = classify_state(&agent.status, Some(task));
    let zone = zone_for_state(state);
    let (x, y) = zone_position(zone, slot);
    let model = agent
        .metadata
        .get("model")
        .or_else(|| agent.metadata.get("llm_model"))
        .or_else(|| agent.metadata.get("provider_model"))
        .cloned()
        .unwrap_or_else(|| {
            status
                .map(|s| model_or_provider(s.model.as_str(), s.provider.as_str()))
                .unwrap_or_else(|| "model unknown".into())
        });

    AgentView {
        name: office_agent_display_name(agent),
        state,
        zone,
        task: task.to_string(),
        model,
        slot,
        x,
        y,
    }
}

fn classify_state(status: &str, task: Option<&str>) -> AgentState {
    let hay = format!("{} {}", status, task.unwrap_or_default()).to_ascii_lowercase();
    if hay.contains("error") || hay.contains("fail") || hay.contains("panic") {
        AgentState::Error
    } else if hay.contains("sync")
        || hay.contains("discord")
        || hay.contains("telegram")
        || hay.contains("slack")
    {
        AgentState::Syncing
    } else if hay.contains("research")
        || hay.contains("benchmark")
        || hay.contains("paper")
        || hay.contains("vector")
    {
        AgentState::Researching
    } else if hay.contains("writing")
        || hay.contains("draft")
        || hay.contains("doc")
        || hay.contains("compose")
    {
        AgentState::Writing
    } else if hay.contains("exec")
        || hay.contains("build")
        || hay.contains("test")
        || hay.contains("deploy")
        || hay.contains("running")
        || hay.contains("active")
        || hay.contains("busy")
    {
        AgentState::Executing
    } else {
        AgentState::Idle
    }
}

fn fallback_task(status: &str) -> &str {
    if status.trim().is_empty() {
        "idle"
    } else {
        status
    }
}

fn model_or_provider(model: &str, provider: &str) -> String {
    if !model.trim().is_empty() {
        model.to_string()
    } else if !provider.trim().is_empty() {
        provider.to_string()
    } else {
        "model unknown".into()
    }
}

fn zone_for_state(state: AgentState) -> OfficeZone {
    match state {
        AgentState::Idle => OfficeZone::Breakroom,
        AgentState::Writing | AgentState::Executing => OfficeZone::Engineering,
        AgentState::Researching | AgentState::Error => OfficeZone::Research,
        AgentState::Syncing => OfficeZone::Comms,
    }
}

fn zone_position(zone: OfficeZone, slot: usize) -> (usize, usize) {
    let (base_x, base_y) = match zone {
        OfficeZone::Engineering => (18, 20),
        OfficeZone::Comms => (64, 20),
        OfficeZone::Research => (38, 36),
        OfficeZone::Breakroom => (72, 37),
        OfficeZone::Kitchen => (10, 38),
    };
    let offsets = [
        (0isize, 0isize),
        (10, 1),
        (-8, 2),
        (5, -2),
        (-12, -1),
        (13, 2),
    ];
    let (dx, dy) = offsets[slot % offsets.len()];
    (
        (base_x as isize + dx).clamp(1, (OFFICE_W - 11) as isize) as usize,
        (base_y as isize + dy).clamp(15, (OFFICE_H - 2) as isize) as usize,
    )
}

fn state_color(state: AgentState) -> Color {
    match state {
        AgentState::Idle => theme::DIM,
        AgentState::Writing => theme::ACCENT,
        AgentState::Executing => theme::GREEN,
        AgentState::Researching => theme::CYAN,
        AgentState::Syncing => theme::BLUE,
        AgentState::Error => theme::RED,
    }
}

fn state_label(state: AgentState) -> &'static str {
    match state {
        AgentState::Idle => "IDLE",
        AgentState::Writing => "WRITING",
        AgentState::Executing => "EXEC",
        AgentState::Researching => "RESEARCH",
        AgentState::Syncing => "SYNC",
        AgentState::Error => "ERROR",
    }
}

fn zone_label(zone: OfficeZone) -> &'static str {
    match zone {
        OfficeZone::Engineering => "engineering",
        OfficeZone::Comms => "comms",
        OfficeZone::Research => "research",
        OfficeZone::Breakroom => "breakroom",
        OfficeZone::Kitchen => "kitchen",
    }
}

fn zone_color(zone: OfficeZone) -> Color {
    match zone {
        OfficeZone::Engineering => theme::ACCENT,
        OfficeZone::Comms => theme::BLUE,
        OfficeZone::Research => theme::CYAN,
        OfficeZone::Breakroom => theme::YELLOW,
        OfficeZone::Kitchen => theme::DIM,
    }
}

fn zone_count(agents: &[AgentView], zone: OfficeZone) -> usize {
    agents.iter().filter(|agent| agent.zone == zone).count()
}

fn draw_agent_sprite(g: &mut PixelGrid, agent: &AgentView) {
    let sx = agent.x;
    let sy = agent.y.saturating_sub(14);
    let shirt = state_color(agent.state);
    let skin = Color::Rgb(0xe0, 0xc0, 0xa0);
    let hair = match agent.slot % 4 {
        0 => Color::Rgb(0x2a, 0x12, 0x08),
        1 => Color::Rgb(0x4a, 0x38, 0x28),
        2 => Color::Rgb(0x6a, 0x4a, 0x2a),
        _ => Color::Rgb(0x3a, 0x28, 0x18),
    };
    let eye = Color::Rgb(0x1a, 0x1a, 0x2e);
    let belt = Color::Rgb(0x3a, 0x2a, 0x1a);
    let pants = Color::Rgb(0x1a, 0x1a, 0x2e);
    let shoe = Color::Rgb(0x1a, 0x12, 0x08);
    let badge = state_color(agent.state);

    let rows: [&[(usize, Color)]; 14] = [
        &[(3, hair), (4, hair), (5, hair), (6, hair)],
        &[
            (2, hair),
            (3, hair),
            (4, hair),
            (5, hair),
            (6, hair),
            (7, hair),
        ],
        &[
            (1, hair),
            (2, hair),
            (3, hair),
            (4, hair),
            (5, hair),
            (6, hair),
            (7, hair),
            (8, hair),
        ],
        &[
            (1, hair),
            (2, skin),
            (3, skin),
            (4, skin),
            (5, skin),
            (6, skin),
            (7, skin),
            (8, hair),
        ],
        &[
            (2, skin),
            (3, eye),
            (4, skin),
            (5, skin),
            (6, eye),
            (7, skin),
        ],
        &[
            (2, skin),
            (3, skin),
            (4, skin),
            (5, skin),
            (6, skin),
            (7, skin),
        ],
        &[(3, skin), (4, skin), (5, skin), (6, skin)],
        &[
            (2, shirt),
            (3, shirt),
            (4, badge),
            (5, shirt),
            (6, shirt),
            (7, shirt),
        ],
        &[
            (1, skin),
            (2, shirt),
            (3, shirt),
            (4, shirt),
            (5, shirt),
            (6, shirt),
            (7, shirt),
            (8, skin),
        ],
        &[
            (2, shirt),
            (3, shirt),
            (4, shirt),
            (5, shirt),
            (6, shirt),
            (7, shirt),
        ],
        &[
            (2, belt),
            (3, belt),
            (4, belt),
            (5, belt),
            (6, belt),
            (7, belt),
        ],
        &[(2, pants), (3, pants), (6, pants), (7, pants)],
        &[(2, pants), (3, pants), (6, pants), (7, pants)],
        &[(2, shoe), (3, shoe), (6, shoe), (7, shoe)],
    ];

    for (row, pixels) in rows.iter().enumerate() {
        for (col, color) in *pixels {
            px(g, sx + *col, sy + row, *color);
        }
    }
}

fn render_speech_bubbles(area: Rect, agents: &[AgentView], buf: &mut Buffer) {
    for agent in agents.iter().take(5) {
        let x = agent.x.saturating_sub(4).min(OFFICE_W - 28) as u16;
        let y = ((agent.y / 2) as u16).saturating_sub(8);
        if y >= area.height {
            continue;
        }
        let text = format!(
            "{} · {}",
            short_name(&agent.name),
            truncate(&agent.task, 16)
        );
        crate::prod::draw::set_str(
            area.x + x,
            area.y + y,
            &truncate(&text, 28),
            Style::default()
                .fg(state_color(agent.state))
                .bg(theme::BG_PANEL),
            area.right().saturating_sub(1),
            buf,
        );
    }
}

fn office_agent_display_name(agent: &crate::api::AgentResponse) -> String {
    let raw = if agent.name.trim().is_empty() {
        agent.id.trim()
    } else {
        agent.name.trim()
    };
    let raw = if raw.is_empty() { "agent" } else { raw };
    collapse_doubled_zeus_prefix(raw)
}

fn collapse_doubled_zeus_prefix(name: &str) -> String {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("zeuszeus-") {
        format!("zeus-{}", &trimmed[9..])
    } else if lower.starts_with("zeuszeus_") {
        format!("zeus_{}", &trimmed[9..])
    } else if lower.starts_with("zeus zeus ") {
        format!("zeus {}", &trimmed[10..])
    } else {
        trimmed.to_string()
    }
}

fn short_name(name: &str) -> String {
    name.split_whitespace().next().unwrap_or(name).to_string()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn put(area: Rect, x: u16, y: u16, text: &str, color: Color, buf: &mut Buffer) {
    if x >= area.width || y >= area.bottom() {
        return;
    }
    let max_x = area.right().saturating_sub(1);
    crate::prod::draw::set_str(
        area.x + x,
        y,
        text,
        Style::default().fg(color).bg(theme::BG_PANEL),
        max_x,
        buf,
    );
}

fn fill_rect(area: Rect, color: Color, buf: &mut Buffer) {
    let right = area.right().min(buf.area.right());
    let bottom = area.bottom().min(buf.area.bottom());
    for y in area.y..bottom {
        for x in area.x..right {
            if let Some(cell) = cell_mut_clamped(buf, x, y) {
                cell.set_symbol(" ").set_style(Style::default().bg(color));
            }
        }
    }
}

/// Cycle Office agent focus slots for Tab/focus keyboard handling.
pub fn cycle_focused(current: Option<u8>) -> Option<u8> {
    match current {
        None => Some(0),
        Some(i) if i + 1 < SLOT_COUNT => Some(i + 1),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_grid_keeps_prototype_dimensions() {
        let grid = build_office();
        assert_eq!(grid.len(), OFFICE_H);
        assert_eq!(grid[0].len(), OFFICE_W);
    }

    #[test]
    fn p1_scene_contains_named_zone_materials() {
        let grid = build_office();
        assert_eq!(grid[0][10], LAMP_GLOW, "ceiling lights from prototype");
        assert_eq!(grid[4][12], WHITEBOARD, "engineering whiteboard");
        assert_eq!(grid[9][6], CRT_SCREEN, "engineering CRT screen");
        assert_eq!(grid[10][57], CRT2, "comms dual monitor");
        assert_eq!(grid[25][25], BOOK6, "research bookshelf");
        assert_eq!(grid[30][56], CARPET3, "break-room carpet edge");
        assert_eq!(grid[32][4], MACHINE1, "kitchen coffee machine");
        assert_eq!(grid[40][52], POT1, "lower plant pot");
    }

    #[test]
    fn cycle_focus_keeps_stable_future_slots() {
        assert_eq!(cycle_focused(None), Some(0));
        assert_eq!(cycle_focused(Some(0)), Some(1));
        assert_eq!(cycle_focused(Some(3)), None);
    }
}
