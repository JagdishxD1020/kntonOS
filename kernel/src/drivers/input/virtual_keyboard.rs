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

/// Active dual-cursor trackpad selection zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DualCursorZone {
    LeftHalf,
    RightHalf,
}

/// Dual-trackpad thumb typing cursor position and selection state.
#[derive(Debug, Clone, Copy)]
pub struct DualTrackpadTypingEngine {
    pub left_cursor_x: u16,
    pub left_cursor_y: u16,
    pub right_cursor_x: u16,
    pub right_cursor_y: u16,
    pub selected_key_left: Option<char>,
    pub selected_key_right: Option<char>,
}

impl DualTrackpadTypingEngine {
    /// Creates a new dual-trackpad typing engine initialized for split keyboard.
    pub const fn new() -> Self {
        Self {
            left_cursor_x: 400,
            left_cursor_y: 850,
            right_cursor_x: 1500,
            right_cursor_y: 850,
            selected_key_left: Some('G'),
            selected_key_right: Some('H'),
        }
    }

    /// Updates the left trackpad cursor position bounded to left half of screen.
    pub fn update_left_cursor(&mut self, dx: i16, dy: i16) -> (u16, u16) {
        let new_x = ((self.left_cursor_x as i32 + dx as i32).max(50)).min(900) as u16;
        let new_y = ((self.left_cursor_y as i32 + dy as i32).max(700)).min(1050) as u16;
        self.left_cursor_x = new_x;
        self.left_cursor_y = new_y;

        // Mock key mapping for left half
        self.selected_key_left = if new_x < 300 {
            Some('A')
        } else if new_x < 600 {
            Some('D')
        } else {
            Some('G')
        };

        (new_x, new_y)
    }

    /// Updates the right trackpad cursor position bounded to right half of screen.
    pub fn update_right_cursor(&mut self, dx: i16, dy: i16) -> (u16, u16) {
        let new_x = ((self.right_cursor_x as i32 + dx as i32).max(1020)).min(1870) as u16;
        let new_y = ((self.right_cursor_y as i32 + dy as i32).max(700)).min(1050) as u16;
        self.right_cursor_x = new_x;
        self.right_cursor_y = new_y;

        // Mock key mapping for right half
        self.selected_key_right = if new_x < 1300 {
            Some('H')
        } else if new_x < 1600 {
            Some('K')
        } else {
            Some('L')
        };

        (new_x, new_y)
    }

    /// Triggers trackpad physical click, returning activated key from target zone.
    pub fn on_trackpad_click(&self, zone: DualCursorZone) -> Option<char> {
        match zone {
            DualCursorZone::LeftHalf => self.selected_key_left,
            DualCursorZone::RightHalf => self.selected_key_right,
        }
    }
}

/// Unit verification for dual-trackpad thumb typing engine.
pub fn verify_dual_trackpad_typing() -> bool {
    let mut engine = DualTrackpadTypingEngine::new();

    // Move left cursor leftward toward 'A'
    engine.update_left_cursor(-200, 0); // 400 - 200 = 200 (< 300 -> 'A')
    if engine.on_trackpad_click(DualCursorZone::LeftHalf) != Some('A') {
        return false;
    }

    // Move right cursor rightward toward 'L'
    engine.update_right_cursor(200, 0); // 1500 + 200 = 1700 (>= 1600 -> 'L')
    if engine.on_trackpad_click(DualCursorZone::RightHalf) != Some('L') {
        return false;
    }

    true
}

/// Haptic tactile feedback pulse profile for keystroke confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeystrokeHapticPulse {
    pub frequency_hz: u16,   // e.g. 170Hz resonant frequency
    pub duration_ms: u8,     // 8ms short crisp click
    pub amplitude_pct: u8,   // 70% strength
    pub target_actuator: DualCursorZone,
}

/// Keystroke haptic pulse coordinator.
#[derive(Debug, Clone, Copy)]
pub struct KeystrokeHapticTrigger {
    pub enabled: bool,
    pub total_pulses_fired: u32,
    pub last_pulse: KeystrokeHapticPulse,
}

impl KeystrokeHapticTrigger {
    /// Creates a new keystroke haptic trigger with crisp 8ms tactile pulses.
    pub const fn new() -> Self {
        Self {
            enabled: true,
            total_pulses_fired: 0,
            last_pulse: KeystrokeHapticPulse {
                frequency_hz: 170,
                duration_ms: 8,
                amplitude_pct: 70,
                target_actuator: DualCursorZone::LeftHalf,
            },
        }
    }

