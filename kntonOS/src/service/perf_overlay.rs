// Gamescope-Style Performance Overlay & HUD Telemetry Service.
// Provides multi-level telemetry rendering, APU metrics, frame latency analysis, and layout controls.

pub const MAX_RENDER_COMMANDS: usize = 64;

// Display detail mode for performance HUD overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudOverlayLevel {
    Off = 0,
    FpsOnly = 1,
    FpsBatteryTdp = 2,
    FullMetrics = 3,
    DiagnosticsHistogram = 4,
}

impl HudOverlayLevel {
    // Cycle to next overlay detail level (0 -> 1 -> 2 -> 3 -> 4 -> 0).
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::FpsOnly,
            Self::FpsOnly => Self::FpsBatteryTdp,
            Self::FpsBatteryTdp => Self::FullMetrics,
            Self::FullMetrics => Self::DiagnosticsHistogram,
            Self::DiagnosticsHistogram => Self::Off,
        }
    }
}

// RGBA color format for HUD vector primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl HudColor {
    // Construct RGBA color.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    // Construct fully opaque RGB color.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const BLACK: Self = Self::rgba(16, 20, 26, 220);
    pub const WHITE: Self = Self::rgb(240, 240, 245);
    pub const GREEN: Self = Self::rgb(80, 220, 100);
    pub const YELLOW: Self = Self::rgb(240, 210, 60);
    pub const RED: Self = Self::rgb(240, 70, 70);
    pub const CYAN: Self = Self::rgb(70, 190, 240);
    pub const ORANGE: Self = Self::rgb(250, 140, 50);
}

// 2D rectangle coordinates for HUD layout elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl HudRect {
    // Construct a new HUD rectangle.
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }
}

// Rendering commands emitted by HUD overlay pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudRenderCommand {
    DrawRect {
        rect: HudRect,
        color: HudColor,
        filled: bool,
    },
    DrawText {
        x: u16,
        y: u16,
        text: &'static str,
        color: HudColor,
    },
    DrawValue {
        x: u16,
        y: u16,
        prefix: &'static str,
        value: u32,
        suffix: &'static str,
        color: HudColor,
    },
    DrawProgressBar {
        rect: HudRect,
        pct: u8,
        fill_color: HudColor,
        bg_color: HudColor,
    },
    DrawHistogramBar {
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        bucket_index: u8,
        color: HudColor,
    },
}

// Command buffer collecting HUD primitives for GPU or compositor presentation.
pub struct HudCommandBuffer {
    commands: [Option<HudRenderCommand>; MAX_RENDER_COMMANDS],
    count: usize,
}

impl HudCommandBuffer {
    // Construct an empty HUD render command buffer.
    pub const fn new() -> Self {
        Self {
            commands: [None; MAX_RENDER_COMMANDS],
            count: 0,
        }
    }

    // Reset buffer command count.
    pub fn clear(&mut self) {
        for slot in self.commands.iter_mut() {
            *slot = None;
        }
        self.count = 0;
    }

    // Push a new rendering command into the buffer.
    pub fn push(&mut self, cmd: HudRenderCommand) -> bool {
        if self.count < MAX_RENDER_COMMANDS {
            self.commands[self.count] = Some(cmd);
            self.count += 1;
            true
        } else {
            false
        }
    }

    // Return the number of active commands in the buffer.
    pub fn count(&self) -> usize {
        self.count
    }

    // Return command reference by index.
    pub fn get(&self, idx: usize) -> Option<&HudRenderCommand> {
        if idx < self.count {
            self.commands[idx].as_ref()
        } else {
            None
        }
    }
}

// Multi-level performance HUD overlay renderer.
pub struct PerfOverlayRenderer {
    pub level: HudOverlayLevel,
}

impl PerfOverlayRenderer {
    // Construct renderer with specified initial detail level.
    pub const fn new(level: HudOverlayLevel) -> Self {
        Self { level }
    }

    // Configure overlay detail level.
    pub fn set_level(&mut self, level: HudOverlayLevel) {
        self.level = level;
    }

    // Query active overlay detail level.
    pub fn level(&self) -> HudOverlayLevel {
        self.level
    }

    // Cycle to next overlay detail level.
    pub fn cycle_level(&mut self) {
        self.level = self.level.next();
    }
}

