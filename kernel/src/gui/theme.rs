/// GUI Theme — GNOME Shell / Adwaita palette
/// Colors in 0x00RRGGBB format.

// ── Adwaita Colors ────────────────────────────────────────────────
pub const BG: u32           = 0x242424;  // dark window bg
pub const SURFACE: u32      = 0x303030;  // card/panel bg
pub const SURFACE_HIER: u32 = 0x3D3D3D;  // elevated surface
pub const ACCENT: u32       = 0x3584E4;  // Adwaita blue
pub const ACCENT_DARK: u32  = 0x1C71D8;  // pressed blue
pub const ACCENT_HOVER: u32 = 0x4A9AF5;  // hover blue
pub const TEXT: u32          = 0xFFFFFF;
pub const TEXT_DIM: u32      = 0x929292;
pub const TEXT_TITLE: u32    = 0xBBBBBB;
pub const BORDER: u32        = 0x1B1B1B;
pub const HOVER_BG: u32      = 0x404040;
pub const PRESS_BG: u32      = 0x505050;

// ── Panel (top bar, GNOME style) ─────────────────────────────────
pub const PANEL_H: u32       = 32;
pub const PANEL_ALPHA: u8    = 200;  // semi-transparent
pub const PANEL_CORNER: u32  = 0;

// ── Window decorations (GNOME style) ─────────────────────────────
pub const TITLEBAR_H: u32    = 32;
pub const CORNER_RADIUS: u32 = 10;
pub const SHADOW_RADIUS: u32 = 14;
pub const WINDOW_MIN_W: u32  = 280;
pub const WINDOW_MIN_H: u32  = 180;
pub const BORDER_W: u32      = 1;

// GNOME buttons — on the RIGHT, close is first
pub const BTN_CLOSE: u32     = 0xE01B24;  // Adwaita red
pub const BTN_MAXIMIZE: u32  = 0x303030;
pub const BTN_MINIMIZE: u32  = 0x303030;
pub const BTN_SIZE: u32      = 24;
pub const BTN_GAP: u32       = 0;
pub const BTN_ICON: u32      = 0x929292;

// ── Dash (GNOME dock) ────────────────────────────────────────────
pub const DASH_W: u32        = 56;
pub const DASH_PADDING: u32  = 8;
pub const DASH_ICON: u32     = 48;
pub const DASH_GAP: u32      = 4;
pub const DASH_CORNER: u32   = 16;
pub const DASH_ALPHA: u8     = 210;

// ── Overview (GNOME Activities) ──────────────────────────────────
pub const OVERVIEW_SEARCH_H: u32 = 48;
pub const OVERVIEW_SEARCH_W: u32 = 480;
pub const OVERVIEW_THUMB_PAD: u32 = 16;

// ── Desktop ──────────────────────────────────────────────────────
pub const ICON_SIZE: u32     = 48;
