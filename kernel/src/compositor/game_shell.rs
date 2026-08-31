//! Handheld Gaming Shell, Game Launcher Carousel & Performance HUD Overlay.
//! Fullscreen gaming shell state machine and controller navigation engine.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use crate::display::color::Color;

/// Operational state of a game title in the handheld shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameLaunchState {
    Idle,
    Launching { progress: u8 },
    Running { pid: u32 },
    Terminated { exit_code: i32 },
}

/// Metadata and visual properties of a single playable game title.
#[derive(Debug, Clone)]
pub struct GameEntry {
    pub title: String,
    pub executable: String,
    pub developer: String,
    pub banner_width: usize,
    pub banner_height: usize,
    pub theme_color: Color,
    pub launch_state: GameLaunchState,
}

impl GameEntry {
    /// Creates a new game entry in the idle state.
    pub fn new(
        title: &str,
        executable: &str,
        developer: &str,
        banner_width: usize,
        banner_height: usize,
        theme_color: Color,
    ) -> Self {
        Self {
            title: String::from(title),
            executable: String::from(executable),
            developer: String::from(developer),
            banner_width,
            banner_height,
            theme_color,
            launch_state: GameLaunchState::Idle,
        }
    }
}

/// Fullscreen game launcher carousel state machine.
#[derive(Debug, Clone)]
pub struct GameShellCarousel {
    pub entries: Vec<GameEntry>,
    pub selected_index: usize,
    pub is_shell_active: bool,
    pub active_pid: Option<u32>,
    pub next_synthetic_pid: u32,
}

impl GameShellCarousel {
    /// Creates an empty carousel instance.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected_index: 0,
            is_shell_active: true,
            active_pid: None,
            next_synthetic_pid: 100,
        }
    }

    /// Adds a game title to the carousel list.
    pub fn add_game(&mut self, entry: GameEntry) {
        self.entries.push(entry);
    }

    /// Selects the next game entry in the carousel with circular wrapping.
    pub fn select_next(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.entries.len();
    }

    /// Selects the previous game entry in the carousel with circular wrapping.
    pub fn select_prev(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = self.entries.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }

    /// Triggers game launch for the currently selected carousel entry.
    pub fn trigger_launch(&mut self) -> Option<u32> {
        if self.entries.is_empty() {
            return None;
        }
        let pid = self.next_synthetic_pid;
        self.next_synthetic_pid = self.next_synthetic_pid.wrapping_add(1);

        let entry = &mut self.entries[self.selected_index];
        entry.launch_state = GameLaunchState::Running { pid };
        self.active_pid = Some(pid);
        self.is_shell_active = false;
        Some(pid)
    }

    /// Terminates the active running game session and restores shell focus.
    pub fn terminate_active_game(&mut self, exit_code: i32) {
        if let Some(pid) = self.active_pid.take() {
            for entry in &mut self.entries {
                if let GameLaunchState::Running { pid: cur_pid } = entry.launch_state {
                    if cur_pid == pid {
                        entry.launch_state = GameLaunchState::Terminated { exit_code };
                        break;
                    }
                }
            }
        }
        self.is_shell_active = true;
    }

    /// Returns a reference to the currently selected game entry.
    pub fn selected_game(&self) -> Option<&GameEntry> {
        self.entries.get(self.selected_index)
    }
}

/// Controller button bitflags for handheld input mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerButtons(pub u16);

impl ControllerButtons {
    pub const DPAD_UP: u16 = 1 << 0;
    pub const DPAD_DOWN: u16 = 1 << 1;
    pub const DPAD_LEFT: u16 = 1 << 2;
    pub const DPAD_RIGHT: u16 = 1 << 3;
    pub const BUTTON_A: u16 = 1 << 4;
    pub const BUTTON_B: u16 = 1 << 5;
    pub const BUTTON_X: u16 = 1 << 6;
    pub const BUTTON_Y: u16 = 1 << 7;
    pub const BUMPER_L: u16 = 1 << 8;
    pub const BUMPER_R: u16 = 1 << 9;
    pub const THUMB_L: u16 = 1 << 10;
    pub const THUMB_R: u16 = 1 << 11;
    pub const GUIDE: u16 = 1 << 12;
    pub const START: u16 = 1 << 13;
    pub const BACK: u16 = 1 << 14;

