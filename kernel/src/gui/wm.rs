/// Window Manager — GNOME-style (minimal titlebar, buttons right, centered title)
use super::{theme, render, input};

pub const MAX_WINDOWS: usize = 16;

#[derive(Clone, Copy, PartialEq)]
pub enum WinState {
    Normal,
    Maximized,
    Minimized,
    SnappedLeft,
    SnappedRight,
}

#[derive(Clone, Copy, PartialEq)]
pub enum WinKind {
    Terminal,
    FileManager,
    TextEditor,
    Settings,
    Custom,
}

pub struct Window {
    pub active: bool,
    pub kind: WinKind,
    pub title: [u8; 40],
    pub title_len: usize,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub state: WinState,
    pub z: u32,
    pub content: [u8; 8192],
    pub content_len: usize,
    pub input_buf: [u8; 512],
    pub input_len: usize,
}

const fn empty_window() -> Window {
    Window {
        active: false, kind: WinKind::Custom,
        title: [0u8; 40], title_len: 0,
        x: 0, y: 0, w: 0, h: 0,
        state: WinState::Normal, z: 0,
        content: [0u8; 8192], content_len: 0,
        input_buf: [0u8; 512], input_len: 0,
    }
}

static mut WINDOWS: [Window; MAX_WINDOWS] = [
    empty_window(), empty_window(), empty_window(), empty_window(),
    empty_window(), empty_window(), empty_window(), empty_window(),
    empty_window(), empty_window(), empty_window(), empty_window(),
    empty_window(), empty_window(), empty_window(), empty_window(),
];
static mut NEXT_Z: u32 = 1;
static mut FOCUSED: usize = 0;
static mut DRAGGING: Option<usize> = None;
static mut DRAG_OX: i32 = 0;
static mut DRAG_OY: i32 = 0;
static mut RESIZING: Option<usize> = None;
static mut RESIZE_EDGE: u8 = 0;

fn sw() -> u32 { crate::display::width() }
fn sh() -> u32 { crate::display::height() }

// ── Window creation ───────────────────────────────────────────────

pub fn create_window(kind: WinKind, title: &[u8], x: i32, y: i32, w: u32, h: u32) -> Option<usize> {
    unsafe {
        for i in 0..MAX_WINDOWS {
            if !WINDOWS[i].active {
                WINDOWS[i].active = true;
                WINDOWS[i].kind = kind;
                let tlen = title.len().min(39);
                WINDOWS[i].title[..tlen].copy_from_slice(&title[..tlen]);
                WINDOWS[i].title_len = tlen;
                WINDOWS[i].x = x;
                WINDOWS[i].y = y;
                WINDOWS[i].w = w.max(theme::WINDOW_MIN_W);
                WINDOWS[i].h = h.max(theme::WINDOW_MIN_H);
                WINDOWS[i].state = WinState::Normal;
                WINDOWS[i].z = NEXT_Z;
                NEXT_Z += 1;
                WINDOWS[i].content_len = 0;
                WINDOWS[i].input_len = 0;
                FOCUSED = i;
                return Some(i);
            }
        }
        None
    }
}

pub fn close_window(idx: usize) {
    unsafe {
        if idx < MAX_WINDOWS {
            WINDOWS[idx].active = false;
            for i in (0..MAX_WINDOWS).rev() {
                if WINDOWS[i].active && WINDOWS[i].state != WinState::Minimized {
                    FOCUSED = i;
                    return;
                }
            }
        }
    }
}

pub fn write_to_window(idx: usize, data: &[u8]) {
    unsafe {
        if idx >= MAX_WINDOWS || !WINDOWS[idx].active { return; }
        let cl = WINDOWS[idx].content_len;
        let copy = data.len().min(8192 - cl);
        WINDOWS[idx].content[cl..cl + copy].copy_from_slice(&data[..copy]);
        WINDOWS[idx].content_len = cl + copy;
    }
}

pub fn clear_window_content(idx: usize) {
    unsafe {
        if idx < MAX_WINDOWS { WINDOWS[idx].content_len = 0; }
    }
}

// ── Hit testing ───────────────────────────────────────────────────

pub fn window_at(mx: u32, my: u32) -> Option<usize> {
    unsafe {
        let mut best: Option<(usize, u32)> = None;
        for i in 0..MAX_WINDOWS {
            if !WINDOWS[i].active || WINDOWS[i].state == WinState::Minimized { continue; }
            let wx = WINDOWS[i].x as u32;
            let wy = WINDOWS[i].y as u32;
            if mx >= wx && mx < wx + WINDOWS[i].w && my >= wy && my < wy + WINDOWS[i].h {
                match best {
                    None => best = Some((i, WINDOWS[i].z)),
                    Some((_, bz)) if WINDOWS[i].z > bz => best = Some((i, WINDOWS[i].z)),
                    _ => {}
                }
            }
        }
        best.map(|(i, _)| i)
    }
}

