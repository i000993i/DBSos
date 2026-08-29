/// egui_core — immediate-mode GUI core (no_std, egui-inspired API)
///
/// API mirrors egui: Context → Ui → widgets → Response
/// Backend renders to GOP framebuffer via our render.rs

use crate::gui::{theme, render, input};

// ── Rect ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self { Self { x, y, w, h } }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn left(&self) -> f32 { self.x }
    pub fn right(&self) -> f32 { self.x + self.w }
    pub fn top(&self) -> f32 { self.y }
    pub fn bottom(&self) -> f32 { self.y + self.h }
}

// ── Color ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self { Self { r, g, b, a: 255 } }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self { Self { r, g, b, a } }
    pub const fn from_u32(c: u32) -> Self {
        Self { r: ((c >> 16) & 0xFF) as u8, g: ((c >> 8) & 0xFF) as u8, b: (c & 0xFF) as u8, a: ((c >> 24) & 0xFF) as u8 }
    }
    pub fn to_u32(&self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }
    pub fn to_rgb(&self) -> u32 { (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32 }
}

// ── Interaction state ─────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
pub struct Response {
    pub rect: Rect,
    pub clicked: bool,
    pub hovered: bool,
    pub dragged: bool,
    pub changed: bool,
    pub lost_focus: bool,
    pub has_focus: bool,
}

impl Response {
    pub fn none() -> Self { Self { rect: Rect::new(0.0, 0.0, 0.0, 0.0), ..Default::default() } }
}

// ── Input state ───────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
pub struct InputState {
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub mouse_down: bool,
    pub mouse_pressed: bool,
    pub mouse_released: bool,
    pub scroll: f32,
    pub key_tab: bool,
    pub key_enter: bool,
    pub key_backspace: bool,
    pub key_escape: bool,
    pub key_char: Option<u8>,
    pub screen_w: f32,
    pub screen_h: f32,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            screen_w: crate::display::width() as f32,
            screen_h: crate::display::height() as f32,
            ..Default::default()
        }
    }
}

// ── Shape (deferred draw commands) ────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub enum Shape {
    Rect { rect: Rect, color: u32, radius: u32, alpha: u8 },
    Text { pos: (f32, f32), text: [u8; 256], len: usize, color: u32 },
    Line { p0: (f32, f32), p1: (f32, f32), color: u32, width: f32 },
}

// ── Painter ───────────────────────────────────────────────────────

pub struct Painter {
    shapes: [Option<Shape>; 1024],
    count: usize,
}

impl Painter {
    pub fn new() -> Self {
        Self {
            shapes: [None; 1024],
            count: 0,
        }
    }

    pub fn rect_filled(&mut self, rect: Rect, radius: u32, color: u32) {
        if self.count < 1024 {
            self.shapes[self.count] = Some(Shape::Rect { rect, color, radius, alpha: 255 });
            self.count += 1;
        }
    }

    pub fn rect_alpha(&mut self, rect: Rect, radius: u32, color: u32, alpha: u8) {
        if self.count < 1024 {
            self.shapes[self.count] = Some(Shape::Rect { rect, color, radius, alpha });
            self.count += 1;
        }
    }

    pub fn text(&mut self, pos: (f32, f32), color: u32, text: &[u8]) {
        if self.count < 1024 {
            let mut buf = [0u8; 256];
            let len = text.len().min(255);
            buf[..len].copy_from_slice(&text[..len]);
            self.shapes[self.count] = Some(Shape::Text { pos, text: buf, len, color });
            self.count += 1;
        }
    }

    pub fn line(&mut self, p0: (f32, f32), p1: (f32, f32), color: u32) {
        if self.count < 1024 {
            self.shapes[self.count] = Some(Shape::Line { p0, p1, color, width: 1.0 });
            self.count += 1;
        }
    }

    pub fn flush(&self) {
        for i in 0..self.count {
            if let Some(ref s) = self.shapes[i] {
                match s {
                    Shape::Rect { rect, color, radius, alpha } => {
                        if *alpha == 255 {
                            render::fill_rounded_rect(
                                rect.x as u32, rect.y as u32,
                                rect.w as u32, rect.h as u32,
                                *color, *radius,
                            );
                        } else {
                            render::fill_rounded_rect_alpha(
                                rect.x as u32, rect.y as u32,
                                rect.w as u32, rect.h as u32,
                                *color, *alpha, *radius,
                            );
                        }
                    }
                    Shape::Text { pos, text, len, color } => {
                        render::draw_text(pos.0 as u32, pos.1 as u32, &text[..*len], *color);
                    }
                    Shape::Line { p0, p1, color, .. } => {
                        let x0 = p0.0 as u32;
                        let y0 = p0.1 as u32;
                        let x1 = p1.0 as u32;
                        if (p0.1 - p1.1).abs() < 0.5 {
                            render::hline(x0.min(x1), y0, (x1).max(x0) - x0 + 1, *color);
                        }
                    }
                }
            }
        }
    }