// Telemetry state of a single APU CPU core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuCoreTelemetry {
    pub core_id: u8,
    pub clock_mhz: u16,
    pub load_pct: u8,
    pub temp_c: u8,
}

impl CpuCoreTelemetry {
    // Construct default CPU core metrics.
    pub const fn new(core_id: u8) -> Self {
        Self {
            core_id,
            clock_mhz: 2400,
            load_pct: 0,
            temp_c: 45,
        }
    }
}

// GPU shader core and VRAM hardware metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuTelemetry {
    pub clock_mhz: u16,
    pub shader_load_pct: u8,
    pub vram_used_mb: u16,
    pub vram_total_mb: u16,
    pub temp_c: u8,
}

impl GpuTelemetry {
    // Construct default GPU telemetry descriptor.
    pub const fn new() -> Self {
        Self {
            clock_mhz: 1600,
            shader_load_pct: 0,
            vram_used_mb: 512,
            vram_total_mb: 8192,
            temp_c: 50,
        }
    }
}

// Cooling fan acoustic and rotational velocity metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanTelemetry {
    pub current_rpm: u16,
    pub target_rpm: u16,
    pub pwm_pct: u8,
}

impl FanTelemetry {
    // Construct default fan telemetry descriptor.
    pub const fn new() -> Self {
        Self {
            current_rpm: 2800,
            target_rpm: 2800,
            pwm_pct: 40,
        }
    }
}

// Battery fuel gauge and real-time power discharge telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryTelemetry {
    pub charge_pct: u8,
    pub discharge_rate_mw: u32,
    pub voltage_mv: u16,
    pub remaining_time_minutes: u16,
}

impl BatteryTelemetry {
    // Construct default battery telemetry descriptor.
    pub const fn new() -> Self {
        Self {
            charge_pct: 85,
            discharge_rate_mw: 15000,
            voltage_mv: 7600,
            remaining_time_minutes: 180,
        }
    }
}

// Combined real-time hardware telemetry snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardwareTelemetry {
    pub cpu_cores: [CpuCoreTelemetry; 4],
    pub gpu: GpuTelemetry,
    pub fan: FanTelemetry,
    pub battery: BatteryTelemetry,
    pub target_tdp_watts: u8,
}

impl HardwareTelemetry {
    // Construct initial hardware telemetry state snapshot.
    pub const fn new() -> Self {
        Self {
            cpu_cores: [
                CpuCoreTelemetry::new(0),
                CpuCoreTelemetry::new(1),
                CpuCoreTelemetry::new(2),
                CpuCoreTelemetry::new(3),
            ],
            gpu: GpuTelemetry::new(),
            fan: FanTelemetry::new(),
            battery: BatteryTelemetry::new(),
            target_tdp_watts: 15,
        }
    }

    // Compute average CPU load across all APU cores.
    pub fn average_cpu_load(&self) -> u8 {
        let total: u16 = self.cpu_cores.iter().map(|c| c.load_pct as u16).sum();
        (total / 4) as u8
    }

    // Compute VRAM utilization percentage.
    pub fn vram_usage_pct(&self) -> u8 {
        if self.gpu.vram_total_mb == 0 {
            0
        } else {
            ((self.gpu.vram_used_mb as u32 * 100) / self.gpu.vram_total_mb as u32) as u8
        }
    }
}

// Central hardware telemetry collector polling APU and battery controller.
pub struct HardwareTelemetryCollector {
    current: HardwareTelemetry,
}

impl HardwareTelemetryCollector {
    // Construct collector initialized with baseline hardware metrics.
    pub const fn new() -> Self {
        Self {
            current: HardwareTelemetry::new(),
        }
    }

    // Update telemetry state for a specific CPU core.
    pub fn update_cpu_core(&mut self, core_id: usize, clock_mhz: u16, load_pct: u8, temp_c: u8) {
        if core_id < 4 {
            self.current.cpu_cores[core_id].clock_mhz = clock_mhz;
            self.current.cpu_cores[core_id].load_pct = load_pct.min(100);
            self.current.cpu_cores[core_id].temp_c = temp_c;
        }
    }