    #[inline(always)]
    pub fn contains(&self, flag: u16) -> bool {
        (self.0 & flag) == flag
    }
}

/// Instantaneous snapshot of handheld gamepad hardware controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerState {
    pub buttons: ControllerButtons,
    pub left_stick_x: i16,
    pub left_stick_y: i16,
    pub right_stick_x: i16,
    pub right_stick_y: i16,
    pub left_trigger: u8,
    pub right_trigger: u8,
}

impl Default for ControllerState {
    fn default() -> Self {
        Self {
            buttons: ControllerButtons(0),
            left_stick_x: 0,
            left_stick_y: 0,
            right_stick_x: 0,
            right_stick_y: 0,
            left_trigger: 0,
            right_trigger: 0,
        }
    }
}

/// Action produced by processing controller navigation input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavAction {
    None,
    NavigatedLeft,
    NavigatedRight,
    LaunchedGame(u32),
    DismissedOrBack,
    ToggledHud,
    ToggledShell,
}

/// Controller navigation processor with thumbstick deadzones and repeat debouncing.
#[derive(Debug, Clone)]
pub struct ControllerNavHandler {
    pub stick_deadzone: i16,
    pub repeat_delay_ms: u32,
    pub repeat_rate_ms: u32,
    pub hold_duration_ms: u32,
    pub last_direction: i8,
    pub prev_buttons: u16,
}

impl ControllerNavHandler {
    /// Creates a controller navigation processor with default deadzone and repeat timings.
    pub fn new() -> Self {
        Self {
            stick_deadzone: 8000,
            repeat_delay_ms: 350,
            repeat_rate_ms: 120,
            hold_duration_ms: 0,
            last_direction: 0,
            prev_buttons: 0,
        }
    }

    /// Evaluates controller state, updates debouncing clocks, and drives carousel navigation.
    pub fn process_input(
        &mut self,
        input: &ControllerState,
        delta_ms: u32,
        carousel: &mut GameShellCarousel,
    ) -> NavAction {
        let pressed_buttons = input.buttons.0 & !self.prev_buttons;
        self.prev_buttons = input.buttons.0;

        // Toggle shell with Guide button
        if (pressed_buttons & ControllerButtons::GUIDE) != 0 {
            carousel.is_shell_active = !carousel.is_shell_active;
            return NavAction::ToggledShell;
        }

        // Toggle performance HUD with Back button
        if (pressed_buttons & ControllerButtons::BACK) != 0 {
            return NavAction::ToggledHud;
        }

        // Launch game on A button press
        if (pressed_buttons & ControllerButtons::BUTTON_A) != 0 {
            if carousel.is_shell_active {
                if let Some(pid) = carousel.trigger_launch() {
                    return NavAction::LaunchedGame(pid);
                }
            }
        }

        // Dismiss or back on B button press
        if (pressed_buttons & ControllerButtons::BUTTON_B) != 0 {
            if !carousel.is_shell_active && carousel.active_pid.is_some() {
                carousel.terminate_active_game(0);
                return NavAction::DismissedOrBack;
            }
        }

        // Determine horizontal navigation intent from D-pad and analog thumbstick
        let mut dir: i8 = 0;
        if input.buttons.contains(ControllerButtons::DPAD_LEFT) || input.left_stick_x < -self.stick_deadzone {
            dir = -1;
        } else if input.buttons.contains(ControllerButtons::DPAD_RIGHT) || input.left_stick_x > self.stick_deadzone {
            dir = 1;
        }

        if dir == 0 {
            self.last_direction = 0;
            self.hold_duration_ms = 0;
            return NavAction::None;
        }

        // Handle navigation repeat logic
        let should_step = if self.last_direction != dir {
            self.last_direction = dir;
            self.hold_duration_ms = 0;
            true
        } else {
            self.hold_duration_ms = self.hold_duration_ms.saturating_add(delta_ms);
            if self.hold_duration_ms >= self.repeat_delay_ms {
                let repeat_phase = self.hold_duration_ms - self.repeat_delay_ms;
                if repeat_phase % self.repeat_rate_ms < delta_ms {
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        if should_step && carousel.is_shell_active {
            if dir < 0 {
                carousel.select_prev();
                NavAction::NavigatedLeft
            } else {
                carousel.select_next();
                NavAction::NavigatedRight
            }
        } else {
            NavAction::None
        }
    }
}