    pub fn clear(&mut self) {
        for i in 0..self.count { self.shapes[i] = None; }
        self.count = 0;
    }
}

// ── Ui layout ─────────────────────────────────────────────────────

pub struct Ui {
    pub rect: Rect,
    cursor: (f32, f32),
    available_w: f32,
    spacing: f32,
    input: InputState,
    painter: *mut Painter,
    id: u64,
}

impl Ui {
    pub fn new(rect: Rect, input: &InputState, painter: *mut Painter, id: u64) -> Self {
        Self {
            rect,
            cursor: (rect.x + 8.0, rect.y + 8.0),
            available_w: rect.w - 16.0,
            spacing: 6.0,
            input: *input,
            painter,
            id,
        }
    }

    pub fn available_width(&self) -> f32 { self.available_w }

    pub fn allocate_space(&mut self, w: f32, h: f32) -> Rect {
        let r = Rect::new(self.cursor.0, self.cursor.1, w, h);
        self.cursor.1 += h + self.spacing;
        r
    }

    // ── Widgets ───────────────────────────────────────────────────

    pub fn label(&mut self, text: &[u8]) -> Response {
        let w = text.len() as f32 * 8.0;
        let h = 12.0;
        let rect = self.allocate_space(w, h);
        unsafe { (*self.painter).text((rect.x, rect.y + 2.0), theme::TEXT, text); }
        Response { rect, ..Default::default() }
    }

    pub fn label_color(&mut self, text: &[u8], color: u32) -> Response {
        let w = text.len() as f32 * 8.0;
        let h = 12.0;
        let rect = self.allocate_space(w, h);
        unsafe { (*self.painter).text((rect.x, rect.y + 2.0), color, text); }
        Response { rect, ..Default::default() }
    }

    pub fn button(&mut self, text: &[u8]) -> Response {
        let w = (text.len() as u32 * 8 + 24) as f32;
        let h = 28.0;
        let rect = self.allocate_space(w, h);

        let mx = self.input.mouse_x;
        let my = self.input.mouse_y;
        let hovered = rect.contains(mx, my);
        let clicked = hovered && self.input.mouse_pressed;

        let bg = if clicked { theme::PRESS_BG }
                 else if hovered { theme::HOVER_BG }
                 else { theme::SURFACE };

        unsafe {
            (*self.painter).rect_filled(rect, 6, bg);
            (*self.painter).text((rect.x + 12.0, rect.y + 8.0), theme::TEXT, text);
        }

        Response { rect, clicked, hovered, ..Default::default() }
    }

    pub fn button_accent(&mut self, text: &[u8]) -> Response {
        let w = (text.len() as u32 * 8 + 24) as f32;
        let h = 28.0;
        let rect = self.allocate_space(w, h);

        let mx = self.input.mouse_x;
        let my = self.input.mouse_y;
        let hovered = rect.contains(mx, my);
        let clicked = hovered && self.input.mouse_pressed;

        let bg = if clicked { 0x5B21B6 } else if hovered { theme::ACCENT_HOVER } else { theme::ACCENT };

        unsafe {
            (*self.painter).rect_filled(rect, 6, bg);
            (*self.painter).text((rect.x + 12.0, rect.y + 8.0), 0xFFFFFF, text);
        }

        Response { rect, clicked, hovered, ..Default::default() }
    }

    pub fn text_edit(&mut self, _id: u64, buf: &mut [u8], len: &mut usize, focused: &mut bool) -> Response {
        let w = self.available_w;
        let h = 28.0;
        let rect = self.allocate_space(w, h);

        let mx = self.input.mouse_x;
        let my = self.input.mouse_y;

        // Focus management
        if self.input.mouse_pressed {
            *focused = rect.contains(mx, my);
        }

        // Background
        let bg = if *focused { 0x1A1A2E } else { theme::BG };
        let border = if *focused { theme::ACCENT } else { theme::BORDER };

        unsafe {
            (*self.painter).rect_filled(rect, 4, bg);
            (*self.painter).line((rect.x, rect.y + rect.h - 1.0), (rect.x + rect.w, rect.y + rect.h - 1.0), border);
        }

        // Text
        if *len > 0 {
            unsafe { (*self.painter).text((rect.x + 8.0, rect.y + 8.0), theme::TEXT, &buf[..*len]); }
        }

        // Cursor
        if *focused && (crate::timer::millis() / 500) % 2 == 0 {
            let cx = rect.x + 8.0 + *len as f32 * 8.0;
            unsafe { (*self.painter).rect_filled(Rect::new(cx, rect.y + 4.0, 2.0, 20.0), 0, theme::TEXT); }
        }

        // Handle input
        if *focused {
            if let Some(c) = self.input.key_char {
                if c == 0x08 {
                    if *len > 0 { *len -= 1; }
                } else if c >= b' ' && *len < buf.len() - 1 {
                    buf[*len] = c;
                    *len += 1;
                }
            }
        }

        Response { rect, has_focus: *focused, ..Default::default() }
    }

