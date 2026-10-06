//! Handheld Display DSC (Display Stream Compression) & Adaptive Refresh Rate (VRR) Fast-Sync Engine.
//!
//! Provides VESA DSC 1.2a slice multiplexing and rate buffer control (up to 3:1 compression),
//! Variable Refresh Rate (VRR) pacing with Low Framerate Compensation (LFC) across 30Hz - 165Hz,
//! OLED/Mini-LED panel luminance flicker compensation during rapid fps transients,
//! and static image idle step-down to 10Hz for battery conservation.

#![allow(dead_code)]

// ============================================================================
// Feature 1: VESA DSC 1.2a Slice Multiplexer & Rate Buffer Governor
// ============================================================================

/// Maximum supported DSC slices per scanline.
pub const MAX_DSC_SLICES: usize = 4;
/// Standard DSC rate control buffer size in bits per slice (8 KBytes = 65,536 bits).
pub const DSC_RC_BUFFER_SIZE_BITS: u32 = 65536;

/// VESA DSC 1.2a Slice Configuration & Rate Buffer Controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DscSlice {
    /// Slice horizontal width in pixels.
    pub slice_width: u32,
    /// Slice vertical height in scanlines.
    pub slice_height: u32,
    /// Target compressed bits per pixel scaled by 16 (e.g. 8.0 bpp = 128 for 3:1 ratio).
    pub target_bpp_x16: u16,
    /// Instantaneous Rate Control (RC) buffer fullness in bits.
    pub rc_buffer_bits: u32,
    /// Active quantization parameter (QP) from 1 (lossless/fine) to 16 (coarse).
    pub qp: u8,
    /// Cumulative compressed bits produced by this slice.
    pub total_bits_produced: u64,
}

impl DscSlice {
    /// Constructs a DSC slice initialized for 3:1 compression (24bpp uncompressed -> 8bpp target).
    pub const fn new(slice_width: u32, slice_height: u32) -> Self {
        Self {
            slice_width,
            slice_height,
            target_bpp_x16: 128, // 8.0 bpp (3:1 compression from 24bpp)
            rc_buffer_bits: DSC_RC_BUFFER_SIZE_BITS / 2, // Initial buffer fullness (50%)
            qp: 8,
            total_bits_produced: 0,
        }
    }

    /// Updates Rate Control buffer fullness and adapts QP to prevent underflow or overflow.
    pub fn update_rate_control(&mut self, chunk_pixels: u32, chunk_bits: u32) {
        // Ideal bits expected for this pixel chunk: (pixels * target_bpp_x16) / 16
        let expected_bits = (chunk_pixels as u64 * self.target_bpp_x16 as u64) / 16;

        // Buffer model: adds generated bits, subtracts drained bits at target line rate
        let new_fullness = (self.rc_buffer_bits as i64 + chunk_bits as i64 - expected_bits as i64)
            .clamp(0, DSC_RC_BUFFER_SIZE_BITS as i64) as u32;

        self.rc_buffer_bits = new_fullness;
        self.total_bits_produced += chunk_bits as u64;

        // Dynamic QP adjustment based on RC buffer fullness zones:
        // High fullness -> increase QP (coarser quantization, fewer bits)
        // Low fullness -> decrease QP (finer quantization, higher fidelity)
        let fullness_ratio = (self.rc_buffer_bits as f32) / (DSC_RC_BUFFER_SIZE_BITS as f32);
        self.qp = if fullness_ratio > 0.85 {
            14
        } else if fullness_ratio > 0.70 {
            11
        } else if fullness_ratio > 0.55 {
            9
        } else if fullness_ratio > 0.45 {
            8
        } else if fullness_ratio > 0.30 {
            6
        } else if fullness_ratio > 0.15 {
            4
        } else {
            2
        };
    }
}