    // Update GPU clock, shader utilization and VRAM consumption.
    pub fn update_gpu(&mut self, clock_mhz: u16, load_pct: u8, vram_mb: u16, temp_c: u8) {
        self.current.gpu.clock_mhz = clock_mhz;
        self.current.gpu.shader_load_pct = load_pct.min(100);
        self.current.gpu.vram_used_mb = vram_mb.min(self.current.gpu.vram_total_mb);
        self.current.gpu.temp_c = temp_c;
    }

    // Update cooling fan rotational speed and PWM duty cycle.
    pub fn update_fan(&mut self, current_rpm: u16, pwm_pct: u8) {
        self.current.fan.current_rpm = current_rpm;
        self.current.fan.pwm_pct = pwm_pct.min(100);
    }

    // Update battery fuel gauge status and power consumption.
    pub fn update_battery(&mut self, charge_pct: u8, discharge_rate_mw: u32, voltage_mv: u16) {
        self.current.battery.charge_pct = charge_pct.min(100);
        self.current.battery.discharge_rate_mw = discharge_rate_mw;
        self.current.battery.voltage_mv = voltage_mv;
        if discharge_rate_mw > 0 {
            // 40,000 mWh battery capacity approximation for Steam Deck
            let remaining_mwh = charge_pct as u32 * 400;
            self.current.battery.remaining_time_minutes = ((remaining_mwh * 60) / discharge_rate_mw).min(600) as u16;
        }
    }

    // Update APU thermal design power limit.
    pub fn set_tdp(&mut self, tdp_watts: u8) {
        self.current.target_tdp_watts = tdp_watts.clamp(5, 30);
    }

    // Retrieve active telemetry snapshot.
    pub fn snapshot(&self) -> HardwareTelemetry {
        self.current
    }
}

pub const FRAME_HISTORY_CAPACITY: usize = 120;
pub const HISTOGRAM_BUCKETS: usize = 8;

// Aggregated frame timing and latency statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameTimeStats {
    pub instantaneous_fps: u32,
    pub avg_frame_time_us: u32,
    pub min_frame_time_us: u32,
    pub max_frame_time_us: u32,
    pub p99_frame_time_us: u32,
    pub stutter_count: u32,
    pub histogram_buckets: [u16; HISTOGRAM_BUCKETS],
    pub total_frames_recorded: u64,
}

impl FrameTimeStats {
    // Construct zeroed frame timing metrics.
    pub const fn new() -> Self {
        Self {
            instantaneous_fps: 0,
            avg_frame_time_us: 0,
            min_frame_time_us: 0,
            max_frame_time_us: 0,
            p99_frame_time_us: 0,
            stutter_count: 0,
            histogram_buckets: [0; HISTOGRAM_BUCKETS],
            total_frames_recorded: 0,
        }
    }
}

// Rolling frame latency circular buffer calculating 99th percentile and frame spikes.
pub struct RollingFrameTimeBuffer {
    samples: [u32; FRAME_HISTORY_CAPACITY],
    head: usize,
    count: usize,
    total_frames: u64,
}

impl RollingFrameTimeBuffer {
    // Construct an empty circular frame time latency buffer.
    pub const fn new() -> Self {
        Self {
            samples: [0; FRAME_HISTORY_CAPACITY],
            head: 0,
            count: 0,
            total_frames: 0,
        }
    }

    // Record a completed frame presentation time in microseconds.
    pub fn record_frame(&mut self, frame_time_us: u32) {
        self.samples[self.head] = frame_time_us;
        self.head = (self.head + 1) % FRAME_HISTORY_CAPACITY;
        if self.count < FRAME_HISTORY_CAPACITY {
            self.count += 1;
        }
        self.total_frames += 1;
    }

    // Determine histogram bucket index from latency in microseconds.
    fn bucket_for_latency(us: u32) -> usize {
        if us < 8_333 {
            0 // > 120 FPS
        } else if us <= 16_666 {
            1 // 60 - 120 FPS
        } else if us <= 20_000 {
            2 // 50 - 60 FPS
        } else if us <= 25_000 {
            3 // 40 - 50 FPS
        } else if us <= 33_333 {
            4 // 30 - 40 FPS
        } else if us <= 50_000 {
            5 // 20 - 30 FPS
        } else if us <= 100_000 {
            6 // 10 - 20 FPS (minor stutter)
        } else {
            7 // < 10 FPS (severe hitch)
        }
    }