    pub fn separator(&mut self) {
        let rect = Rect::new(self.rect.x, self.cursor.1, self.rect.w, 1.0);
        unsafe { (*self.painter).line((rect.x, rect.y), (rect.x + rect.w, rect.y), theme::BORDER); }
        self.cursor.1 += self.spacing + 1.0;
    }

    pub fn spacing(&mut self, h: f32) { self.cursor.1 += h; }

    pub fn horizontal<F: FnMut(&mut Ui)>(&mut self, mut f: F) {
        let start_y = self.cursor.1;
        let start_x = self.cursor.0;
        let mut sub = Ui::new(
            Rect::new(start_x, start_y, self.available_w, 100.0),
            &self.input, self.painter, self.id + 1,
        );
        f(&mut sub);
        self.cursor.1 = sub.cursor.1 + self.spacing;
    }

    pub fn vertical<F: FnMut(&mut Ui)>(&mut self, mut f: F) {
        let start_y = self.cursor.1;
        let mut sub = Ui::new(
            Rect::new(self.cursor.0, start_y, self.available_w, self.rect.h - (start_y - self.rect.y)),
            &self.input, self.painter, self.id + 1,
        );
        f(&mut sub);
        self.cursor.1 = sub.cursor.1 + self.spacing;
    }

    pub fn scroll_area<F: FnMut(&mut Ui)>(&mut self, h: f32, scroll: &mut f32, mut f: F) {
        let rect = Rect::new(self.cursor.0, self.cursor.1, self.available_w, h);

        // Scroll with mouse wheel
        if rect.contains(self.input.mouse_x, self.input.mouse_y) {
            *scroll -= self.input.scroll;
            if *scroll < 0.0 { *scroll = 0.0; }
        }

        // Clip region
        let mut sub = Ui::new(rect, &self.input, self.painter, self.id + 1);
        sub.cursor.1 -= *scroll;
        f(&mut sub);

        // Scrollbar
        let total_h = sub.cursor.1 - rect.y + *scroll;
        if total_h > h {
            let bar_h = (h * h / total_h).max(20.0);
            let bar_y = rect.y + (*scroll / total_h) * (h - bar_h);
            unsafe {
                (*self.painter).rect_alpha(
                    Rect::new(rect.right() - 6.0, bar_y, 4.0, bar_h),
                    2, theme::TEXT_DIM, 128,
                );
            }
        }

        self.cursor.1 += h + self.spacing;
    }

    pub fn set_input(&mut self, input: &InputState) { self.input = *input; }
}

// ── Context ───────────────────────────────────────────────────────

pub struct Context {
    pub input: InputState,
    pub painter: Painter,
    frame: u64,
}

impl Context {
    pub fn new() -> Self {
        Self {
            input: InputState::new(),
            painter: Painter::new(),
            frame: 0,
        }
    }

    pub fn begin_frame(&mut self) {
        // Sync input
        self.input.mouse_x = input::mouse_x() as f32;
        self.input.mouse_y = input::mouse_y() as f32;
        self.input.mouse_pressed = false;
        self.input.mouse_released = false;
        self.input.key_char = None;
        self.input.key_enter = false;
        self.input.key_backspace = false;
        self.input.screen_w = crate::display::width() as f32;
        self.input.screen_h = crate::display::height() as f32;

        // Process events
        while let Some(ev) = input::next_event() {
            match ev {
                input::Event::MouseMove { x, y, .. } => {
                    self.input.mouse_x = x as f32;
                    self.input.mouse_y = y as f32;
                }
                input::Event::MouseDown { .. } => {
                    self.input.mouse_pressed = true;
                    self.input.mouse_down = true;
                }
                input::Event::MouseUp { .. } => {
                    self.input.mouse_released = true;
                    self.input.mouse_down = false;
                }
                input::Event::CharInput(c) => {
                    self.input.key_char = Some(c);
                    if c == b'\r' { self.input.key_enter = true; }
                    if c == 0x08 { self.input.key_backspace = true; }
                }
                _ => {}
            }
        }

        // Sync mouse from hardware
        let hmx = crate::driver::mouse::x();
        let hmy = crate::driver::mouse::y();
        self.input.mouse_x = hmx as f32;
        self.input.mouse_y = hmy as f32;
        if crate::driver::mouse::left() && !self.input.mouse_down {
            self.input.mouse_pressed = true;
            self.input.mouse_down = true;
        } else if !crate::driver::mouse::left() && self.input.mouse_down {
            self.input.mouse_released = true;
            self.input.mouse_down = false;
        }

        self.painter.clear();
        self.frame += 1;
    }

    pub fn end_frame(&self) {
        self.painter.flush();
    }

    pub fn screen_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.input.screen_w, self.input.screen_h)
    }

    pub fn make_ui(&mut self, rect: Rect, id: u64) -> Ui {
        Ui::new(rect, &self.input, &mut self.painter as *mut Painter, id)
    }

    pub fn input(&self) -> &InputState { &self.input }
    pub fn frame_count(&self) -> u64 { self.frame }
}
