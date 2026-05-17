// S91 audit: reviewed by zeus107
// S92: onboarding audit complete — all 7 save paths verified
// S104 #49: Expanded with missing JSX C.* constants (selection, border, borderBright)
//! and retro RPG agent persona colors.
//! Color palette for The Office — Zeus warm dark terminal palette.
//! Ported from the JSX prototype's `C` constant object.
//! Source: docs/prd/1485749168466300990_zeus-tui-onboarding.jsx
//! Some colors are reserved for S91+ features (day/night cycle, status glow).

use ratatui::style::Color;

// Terminal palette
pub const BG: Color = Color::Rgb(10, 10, 15);
pub const FG: Color = Color::Rgb(212, 207, 200);
pub const DIM: Color = Color::Rgb(90, 86, 80);
pub const MUTED: Color = Color::Rgb(58, 54, 50);
pub const ACCENT: Color = Color::Rgb(255, 60, 20);
pub const ACCENT_DIM: Color = Color::Rgb(160, 48, 26);
pub const ACCENT_BRIGHT: Color = Color::Rgb(255, 104, 66);
pub const GREEN: Color = Color::Rgb(34, 197, 94);
pub const GREEN_DIM: Color = Color::Rgb(26, 74, 46);
pub const YELLOW: Color = Color::Rgb(234, 179, 8);
pub const YELLOW_DIM: Color = Color::Rgb(107, 90, 16);
pub const BLUE: Color = Color::Rgb(59, 130, 246);
pub const CYAN: Color = Color::Rgb(6, 182, 212);
pub const RED: Color = Color::Rgb(239, 68, 68);
pub const PURPLE: Color = Color::Rgb(168, 85, 247);
pub const WHITE: Color = Color::Rgb(240, 236, 230);
pub const WARM_BG: Color = Color::Rgb(18, 16, 14);
pub const SELECTION: Color = Color::Rgb(42, 24, 16);    // C.selection  #2a1810
pub const BORDER: Color = Color::Rgb(46, 34, 24);       // C.border     #2e2218
pub const BORDER_BRIGHT: Color = Color::Rgb(90, 56, 32); // C.borderBright #5a3820

// ── Retro RPG agent persona colors ───────────────────────────────────────────
// Each agent archetype maps to a distinct terminal color for sprite tinting
// and nametag rendering in the Office RPG view.
pub const PERSONA_ARCHITECT: Color = Color::Rgb(59, 130, 246);   // blue   — planner/builder
pub const PERSONA_ANALYST:   Color = Color::Rgb(6, 182, 212);    // cyan   — researcher
pub const PERSONA_GUARDIAN:  Color = Color::Rgb(34, 197, 94);    // green  — security/audit
pub const PERSONA_HERALD:    Color = Color::Rgb(234, 179, 8);    // yellow — comms/relay
pub const PERSONA_WRAITH:    Color = Color::Rgb(168, 85, 247);   // purple — background/silent
pub const PERSONA_VANGUARD:  Color = Color::Rgb(255, 60, 20);    // accent — lead/coordinator
pub const PERSONA_ORACLE:    Color = Color::Rgb(255, 104, 66);   // accent bright — inference
pub const PERSONA_FORGE:     Color = Color::Rgb(255, 154, 64);   // lamp/amber — builder/deploy

// Status glow colors (used for agent state halos in RPG view)
pub const STATUS_ACTIVE:   Color = Color::Rgb(34, 197, 94);   // green — working
pub const STATUS_IDLE:     Color = Color::Rgb(234, 179, 8);   // yellow — waiting
pub const STATUS_ERROR:    Color = Color::Rgb(239, 68, 68);   // red — failed
pub const STATUS_OFFLINE:  Color = Color::Rgb(90, 86, 80);    // dim — disconnected
pub const STATUS_SYNCING:  Color = Color::Rgb(59, 130, 246);  // blue — deploying/syncing

