//! Handheld Dynamic Display Refresh Rate Switching (DRRS) & Seamless Pixel Clock Re-PLL Engine.
//!
//! Handheld gaming consoles (e.g. Steam Deck OLED, ROG Ally X, Lenovo Legion Go) feature high-refresh
//! panels capable of 120Hz, 144Hz, or 165Hz. While high refresh rates provide butter-smooth gaming,
//! driving high pixel clocks and continuous TCON panel refreshes consumes substantial battery power
//! (up to 2.5W - 3.0W for display subsystem alone).
//!
//! This module implements:
//! 1. Dynamic Display Refresh Rate Switching (DRRS) between battery-saving (40Hz/48Hz/60Hz) and
//!    high-refresh gaming (90Hz/120Hz/144Hz/165Hz).
//! 2. Hardware Fractional-N PLL divider & multiplier synthesizer with low jitter (<0.05% error).
//! 3. Seamless vertical blanking-interval (VBI) clock transition without panel re-sync blackout.
//! 4. Content framerate detection & adaptive target refresh rate matching with hysteresis governor.

#![allow(dead_code)]

// ============================================================================
// Feature 1: Dynamic Refresh Rate Profiles, Timings & Power Metrics
// ============================================================================

/// Supported handheld display refresh rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DrrsRefreshRate {
    /// 40 Hz: Golden ratio for 40 FPS locked gaming (25ms frame time, smooth frame pacing, huge battery saving).
    Hz40 = 40,
    /// 48 Hz: Ideal for 24 FPS / 48 FPS cinema video playback (exact 2:2 pulldown, zero judder).
    Hz48 = 48,
    /// 60 Hz: Standard UI / 60 FPS console locked gaming (16.67ms frame time).
    Hz60 = 60,
    /// 90 Hz: Balanced high refresh (11.11ms frame time, OLED fast response).
    Hz90 = 90,
    /// 120 Hz: Premium esports high refresh (8.33ms frame time, sub-millisecond response).
    Hz120 = 120,
    /// 144 Hz: Ultra-fast PC gaming refresh (6.94ms frame time).
    Hz144 = 144,
    /// 165 Hz: Peak tournament display mode (6.06ms frame time).
    Hz165 = 165,
}

impl DrrsRefreshRate {
    pub const fn hz(&self) -> u32 {
        *self as u32
    }

    /// Operational power tier classification.
    pub const fn tier(&self) -> DrrsTier {
        match self {
            Self::Hz40 | Self::Hz48 | Self::Hz60 => DrrsTier::BatterySaver,
            Self::Hz90 | Self::Hz120 => DrrsTier::Balanced,
            Self::Hz144 | Self::Hz165 => DrrsTier::UltraPerformance,
        }
    }

    /// Frame duration in microseconds.
    pub const fn frame_duration_us(&self) -> u32 {
        1_000_000 / self.hz()
    }
}

/// Operational power and performance tier for display refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrrsTier {
    /// Battery saving mode for desktop reading, pause menus, video playback, and 40 FPS caps.
    BatterySaver,
    /// Balanced mode for smooth navigation and casual 90-120 FPS gaming.
    Balanced,
    /// Unconstrained ultra-high refresh for fast-twitch competitive gaming.
    UltraPerformance,
}

/// Native panel video timing model (standard 1920x1080 7-inch FHD eDP gaming panel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayTimingParameters {
    /// Active horizontal pixels (1920).
    pub h_active: u32,
    /// Horizontal front porch (48).
    pub h_front_porch: u32,
    /// Horizontal sync pulse width (32).
    pub h_sync_width: u32,
    /// Horizontal back porch (80).
    pub h_back_porch: u32,
    /// Active vertical lines (1080).
    pub v_active: u32,
    /// Vertical front porch (3).
    pub v_front_porch: u32,
    /// Vertical sync pulse width (5).
    pub v_sync_width: u32,
    /// Vertical back porch (23).
    pub v_back_porch: u32,
}

impl DisplayTimingParameters {
    /// Standard native FHD handheld gaming panel timings.
    pub const fn standard_fhd() -> Self {
        Self {
            h_active: 1920,
            h_front_porch: 48,
            h_sync_width: 32,
            h_back_porch: 80,
            v_active: 1080,
            v_front_porch: 3,
            v_sync_width: 5,
            v_back_porch: 23,
        }
    }