    // Calculate aggregated frame timing, p99 latency, and stutter metrics.
    pub fn compute_stats(&self) -> FrameTimeStats {
        if self.count == 0 {
            return FrameTimeStats::new();
        }

        let mut sum: u64 = 0;
        let mut min = u32::MAX;
        let mut max = 0;
        let mut buckets = [0u16; HISTOGRAM_BUCKETS];

        // Local copy for in-place insertion sorting to determine percentiles in `#![no_std]`
        let mut sorted_copy = [0u32; FRAME_HISTORY_CAPACITY];
        for i in 0..self.count {
            let val = self.samples[i];
            sorted_copy[i] = val;
            sum += val as u64;
            if val < min {
                min = val;
            }
            if val > max {
                max = val;
            }
            let b_idx = Self::bucket_for_latency(val);
            buckets[b_idx] = buckets[b_idx].saturating_add(1);
        }

        let avg = (sum / self.count as u64) as u32;

        // Insertion sort on the active sample window
        for i in 1..self.count {
            let key = sorted_copy[i];
            let mut j = i;
            while j > 0 && sorted_copy[j - 1] > key {
                sorted_copy[j] = sorted_copy[j - 1];
                j -= 1;
            }
            sorted_copy[j] = key;
        }

        // 99th percentile index
        let p99_idx = ((self.count * 99) / 100).min(self.count.saturating_sub(1));
        let p99 = sorted_copy[p99_idx];

        // Stutter detection: count frames exceeding 1.5x of the average frame time
        let stutter_threshold = avg + (avg / 2);
        let mut stutters = 0u32;
        for i in 0..self.count {
            if self.samples[i] > stutter_threshold {
                stutters += 1;
            }
        }

        // Instantaneous FPS based on most recent frame
        let last_idx = if self.head == 0 {
            self.count.saturating_sub(1)
        } else {
            self.head - 1
        };
        let last_us = self.samples[last_idx];
        let fps = if last_us > 0 {
            1_000_000 / last_us
        } else {
            0
        };

        FrameTimeStats {
            instantaneous_fps: fps,
            avg_frame_time_us: avg,
            min_frame_time_us: if min == u32::MAX { 0 } else { min },
            max_frame_time_us: max,
            p99_frame_time_us: p99,
            stutter_count: stutters,
            histogram_buckets: buckets,
            total_frames_recorded: self.total_frames,
        }
    }
}

// Screen quadrant anchor for overlay positioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenAnchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ScreenAnchor {
    // Cycle to next corner anchor position clockwise.
    pub fn next(self) -> Self {
        match self {
            Self::TopLeft => Self::TopRight,
            Self::TopRight => Self::BottomRight,
            Self::BottomRight => Self::BottomLeft,
            Self::BottomLeft => Self::TopLeft,
        }
    }
}

// Layout configuration for overlay scale, opacity, and positioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudLayoutConfig {
    pub anchor: ScreenAnchor,
    pub scale_pct: u8,
    pub opacity_pct: u8,
    pub margin_x: u16,
    pub margin_y: u16,
    pub screen_width: u16,
    pub screen_height: u16,
}

impl HudLayoutConfig {
    // Construct default layout configuration for display dimensions.
    pub const fn new(screen_width: u16, screen_height: u16) -> Self {
        Self {
            anchor: ScreenAnchor::TopLeft,
            scale_pct: 100,
            opacity_pct: 90,
            margin_x: 12,
            margin_y: 12,
            screen_width,
            screen_height,
        }
    }

    // Configure overlay screen corner anchor.
    pub fn set_anchor(&mut self, anchor: ScreenAnchor) {
        self.anchor = anchor;
    }

    // Cycle screen corner anchor.
    pub fn cycle_anchor(&mut self) {
        self.anchor = self.anchor.next();
    }

    // Set background opacity percentage [20..=100].
    pub fn set_opacity(&mut self, opacity: u8) {
        self.opacity_pct = opacity.clamp(20, 100);
    }

    // Set scaling factor percentage [50..=200].
    pub fn set_scale(&mut self, scale: u8) {
        self.scale_pct = scale.clamp(50, 200);
    }

