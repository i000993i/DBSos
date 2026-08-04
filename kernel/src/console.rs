/// Framebuffer text console (GOP)

static mut FB_ADDR: u64 = 0;
static mut FB_WIDTH: u32 = 0;
static mut FB_HEIGHT: u32 = 0;
static mut FB_STRIDE: u32 = 0;

static mut CX: u32 = 0;
static mut CY: u32 = 0;

const FONT_W: u32 = 8;
const FONT_H: u32 = 16;

const COL_WHITE: u32 = 0x00FFFFFF;
const COL_BLACK: u32 = 0x00000000;

fn pix(x: u32, y: u32, c: u32) {
    unsafe {
        if x >= FB_WIDTH || y >= FB_HEIGHT { return; }
        let addr = FB_ADDR + (y * FB_STRIDE + x) as u64 * 4;
        core::ptr::write_volatile(addr as *mut u32, c);
    }
}

pub fn draw_char(x: u32, y: u32, ch: u8, fg: u32, bg: u32) {
    let font = &crate::font::FONT_8X16;
    let off = (ch as usize) * 16;
    for row in 0..16u32 {
        let bits = font[off + row as usize];
        for col in 0..8u32 {
            pix(x + col, y + row, if bits & (0x80 >> (col as u8)) != 0 { fg } else { bg });
        }
    }
}

fn scroll() {
    unsafe {
        let row = FB_STRIDE as u64 * 4;
        let h = FB_HEIGHT as u64;
        core::ptr::copy(
            (FB_ADDR + FONT_H as u64 * row) as *const u8,
            FB_ADDR as *mut u8,
            ((h - FONT_H as u64) * row) as usize,
        );
        core::ptr::write_bytes(
            (FB_ADDR + (h - FONT_H as u64) * row) as *mut u8,
            0,
            (FONT_H as u64 * row) as usize,
        );
        CY = h as u32 - FONT_H;
    }
}

pub fn putchar(c: u8) {
    unsafe {
        match c {
            b'\n' | b'\r' => {
                CX = 0;
                CY += FONT_H;
                if CY + FONT_H > FB_HEIGHT { scroll(); }
            }
            0x08 => {
                if CX >= FONT_W { CX -= FONT_W; }
                draw_char(CX, CY, b' ', COL_WHITE, COL_BLACK);
            }
            _ => {
                draw_char(CX, CY, c, COL_WHITE, COL_BLACK);
                CX += FONT_W;
                if CX + FONT_W > FB_WIDTH { CX = 0; CY += FONT_H; }
                if CY + FONT_H > FB_HEIGHT { scroll(); }
            }
        }
    }
}

pub fn write_str(s: &str) {
    for &c in s.as_bytes() { putchar(c); }
}

pub fn clear_screen() {
    unsafe {
        let total = (FB_HEIGHT as u64 * FB_STRIDE as u64 * 4) as usize;
        core::ptr::write_bytes(FB_ADDR as *mut u8, 0, total);
        CX = 0; CY = 0;
    }
}

pub fn set_cursor(x_col: u32, y_row: u32) {
    unsafe { CX = x_col * FONT_W; CY = y_row * FONT_H; }
}

pub fn get_cursor() -> (u32, u32) {
    unsafe { (CX / FONT_W, CY / FONT_H) }
}

pub fn draw_rect(x: u32, y: u32, w: u32, h: u32, color: u32) {
    for dy in 0..h {
        for dx in 0..w {
            pix(x + dx, y + dy, color);
        }
    }
}

pub fn screen_rows() -> u32 {
    unsafe { FB_HEIGHT / FONT_H }
}

pub fn screen_cols() -> u32 {
    unsafe { FB_WIDTH / FONT_W }
}

pub fn init(fb_addr: u64, width: u32, height: u32, stride: u32) {
    unsafe {
        FB_ADDR = fb_addr;
        FB_WIDTH = width;
        FB_HEIGHT = height;
        FB_STRIDE = stride;
    }
    clear_screen();
}