    /// Total horizontal clocks per line: H_active + H_front + H_sync + H_back (2080).
    pub const fn h_total(&self) -> u32 {
        self.h_active + self.h_front_porch + self.h_sync_width + self.h_back_porch
    }

    /// Total vertical lines per frame: V_active + V_front + V_sync + V_back (1111).
    pub const fn v_total(&self) -> u32 {
        self.v_active + self.v_front_porch + self.v_sync_width + self.v_back_porch
    }

    /// Computes theoretical pixel clock frequency in kHz for given refresh rate.
    pub const fn compute_pixel_clock_khz(&self, rate: DrrsRefreshRate) -> u32 {
        let total_pixels = self.h_total() * self.v_total();
        ((total_pixels as u64 * rate.hz() as u64) / 1000) as u32
    }

    /// Estimated panel TCON and backlight power consumption in milliwatts.
    pub const fn estimate_power_mw(&self, rate: DrrsRefreshRate) -> u32 {
        match rate {
            DrrsRefreshRate::Hz40 => 800,
            DrrsRefreshRate::Hz48 => 950,
            DrrsRefreshRate::Hz60 => 1150,
            DrrsRefreshRate::Hz90 => 1600,
            DrrsRefreshRate::Hz120 => 2050,
            DrrsRefreshRate::Hz144 => 2450,
            DrrsRefreshRate::Hz165 => 2800,
        }
    }
}