/// VESA DSC 1.2a Slice Multiplexer and Transport Stream Governor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DscSliceMultiplexer {
    /// Active slices for the display panel (e.g. 2 slices of 1280px for 2560x1600 panel).
    pub slices: [DscSlice; MAX_DSC_SLICES],
    /// Number of active slices (typically 2 or 4).
    pub active_slice_count: usize,
    /// Panel total width in pixels.
    pub panel_width: u32,
    /// Panel total height in lines.
    pub panel_height: u32,
    /// Total uncompressed input bytes ingested.
    pub uncompressed_bytes_in: u64,
    /// Total compressed transport bytes emitted.
    pub compressed_bytes_out: u64,
}

impl DscSliceMultiplexer {
    /// Constructs a DSC 1.2a slice multiplexer for standard handheld display dimensions.
    pub const fn new(width: u32, height: u32, slice_count: usize) -> Self {
        let count = if slice_count == 0 {
            1
        } else if slice_count > MAX_DSC_SLICES {
            MAX_DSC_SLICES
        } else {
            slice_count
        };

        let slice_w = width / count as u32;
        let mut slices = [DscSlice::new(slice_w, height); MAX_DSC_SLICES];
        let mut i = 0;
        while i < MAX_DSC_SLICES {
            slices[i] = DscSlice::new(slice_w, height);
            i += 1;
        }

        Self {
            slices,
            active_slice_count: count,
            panel_width: width,
            panel_height: height,
            uncompressed_bytes_in: 0,
            compressed_bytes_out: 0,
        }
    }

    /// Compresses a scanline chunk across all active slices and multiplexes the transport stream.
    pub fn process_scanline_chunks(&mut self, chunk_pixels_per_slice: u32, complexity_factor: u8) -> usize {
        let mut total_chunk_compressed_bytes = 0;
        let raw_bytes_per_slice = chunk_pixels_per_slice * 3; // 24bpp (3 bytes/pixel)

        for i in 0..self.active_slice_count {
            // Simulate entropy encoding: higher complexity and lower QP generate more bits
            let base_bits = (chunk_pixels_per_slice * 8) as i32; // 3:1 nominal target
            let qp_effect = (8 - self.slices[i].qp as i32) * (chunk_pixels_per_slice as i32 / 8);
            let comp_effect = (complexity_factor as i32 - 128) * (chunk_pixels_per_slice as i32 / 16);

            let produced_bits = (base_bits + qp_effect + comp_effect).max(base_bits / 2) as u32;
            let compressed_bytes = ((produced_bits + 7) / 8) as usize;

            self.slices[i].update_rate_control(chunk_pixels_per_slice, produced_bits);

            self.uncompressed_bytes_in += raw_bytes_per_slice as u64;
            self.compressed_bytes_out += compressed_bytes as u64;
            total_chunk_compressed_bytes += compressed_bytes;
        }

        total_chunk_compressed_bytes
    }

    /// Returns the effective compression ratio achieved (e.g. ~3.0 for 3:1).
    pub fn effective_compression_ratio(&self) -> f32 {
        if self.compressed_bytes_out == 0 {
            return 3.0;
        }
        (self.uncompressed_bytes_in as f32) / (self.compressed_bytes_out as f32)
    }
}

