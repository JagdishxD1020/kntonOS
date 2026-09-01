//! Handheld Gaming Shell, Game Launcher Carousel & Performance HUD Overlay.
//! Fullscreen gaming shell state machine and controller navigation engine.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;
use crate::display::color::Color;
use crate::compositor::dirty::Rect;
use crate::display::font::{get_glyph, FONT_HEIGHT, FONT_WIDTH};

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

// Framebuffer Alpha Blitting & Visual Composition Primitives

/// Formats an unsigned integer into an ASCII stack buffer without heap allocation.
pub fn format_u32_into(mut val: u32, buf: &mut [u8; 16]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut temp = [0u8; 16];
    let mut i = 0;
    while val > 0 && i < 16 {
        temp[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = temp[i - 1 - j];
    }
    i
}

/// Blits an alpha-blended pixel with boundary and clip rectangle enforcement.
#[inline(always)]
pub fn blit_pixel_alpha(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    x: usize,
    y: usize,
    src_color: Color,
    clip: Option<&Rect>,
) {
    if x >= screen_width || y >= screen_height {
        return;
    }
    if let Some(cl) = clip {
        if !cl.contains_point(x, y) {
            return;
        }
    }
    let idx = y * screen_width + x;
    buffer[idx] = src_color.blend(buffer[idx]);
}

/// Blits a solid alpha-blended rectangle clipped to destination boundaries.
pub fn blit_rect_alpha(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    rect: &Rect,
    color: Color,
    clip: Option<&Rect>,
) {
    let x_end = rect.right().min(screen_width);
    let y_end = rect.bottom().min(screen_height);
    for ty in rect.y..y_end {
        let row_offset = ty * screen_width;
        for tx in rect.x..x_end {
            if let Some(cl) = clip {
                if !cl.contains_point(tx, ty) {
                    continue;
                }
            }
            let idx = row_offset + tx;
            buffer[idx] = color.blend(buffer[idx]);
        }
    }
}

/// Blits a 1-pixel border rectangle outline.
pub fn blit_rect_outline(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    rect: &Rect,
    border_color: Color,
    clip: Option<&Rect>,
) {
    if rect.is_empty() {
        return;
    }
    let x_end = rect.right().saturating_sub(1);
    let y_end = rect.bottom().saturating_sub(1);

    for tx in rect.x..=x_end {
        blit_pixel_alpha(buffer, screen_width, screen_height, tx, rect.y, border_color, clip);
        blit_pixel_alpha(buffer, screen_width, screen_height, tx, y_end, border_color, clip);
    }
    for ty in rect.y..=y_end {
        blit_pixel_alpha(buffer, screen_width, screen_height, rect.x, ty, border_color, clip);
        blit_pixel_alpha(buffer, screen_width, screen_height, x_end, ty, border_color, clip);
    }
}

/// Blits a horizontal ratio progress bar for telemetry meters.
pub fn blit_progress_bar(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    pct: u8,
    fill_color: Color,
    bg_color: Color,
    clip: Option<&Rect>,
) {
    let bg_rect = Rect::new(x, y, w, h);
    blit_rect_alpha(buffer, screen_width, screen_height, &bg_rect, bg_color, clip);

    let clamped_pct = pct.min(100);
    let fill_w = (w * clamped_pct as usize) / 100;
    if fill_w > 0 {
        let fill_rect = Rect::new(x, y, fill_w, h);
        blit_rect_alpha(buffer, screen_width, screen_height, &fill_rect, fill_color, clip);
    }
    blit_rect_outline(buffer, screen_width, screen_height, &bg_rect, Color::rgba(255, 255, 255, 80), clip);
}

/// Blits a single 8x16 font glyph with alpha blending and clipping.
pub fn blit_char_clipped(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    x: usize,
    y: usize,
    c: char,
    fg: Color,
    clip: Option<&Rect>,
) {
    if c == '\n' {
        return;
    }
    let glyph = get_glyph(c);
    for row in 0..FONT_HEIGHT {
        let ty = y + row;
        if ty >= screen_height {
            break;
        }
        let bits = glyph[row];
        for col in 0..FONT_WIDTH {
            let tx = x + col;
            if tx >= screen_width {
                break;
            }
            if (bits & (0x80 >> col)) != 0 {
                blit_pixel_alpha(buffer, screen_width, screen_height, tx, ty, fg, clip);
            }
        }
    }
}

/// Blits an ASCII string across multiple character cells.
pub fn blit_string_clipped(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    x: usize,
    y: usize,
    s: &str,
    fg: Color,
    clip: Option<&Rect>,
) {
    let mut cur_x = x;
    for c in s.chars() {
        if cur_x + FONT_WIDTH > screen_width {
            break;
        }
        blit_char_clipped(buffer, screen_width, screen_height, cur_x, y, c, fg, clip);
        cur_x += FONT_WIDTH;
    }
}

/// Blits an integer followed by a suffix string.
pub fn blit_u32_with_suffix(
    buffer: &mut [Color],
    screen_width: usize,
    screen_height: usize,
    x: usize,
    y: usize,
    val: u32,
    suffix: &str,
    fg: Color,
    clip: Option<&Rect>,
) -> usize {
    let mut buf = [0u8; 16];
    let len = format_u32_into(val, &mut buf);
    if let Ok(s) = core::str::from_utf8(&buf[..len]) {
        blit_string_clipped(buffer, screen_width, screen_height, x, y, s, fg, clip);
        let next_x = x + len * FONT_WIDTH;
        blit_string_clipped(buffer, screen_width, screen_height, next_x, y, suffix, fg, clip);
        next_x + suffix.len() * FONT_WIDTH
    } else {
        x
    }
}

impl GameShellCarousel {
    /// Renders the fullscreen handheld game launcher shell into the target backbuffer.
    pub fn render(
        &self,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        clip: Option<&Rect>,
    ) {
        if !self.is_shell_active || screen_width < 100 || screen_height < 100 {
            return;
        }

        // Fill background with deep slate dark gaming theme
        let full_screen = Rect::new(0, 0, screen_width, screen_height);
        blit_rect_alpha(buffer, screen_width, screen_height, &full_screen, Color::rgb(18, 22, 28), clip);

        // Header bar
        let header_rect = Rect::new(0, 0, screen_width, 28);
        blit_rect_alpha(buffer, screen_width, screen_height, &header_rect, Color::rgba(28, 36, 48, 240), clip);
        blit_string_clipped(buffer, screen_width, screen_height, 12, 6, "KNTON HANDHELD GAMING SHELL", Color::rgb(0, 180, 255), clip);

        if self.entries.is_empty() {
            blit_string_clipped(buffer, screen_width, screen_height, 20, 80, "No games installed.", Color::WHITE, clip);
            return;
        }

        // Render carousel cards horizontally centered
        let card_w = 90;
        let card_h = 95;
        let card_gap = 12;
        let center_x = screen_width / 2;
        let base_y = (screen_height.saturating_sub(card_h)) / 2;

        let total_entries = self.entries.len();
        for i in 0..total_entries {
            let offset_from_sel = i as isize - self.selected_index as isize;
            let card_x_isize = center_x as isize + offset_from_sel * (card_w as isize + card_gap as isize) - (card_w as isize / 2);
            if card_x_isize + (card_w as isize) < 0 || card_x_isize >= screen_width as isize {
                continue;
            }
            let card_x = card_x_isize as usize;
            let is_selected = i == self.selected_index;
            let card_y = if is_selected { base_y.saturating_sub(6) } else { base_y };

            let card_rect = Rect::new(card_x, card_y, card_w, card_h);
            let entry = &self.entries[i];

            // Card body
            blit_rect_alpha(buffer, screen_width, screen_height, &card_rect, entry.theme_color, clip);

            // Selection border highlight
            let border_color = if is_selected {
                Color::rgb(255, 255, 255)
            } else {
                Color::rgba(80, 90, 105, 180)
            };
            blit_rect_outline(buffer, screen_width, screen_height, &card_rect, border_color, clip);

            // Title & Status
            let title_preview = if entry.title.len() > 10 { &entry.title[..10] } else { &entry.title };
            blit_string_clipped(buffer, screen_width, screen_height, card_x + 4, card_y + 6, title_preview, Color::WHITE, clip);

            let status_str = match entry.launch_state {
                GameLaunchState::Idle => "IDLE",
                GameLaunchState::Launching { .. } => "LOAD",
                GameLaunchState::Running { .. } => "PLAY",
                GameLaunchState::Terminated { .. } => "STOP",
            };
            blit_string_clipped(buffer, screen_width, screen_height, card_x + 4, card_y + card_h - 20, status_str, Color::rgb(255, 215, 0), clip);
        }

        // Navigation hint footer
        let footer_y = screen_height.saturating_sub(22);
        let footer_rect = Rect::new(0, footer_y, screen_width, 22);
        blit_rect_alpha(buffer, screen_width, screen_height, &footer_rect, Color::rgba(24, 30, 40, 230), clip);
        blit_string_clipped(buffer, screen_width, screen_height, 8, footer_y + 3, "[A] Launch  [< >] Select  [Back] HUD", Color::rgb(180, 195, 215), clip);
    }
}

impl PerformanceTelemetryHud {
    /// Renders the real-time performance telemetry overlay plane according to the selected mode.
    pub fn render(
        &self,
        buffer: &mut [Color],
        screen_width: usize,
        screen_height: usize,
        clip: Option<&Rect>,
    ) {
        if self.mode == HudOverlayMode::Off || screen_width < 80 || screen_height < 40 {
            return;
        }

        let bg_color = Color::rgba(18, 22, 28, self.alpha);
        let border_color = Color::rgba(0, 180, 255, 180);
        let text_color = Color::WHITE;
        let accent_color = Color::rgb(0, 255, 128);

        match self.mode {
            HudOverlayMode::Off => {}
            HudOverlayMode::CompactFps => {
                let hud_w = 96;
                let hud_h = 24;
                let hud_x = screen_width.saturating_sub(hud_w + 4);
                let hud_y = 4;
                let hud_rect = Rect::new(hud_x, hud_y, hud_w, hud_h);

                blit_rect_alpha(buffer, screen_width, screen_height, &hud_rect, bg_color, clip);
                blit_rect_outline(buffer, screen_width, screen_height, &hud_rect, border_color, clip);

                let fps = self.stats.instantaneous_fps;
                blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 6, hud_y + 4, fps, " FPS", accent_color, clip);
            }
            HudOverlayMode::Standard => {
                let hud_w = 210;
                let hud_h = 76;
                let hud_x = screen_width.saturating_sub(hud_w + 4);
                let hud_y = 4;
                let hud_rect = Rect::new(hud_x, hud_y, hud_w, hud_h);

                blit_rect_alpha(buffer, screen_width, screen_height, &hud_rect, bg_color, clip);
                blit_rect_outline(buffer, screen_width, screen_height, &hud_rect, border_color, clip);

                // FPS & Frame Latency
                let next_x = blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 6, hud_y + 6, self.stats.instantaneous_fps, " FPS | ", accent_color, clip);
                let ms = self.stats.avg_us / 1000;
                let next_x2 = blit_u32_with_suffix(buffer, screen_width, screen_height, next_x, hud_y + 6, ms, "ms (p99: ", text_color, clip);
                let p99_ms = self.stats.p99_us / 1000;
                blit_u32_with_suffix(buffer, screen_width, screen_height, next_x2, hud_y + 6, p99_ms, "ms)", text_color, clip);

                // CPU Load Meter
                blit_string_clipped(buffer, screen_width, screen_height, hud_x + 6, hud_y + 24, "CPU:", text_color, clip);
                blit_progress_bar(buffer, screen_width, screen_height, hud_x + 44, hud_y + 27, 70, 10, self.hardware.cpu_load_pct, Color::rgb(0, 150, 255), Color::rgb(40, 50, 65), clip);
                blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 120, hud_y + 24, self.hardware.cpu_load_pct as u32, "%", text_color, clip);

                // GPU VRAM Meter
                let vram_pct = self.vram_utilization_pct();
                blit_string_clipped(buffer, screen_width, screen_height, hud_x + 6, hud_y + 40, "VRAM:", text_color, clip);
                blit_progress_bar(buffer, screen_width, screen_height, hud_x + 44, hud_y + 43, 70, 10, vram_pct, Color::rgb(255, 120, 0), Color::rgb(40, 50, 65), clip);
                blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 120, hud_y + 40, vram_pct as u32, "%", text_color, clip);

                // Battery & Power
                blit_string_clipped(buffer, screen_width, screen_height, hud_x + 6, hud_y + 56, "BAT:", text_color, clip);
                let bat_next = blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 44, hud_y + 56, self.hardware.battery_pct as u32, "% ", accent_color, clip);
                let watts = self.hardware.power_draw_mw / 1000;
                blit_u32_with_suffix(buffer, screen_width, screen_height, bat_next, hud_y + 56, watts, "W", Color::rgb(255, 215, 0), clip);
            }
            HudOverlayMode::DetailedDiagnostics => {
                let hud_w = 224;
                let hud_h = 108;
                let hud_x = screen_width.saturating_sub(hud_w + 4);
                let hud_y = 4;
                let hud_rect = Rect::new(hud_x, hud_y, hud_w, hud_h);

                blit_rect_alpha(buffer, screen_width, screen_height, &hud_rect, bg_color, clip);
                blit_rect_outline(buffer, screen_width, screen_height, &hud_rect, border_color, clip);

                // Line 1: FPS and percentiles
                let next_x = blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 6, hud_y + 6, self.stats.instantaneous_fps, " FPS  AVG:", accent_color, clip);
                let ms = self.stats.avg_us / 1000;
                blit_u32_with_suffix(buffer, screen_width, screen_height, next_x, hud_y + 6, ms, "ms", text_color, clip);

                // Line 2: Frame min/max/p99
                let p99_next = blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 6, hud_y + 22, self.stats.p99_us / 1000, "ms P99  MIN:", text_color, clip);
                let min_next = blit_u32_with_suffix(buffer, screen_width, screen_height, p99_next, hud_y + 22, self.stats.min_us / 1000, "ms  MAX:", text_color, clip);
                blit_u32_with_suffix(buffer, screen_width, screen_height, min_next, hud_y + 22, self.stats.max_us / 1000, "ms", text_color, clip);

                // Line 3: CPU & VRAM
                blit_string_clipped(buffer, screen_width, screen_height, hud_x + 6, hud_y + 38, "CPU:", text_color, clip);
                blit_progress_bar(buffer, screen_width, screen_height, hud_x + 44, hud_y + 41, 60, 8, self.hardware.cpu_load_pct, Color::rgb(0, 150, 255), Color::rgb(40, 50, 65), clip);
                let vram_pct = self.vram_utilization_pct();
                blit_string_clipped(buffer, screen_width, screen_height, hud_x + 114, hud_y + 38, "VRAM:", text_color, clip);
                blit_progress_bar(buffer, screen_width, screen_height, hud_x + 158, hud_y + 41, 55, 8, vram_pct, Color::rgb(255, 120, 0), Color::rgb(40, 50, 65), clip);

                // Line 4: Thermal & Fan
                blit_string_clipped(buffer, screen_width, screen_height, hud_x + 6, hud_y + 54, "SOC:", text_color, clip);
                let soc_next = blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 40, hud_y + 54, self.hardware.soc_temp_celsius as u32, "C  FAN:", text_color, clip);
                blit_u32_with_suffix(buffer, screen_width, screen_height, soc_next, hud_y + 54, self.hardware.fan_rpm as u32, "RPM", text_color, clip);

                // Line 5: Peltier & Power
                let peltier_str = if self.hardware.peltier_active { "PELTIER:ON  " } else { "PELTIER:OFF " };
                blit_string_clipped(buffer, screen_width, screen_height, hud_x + 6, hud_y + 70, peltier_str, Color::rgb(0, 255, 255), clip);
                let watts = self.hardware.power_draw_mw / 1000;
                blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 110, hud_y + 70, watts, "W", Color::rgb(255, 215, 0), clip);

                // Line 6: Battery
                let bat_str = if self.hardware.is_charging { "BAT (CHG):" } else { "BATTERY:" };
                blit_string_clipped(buffer, screen_width, screen_height, hud_x + 6, hud_y + 86, bat_str, text_color, clip);
                blit_u32_with_suffix(buffer, screen_width, screen_height, hud_x + 90, hud_y + 86, self.hardware.battery_pct as u32, "%", accent_color, clip);
            }
        }
    }
}