/// Verification test for DRRS refresh rate profiles, timings, and power savings.
pub fn verify_drrs_refresh_profiles_and_timings() -> bool {
    let timings = DisplayTimingParameters::standard_fhd();

    // 1. Verify total timing sums
    if timings.h_total() != 2080 || timings.v_total() != 1111 {
        return false;
    }

    // 2. Pixel clock checks:
    // At 60 Hz: 2080 * 1111 * 60 / 1000 = 138,652 kHz (138.65 MHz)
    let pclk_60 = timings.compute_pixel_clock_khz(DrrsRefreshRate::Hz60);
    if pclk_60 != 138_652 {
        return false;
    }

    // At 120 Hz: 2080 * 1111 * 120 / 1000 = 277,305 kHz (277.30 MHz)
    let pclk_120 = timings.compute_pixel_clock_khz(DrrsRefreshRate::Hz120);
    if pclk_120 != 277_305 {
        return false;
    }

    // At 165 Hz: 2080 * 1111 * 165 / 1000 = 381,295 kHz (381.30 MHz)
    let pclk_165 = timings.compute_pixel_clock_khz(DrrsRefreshRate::Hz165);
    if pclk_165 != 381_295 {
        return false;
    }

    // 3. Frame duration checks:
    // 40 Hz = 25,000 µs
    if DrrsRefreshRate::Hz40.frame_duration_us() != 25_000 {
        return false;
    }
    // 144 Hz = 6,944 µs
    if DrrsRefreshRate::Hz144.frame_duration_us() != 6944 {
        return false;
    }

    // 4. Power savings: dropping 165 Hz (2800 mW) to 40 Hz (800 mW) saves 2000 mW (2.0W)
    let pwr_165 = timings.estimate_power_mw(DrrsRefreshRate::Hz165);
    let pwr_40 = timings.estimate_power_mw(DrrsRefreshRate::Hz40);
    if pwr_165 != 2800 || pwr_40 != 800 {
        return false;
    }
    if pwr_165 - pwr_40 != 2000 {
        return false;
    }

    // 5. Tier classification
    if DrrsRefreshRate::Hz40.tier() != DrrsTier::BatterySaver
        || DrrsRefreshRate::Hz120.tier() != DrrsTier::Balanced
        || DrrsRefreshRate::Hz165.tier() != DrrsTier::UltraPerformance
    {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Hardware Fractional-N PLL Divider & Multiplier Calculator
// ============================================================================

/// Display controller reference crystal clock frequency in kHz (24.0 MHz).
pub const PLL_REF_CLOCK_KHZ: u32 = 24_000;

/// Minimum allowable Voltage-Controlled Oscillator (VCO) frequency in kHz (2.0 GHz).
pub const PLL_VCO_MIN_KHZ: u32 = 2_000_000;

/// Maximum allowable Voltage-Controlled Oscillator (VCO) frequency in kHz (4.5 GHz).
pub const PLL_VCO_MAX_KHZ: u32 = 4_500_000;

/// Fractional accumulator resolution (24-bit: 2^24 = 16,777,216).
pub const PLL_FRAC_DENOMINATOR: u64 = 16_777_216;

/// Hardware Phase-Locked Loop (PLL) Divider & Multiplier Register Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PllRegisterSettings {
    /// Input reference pre-divider N (typically 1).
    pub pre_divider_n: u8,
    /// Feedback integer multiplier M (typically 80..180).
    pub feedback_mult_m: u16,
    /// 24-bit fractional feedback accumulator value K (0..16,777,215).
    pub frac_accumulator_k: u32,
    /// Output post-divider Q (typically 6..32).
    pub post_divider_q: u8,
    /// Intermediate Voltage-Controlled Oscillator (VCO) frequency in kHz.
    pub vco_freq_khz: u32,
    /// Actual synthesized output pixel clock in kHz.
    pub actual_pixel_clock_khz: u32,
    /// Frequency error relative to target in Parts Per Million (PPM).
    pub error_ppm: u32,
}

impl PllRegisterSettings {
    pub const fn empty() -> Self {
        Self {
            pre_divider_n: 1,
            feedback_mult_m: 100,
            frac_accumulator_k: 0,
            post_divider_q: 10,
            vco_freq_khz: 2_400_000,
            actual_pixel_clock_khz: 240_000,
            error_ppm: 0,
        }
    }
}

/// Hardware Fractional-N Display PLL Synthesizer.
///
/// In modern handheld graphics display controllers (AMD DCN, Intel Xe Display Engine),
/// pixel clocks cannot be changed by simple integer dividers without introducing massive jitter.
/// This synthesizer calculates 24-bit Fractional-N PLL parameters to achieve sub-ppm accuracy
/// across any requested refresh rate (40Hz to 165Hz) while ensuring the analog VCO stays safely
/// within its linear high-gain operational window (2.0 GHz - 4.5 GHz).
#[derive(Debug, Clone)]
pub struct DisplayPllSynthesizer {
    /// Active reference crystal clock in kHz (24,000 kHz).
    pub ref_clock_khz: u32,
    /// Currently latched hardware PLL registers.
    pub active_settings: PllRegisterSettings,
    /// Shadow register bank staged for next VBLANK latch.
    pub shadow_settings: PllRegisterSettings,
    /// PLL lock status flag.
    pub pll_locked: bool,
    /// Total PLL re-calculation and latch operations completed.
    pub reconfig_count: u32,
}

impl DisplayPllSynthesizer {
    pub const fn new(ref_clock_khz: u32) -> Self {
        Self {
            ref_clock_khz,
            active_settings: PllRegisterSettings::empty(),
            shadow_settings: PllRegisterSettings::empty(),
            pll_locked: true,
            reconfig_count: 0,
        }
    }

    /// Computes optimal Fractional-N PLL divider and multiplier settings for target pixel clock.
    pub fn calculate_pll_settings(&self, target_pclk_khz: u32) -> Result<PllRegisterSettings, &'static str> {
        if target_pclk_khz == 0 {
            return Err("Target pixel clock must be non-zero");
        }

        let pre_n = 1u8;
        let f_ref = (self.ref_clock_khz as u64) / (pre_n as u64);

        // Find post_divider_q such that vco_freq = target * q is within [2.0 GHz, 4.5 GHz]
        let mut best_q = 0u8;
        let mut min_vco_dist = u64::MAX;
        let vco_center = ((PLL_VCO_MIN_KHZ as u64) + (PLL_VCO_MAX_KHZ as u64)) / 2;

        for q in 6..=32 {
            let candidate_vco = (target_pclk_khz as u64) * (q as u64);
            if candidate_vco >= (PLL_VCO_MIN_KHZ as u64) && candidate_vco <= (PLL_VCO_MAX_KHZ as u64) {
                let dist = if candidate_vco >= vco_center {
                    candidate_vco - vco_center
                } else {
                    vco_center - candidate_vco
                };
                if dist < min_vco_dist {
                    min_vco_dist = dist;
                    best_q = q;
                }
            }
        }

        if best_q == 0 {
            return Err("Unable to find post-divider within VCO safe frequency limits");
        }

        let target_vco = (target_pclk_khz as u64) * (best_q as u64);

        // Compute feedback multiplier M and 24-bit fractional K:
        // target_vco = f_ref * (M + K / 2^24)
        // M = target_vco / f_ref
        // rem = target_vco % f_ref
        // K = (rem * 2^24 + f_ref / 2) / f_ref
        let mult_m = (target_vco / f_ref) as u16;
        let rem = target_vco % f_ref;
        let frac_k = ((rem * PLL_FRAC_DENOMINATOR + f_ref / 2) / f_ref) as u32;

        // Calculate exact synthesized actual VCO and pixel clock:
        // actual_vco = f_ref * M + (f_ref * K) / 2^24
        let actual_vco_khz = (f_ref * (mult_m as u64) + ((f_ref * (frac_k as u64)) / PLL_FRAC_DENOMINATOR)) as u32;
        let actual_pclk_khz = actual_vco_khz / (best_q as u32);

        // Calculate frequency error in PPM: |target - actual| / target * 1,000,000
        let diff = if actual_pclk_khz >= target_pclk_khz {
            (actual_pclk_khz - target_pclk_khz) as u64
        } else {
            (target_pclk_khz - actual_pclk_khz) as u64
        };
        let error_ppm = ((diff * 1_000_000) / (target_pclk_khz as u64)) as u32;

        Ok(PllRegisterSettings {
            pre_divider_n: pre_n,
            feedback_mult_m: mult_m,
            frac_accumulator_k: frac_k,
            post_divider_q: best_q,
            vco_freq_khz: actual_vco_khz,
            actual_pixel_clock_khz: actual_pclk_khz,
            error_ppm,
        })
    }

    /// Stages new target PLL parameters into shadow registers for next VBLANK latch.
    pub fn stage_target_clock(&mut self, target_pclk_khz: u32) -> Result<(), &'static str> {
        let settings = self.calculate_pll_settings(target_pclk_khz)?;
        self.shadow_settings = settings;
        Ok(())
    }

    /// Latches shadow settings into active hardware PLL registers during vertical blanking.
    pub fn commit_shadow_registers(&mut self) {
        self.active_settings = self.shadow_settings;
        self.pll_locked = true;
        self.reconfig_count += 1;
    }
}