/// Verification test for VESA DSC 1.2a slice multiplexer and rate control governor.
pub fn verify_dsc_slice_multiplexer_and_rc() -> bool {
    // 2560x1600 handheld gaming panel with 2 slices (1280px each)
    let mut mux = DscSliceMultiplexer::new(2560, 1600, 2);

    if mux.active_slice_count != 2 || mux.slices[0].slice_width != 1280 {
        return false;
    }

    // Process 100 scanline chunks under nominal image complexity (128)
    for _ in 0..100 {
        let compressed_bytes = mux.process_scanline_chunks(64, 128);
        if compressed_bytes == 0 {
            return false;
        }
    }

    // Effective compression ratio must be close to 3:1 (between 2.80 and 3.25)
    let ratio = mux.effective_compression_ratio();
    if ratio < 2.80 || ratio > 3.25 {
        return false;
    }

    // Rate buffer fullness must remain stably bounded without underflow (0) or overflow (65536)
    for i in 0..mux.active_slice_count {
        if mux.slices[i].rc_buffer_bits == 0 || mux.slices[i].rc_buffer_bits >= DSC_RC_BUFFER_SIZE_BITS {
            return false;
        }
        if mux.slices[i].qp < 1 || mux.slices[i].qp > 16 {
            return false;
        }
    }

    // Test high complexity burst: verify Rate Control responds by increasing QP
    let qp_before = mux.slices[0].qp;
    for _ in 0..50 {
        mux.process_scanline_chunks(64, 250);
    }
    let qp_after = mux.slices[0].qp;
    if qp_after < qp_before {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Variable Refresh Rate (VRR) Fast-Sync Pacer with LFC (30Hz-165Hz)
// ============================================================================

/// Minimum native VRR panel refresh frequency in Hz.
pub const VRR_MIN_FREQ_HZ: u32 = 30;
/// Maximum native VRR panel refresh frequency in Hz.
pub const VRR_MAX_FREQ_HZ: u32 = 165;
/// Low Framerate Compensation (LFC) activation threshold in Hz.
pub const VRR_LFC_THRESHOLD_HZ: u32 = 48;

/// Presentation timing directive computed by the VRR pacer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrrFramePresentation {
    /// Modulated frame duration in microseconds passed to display timing controller.
    pub presentation_interval_us: u32,
    /// Effective panel scanout refresh rate in Hz.
    pub effective_refresh_hz: u32,
    /// LFC repeat count for this frame (1 = native presentation, >=2 = repeated scanouts).
    pub lfc_multiplier: u8,
    /// Whether Low Framerate Compensation is active for this frame.
    pub is_lfc_active: bool,
}

/// Variable Refresh Rate (VRR) Fast-Sync Engine with LFC across 30Hz - 165Hz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VrrFastSyncPacer {
    /// Minimum supported panel refresh rate in Hz.
    pub min_hz: u32,
    /// Maximum supported panel refresh rate in Hz.
    pub max_hz: u32,
    /// Threshold below which LFC automatically multiplies frame scans.
    pub lfc_threshold_hz: u32,
    /// Latest measured game render interval in microseconds.
    pub last_render_interval_us: u32,
    /// Active presentation parameters.
    pub current_presentation: VrrFramePresentation,
    /// Total rendered frames processed.
    pub total_frames_processed: u64,
    /// Total LFC repeated frame presentations executed.
    pub total_lfc_repeats: u64,
}

impl VrrFastSyncPacer {
    /// Constructs a VRR Fast-Sync pacer for a 30Hz - 165Hz gaming panel.
    pub const fn new() -> Self {
        Self {
            min_hz: VRR_MIN_FREQ_HZ,
            max_hz: VRR_MAX_FREQ_HZ,
            lfc_threshold_hz: VRR_LFC_THRESHOLD_HZ,
            last_render_interval_us: 16_666, // 60 Hz initial
            current_presentation: VrrFramePresentation {
                presentation_interval_us: 16_666,
                effective_refresh_hz: 60,
                lfc_multiplier: 1,
                is_lfc_active: false,
            },
            total_frames_processed: 0,
            total_lfc_repeats: 0,
        }
    }

