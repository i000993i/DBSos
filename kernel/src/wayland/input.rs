/// Wayland compositor input — translates DBSos input events to Wayland protocol
///
/// Maps PS/2 mouse/keyboard to wl_pointer, wl_keyboard events.

use super::protocol::*;
use super::surface;

/// Input state for the compositor
pub struct InputState {
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub mouse_buttons: u32,
    pub keyboard_serial: u32,
    pub pointer_serial: u32,
    pub focused_surface: WlId,
    pub focused_surface_idx: Option<usize>,
}

static mut INPUT: InputState = InputState {
    mouse_x: 0, mouse_y: 0, mouse_buttons: 0,
    keyboard_serial: 0, pointer_serial: 0,
    focused_surface: 0, focused_surface_idx: None,
};

pub fn init() {}

pub fn get_input() -> &'static InputState {
    unsafe { &INPUT }
}

pub fn get_input_mut() -> &'static mut InputState {
    unsafe { &mut INPUT }
}

/// Poll hardware input and update state
pub fn poll() {
    let mx = crate::driver::mouse::x();
    let my = crate::driver::mouse::y();
    let lb = crate::driver::mouse::left();

    unsafe {
        INPUT.mouse_x = mx;
        INPUT.mouse_y = my;

        // Find surface under cursor
        let (idx, id) = match surface::surface_at(mx as u32, my as u32) {
            Some(v) => v,
            None => {
                INPUT.focused_surface = 0;
                INPUT.focused_surface_idx = None;
                return;
            }
        };

        INPUT.focused_surface = id;
        INPUT.focused_surface_idx = Some(idx);

        // Button state change
        let prev = INPUT.mouse_buttons;
        let curr = if lb { 1 } else { 0 };
        if curr != prev {
            INPUT.pointer_serial += 1;
            INPUT.mouse_buttons = curr;

            // Send button event to focused surface
            if id != 0 {
                let state = if curr == 1 {
                    WL_POINTER_BUTTON_STATE_PRESSED
                } else {
                    WL_POINTER_BUTTON_STATE_RELEASED
                };
                send_pointer_button(id, 0x110, state);  // BTN_LEFT = 0x110
            }
        }

        // Motion
        if prev != 0 || curr != 0 {
            send_pointer_motion(id, mx, my);
        }
    }
}

/// Process keyboard input
pub fn poll_keyboard() {
    if let Some(c) = crate::driver::ps2::poll_char() {
        let input = get_input();
        if input.focused_surface != 0 {
            // Map ASCII to Linux keycode (simplified)
            let key = ascii_to_keycode(c);
            if key != 0 {
                send_key_event(input.focused_surface, key, WL_KEYBOARD_KEY_STATE_PRESSED);
                send_key_event(input.focused_surface, key, WL_KEYBOARD_KEY_STATE_RELEASED);
            }
        }
    }
}

fn ascii_to_keycode(ascii: u8) -> u32 {
    match ascii {
        b'a'..=b'z' => (ascii - b'a' + 30) as u32,  // KEY_A = 30
        b'0'..=b'9' => (ascii - b'0' + 11) as u32,   // KEY_1 = 2
        b' ' => 57,     // KEY_SPACE
        b'\n' => 28,    // KEY_ENTER
        b'\t' => 15,    // KEY_TAB
        0x08 => 14,     // KEY_BACKSPACE
        0x1B => 1,      // KEY_ESC
        _ => 0,
    }
}

/// Send pointer enter event
pub fn send_pointer_enter(_surface_id: WlId, _x: i32, _y: i32) {
    // In a real compositor, we'd queue this event for the client
    // For now, it's handled internally
}

/// Send pointer motion event
fn send_pointer_motion(surface_id: WlId, x: i32, y: i32) {
    if let Some(surf) = surface::find_surface(surface_id) {
        // Convert screen coords to surface-local coords
        let _local_x = (x - surf.x) * 256;  // Fixed point 24.8
        let _local_y = (y - surf.y) * 256;
        // In a real compositor, this would be sent as wl_pointer.motion event
    }
}

/// Send pointer button event
fn send_pointer_button(_surface_id: WlId, _button: u32, _state: u32) {
    // In a real compositor, this would be sent as wl_pointer.button event
}

/// Send keyboard key event
fn send_key_event(_surface_id: WlId, _key: u32, _state: u32) {
    // In a real compositor, this would be sent as wl_keyboard.key event
}

/// Send frame callback to surface
pub fn send_frame_done(surface_id: WlId) {
    if let Some(surf) = surface::find_surface(surface_id) {
        if surf.callback_id != 0 {
            // In a real compositor, this would be sent as wl_callback.done
        }
    }
}
