//! Handheld On-Screen Virtual Keyboard & Dual-Trackpad Typing Engine.

#![allow(dead_code)]

/// Supported on-screen virtual keyboard layout types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualKeyboardLayoutType {
    StandardQwerty,
    CompactThumbSplit,
    GamepadRadial,
}

/// Keyboard visibility and composition state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualKeyboardState {
    Hidden,
    AnimatingIn,
    Visible,
    AnimatingOut,
}

/// Virtual key geometry and representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualKeyDescriptor {
    pub key_code: u8, // Virtual key code (e.g. VK_A = 0x41)
    pub char_code: char,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// Virtual keyboard layout geometry coordinator.
#[derive(Debug, Clone, Copy)]
pub struct VirtualKeyboardLayoutManager {
    pub active_layout: VirtualKeyboardLayoutType,
    pub current_state: VirtualKeyboardState,
    pub screen_width: u16,
    pub screen_height: u16,
    pub keyboard_height: u16,
}

impl VirtualKeyboardLayoutManager {
    /// Creates a new virtual keyboard manager for a 1920x1080 display.
    pub const fn new() -> Self {
        Self {
            active_layout: VirtualKeyboardLayoutType::CompactThumbSplit,
            current_state: VirtualKeyboardState::Hidden,
            screen_width: 1920,
            screen_height: 1080,
            keyboard_height: 380,
        }
    }

    /// Sets keyboard visibility state.
    pub fn set_state(&mut self, state: VirtualKeyboardState) {
        self.current_state = state;
    }

    /// Sets active keyboard layout.
    pub fn set_layout(&mut self, layout: VirtualKeyboardLayoutType) {
        self.active_layout = layout;
    }

    /// Checks if a given coordinate lies within the virtual keyboard bounds.
    pub fn contains_point(&self, x: u16, y: u16) -> bool {
        if self.current_state != VirtualKeyboardState::Visible {
            return false;
        }
        let top_y = self.screen_height.saturating_sub(self.keyboard_height);
        x < self.screen_width && y >= top_y && y < self.screen_height
    }
}

/// Unit verification for virtual keyboard state machine and layouts.
pub fn verify_keyboard_layouts() -> bool {
    let mut mgr = VirtualKeyboardLayoutManager::new();
    if mgr.current_state != VirtualKeyboardState::Hidden {
        return false;
    }

    mgr.set_state(VirtualKeyboardState::Visible);
    if !mgr.contains_point(500, 800) { // 1080 - 380 = 700; y=800 is inside
        return false;
    }
    if mgr.contains_point(500, 600) { // y=600 is above keyboard
        return false;
    }

    mgr.set_layout(VirtualKeyboardLayoutType::GamepadRadial);
    if mgr.active_layout != VirtualKeyboardLayoutType::GamepadRadial {
        return false;
    }

    true
}