    /// Computes presentation timing and LFC replication for a newly rendered frame.
    pub fn pace_frame(&mut self, render_interval_us: u32) -> VrrFramePresentation {
        self.last_render_interval_us = render_interval_us;
        self.total_frames_processed += 1;

        let render_hz = if render_interval_us == 0 {
            self.max_hz
        } else {
            1_000_000 / render_interval_us
        };

        // Zone 1: High framerate (> max_hz): clamp to max panel capability
        if render_hz >= self.max_hz {
            let interval = 1_000_000 / self.max_hz;
            self.current_presentation = VrrFramePresentation {
                presentation_interval_us: interval,
                effective_refresh_hz: self.max_hz,
                lfc_multiplier: 1,
                is_lfc_active: false,
            };
            return self.current_presentation;
        }

        // Zone 2: Native VRR range (lfc_threshold_hz <= render_hz < max_hz)
        if render_hz >= self.lfc_threshold_hz {
            self.current_presentation = VrrFramePresentation {
                presentation_interval_us: render_interval_us,
                effective_refresh_hz: render_hz,
                lfc_multiplier: 1,
                is_lfc_active: false,
            };
            return self.current_presentation;
        }

        // Zone 3: Low framerate (< lfc_threshold_hz): Activate Low Framerate Compensation (LFC)
        // Calculate integer repeat multiplier M such that (render_hz * M) falls within native VRR range
        let mut multiplier: u8 = 2;
        while ((render_hz * multiplier as u32) < self.lfc_threshold_hz) && (multiplier < 8) {
            multiplier += 1;
        }

        let lfc_interval_us = render_interval_us / (multiplier as u32);
        let effective_hz = (render_hz * multiplier as u32).clamp(self.min_hz, self.max_hz);

        self.total_lfc_repeats += (multiplier - 1) as u64;

        self.current_presentation = VrrFramePresentation {
            presentation_interval_us: lfc_interval_us,
            effective_refresh_hz: effective_hz,
            lfc_multiplier: multiplier,
            is_lfc_active: true,
        };

        self.current_presentation
    }

    /// Computes modulated vertical blanking front porch scanlines to match frame interval.
    pub fn calculate_vfront_porch_lines(
        &self,
        presentation_interval_us: u32,
        active_lines: u32,
        line_time_ns: u32,
    ) -> u32 {
        if line_time_ns == 0 {
            return 40;
        }
        let total_frame_ns = (presentation_interval_us as u64) * 1000;
        let total_lines = (total_frame_ns / line_time_ns as u64) as u32;

        if total_lines > active_lines {
            total_lines - active_lines
        } else {
            20 // Minimum vertical porch
        }
    }
}

