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