// Office furniture
pub const FLOOR1: Color = Color::Rgb(42, 32, 24);
pub const FLOOR2: Color = Color::Rgb(36, 28, 20);
pub const WALL1: Color = Color::Rgb(30, 24, 18);
pub const WALL2: Color = Color::Rgb(24, 20, 16);
pub const WOOD1: Color = Color::Rgb(74, 56, 40);
pub const WOOD2: Color = Color::Rgb(62, 46, 32);
pub const WOOD3: Color = Color::Rgb(90, 68, 48);
pub const PLANT1: Color = Color::Rgb(45, 90, 40);
pub const PLANT2: Color = Color::Rgb(30, 74, 26);
pub const PLANT3: Color = Color::Rgb(58, 104, 48);
pub const SCREEN1: Color = Color::Rgb(26, 58, 90);
pub const SCREEN2: Color = Color::Rgb(42, 90, 138);
pub const SCREEN3: Color = Color::Rgb(10, 42, 74);
pub const CHAIR1: Color = Color::Rgb(58, 42, 26);
pub const CHAIR2: Color = Color::Rgb(74, 58, 42);
pub const LAMP1: Color = Color::Rgb(255, 154, 64);
pub const LAMP2: Color = Color::Rgb(255, 208, 128);
pub const RUG1: Color = Color::Rgb(74, 32, 32);
pub const RUG2: Color = Color::Rgb(90, 40, 40);
pub const SOFA1: Color = Color::Rgb(90, 58, 42);
pub const SOFA2: Color = Color::Rgb(106, 74, 58);
pub const SOFA3: Color = Color::Rgb(74, 46, 30);
pub const COFFEE: Color = Color::Rgb(42, 26, 14);

// ── Rich furniture colors (S104 #51) ─────────────────────────────────────────
// CRT monitors
pub const CRT_FRAME:  Color = Color::Rgb(38, 38, 42);      // dark grey bezel
pub const CRT_SCREEN: Color = Color::Rgb(8, 22, 8);        // dark phosphor off
pub const CRT_TEXT1:  Color = Color::Rgb(0, 255, 80);      // bright green text
pub const CRT_TEXT2:  Color = Color::Rgb(0, 200, 60);      // mid green text
pub const CRT_TEXT3:  Color = Color::Rgb(0, 140, 40);      // dim green text
pub const CRT2:       Color = Color::Rgb(14, 38, 68);      // comms blue screen
// Chairs
pub const CHAIR_WHEEL: Color = Color::Rgb(28, 28, 30);     // dark rubber wheel
pub const CHAIR_BACK:  Color = Color::Rgb(50, 36, 22);     // chair back (slightly lighter)
// Whiteboard
pub const WB_BORDER:   Color = Color::Rgb(180, 160, 140);  // whiteboard aluminium frame
pub const WHITEBOARD:  Color = Color::Rgb(240, 238, 232);  // white board surface
pub const WB_TEXT:     Color = Color::Rgb(60, 80, 200);    // blue marker text
// Picture frames & posters
pub const FRAME1:      Color = Color::Rgb(90, 60, 30);     // warm wood frame
pub const FRAME2:      Color = Color::Rgb(40, 40, 50);     // dark metal frame
pub const POSTER1:     Color = Color::Rgb(200, 80, 40);    // warm poster fill
pub const POSTER2:     Color = Color::Rgb(40, 100, 160);   // cool poster fill
// Desk props
pub const PAPER:       Color = Color::Rgb(220, 214, 200);  // paper/document
pub const MUG1:        Color = Color::Rgb(180, 60, 40);    // red mug
pub const MUG2:        Color = Color::Rgb(40, 100, 60);    // green mug
pub const LED1:        Color = Color::Rgb(0, 255, 100);    // status LED green
// Bookshelves
pub const SHELF1:      Color = Color::Rgb(68, 50, 34);     // shelf wood light
pub const SHELF2:      Color = Color::Rgb(52, 38, 24);     // shelf wood dark (divider)
pub const SHELF3:      Color = Color::Rgb(58, 44, 30);     // break room shelf
pub const BOOK1:       Color = Color::Rgb(180, 50, 50);    // red book
pub const BOOK2:       Color = Color::Rgb(50, 120, 200);   // blue book
pub const BOOK3:       Color = Color::Rgb(200, 160, 40);   // yellow book
pub const BOOK4:       Color = Color::Rgb(60, 160, 80);    // green book
pub const BOOK5:       Color = Color::Rgb(160, 60, 180);   // purple book
pub const BOOK6:       Color = Color::Rgb(200, 100, 40);   // orange book
pub const BOOK7:       Color = Color::Rgb(60, 180, 180);   // teal book
// Sofa
pub const SOFA_PILLOW: Color = Color::Rgb(200, 160, 100);  // cream pillow

// S104 #52: Additional props palette
pub const COOLER1: Color = Color::Rgb(160, 184, 200);
pub const COOLER2: Color = Color::Rgb(138, 160, 176);
pub const WATER_BLUE: Color = Color::Rgb(96, 160, 208);
pub const MACHINE1: Color = Color::Rgb(74, 74, 74);
pub const MACHINE2: Color = Color::Rgb(58, 58, 58);
pub const MACHINE_BTN: Color = Color::Rgb(255, 60, 20);
pub const CLOCK: Color = Color::Rgb(208, 200, 184);
pub const CLOCK_HAND: Color = Color::Rgb(42, 42, 42);
pub const LAMP_GLOW: Color = Color::Rgb(58, 42, 24);