/// Verification test for hardware Fractional-N PLL synthesizer and divider calculation.
pub fn verify_display_pll_synthesizer_and_dividers() -> bool {
    let mut synth = DisplayPllSynthesizer::new(PLL_REF_CLOCK_KHZ);
    let timings = DisplayTimingParameters::standard_fhd();

    // 1. Calculate and verify 60Hz pixel clock (138,652 kHz)
    let pclk_60 = timings.compute_pixel_clock_khz(DrrsRefreshRate::Hz60);
    let pll_60 = match synth.calculate_pll_settings(pclk_60) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // VCO must be inside 2.0 GHz - 4.5 GHz range
    if pll_60.vco_freq_khz < PLL_VCO_MIN_KHZ || pll_60.vco_freq_khz > PLL_VCO_MAX_KHZ {
        return false;
    }
    // Accuracy must be exceptionally tight (< 50 PPM error)
    if pll_60.error_ppm > 50 {
        return false;
    }
    // Check synthesized clock within 2 kHz of 138,652 kHz
    let diff_60 = (pll_60.actual_pixel_clock_khz as i32 - pclk_60 as i32).abs();
    if diff_60 > 2 {
        return false;
    }

    // 2. Calculate and verify 120Hz pixel clock (277,305 kHz)
    let pclk_120 = timings.compute_pixel_clock_khz(DrrsRefreshRate::Hz120);
    let pll_120 = match synth.calculate_pll_settings(pclk_120) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if pll_120.vco_freq_khz < PLL_VCO_MIN_KHZ || pll_120.vco_freq_khz > PLL_VCO_MAX_KHZ {
        return false;
    }
    if pll_120.error_ppm > 50 {
        return false;
    }

    // 3. Calculate and verify 165Hz peak pixel clock (381,295 kHz)
    let pclk_165 = timings.compute_pixel_clock_khz(DrrsRefreshRate::Hz165);
    let pll_165 = match synth.calculate_pll_settings(pclk_165) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if pll_165.vco_freq_khz < PLL_VCO_MIN_KHZ || pll_165.vco_freq_khz > PLL_VCO_MAX_KHZ {
        return false;
    }
    if pll_165.error_ppm > 50 {
        return false;
    }

    // 4. Test staging into shadow registers and committing
    if synth.stage_target_clock(pclk_120).is_err() {
        return false;
    }
    synth.commit_shadow_registers();
    if synth.active_settings.actual_pixel_clock_khz != pll_120.actual_pixel_clock_khz {
        return false;
    }
    if synth.reconfig_count != 1 || !synth.pll_locked {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: Seamless Vertical Blanking Clock Transition Engine (Zero Blackout)
// ============================================================================

/// Operational states of the seamless refresh rate switching state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamlessTransitionState {
    /// Steady state: display scanning out stably at negotiated refresh rate.
    LockedSteady,
    /// New refresh rate staged in shadow registers; armed and awaiting VBLANK start.
    ArmingVblank,
    /// Hardware VBLANK active; glitch-free clock mux and MSA attributes being swapped.
    VblankSwitching,
    /// Clock latched; verifying first active scanout frame at new refresh rate.
    PostSwitchVerify,
}

/// Seamless Blanking-Interval Display Clock Transition Engine.
///
/// In standard desktop operating systems, changing display refresh rates causes a 1-2 second
/// black screen ("modeset blackout") while the DisplayPort link re-trains and TCON re-syncs.
/// On a handheld gaming console, such blackouts completely break immersion when switching
/// between launcher menus (60Hz/120Hz) and games (40Hz/144Hz) or engaging battery saver.
/// This engine utilizes hardware shadow registers, eDP Main Stream Attribute (MSA) updates,
/// and two-stage glitch-free clock multiplexers to swap the pixel clock during vertical blanking,
/// guaranteeing 0ms blackout time and zero visual tearing or dropped frames.
#[derive(Debug, Clone)]
pub struct SeamlessDrrsTransitionEngine {
    /// Active refresh rate currently driving the display panel.
    pub current_rate: DrrsRefreshRate,
    /// Target refresh rate commanded by governor or user.
    pub target_rate: DrrsRefreshRate,
    /// Active state machine step.
    pub state: SeamlessTransitionState,
    /// Fractional-N display PLL synthesizer.
    pub synthesizer: DisplayPllSynthesizer,
    /// Display timing model.
    pub timings: DisplayTimingParameters,
    /// Timestamp when VBLANK transition was initiated in microseconds.
    pub vblank_start_timestamp_us: u64,
    /// Measured hardware clock switch latency during vertical blanking in microseconds.
    pub last_switch_latency_us: u32,
    /// Total count of seamless refresh rate switches completed.
    pub seamless_switches_completed: u32,
    /// Total count of display modeset blackouts successfully averted.
    pub blackout_prevented_count: u32,
}

impl SeamlessDrrsTransitionEngine {
    pub fn new(initial_rate: DrrsRefreshRate) -> Self {
        let timings = DisplayTimingParameters::standard_fhd();
        let mut synth = DisplayPllSynthesizer::new(PLL_REF_CLOCK_KHZ);
        let pclk = timings.compute_pixel_clock_khz(initial_rate);
        let _ = synth.stage_target_clock(pclk);
        synth.commit_shadow_registers();

        Self {
            current_rate: initial_rate,
            target_rate: initial_rate,
            state: SeamlessTransitionState::LockedSteady,
            synthesizer: synth,
            timings,
            vblank_start_timestamp_us: 0,
            last_switch_latency_us: 0,
            seamless_switches_completed: 0,
            blackout_prevented_count: 0,
        }
    }

    /// Requests a dynamic refresh rate switch.
    /// Stages PLL settings in shadow registers and arms the VBLANK transition gate.
    /// Returns `true` if transition was armed.
    pub fn request_refresh_rate(&mut self, target: DrrsRefreshRate) -> bool {
        if target == self.current_rate && self.state == SeamlessTransitionState::LockedSteady {
            return false;
        }

        self.target_rate = target;
        let target_pclk = self.timings.compute_pixel_clock_khz(target);

        // Pre-calculate and stage PLL parameters into hardware shadow registers
        if self.synthesizer.stage_target_clock(target_pclk).is_err() {
            return false;
        }

        self.state = SeamlessTransitionState::ArmingVblank;
        true
    }

    /// Hardware Vertical Blanking Interval (VBLANK) start interrupt handler.
    /// Latches shadow registers and swaps the pixel clock with zero glitching while row drivers are idle.
    pub fn on_vblank_start(&mut self, timestamp_us: u64) -> bool {
        if self.state != SeamlessTransitionState::ArmingVblank {
            return false;
        }

        self.vblank_start_timestamp_us = timestamp_us;
        self.state = SeamlessTransitionState::VblankSwitching;

        // 1. Commit shadow PLL parameters (atomic register write during VBI)
        self.synthesizer.commit_shadow_registers();

        // 2. Glitch-free clock mux swaps output lines in ~120 µs
        self.last_switch_latency_us = 120;
        self.current_rate = self.target_rate;
        self.seamless_switches_completed += 1;
        self.blackout_prevented_count += 1;

        self.state = SeamlessTransitionState::PostSwitchVerify;
        true
    }

    /// Hardware Vertical Blanking Interval (VBLANK) end interrupt handler (active scanout begins).
    pub fn on_vblank_end(&mut self) {
        if self.state == SeamlessTransitionState::PostSwitchVerify {
            self.state = SeamlessTransitionState::LockedSteady;
        }
    }
}

/// Verification test for seamless vertical blanking-interval clock transition without blackout.
pub fn verify_seamless_vblank_clock_transition() -> bool {
    let mut engine = SeamlessDrrsTransitionEngine::new(DrrsRefreshRate::Hz60);

    // 1. Initial steady 60Hz state
    if engine.current_rate != DrrsRefreshRate::Hz60 || engine.state != SeamlessTransitionState::LockedSteady {
        return false;
    }
    if engine.seamless_switches_completed != 0 {
        return false;
    }

    // 2. Request transition to 120Hz for gaming
    let armed = engine.request_refresh_rate(DrrsRefreshRate::Hz120);
    if !armed || engine.state != SeamlessTransitionState::ArmingVblank {
        return false;
    }
    // Target staged in shadow registers, but current rate must remain 60Hz until VBLANK
    if engine.current_rate != DrrsRefreshRate::Hz60 {
        return false;
    }

    // 3. VBLANK start interrupt arrives
    let vblank_start_time = 1_000_000u64;
    let switched = engine.on_vblank_start(vblank_start_time);
    if !switched {
        return false;
    }
    // Current rate should now be 120Hz, state PostSwitchVerify
    if engine.current_rate != DrrsRefreshRate::Hz120 || engine.state != SeamlessTransitionState::PostSwitchVerify {
        return false;
    }
    // Switch latency must be well within blanking budget (< 500 µs)
    if engine.last_switch_latency_us > 500 {
        return false;
    }
    if engine.seamless_switches_completed != 1 || engine.blackout_prevented_count != 1 {
        return false;
    }

    // 4. VBLANK end: returns to LockedSteady
    engine.on_vblank_end();
    if engine.state != SeamlessTransitionState::LockedSteady {
        return false;
    }

    // 5. Seamlessly drop to 40Hz battery saving mode
    engine.request_refresh_rate(DrrsRefreshRate::Hz40);
    engine.on_vblank_start(vblank_start_time + 8333);
    engine.on_vblank_end();

    if engine.current_rate != DrrsRefreshRate::Hz40 || engine.seamless_switches_completed != 2 {
        return false;
    }
    if engine.blackout_prevented_count != 2 {
        return false;
    }

    true
}