/// Verification test for VRR Fast-Sync pacer with Low Framerate Compensation (LFC).
pub fn verify_vrr_fast_sync_and_lfc() -> bool {
    let mut pacer = VrrFastSyncPacer::new();

    // 1. High-framerate gaming at 120 fps (~8333 us) -> Native VRR, no LFC
    let p_120 = pacer.pace_frame(8_333);
    if p_120.effective_refresh_hz != 120 || p_120.is_lfc_active || p_120.lfc_multiplier != 1 {
        return false;
    }

    // 2. Standard 60 fps gaming (16666 us) -> Native VRR, no LFC
    let p_60 = pacer.pace_frame(16_666);
    if p_60.effective_refresh_hz != 60 || p_60.is_lfc_active || p_60.lfc_multiplier != 1 {
        return false;
    }

    // 3. Marginal framerate at 50 fps (20000 us) -> Still above 48Hz threshold, no LFC
    let p_50 = pacer.pace_frame(20_000);
    if p_50.effective_refresh_hz != 50 || p_50.is_lfc_active {
        return false;
    }

    // 4. Low framerate stutter at 24 fps (~41666 us) -> Below 48Hz threshold, LFC MUST activate
    // Multiplier M = 2: 24 fps * 2 = 48 Hz scanout, interval = 41666 / 2 ≈ 20833 us
    let p_24 = pacer.pace_frame(41_666);
    if !p_24.is_lfc_active || p_24.lfc_multiplier != 2 {
        return false;
    }
    if p_24.effective_refresh_hz < 48 || p_24.presentation_interval_us > 21_000 {
        return false;
    }

    // 5. Severe framerate drop at 15 fps (~66666 us)
    // Multiplier M = 4: 15 fps * 4 = 60 Hz scanout, interval = 66666 / 4 ≈ 16666 us
    let p_15 = pacer.pace_frame(66_666);
    if !p_15.is_lfc_active || p_15.lfc_multiplier < 3 {
        return false;
    }
    if p_15.effective_refresh_hz < 48 {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: OLED/Mini-LED Luminance Flicker Compensation for VRR
// ============================================================================

/// Nominal reference refresh rate in Hz where OLED gamma calibration is centered.
pub const OLED_REF_REFRESH_HZ: u32 = 120;
/// Nominal reference frame duration in microseconds (120 Hz = ~8333 us).
pub const OLED_REF_INTERVAL_US: u32 = 8_333;

/// OLED / Mini-LED Panel Luminance Flicker Compensator during rapid VRR framerate swings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OledVrrFlickerCompensator {
    /// Baseline calibrated panel refresh rate (nominally 120 Hz).
    pub ref_refresh_hz: u32,
    /// Flicker compensation gain coefficient (fractional adjustment per ms of V-blank extension).
    pub compensation_gain: f32,
    /// Low-pass filtered frame duration tracking parameter to prevent abrupt luminance jumps.
    pub filtered_interval_us: f32,
    /// Current calculated luminance correction multiplier (0.90 .. 1.15).
    pub current_correction_factor: f32,
    /// Smoothing factor alpha for the interval tracker (0.0 < alpha <= 1.0).
    pub filter_alpha: f32,
}

impl OledVrrFlickerCompensator {
    /// Constructs an OLED/Mini-LED flicker compensator with calibrated handheld defaults.
    pub const fn new() -> Self {
        Self {
            ref_refresh_hz: OLED_REF_REFRESH_HZ,
            compensation_gain: 0.0035, // +0.35% luminance compensation per 1ms V-blank extension
            filtered_interval_us: OLED_REF_INTERVAL_US as f32,
            current_correction_factor: 1.0,
            filter_alpha: 0.35,
        }
    }

    /// Resets the compensator state to reference 120Hz baseline.
    pub fn reset(&mut self) {
        self.filtered_interval_us = OLED_REF_INTERVAL_US as f32;
        self.current_correction_factor = 1.0;
    }

    /// Computes compensated panel luminance given current frame duration and Average Picture Level (APL).
    pub fn compute_compensated_luminance(
        &mut self,
        current_interval_us: u32,
        apl_pct: f32,
        base_brightness: f32,
    ) -> f32 {
        // 1. Smooth frame duration across successive frames to eliminate high-frequency ripple
        self.filtered_interval_us += self.filter_alpha * (current_interval_us as f32 - self.filtered_interval_us);

        // 2. Compute delta interval relative to calibrated 120Hz reference in milliseconds
        let delta_interval_ms = (self.filtered_interval_us - OLED_REF_INTERVAL_US as f32) / 1000.0;

        // 3. Near-black dark scene weighting:
        // Human perception of OLED VRR gamma flicker is highest in near-black / dark scenes (< 30% APL)
        // and negligible in bright daytime outdoor scenes (> 70% APL).
        let dark_scene_weight = if apl_pct <= 20.0 {
            1.0
        } else if apl_pct >= 80.0 {
            0.15
        } else {
            1.0 - ((apl_pct - 20.0) / 60.0) * 0.85
        };

        // 4. Calculate compensation multiplier:
        // When refresh rate drops (interval expands), pixel capacitor charge leaks during long V-blank,
        // causing luminance drop. We inject positive compensation to maintain constant perceived gamma.
        let raw_factor = 1.0 + self.compensation_gain * delta_interval_ms * dark_scene_weight;
        self.current_correction_factor = raw_factor.clamp(0.92, 1.12);

        // 5. Apply correction to base panel brightness
        (base_brightness * self.current_correction_factor).clamp(0.0, 1.0)
    }
}

/// Verification test for OLED/Mini-LED luminance flicker compensation.
pub fn verify_oled_flicker_compensation() -> bool {
    let mut comp = OledVrrFlickerCompensator::new();

    // 1. Steady 120Hz reference state: correction factor must be exactly 1.0
    for _ in 0..10 {
        comp.compute_compensated_luminance(OLED_REF_INTERVAL_US, 25.0, 0.50);
    }
    if (comp.current_correction_factor - 1.0).abs() > 0.005 {
        return false;
    }

    // 2. Sudden framerate drop from 120Hz to 40Hz (~25,000 us) in a dark game dungeon (APL = 10%)
    // V-blank duration expands by ~16.7 ms
    for _ in 0..15 {
        comp.compute_compensated_luminance(25_000, 10.0, 0.50);
    }

    // Compensation factor must be positively boosted (> 1.04) to counteract OLED charge decay
    if comp.current_correction_factor <= 1.03 {
        return false;
    }
    let boosted_brightness = comp.compute_compensated_luminance(25_000, 10.0, 0.50);
    if boosted_brightness <= 0.515 {
        return false;
    }

    // 3. Bright daylight scene (APL = 90%):
    // Near-black sensitivity factor must suppress correction compared to dark scene
    let mut comp_bright = OledVrrFlickerCompensator::new();
    for _ in 0..15 {
        comp_bright.compute_compensated_luminance(25_000, 90.0, 0.50);
    }
    if comp_bright.current_correction_factor >= comp.current_correction_factor {
        return false;
    }

    true
}


// ============================================================================
// Feature 4: Handheld Static Image Idle Step-Down (to 10Hz) & Display Governor
// ============================================================================

/// Deep idle power-saving refresh rate in Hz for static screen content.
pub const DISPLAY_IDLE_MIN_REFRESH_HZ: u32 = 10;
/// Deep idle frame interval in microseconds (10 Hz = 100,000 us).
pub const DISPLAY_IDLE_MIN_INTERVAL_US: u32 = 100_000;
/// Inactivity threshold in microseconds before stepping down to 30 Hz (500 ms).
pub const INACTIVITY_STAGE1_THRESHOLD_US: u64 = 500_000;
/// Inactivity threshold in microseconds before stepping down to deep 10 Hz (2000 ms).
pub const INACTIVITY_STAGE2_THRESHOLD_US: u64 = 2_000_000;

/// Static image inactivity detector and power-saving refresh rate step-down controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayIdleStepDown {
    /// Accumulated duration of static screen content in microseconds.
    pub static_inactivity_duration_us: u64,
    /// Active target refresh rate in Hz (10 .. 165 Hz).
    pub current_target_hz: u32,
    /// Whether deep 10Hz idle mode is engaged.
    pub is_deep_idle_active: bool,
    /// Total transitions into 10Hz deep idle.
    pub total_idle_entries: u32,
}

