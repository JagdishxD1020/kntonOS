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

/// Maximum frame duration history preserved for statistical aggregation.
pub const ROLLING_FRAME_BUFFER_CAP: usize = 60;

/// Rolling 60-frame latency tracker calculating instantaneous FPS, min, avg, max, and 99th percentile.
#[derive(Debug, Clone, Copy)]
pub struct RollingFrameStats {
    pub frame_times_us: [u32; ROLLING_FRAME_BUFFER_CAP],
    pub head: usize,
    pub count: usize,
    pub last_frame_us: u32,
    pub min_us: u32,
    pub avg_us: u32,
    pub max_us: u32,
    pub p99_us: u32,
    pub instantaneous_fps: u32,
}

impl RollingFrameStats {
    /// Creates an initialized frame latency tracker.
    pub fn new() -> Self {
        Self {
            frame_times_us: [16_666; ROLLING_FRAME_BUFFER_CAP],
            head: 0,
            count: 0,
            last_frame_us: 16_666,
            min_us: 16_666,
            avg_us: 16_666,
            max_us: 16_666,
            p99_us: 16_666,
            instantaneous_fps: 60,
        }
    }

    /// Appends a new frame presentation duration in microseconds and recalculates statistics.
    pub fn push_frame_time(&mut self, us: u32) {
        let valid_us = us.max(1);
        self.frame_times_us[self.head] = valid_us;
        self.head = (self.head + 1) % ROLLING_FRAME_BUFFER_CAP;
        if self.count < ROLLING_FRAME_BUFFER_CAP {
            self.count += 1;
        }
        self.last_frame_us = valid_us;
        self.instantaneous_fps = (1_000_000 / valid_us).min(999);
        self.compute_statistics();
    }

    /// Computes minimum, average, maximum, and 99th percentile frame latency over the rolling window.
    pub fn compute_statistics(&mut self) {
        if self.count == 0 {
            return;
        }

        let mut sorted = [0u32; ROLLING_FRAME_BUFFER_CAP];
        let mut sum: u64 = 0;
        let mut min_val = u32::MAX;
        let mut max_val = 0u32;

        for i in 0..self.count {
            let val = self.frame_times_us[i];
            sorted[i] = val;
            sum += val as u64;
            if val < min_val {
                min_val = val;
            }
            if val > max_val {
                max_val = val;
            }
        }

        // In-place insertion sort over the active sample window
        for i in 1..self.count {
            let key = sorted[i];
            let mut j = i;
            while j > 0 && sorted[j - 1] > key {
                sorted[j] = sorted[j - 1];
                j -= 1;
            }
            sorted[j] = key;
        }

        self.min_us = min_val;
        self.max_us = max_val;
        self.avg_us = (sum / self.count as u64) as u32;

        // 99th percentile index for the active count
        let p99_idx = ((self.count * 99 + 99) / 100).saturating_sub(1).min(self.count - 1);
        self.p99_us = sorted[p99_idx];
    }
}

/// Instantaneous battery, power draw, and active cooling status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThermalPowerStatus {
    pub battery_pct: u8,
    pub is_charging: bool,
    pub power_draw_mw: u32,
    pub peltier_active: bool,
    pub fan_rpm: u16,
    pub soc_temp_celsius: u8,
    pub cpu_load_pct: u8,
    pub gpu_vram_used_mb: u32,
    pub gpu_vram_total_mb: u32,
}

impl Default for ThermalPowerStatus {
    fn default() -> Self {
        Self {
            battery_pct: 85,
            is_charging: false,
            power_draw_mw: 14_500,
            peltier_active: true,
            fan_rpm: 3200,
            soc_temp_celsius: 54,
            cpu_load_pct: 42,
            gpu_vram_used_mb: 2850,
            gpu_vram_total_mb: 8192,
        }
    }
}

/// Visual presentation detail level for the performance telemetry overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudOverlayMode {
    Off,
    CompactFps,
    Standard,
    DetailedDiagnostics,
}

/// Real-time performance telemetry overlay manager.
#[derive(Debug, Clone)]
pub struct PerformanceTelemetryHud {
    pub stats: RollingFrameStats,
    pub hardware: ThermalPowerStatus,
    pub mode: HudOverlayMode,
    pub alpha: u8,
}

impl PerformanceTelemetryHud {
    /// Creates a performance HUD overlay with standard layout and default telemetry.
    pub fn new() -> Self {
        Self {
            stats: RollingFrameStats::new(),
            hardware: ThermalPowerStatus::default(),
            mode: HudOverlayMode::Standard,
            alpha: 210,
        }
    }

    /// Records a new frame duration in microseconds into the rolling statistics engine.
    pub fn push_frame_time(&mut self, us: u32) {
        self.stats.push_frame_time(us);
    }

    /// Cycles the HUD overlay visibility mode through Off, Compact, Standard, and Detailed.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            HudOverlayMode::Off => HudOverlayMode::CompactFps,
            HudOverlayMode::CompactFps => HudOverlayMode::Standard,
            HudOverlayMode::Standard => HudOverlayMode::DetailedDiagnostics,
            HudOverlayMode::DetailedDiagnostics => HudOverlayMode::Off,
        };
    }

    /// Calculates the percentage of allocated GPU video memory.
    pub fn vram_utilization_pct(&self) -> u8 {
        if self.hardware.gpu_vram_total_mb == 0 {
            0
        } else {
            ((self.hardware.gpu_vram_used_mb as u64 * 100) / self.hardware.gpu_vram_total_mb as u64) as u8
        }
    }
}