pub fn hit_test(mx: u32, my: u32) -> HitResult {
    unsafe {
        if let Some(fi) = active_window() {
            let wx = WINDOWS[fi].x as u32;
            let wy = WINDOWS[fi].y as u32;
            let ww = WINDOWS[fi].w;
            if my >= wy && my < wy + theme::TITLEBAR_H && mx >= wx && mx < wx + ww {
                // GNOME buttons on the RIGHT: close, maximize, minimize
                let btn_y = wy + (theme::TITLEBAR_H - theme::BTN_SIZE) / 2;

                // Close button (rightmost)
                let close_x = wx + ww - theme::BTN_SIZE;
                if mx >= close_x && mx < close_x + theme::BTN_SIZE
                    && my >= btn_y && my < btn_y + theme::BTN_SIZE {
                    return HitResult::CloseButton(fi);
                }
                // Maximize button
                let max_x = close_x - theme::BTN_SIZE;
                if mx >= max_x && mx < max_x + theme::BTN_SIZE
                    && my >= btn_y && my < btn_y + theme::BTN_SIZE {
                    return HitResult::MaximizeButton(fi);
                }
                // Minimize button
                let min_x = max_x - theme::BTN_SIZE;
                if mx >= min_x && mx < min_x + theme::BTN_SIZE
                    && my >= btn_y && my < btn_y + theme::BTN_SIZE {
                    return HitResult::MinimizeButton(fi);
                }

                return HitResult::TitleBar(fi);
            }
            // Resize edges
            let wx_end = wx + ww;
            let wy_end = wy + WINDOWS[fi].h;
            if mx >= wx_end - 4 && my >= wy_end - 4 {
                return HitResult::ResizeCorner(fi);
            }
            if mx >= wx_end - 4 {
                return HitResult::ResizeRight(fi);
            }
            if my >= wy_end - 4 {
                return HitResult::ResizeBottom(fi);
            }
        }
        if let Some(idx) = window_at(mx, my) {
            HitResult::Client(idx)
        } else {
            HitResult::Desktop
        }
    }
}

#[derive(Debug)]
pub enum HitResult {
    CloseButton(usize),
    MinimizeButton(usize),
    MaximizeButton(usize),
    TitleBar(usize),
    ResizeRight(usize),
    ResizeBottom(usize),
    ResizeCorner(usize),
    Client(usize),
    Desktop,
}

// ── Input processing ──────────────────────────────────────────────

pub fn process_event(ev: input::Event) {
    match ev {
        input::Event::MouseMove { x, y, .. } => {
            if let Some(di) = unsafe { DRAGGING } {
                unsafe {
                    WINDOWS[di].x = x - DRAG_OX;
                    WINDOWS[di].y = y - DRAG_OY;
                    super::NEED_REDRAW = true;
                }
            }
            if let Some(ri) = unsafe { RESIZING } {
                unsafe {
                    let edge = RESIZE_EDGE;
                    let wx = WINDOWS[ri].x as u32;
                    if edge & 1 != 0 {
                        WINDOWS[ri].w = (x as u32 - wx).max(theme::WINDOW_MIN_W);
                    }
                    if edge & 2 != 0 {
                        let wy = WINDOWS[ri].y as u32;
                        WINDOWS[ri].h = (y as u32 - wy).max(theme::WINDOW_MIN_H);
                    }
                    super::NEED_REDRAW = true;
                }
            }
        }
        input::Event::MouseDown { x, y, btn } => {
            if btn == input::MouseButton::Left {
                match hit_test(x as u32, y as u32) {
                    HitResult::CloseButton(i) => close_window(i),
                    HitResult::MinimizeButton(i) => {
                        unsafe {
                            WINDOWS[i].state = WinState::Minimized;
                            super::NEED_REDRAW = true;
                        }
                    }
                    HitResult::MaximizeButton(i) => {
                        unsafe {
                            WINDOWS[i].state = if WINDOWS[i].state == WinState::Maximized {
                                WinState::Normal
                            } else {
                                WinState::Maximized
                            };
                            super::NEED_REDRAW = true;
                        }
                    }
                    HitResult::TitleBar(i) => {
                        unsafe {
                            DRAGGING = Some(i);
                            DRAG_OX = x - WINDOWS[i].x;
                            DRAG_OY = y - WINDOWS[i].y;
                            WINDOWS[i].z = NEXT_Z;
                            NEXT_Z += 1;
                            FOCUSED = i;
                            super::NEED_REDRAW = true;
                        }
                    }
                    HitResult::ResizeRight(i) => {
                        unsafe { RESIZING = Some(i); RESIZE_EDGE = 1; }
                    }
                    HitResult::ResizeBottom(i) => {
                        unsafe { RESIZING = Some(i); RESIZE_EDGE = 2; }
                    }
                    HitResult::ResizeCorner(i) => {
                        unsafe { RESIZING = Some(i); RESIZE_EDGE = 3; }
                    }
                    HitResult::Client(i) => {
                        unsafe {
                            FOCUSED = i;
                            WINDOWS[i].z = NEXT_Z;
                            NEXT_Z += 1;
                            super::NEED_REDRAW = true;
                        }
                    }
                    HitResult::Desktop => {}
                }
            }
        }
        input::Event::MouseUp { btn, .. } => {
            if btn == input::MouseButton::Left {
                unsafe { DRAGGING = None; RESIZING = None; }
            }
        }
        _ => {}
    }
}