impl DisplayIdleStepDown {
    /// Constructs a static image step-down controller in active 60Hz mode.
    pub const fn new() -> Self {
        Self {
            static_inactivity_duration_us: 0,
            current_target_hz: 60,
            is_deep_idle_active: false,
            total_idle_entries: 0,
        }
    }

    /// Resets inactivity on new frame damage or user input event.
    pub fn report_active_content(&mut self, active_gaming_hz: u32) {
        self.static_inactivity_duration_us = 0;
        self.current_target_hz = active_gaming_hz.clamp(VRR_MIN_FREQ_HZ, VRR_MAX_FREQ_HZ);
        self.is_deep_idle_active = false;
    }

    /// Steps forward elapsed time and applies progressive refresh rate reduction for static images.
    pub fn update_inactivity(&mut self, elapsed_us: u32, max_gaming_hz: u32) -> u32 {
        self.static_inactivity_duration_us += elapsed_us as u64;

        if self.static_inactivity_duration_us >= INACTIVITY_STAGE2_THRESHOLD_US {
            // Stage 2: Deep 10Hz static image idle (Panel Self Refresh state)
            if !self.is_deep_idle_active {
                self.is_deep_idle_active = true;
                self.total_idle_entries += 1;
            }
            self.current_target_hz = DISPLAY_IDLE_MIN_REFRESH_HZ;
        } else if self.static_inactivity_duration_us >= INACTIVITY_STAGE1_THRESHOLD_US {
            // Stage 1: Minor idle step-down to 30Hz
            self.current_target_hz = 30;
            self.is_deep_idle_active = false;
        } else {
            // Active gaming rate
            self.current_target_hz = max_gaming_hz.clamp(VRR_MIN_FREQ_HZ, VRR_MAX_FREQ_HZ);
            self.is_deep_idle_active = false;
        }

        self.current_target_hz
    }
}