    // Compute top-left origin coordinates (x, y) for an overlay of given dimensions.
    pub fn calculate_origin(&self, overlay_w: u16, overlay_h: u16) -> (u16, u16) {
        let scaled_w = ((overlay_w as u32 * self.scale_pct as u32) / 100) as u16;
        let scaled_h = ((overlay_h as u32 * self.scale_pct as u32) / 100) as u16;

        match self.anchor {
            ScreenAnchor::TopLeft => (self.margin_x, self.margin_y),
            ScreenAnchor::TopRight => (
                self.screen_width.saturating_sub(scaled_w + self.margin_x),
                self.margin_y,
            ),
            ScreenAnchor::BottomLeft => (
                self.margin_x,
                self.screen_height.saturating_sub(scaled_h + self.margin_y),
            ),
            ScreenAnchor::BottomRight => (
                self.screen_width.saturating_sub(scaled_w + self.margin_x),
                self.screen_height.saturating_sub(scaled_h + self.margin_y),
            ),
        }
    }
}

impl PerfOverlayRenderer {
    // Emit drawing commands corresponding to the active overlay detail level.
    pub fn generate_commands(
        &self,
        telemetry: &HardwareTelemetry,
        stats: &FrameTimeStats,
        layout: &HudLayoutConfig,
        out_buffer: &mut HudCommandBuffer,
    ) {
        out_buffer.clear();
        if self.level == HudOverlayLevel::Off {
            return;
        }

        let bg_alpha = ((layout.opacity_pct as u32 * 220) / 100) as u8;
        let bg_color = HudColor::rgba(16, 20, 26, bg_alpha);
        let fps_color = if stats.instantaneous_fps >= 55 {
            HudColor::GREEN
        } else if stats.instantaneous_fps >= 30 {
            HudColor::YELLOW
        } else {
            HudColor::RED
        };

        match self.level {
            HudOverlayLevel::Off => {}
            HudOverlayLevel::FpsOnly => {
                let (ox, oy) = layout.calculate_origin(68, 26);
                out_buffer.push(HudRenderCommand::DrawRect {
                    rect: HudRect::new(ox, oy, 68, 26),
                    color: bg_color,
                    filled: true,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 6,
                    prefix: "",
                    value: stats.instantaneous_fps,
                    suffix: " FPS",
                    color: fps_color,
                });
            }
            HudOverlayLevel::FpsBatteryTdp => {
                let (ox, oy) = layout.calculate_origin(160, 42);
                out_buffer.push(HudRenderCommand::DrawRect {
                    rect: HudRect::new(ox, oy, 160, 42),
                    color: bg_color,
                    filled: true,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 6,
                    prefix: "",
                    value: stats.instantaneous_fps,
                    suffix: " FPS | ",
                    color: fps_color,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 80,
                    y: oy + 6,
                    prefix: "",
                    value: telemetry.target_tdp_watts as u32,
                    suffix: "W TDP",
                    color: HudColor::CYAN,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 22,
                    prefix: "BAT: ",
                    value: telemetry.battery.charge_pct as u32,
                    suffix: "%",
                    color: HudColor::WHITE,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 80,
                    y: oy + 22,
                    prefix: "",
                    value: telemetry.battery.remaining_time_minutes as u32,
                    suffix: "m rem",
                    color: HudColor::WHITE,
                });
            }
            HudOverlayLevel::FullMetrics => {
                let (ox, oy) = layout.calculate_origin(210, 88);
                out_buffer.push(HudRenderCommand::DrawRect {
                    rect: HudRect::new(ox, oy, 210, 88),
                    color: bg_color,
                    filled: true,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 6,
                    prefix: "",
                    value: stats.instantaneous_fps,
                    suffix: " FPS",
                    color: fps_color,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 100,
                    y: oy + 6,
                    prefix: "AVG: ",
                    value: stats.avg_frame_time_us / 1000,
                    suffix: " ms",
                    color: HudColor::WHITE,
                });

                // CPU Load Progress Bar
                let cpu_load = telemetry.average_cpu_load();
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 22,
                    prefix: "CPU: ",
                    value: cpu_load as u32,
                    suffix: "%",
                    color: HudColor::CYAN,
                });
                out_buffer.push(HudRenderCommand::DrawProgressBar {
                    rect: HudRect::new(ox + 80, oy + 25, 70, 8),
                    pct: cpu_load,
                    fill_color: HudColor::CYAN,
                    bg_color: HudColor::rgba(40, 50, 65, 200),
                });

                // GPU Load Progress Bar
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 38,
                    prefix: "GPU: ",
                    value: telemetry.gpu.shader_load_pct as u32,
                    suffix: "%",
                    color: HudColor::ORANGE,
                });
                out_buffer.push(HudRenderCommand::DrawProgressBar {
                    rect: HudRect::new(ox + 80, oy + 41, 70, 8),
                    pct: telemetry.gpu.shader_load_pct,
                    fill_color: HudColor::ORANGE,
                    bg_color: HudColor::rgba(40, 50, 65, 200),
                });

                // VRAM & Fan
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 54,
                    prefix: "VRAM: ",
                    value: telemetry.gpu.vram_used_mb as u32,
                    suffix: " MB",
                    color: HudColor::WHITE,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 70,
                    prefix: "FAN: ",
                    value: telemetry.fan.current_rpm as u32,
                    suffix: " RPM",
                    color: HudColor::WHITE,
                });
            }
            HudOverlayLevel::DiagnosticsHistogram => {
                let (ox, oy) = layout.calculate_origin(240, 132);
                out_buffer.push(HudRenderCommand::DrawRect {
                    rect: HudRect::new(ox, oy, 240, 132),
                    color: bg_color,
                    filled: true,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 6,
                    prefix: "",
                    value: stats.instantaneous_fps,
                    suffix: " FPS",
                    color: fps_color,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 80,
                    y: oy + 6,
                    prefix: "P99: ",
                    value: stats.p99_frame_time_us / 1000,
                    suffix: " ms",
                    color: HudColor::YELLOW,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 160,
                    y: oy + 6,
                    prefix: "STUT: ",
                    value: stats.stutter_count,
                    suffix: "",
                    color: if stats.stutter_count > 0 { HudColor::RED } else { HudColor::GREEN },
                });

                // CPU & GPU metrics
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 6,
                    y: oy + 22,
                    prefix: "CPU: ",
                    value: telemetry.average_cpu_load() as u32,
                    suffix: "%",
                    color: HudColor::CYAN,
                });
                out_buffer.push(HudRenderCommand::DrawValue {
                    x: ox + 120,
                    y: oy + 22,
                    prefix: "GPU: ",
                    value: telemetry.gpu.shader_load_pct as u32,
                    suffix: "%",
                    color: HudColor::ORANGE,
                });

                out_buffer.push(HudRenderCommand::DrawText {
                    x: ox + 6,
                    y: oy + 40,
                    text: "LATENCY HISTOGRAM",
                    color: HudColor::WHITE,
                });

                // Render 8 histogram bars for latency distribution
                for i in 0..HISTOGRAM_BUCKETS {
                    let bar_x = ox + 6 + (i as u16 * 26);
                    let bar_y = oy + 60;
                    let height = (stats.histogram_buckets[i] as u16 * 4).min(40);
                    let bar_color = if i < 2 {
                        HudColor::GREEN
                    } else if i < 5 {
                        HudColor::YELLOW
                    } else {
                        HudColor::RED
                    };
                    out_buffer.push(HudRenderCommand::DrawHistogramBar {
                        x: bar_x,
                        y: bar_y,
                        width: 20,
                        height,
                        bucket_index: i as u8,
                        color: bar_color,
                    });
                }
            }
        }
    }
}