// ── Accessors ─────────────────────────────────────────────────────

pub fn active_window() -> Option<usize> {
    unsafe {
        if FOCUSED < MAX_WINDOWS && WINDOWS[FOCUSED].active { Some(FOCUSED) }
        else { None }
    }
}

pub fn with_window<F, R>(idx: usize, f: F) -> Option<R> where F: FnOnce(&Window) -> R {
    unsafe {
        if idx < MAX_WINDOWS && WINDOWS[idx].active {
            Some(f(&WINDOWS[idx]))
        } else {
            None
        }
    }
}

pub fn with_window_mut<F>(idx: usize, mut f: F) where F: FnMut(&mut Window) {
    unsafe {
        if idx < MAX_WINDOWS && WINDOWS[idx].active {
            f(&mut WINDOWS[idx]);
        }
    }
}

pub fn for_each_window<F: FnMut(usize, &Window)>(mut f: F) {
    unsafe {
        for i in 0..MAX_WINDOWS {
            if WINDOWS[i].active {
                f(i, &WINDOWS[i]);
            }
        }
    }
}

pub fn window_count() -> usize {
    unsafe { (0..MAX_WINDOWS).filter(|&i| WINDOWS[i].active).count() }
}

pub fn focused_idx() -> usize { unsafe { FOCUSED } }

pub unsafe fn window_input_len(idx: usize) -> usize {
    if idx < MAX_WINDOWS { WINDOWS[idx].input_len } else { 0 }
}

pub fn toggle_minimize(idx: usize) {
    unsafe {
        if idx >= MAX_WINDOWS || !WINDOWS[idx].active { return; }
        if WINDOWS[idx].state == WinState::Minimized {
            WINDOWS[idx].state = WinState::Normal;
            FOCUSED = idx;
        } else {
            WINDOWS[idx].state = WinState::Minimized;
        }
        super::NEED_REDRAW = true;
    }
}

// ── Rendering — GNOME window style ────────────────────────────────