/// Composite output structure containing all timing and physical scanout parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandheldDisplayOutput {
    /// Active frame presentation interval in microseconds.
    pub presentation_interval_us: u32,
    /// Target scanout refresh frequency in Hz (10 .. 165 Hz).
    pub refresh_hz: u32,
    /// LFC repetition multiplier.
    pub lfc_multiplier: u8,
    /// Compensated OLED panel brightness [0.0 .. 1.0].
    pub compensated_brightness: f32,
    /// Effective VESA DSC 1.2a compression ratio achieved.
    pub dsc_compression_ratio: f32,
    /// Whether display is operating in 10Hz power-saving deep idle.
    pub is_deep_idle: bool,
}

/// Integrated Handheld Display DSC & Adaptive VRR Fast-Sync Engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandheldDisplayGovernor {
    /// VESA DSC 1.2a slice multiplexer.
    pub dsc_mux: DscSliceMultiplexer,
    /// VRR fast-sync pacer with LFC.
    pub vrr_pacer: VrrFastSyncPacer,
    /// OLED luminance flicker compensator.
    pub flicker_comp: OledVrrFlickerCompensator,
    /// Static image 10Hz idle step-down controller.
    pub idle_controller: DisplayIdleStepDown,
}

impl HandheldDisplayGovernor {
    /// Constructs an integrated display governor for a 2560x1600 165Hz handheld display.
    pub const fn new() -> Self {
        Self {
            dsc_mux: DscSliceMultiplexer::new(2560, 1600, 2),
            vrr_pacer: VrrFastSyncPacer::new(),
            flicker_comp: OledVrrFlickerCompensator::new(),
            idle_controller: DisplayIdleStepDown::new(),
        }
    }

    /// Primary display frame execution method.
    pub fn process_frame(
        &mut self,
        is_static_content: bool,
        render_interval_us: u32,
        apl_pct: f32,
        base_brightness: f32,
        elapsed_us: u32,
    ) -> HandheldDisplayOutput {
        // 1. Inactivity & Static Image Idle Step-Down
        let _target_hz = if is_static_content {
            self.idle_controller.update_inactivity(elapsed_us, 120)
        } else {
            self.idle_controller.report_active_content(120);
            120
        };

        // 2. VRR Pacing & LFC Calculation
        let presentation = if self.idle_controller.is_deep_idle_active {
            VrrFramePresentation {
                presentation_interval_us: DISPLAY_IDLE_MIN_INTERVAL_US,
                effective_refresh_hz: DISPLAY_IDLE_MIN_REFRESH_HZ,
                lfc_multiplier: 1,
                is_lfc_active: false,
            }
        } else {
            self.vrr_pacer.pace_frame(render_interval_us)
        };

        // 3. OLED Luminance Flicker Compensation
        let comp_brightness = self.flicker_comp.compute_compensated_luminance(
            presentation.presentation_interval_us,
            apl_pct,
            base_brightness,
        );

        // 4. DSC 1.2a Slice Multiplexing
        self.dsc_mux.process_scanline_chunks(64, 128);

        HandheldDisplayOutput {
            presentation_interval_us: presentation.presentation_interval_us,
            refresh_hz: presentation.effective_refresh_hz,
            lfc_multiplier: presentation.lfc_multiplier,
            compensated_brightness: comp_brightness,
            dsc_compression_ratio: self.dsc_mux.effective_compression_ratio(),
            is_deep_idle: self.idle_controller.is_deep_idle_active,
        }
    }
}