// Integrated Gamescope-style Performance Overlay & Telemetry Service daemon.
pub struct GamescopeOverlayService {
    pub renderer: PerfOverlayRenderer,
    pub collector: HardwareTelemetryCollector,
    pub frame_buffer: RollingFrameTimeBuffer,
    pub layout: HudLayoutConfig,
    pub command_buffer: HudCommandBuffer,
}

impl GamescopeOverlayService {
    // Construct unified performance overlay service for display viewport.
    pub const fn new(screen_width: u16, screen_height: u16) -> Self {
        Self {
            renderer: PerfOverlayRenderer::new(HudOverlayLevel::FpsOnly),
            collector: HardwareTelemetryCollector::new(),
            frame_buffer: RollingFrameTimeBuffer::new(),
            layout: HudLayoutConfig::new(screen_width, screen_height),
            command_buffer: HudCommandBuffer::new(),
        }
    }

    // Record frame latency and rebuild HUD presentation commands.
    pub fn record_frame(&mut self, frame_time_us: u32) {
        self.frame_buffer.record_frame(frame_time_us);
    }

    // Refresh overlay render commands based on latest frame and hardware stats.
    pub fn update_render_commands(&mut self) -> usize {
        let stats = self.frame_buffer.compute_stats();
        let telemetry = self.collector.snapshot();
        self.renderer.generate_commands(&telemetry, &stats, &self.layout, &mut self.command_buffer);
        self.command_buffer.count()
    }
}

