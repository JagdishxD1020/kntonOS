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





// 32-bit RGBA color representation for composition and alpha blending.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]

pub struct ShellColorRgba {

    pub r: u8,

    pub g: u8,

    pub b: u8,

    pub a: u8,

}



impl ShellColorRgba {

    // Construct RGBA color.

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {

        Self { r, g, b, a }

    }



    // Blend source color over destination using standard non-premultiplied alpha blending.

    pub fn blend_over(self, dst: Self) -> Self {

        let src_a = self.a as u32;

        let dst_a = dst.a as u32;

        let out_a = src_a + (dst_a * (255 - src_a) + 127) / 255;

        if out_a == 0 {

            return Self::new(0, 0, 0, 0);

        }

        let out_r = ((self.r as u32 * src_a) + (dst.r as u32 * dst_a * (255 - src_a) + 127) / 255) / out_a;

        let out_g = ((self.g as u32 * src_a) + (dst.g as u32 * dst_a * (255 - src_a) + 127) / 255) / out_a;

        let out_b = ((self.b as u32 * src_a) + (dst.b as u32 * dst_a * (255 - src_a) + 127) / 255) / out_a;



        Self::new(

            core::cmp::min(out_r, 255) as u8,

            core::cmp::min(out_g, 255) as u8,

            core::cmp::min(out_b, 255) as u8,

            core::cmp::min(out_a, 255) as u8,

        )

    }

}



// Acrylic glass blur filter simulator for quick-access and HUD backgrounds.

pub struct AcrylicBlurFilter;



impl AcrylicBlurFilter {

    // Apply dual-pass box blur on a 1D pixel row buffer simulating acrylic frost effect.

    pub fn box_blur_row(src: &[ShellColorRgba], dst: &mut [ShellColorRgba], radius: usize) {

        if src.is_empty() || dst.len() < src.len() {

            return;

        }

        let len = src.len();

        let mut i = 0;

        while i < len {

            let start = if i >= radius { i - radius } else { 0 };

            let end = if i + radius < len { i + radius } else { len - 1 };

            let count = (end - start + 1) as u32;



            let mut sum_r: u32 = 0;

            let mut sum_g: u32 = 0;

            let mut sum_b: u32 = 0;

            let mut sum_a: u32 = 0;



            let mut j = start;

            while j <= end {

                sum_r += src[j].r as u32;

                sum_g += src[j].g as u32;

                sum_b += src[j].b as u32;

                sum_a += src[j].a as u32;

                j += 1;

            }



            dst[i] = ShellColorRgba::new(

                (sum_r / count) as u8,

                (sum_g / count) as u8,

                (sum_b / count) as u8,

                (sum_a / count) as u8,

            );

            i += 1;

        }

    }

}



// Compositor surface layer identifier in the visual hierarchy.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]

pub enum CompositorLayerType {

    GameBackground = 0,

    SystemNotification = 1,

    QuickSettingsMenu = 2,

    HardwareHudOverlay = 3,

    ToastMessage = 4,

}



// Surface descriptor representing a compositable handheld visual layer.

#[derive(Debug, Clone, PartialEq)]

pub struct CompositorSurface {

    pub layer_type: CompositorLayerType,

    pub width: u32,

    pub height: u32,

    pub opacity: f32,

    pub visible: bool,

    pub z_order: u32,

}



impl CompositorSurface {

    // Construct new surface.

    pub const fn new(layer_type: CompositorLayerType, width: u32, height: u32, z_order: u32) -> Self {

        Self {

            layer_type,

            width,

            height,

            opacity: 1.0,

            visible: true,

            z_order,

        }

    }

}


// Handheld gamepad button inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandheldGamepadButton {
    A,
    B,
    X,
    Y,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    LeftBumper,
    RightBumper,
    GuideButton,
    QuickAccessButton,
}

// Touchscreen gesture inputs for handheld navigation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HandheldTouchGesture {
    Tap { x: u32, y: u32 },
    SwipeRightEdge,
    SwipeBottomEdge,
}

// Target routing destination for input events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputEventRoutingTarget {
    ActiveGameProcess,
    QuickSettingsOverlay,
    HardwareHud,
    SystemModal,
}

// Handheld input event router and guide button interceptor.
pub struct HandheldInputRouter {
    pub guide_button_pressed: bool,
    pub quick_access_button_pressed: bool,
    pub active_routing_target: InputEventRoutingTarget,
}

impl HandheldInputRouter {
    // Construct default router directing events to the active game process.
    pub const fn new() -> Self {
        Self {
            guide_button_pressed: false,
            quick_access_button_pressed: false,
            active_routing_target: InputEventRoutingTarget::ActiveGameProcess,
        }
    }

    // Route gamepad button press event, intercepting QuickAccess/Guide buttons.
    pub fn route_button_press(
        &mut self,
        button: HandheldGamepadButton,
        quick_settings: &mut QuickSettingsOverlay,
    ) -> InputEventRoutingTarget {
        match button {
            HandheldGamepadButton::QuickAccessButton | HandheldGamepadButton::GuideButton => {
                let now_visible = quick_settings.toggle_visibility();
                self.active_routing_target = if now_visible {
                    InputEventRoutingTarget::QuickSettingsOverlay
                } else {
                    InputEventRoutingTarget::ActiveGameProcess
                };
                self.quick_access_button_pressed = true;
                self.active_routing_target
            }
            _ => {
                if quick_settings.visible {
                    InputEventRoutingTarget::QuickSettingsOverlay
                } else {
                    InputEventRoutingTarget::ActiveGameProcess
                }
            }
        }
    }

    // Route touchscreen gesture, intercepting right edge swipes to open Quick Settings.
    pub fn route_touch_gesture(
        &mut self,
        gesture: HandheldTouchGesture,
        quick_settings: &mut QuickSettingsOverlay,
    ) -> InputEventRoutingTarget {
        match gesture {
            HandheldTouchGesture::SwipeRightEdge => {
                quick_settings.visible = true;
                self.active_routing_target = InputEventRoutingTarget::QuickSettingsOverlay;
                InputEventRoutingTarget::QuickSettingsOverlay
            }
            _ => {
                if quick_settings.visible {
                    InputEventRoutingTarget::QuickSettingsOverlay
                } else {
                    InputEventRoutingTarget::ActiveGameProcess
                }
            }
        }
    }
}

