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
        let p99_idx = (self.count.saturating_sub(1) * 99) / 100;
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


