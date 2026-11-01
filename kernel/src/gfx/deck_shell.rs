// Handheld Game Mode UI Compositor & Hardware HUD Overlay (TASK-KNT-316)

// Real-time hardware telemetry, performance HUD, and handheld system metrics.









// Handheld display dynamic refresh rate modes (DRRS).

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]

pub enum DisplayRefreshRate {

    Hz40,

    Hz60,

    Hz120,

}



impl DisplayRefreshRate {

    // Get numeric refresh rate in Hertz.

    pub const fn to_hz(self) -> u32 {

        match self {

            DisplayRefreshRate::Hz40 => 40,

            DisplayRefreshRate::Hz60 => 60,

            DisplayRefreshRate::Hz120 => 120,

        }

    }



    // Get nominal frame time target in milliseconds.

    pub fn frame_time_ms(self) -> f32 {

        match self {

            DisplayRefreshRate::Hz40 => 25.0,

            DisplayRefreshRate::Hz60 => 16.6667,

            DisplayRefreshRate::Hz120 => 8.3333,

        }

    }

}



// Active cooling fan profile for the handheld APU.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]

pub enum FanSpeedCurveMode {

    Quiet,

    Balanced,

    Performance,

    MaxSpeed,

}



// Real-time hardware performance metrics for HUD overlay.

#[derive(Debug, Clone, Copy, PartialEq)]

pub struct HudPerformanceMetrics {

    pub fps: f32,

    pub frame_time_ms: f32,

    pub frame_time_variance_ms: f32,

    pub tdp_watts: u32,

    pub refresh_rate: DisplayRefreshRate,

    pub battery_level_pct: u8,

    pub estimated_runtime_minutes: u16,

    pub fan_curve_mode: FanSpeedCurveMode,

    pub apu_temp_celsius: f32,

}



impl HudPerformanceMetrics {

    // Create new performance telemetry snapshot with default handheld specs.

    pub const fn new() -> Self {

        Self {

            fps: 60.0,

            frame_time_ms: 16.6667,

            frame_time_variance_ms: 0.12,

            tdp_watts: 15,

            refresh_rate: DisplayRefreshRate::Hz60,

            battery_level_pct: 85,

            estimated_runtime_minutes: 180,

            fan_curve_mode: FanSpeedCurveMode::Balanced,

            apu_temp_celsius: 65.0,

        }

    }



    // Validate TDP within handheld boundaries [5W - 30W].

    pub fn is_tdp_safe(&self) -> bool {

        self.tdp_watts >= 5 && self.tdp_watts <= 30

    }

}



// Performance HUD overlay presentation detail level.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]

pub enum HudDetailLevel {

    Off = 0,

    FpsOnly = 1,

    BasicStats = 2,

    DetailedGraph = 3,

    FullDiagnostics = 4,

}



// Handheld Quick Settings Overlay menu state.

#[derive(Debug, Clone)]

pub struct QuickSettingsOverlay {

    pub visible: bool,

    pub hud_level: HudDetailLevel,

    pub target_tdp_watts: u32,

    pub refresh_rate: DisplayRefreshRate,

    pub fan_curve_mode: FanSpeedCurveMode,

    pub acrylic_blur_radius: u32,

    pub background_alpha: f32,

}



impl QuickSettingsOverlay {

    // Construct default Quick Settings sidebar overlay.

    pub const fn new() -> Self {

        Self {

            visible: false,

            hud_level: HudDetailLevel::FpsOnly,

            target_tdp_watts: 15,

            refresh_rate: DisplayRefreshRate::Hz60,

            fan_curve_mode: FanSpeedCurveMode::Balanced,

            acrylic_blur_radius: 12,

            background_alpha: 0.85,

        }

    }



    // Toggle quick settings menu visibility.

    pub fn toggle_visibility(&mut self) -> bool {

        self.visible = !self.visible;

        self.visible

    }



    // Set APU thermal design power in watts within safe boundaries [5W - 30W].

    pub fn set_tdp_watts(&mut self, tdp: u32) -> bool {

        if (5..=30).contains(&tdp) {

            self.target_tdp_watts = tdp;

            true

        } else {

            false

        }

    }



    // Update dynamic refresh rate mode.

    pub fn set_refresh_rate(&mut self, rate: DisplayRefreshRate) {

        self.refresh_rate = rate;

    }



    // Cycle HUD presentation level.

    pub fn cycle_hud_level(&mut self) -> HudDetailLevel {

        self.hud_level = match self.hud_level {

            HudDetailLevel::Off => HudDetailLevel::FpsOnly,

            HudDetailLevel::FpsOnly => HudDetailLevel::BasicStats,

            HudDetailLevel::BasicStats => HudDetailLevel::DetailedGraph,

            HudDetailLevel::DetailedGraph => HudDetailLevel::FullDiagnostics,

            HudDetailLevel::FullDiagnostics => HudDetailLevel::Off,

        };

        self.hud_level

    }

}