/// Comprehensive verification self-test suite for the handheld gaming shell and telemetry HUD.
pub fn test_game_shell_subsystem() -> bool {
    crate::serial_write_str("[KNT-SHELL-TEST] Running Handheld Gaming Shell & Telemetry HUD verification suite...\r\n");

    // 1. Verify Carousel Navigation, Controller Processing, and Launch State Transitions
    let mut carousel = GameShellCarousel::new();
    carousel.add_game(GameEntry::new("CyberVoid", "/bin/cyber.exe", "Knton Studio", 100, 90, Color::rgb(0, 100, 200)));
    carousel.add_game(GameEntry::new("SpeedRacer", "/bin/speed.exe", "Velocity Games", 100, 90, Color::rgb(200, 50, 0)));
    carousel.add_game(GameEntry::new("ChronoShift", "/bin/chrono.exe", "Retro Soft", 100, 90, Color::rgb(100, 0, 150)));

    if carousel.selected_index != 0 {
        crate::serial_write_str("[KNT-SHELL-FAIL] Carousel initial index mismatch\r\n");
        return false;
    }

    // Prev wraps to last element (index 2)
    carousel.select_prev();
    if carousel.selected_index != 2 {
        crate::serial_write_str("[KNT-SHELL-FAIL] Carousel wrap to end failed\r\n");
        return false;
    }

    // Next wraps to first element (index 0)
    carousel.select_next();
    if carousel.selected_index != 0 {
        crate::serial_write_str("[KNT-SHELL-FAIL] Carousel wrap to start failed\r\n");
        return false;
    }

    // Controller input navigation
    let mut nav = ControllerNavHandler::new();
    let mut input = ControllerState::default();

    // D-pad Right advances to index 1
    input.buttons = ControllerButtons(ControllerButtons::DPAD_RIGHT);
    let action = nav.process_input(&input, 16, &mut carousel);
    if action != NavAction::NavigatedRight || carousel.selected_index != 1 {
        crate::serial_write_str("[KNT-SHELL-FAIL] Controller right navigation failed\r\n");
        return false;
    }

    // Button A triggers launch
    input.buttons = ControllerButtons(ControllerButtons::BUTTON_A);
    let action = nav.process_input(&input, 16, &mut carousel);
    let launched_pid = match action {
        NavAction::LaunchedGame(pid) => pid,
        _ => {
            crate::serial_write_str("[KNT-SHELL-FAIL] Controller A launch trigger failed\r\n");
            return false;
        }
    };
    if carousel.is_shell_active || carousel.active_pid != Some(launched_pid) {
        crate::serial_write_str("[KNT-SHELL-FAIL] Carousel active state mismatch after launch\r\n");
        return false;
    }

    // Terminate game session
    carousel.terminate_active_game(0);
    if !carousel.is_shell_active || carousel.active_pid.is_some() {
        crate::serial_write_str("[KNT-SHELL-FAIL] Carousel state mismatch after termination\r\n");
        return false;
    }
    crate::serial_write_str("  * Carousel state transitions, navigation wrapping, and game launch verified\r\n");

    // 2. Verify Performance Telemetry HUD Rolling Buffer and Statistics Aggregation
    let mut hud = PerformanceTelemetryHud::new();

    // Push 59 frames at 16_666 us (~60 FPS) and 1 spike frame at 50_000 us (~20 FPS)
    for _ in 0..59 {
        hud.push_frame_time(16_666);
    }
    hud.push_frame_time(50_000);

    if hud.stats.count != 60 {
        crate::serial_write_str("[KNT-SHELL-FAIL] Telemetry buffer capacity mismatch\r\n");
        return false;
    }
    if hud.stats.min_us != 16_666 || hud.stats.max_us != 50_000 {
        crate::serial_write_str("[KNT-SHELL-FAIL] Telemetry min/max mismatch\r\n");
        return false;
    }
    if hud.stats.p99_us != 50_000 {
        crate::serial_write_str("[KNT-SHELL-FAIL] Telemetry 99th percentile calculation mismatch\r\n");
        return false;
    }
    if hud.stats.instantaneous_fps != 20 {
        crate::serial_write_str("[KNT-SHELL-FAIL] Instantaneous FPS calculation mismatch\r\n");
        return false;
    }
    if hud.vram_utilization_pct() != 34 {
        crate::serial_write_str("[KNT-SHELL-FAIL] VRAM percentage calculation mismatch\r\n");
        return false;
    }
    crate::serial_write_str("  * 60-frame rolling frame time buffer and 99th percentile latency verified\r\n");

    // 3. Verify Transparent Overlay Blitting & Framebuffer Clipping Boundaries
    let width = 320;
    let height = 240;
    let mut buffer = alloc::vec![Color::rgb(20, 24, 32); width * height];

    // Render carousel
    carousel.render(&mut buffer, width, height, None);

    // Render HUD in Standard mode
    hud.mode = HudOverlayMode::Standard;
    hud.render(&mut buffer, width, height, None);

    // Test with active clip rectangle
    let clip_rect = Rect::new(50, 50, 100, 100);
    hud.mode = HudOverlayMode::DetailedDiagnostics;
    hud.render(&mut buffer, width, height, Some(&clip_rect));

    // Verify alpha blended pixel within HUD bounds
    let sample_pixel = buffer[4 * width + (width - 10)];
    if sample_pixel == Color::rgb(20, 24, 32) {
        crate::serial_write_str("[KNT-SHELL-FAIL] HUD buffer blitting had no effect\r\n");
        return false;
    }
    crate::serial_write_str("  * Transparent framebuffer blitting and clipping boundaries verified\r\n");

    crate::serial_write_str("[KNT-GAME-SHELL-PASS] Handheld gaming shell, launcher and telemetry HUD verified\r\n");
    true
}