    /// Triggers a localized tactile click on the corresponding trackpad actuator.
    pub fn trigger_keystroke_feedback(&mut self, zone: DualCursorZone) -> KeystrokeHapticPulse {
        self.total_pulses_fired += 1;
        self.last_pulse = KeystrokeHapticPulse {
            frequency_hz: 170,
            duration_ms: 8,
            amplitude_pct: 70,
            target_actuator: zone,
        };
        self.last_pulse
    }
}

/// Unit verification for keystroke haptic trigger.
pub fn verify_keystroke_haptic_trigger() -> bool {
    let mut trigger = KeystrokeHapticTrigger::new();

    let p_left = trigger.trigger_keystroke_feedback(DualCursorZone::LeftHalf);
    if p_left.target_actuator != DualCursorZone::LeftHalf || p_left.duration_ms != 8 || trigger.total_pulses_fired != 1 {
        return false;
    }

    let p_right = trigger.trigger_keystroke_feedback(DualCursorZone::RightHalf);
    if p_right.target_actuator != DualCursorZone::RightHalf || trigger.total_pulses_fired != 2 {
        return false;
    }

    true
}

/// Synthesized Win32 window message event type.
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_CHAR: u32 = 0x0102;

/// Synthesized Win32 keyboard message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Win32KeyMessage {
    pub message: u32,
    pub wparam: usize, // Virtual key code or UTF-16 character
    pub lparam: isize, // Repeat count, scan code, flags
}

/// Win32 virtual keystroke event synthesizer.
#[derive(Debug, Clone, Copy)]
pub struct Win32KeySynthesizer {
    pub generated_events_count: u32,
}

impl Win32KeySynthesizer {
    /// Creates a new Win32 key synthesizer.
    pub const fn new() -> Self {
        Self {
            generated_events_count: 0,
        }
    }

    /// Synthesizes the standard sequence of WM_KEYDOWN, WM_CHAR, and WM_KEYUP for a character.
    pub fn synthesize_keystroke(&mut self, c: char, out_events: &mut [Win32KeyMessage; 3]) -> usize {
        let vk_code = match c {
            'A'..='Z' => c as usize,
            'a'..='z' => (c as u8 - b'a' + b'A') as usize,
            '0'..='9' => c as usize,
            ' ' => 0x20,
            '\n' => 0x0D, // VK_RETURN
            _ => 0,
        };

        // WM_KEYDOWN
        out_events[0] = Win32KeyMessage {
            message: WM_KEYDOWN,
            wparam: vk_code,
            lparam: 1, // Repeat 1
        };

        // WM_CHAR
        out_events[1] = Win32KeyMessage {
            message: WM_CHAR,
            wparam: c as usize,
            lparam: 1,
        };

        // WM_KEYUP
        out_events[2] = Win32KeyMessage {
            message: WM_KEYUP,
            wparam: vk_code,
            lparam: (1 << 31) | (1 << 30) | 1, // Transition state = 1, previous state = 1
        };

        self.generated_events_count += 3;
        3
    }
}

/// Unit verification for Win32 keystroke synthesizer.
pub fn verify_win32_key_synthesizer() -> bool {
    let mut synth = Win32KeySynthesizer::new();
    let mut events = [Win32KeyMessage { message: 0, wparam: 0, lparam: 0 }; 3];

    let count = synth.synthesize_keystroke('A', &mut events);
    if count != 3 || synth.generated_events_count != 3 {
        return false;
    }

    if events[0].message != WM_KEYDOWN || events[0].wparam != 0x41 {
        return false;
    }
    if events[1].message != WM_CHAR || events[1].wparam != 0x41 {
        return false;
    }
    if events[2].message != WM_KEYUP || events[2].wparam != 0x41 {
        return false;
    }

    true
}

/// Comprehensive verification self-test suite for Handheld On-Screen Virtual Keyboard & Dual-Trackpad Typing Engine.
pub fn test_virtual_keyboard_subsystem() -> bool {
    crate::serial_write_str("[KNT-INPUT] Testing Handheld Virtual Keyboard & Dual-Trackpad Typing Engine...\r\n");

    if !verify_keyboard_layouts() {
        crate::serial_write_str("[KNT-PANIC] Virtual keyboard layouts verification failed!\r\n");
        return false;
    }
    if !verify_dual_trackpad_typing() {
        crate::serial_write_str("[KNT-PANIC] Dual-trackpad thumb typing verification failed!\r\n");
        return false;
    }
    if !verify_keystroke_haptic_trigger() {
        crate::serial_write_str("[KNT-PANIC] Keystroke haptic trigger verification failed!\r\n");
        return false;
    }
    if !verify_win32_key_synthesizer() {
        crate::serial_write_str("[KNT-PANIC] Win32 key synthesizer verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-VKEYBOARD-PASS] Handheld on-screen virtual keyboard and dual-trackpad typing verified\r\n");
    true
}