// Comprehensive self-test suite verifying overlay rendering, telemetry collector, and frame latency buffer.
pub fn verify_perf_overlay_subsystem() -> bool {
    let mut service = GamescopeOverlayService::new(1280, 800);

    // 1. Verify Hardware Telemetry Collector
    service.collector.update_cpu_core(0, 2800, 40, 52);
    service.collector.update_cpu_core(1, 2800, 60, 54);
    service.collector.update_cpu_core(2, 2800, 30, 50);
    service.collector.update_cpu_core(3, 2800, 70, 56);
    let telem = service.collector.snapshot();
    if telem.average_cpu_load() != 50 {
        return false;
    }

    service.collector.update_gpu(1600, 85, 4096, 62);
    let telem2 = service.collector.snapshot();
    if telem2.gpu.shader_load_pct != 85 || telem2.vram_usage_pct() != 50 {
        return false;
    }

    service.collector.update_fan(3400, 60);
    service.collector.update_battery(80, 20000, 7600);
    service.collector.set_tdp(25);
    let telem3 = service.collector.snapshot();
    if telem3.fan.current_rpm != 3400 || telem3.target_tdp_watts != 25 {
        return false;
    }
    if telem3.battery.remaining_time_minutes != 96 {
        return false;
    }

    // 2. Verify Rolling Frame-Time Latency Buffer & P99 Latency
    for _ in 0..59 {
        service.record_frame(16_666);
    }
    service.record_frame(40_000);

    let stats = service.frame_buffer.compute_stats();
    if stats.total_frames_recorded != 60 {
        return false;
    }
    if stats.instantaneous_fps != 25 {
        return false;
    }
    if stats.min_frame_time_us != 16_666 || stats.max_frame_time_us != 40_000 {
        return false;
    }
    if stats.p99_frame_time_us != 40_000 {
        return false;
    }
    if stats.stutter_count != 1 {
        return false;
    }

    // 3. Verify HUD Layout Anchor and Positioning
    let (tl_x, tl_y) = service.layout.calculate_origin(100, 50);
    if tl_x != 12 || tl_y != 12 {
        return false;
    }

    service.layout.set_anchor(ScreenAnchor::TopRight);
    let (tr_x, tr_y) = service.layout.calculate_origin(100, 50);
    if tr_x != (1280 - 100 - 12) || tr_y != 12 {
        return false;
    }

    service.layout.set_anchor(ScreenAnchor::BottomRight);
    let (br_x, br_y) = service.layout.calculate_origin(100, 50);
    if br_x != (1280 - 100 - 12) || br_y != (800 - 50 - 12) {
        return false;
    }

    service.layout.set_anchor(ScreenAnchor::TopLeft);
    service.layout.set_opacity(75);
    service.layout.set_scale(100);

    // 4. Verify Multi-Level HUD Overlay Command Generation
    service.renderer.set_level(HudOverlayLevel::FpsOnly);
    let cmd_cnt1 = service.update_render_commands();
    if cmd_cnt1 != 2 {
        return false;
    }

    service.renderer.set_level(HudOverlayLevel::FpsBatteryTdp);
    let cmd_cnt2 = service.update_render_commands();
    if cmd_cnt2 < 4 {
        return false;
    }

    service.renderer.set_level(HudOverlayLevel::FullMetrics);
    let cmd_cnt3 = service.update_render_commands();
    if cmd_cnt3 < 7 {
        return false;
    }

    service.renderer.set_level(HudOverlayLevel::DiagnosticsHistogram);
    let cmd_cnt4 = service.update_render_commands();
    if cmd_cnt4 < 12 {
        return false;
    }

    service.renderer.set_level(HudOverlayLevel::Off);
    let cmd_cnt_off = service.update_render_commands();
    if cmd_cnt_off != 0 {
        return false;
    }

    true
}