pub fn draw_window_frame(idx: usize) {
    unsafe {
        if idx >= MAX_WINDOWS || !WINDOWS[idx].active { return; }
        if WINDOWS[idx].state == WinState::Minimized { return; }

        let mut wx = WINDOWS[idx].x as u32;
        let mut wy = WINDOWS[idx].y as u32;
        let mut ww = WINDOWS[idx].w;
        let mut wh = WINDOWS[idx].h;

        // Maximized: fill screen minus panel
        if WINDOWS[idx].state == WinState::Maximized {
            wx = 0; wy = theme::PANEL_H;
            ww = sw(); wh = sh() - theme::PANEL_H;
        }
        if WINDOWS[idx].state == WinState::SnappedLeft {
            wx = 0; wy = theme::PANEL_H;
            ww = sw() / 2; wh = sh() - theme::PANEL_H;
        }
        if WINDOWS[idx].state == WinState::SnappedRight {
            wy = theme::PANEL_H;
            ww = sw() / 2; wh = sh() - theme::PANEL_H;
            wx = sw() - ww;
        }

        let is_focused = FOCUSED == idx;

        // Shadow
        render::draw_shadow(wx, wy, ww, wh, theme::CORNER_RADIUS);

        // Window background
        render::fill_rounded_rect(wx, wy, ww, wh, theme::BG, theme::CORNER_RADIUS);

        // Focused border highlight (Adwaita blue)
        if is_focused {
            render::draw_rect_border(wx, wy, ww, wh, theme::CORNER_RADIUS, theme::ACCENT);
        }

        // Title bar
        let tb_h = theme::TITLEBAR_H;
        render::fill_rect(wx + 1, wy + 1, ww - 2, tb_h - 1, theme::SURFACE);

        // Title text (centered)
        let title = &WINDOWS[idx].title[..WINDOWS[idx].title_len];
        let title_w = title.len() as u32 * 8;
        let title_x = wx + (ww - title_w) / 2;
        render::draw_text(title_x, wy + 10, title, if is_focused { theme::TEXT } else { theme::TEXT_DIM });

        // GNOME buttons (right side): close, maximize, minimize
        let btn_y = wy + (tb_h - theme::BTN_SIZE) / 2;

        // Close button (red)
        let close_x = wx + ww - theme::BTN_SIZE - 4;
        render::fill_rounded_rect(close_x, btn_y, theme::BTN_SIZE, theme::BTN_SIZE,
            theme::BTN_CLOSE, theme::BTN_SIZE / 2);
        // × symbol
        render::draw_text(close_x + 8, btn_y + 6, b"x", 0xFFFFFF);

        // Maximize button
        let max_x = close_x - theme::BTN_SIZE;
        let mx = input::mouse_x() as u32;
        let my = input::mouse_y() as u32;
        let max_hover = mx >= max_x && mx < max_x + theme::BTN_SIZE
            && my >= btn_y && my < btn_y + theme::BTN_SIZE;
        let max_bg = if max_hover { theme::HOVER_BG } else { theme::BTN_MAXIMIZE };
        render::fill_rounded_rect(max_x, btn_y, theme::BTN_SIZE, theme::BTN_SIZE,
            max_bg, 4);
        // □ symbol
        render::draw_text(max_x + 7, btn_y + 6, b"[]", theme::BTN_ICON);

        // Minimize button
        let min_x = max_x - theme::BTN_SIZE;
        let min_hover = mx >= min_x && mx < min_x + theme::BTN_SIZE
            && my >= btn_y && my < btn_y + theme::BTN_SIZE;
        let min_bg = if min_hover { theme::HOVER_BG } else { theme::BTN_MINIMIZE };
        render::fill_rounded_rect(min_x, btn_y, theme::BTN_SIZE, theme::BTN_SIZE,
            min_bg, 4);
        // — symbol
        render::draw_text(min_x + 8, btn_y + 6, b"-", theme::BTN_ICON);

        // Content area
        let content_y = wy + tb_h;
        let content_h = wh - tb_h;
        if content_h > 0 {
            render::fill_rect(wx + 1, content_y, ww - 2, content_h - 1, theme::BG);

            let content = &WINDOWS[idx].content[..WINDOWS[idx].content_len];
            let mut line_starts = [0usize; 256];
            let mut line_count;
            line_starts[0] = 0;
            line_count = 1;
            for (i, &c) in content.iter().enumerate() {
                if (c == b'\n' || c == b'\r') && i + 1 < content.len() && line_count < 256 {
                    line_starts[line_count] = i + 1;
                    line_count += 1;
                }
            }
            let mut cy = content_y + 8;
            let max_lines = (content_h / 10) as usize;
            let start = line_count.saturating_sub(max_lines);
            for li in start..line_count {
                if cy + 10 > content_y + content_h { break; }
                let ls = line_starts[li];
                let le = if li + 1 < line_count { line_starts[li + 1] } else { content.len() };
                let mut begin = ls;
                while begin < le && (content[begin] == b'\r' || content[begin] == b'\n') { begin += 1; }
                if begin < le {
                    render::draw_text(wx + 12, cy, &content[begin..le], theme::TEXT);
                }
                cy += 10;
            }

            // Terminal prompt
            if WINDOWS[idx].kind == WinKind::Terminal {
                let input = &WINDOWS[idx].input_buf[..WINDOWS[idx].input_len];
                render::draw_text(wx + 12, cy + 4, b"> ", theme::ACCENT);
                render::draw_text(wx + 28, cy + 4, input, theme::TEXT);
                let cx = wx + 28 + WINDOWS[idx].input_len as u32 * 8;
                if (crate::timer::millis() / 500) % 2 == 0 {
                    render::fill_rect(cx, cy + 2, 8, 10, theme::TEXT);
                }
            }
        }
    }
}