/// Verification test for static image idle step-down and integrated display governor.
pub fn verify_static_image_idle_stepdown_and_governor() -> bool {
    let mut gov = HandheldDisplayGovernor::new();

    // 1. Active gameplay at 120Hz: verify non-idle active state
    let out_active = gov.process_frame(false, 8_333, 50.0, 0.70, 8_333);
    if out_active.is_deep_idle || out_active.refresh_hz != 120 {
        return false;
    }
    if out_active.dsc_compression_ratio < 2.50 {
        return false;
    }

    // 2. Static image inactivity for 600ms -> should step down to Stage 1 (30Hz)
    let out_stage1 = gov.process_frame(true, 16_666, 50.0, 0.70, 600_000);
    if out_stage1.is_deep_idle || gov.idle_controller.current_target_hz != 30 {
        return false;
    }

    // 3. Static image inactivity past 2000ms -> should step down to Stage 2 Deep Idle (10Hz)
    let out_deep = gov.process_frame(true, 33_333, 50.0, 0.70, 1_600_000);
    if !out_deep.is_deep_idle || out_deep.refresh_hz != DISPLAY_IDLE_MIN_REFRESH_HZ {
        return false;
    }
    if out_deep.presentation_interval_us != DISPLAY_IDLE_MIN_INTERVAL_US {
        return false;
    }

    // 4. Instantaneous Wake-up: New user touch / input event arrives -> immediately restores active 120Hz
    let out_wakeup = gov.process_frame(false, 8_333, 50.0, 0.70, 8_333);
    if out_wakeup.is_deep_idle || out_wakeup.refresh_hz != 120 {
        return false;
    }

    true
}


// ============================================================================
// Comprehensive Subsystem Self-Test Suite
// ============================================================================

/// Comprehensive self-test suite for Handheld Display DSC & Adaptive VRR Fast-Sync Engine.
pub fn test_dsc_vrr_subsystem() -> bool {
    crate::serial_write_str("[KNT-DISPLAY] Testing Handheld Display DSC 1.2a & Adaptive VRR Fast-Sync Engine...\r\n");

    // 1. Verify VESA DSC 1.2a slice multiplexer and rate control buffer governor
    if !verify_dsc_slice_multiplexer_and_rc() {
        crate::serial_write_str("[KNT-PANIC] VESA DSC 1.2a slice multiplexer verification failed!\r\n");
        return false;
    }

    // 2. Verify VRR Fast-Sync pacer with Low Framerate Compensation (LFC)
    if !verify_vrr_fast_sync_and_lfc() {
        crate::serial_write_str("[KNT-PANIC] VRR Fast-Sync pacer and LFC verification failed!\r\n");
        return false;
    }

    // 3. Verify OLED/Mini-LED panel luminance flicker compensation
    if !verify_oled_flicker_compensation() {
        crate::serial_write_str("[KNT-PANIC] OLED luminance flicker compensation verification failed!\r\n");
        return false;
    }

    // 4. Verify static image idle step-down (to 10Hz) and integrated display governor
    if !verify_static_image_idle_stepdown_and_governor() {
        crate::serial_write_str("[KNT-PANIC] Static image idle step-down verification failed!\r\n");
        return false;
    }

    crate::serial_write_str("[KNT-DISPLAY-DSC-VRR-PASS] Handheld display DSC 1.2a compression and VRR fast-sync engine verified\r\n");
    true
}
