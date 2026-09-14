//! Handheld Fanless Whisper Mode & Acoustic Noise-Constrained Frequency Governor.

#![allow(dead_code)]

/// Operating profile modes with acoustic constraints and TDP ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperOperatingProfile {
    Silent,   // 0 dB, passive cooling, TDP capped at 6W
    Whisper,  // Acoustic ceiling < 24 dBA, max 1800 RPM, TDP dynamic 8-15W
    Balanced, // Acoustic ceiling < 32 dBA, max 3200 RPM, TDP 15-22W
    Turbo,    // Unconstrained cooling, full APU 28W+
}

impl WhisperOperatingProfile {
    /// Returns the acoustic ceiling in dBA for the profile.
    pub const fn acoustic_ceiling_dba(&self) -> u8 {
        match self {
            Self::Silent => 0,
            Self::Whisper => 24,
            Self::Balanced => 32,
            Self::Turbo => 50,
        }
    }

    /// Returns the maximum allowable fan RPM for the profile.
    pub const fn max_fan_rpm(&self) -> u16 {
        match self {
            Self::Silent => 0,
            Self::Whisper => 1800,
            Self::Balanced => 3200,
            Self::Turbo => 6500,
        }
    }

    /// Returns the base TDP ceiling in Watts.
    pub const fn tdp_ceiling_watts(&self) -> u8 {
        match self {
            Self::Silent => 6,
            Self::Whisper => 15,
            Self::Balanced => 22,
            Self::Turbo => 35,
        }
    }
}

/// Unit verification for operating profile modes.
pub fn verify_operating_profiles() -> bool {
    let silent = WhisperOperatingProfile::Silent;
    if silent.acoustic_ceiling_dba() != 0 || silent.max_fan_rpm() != 0 || silent.tdp_ceiling_watts() != 6 {
        return false;
    }

    let whisper = WhisperOperatingProfile::Whisper;
    if whisper.acoustic_ceiling_dba() != 24 || whisper.max_fan_rpm() != 1800 || whisper.tdp_ceiling_watts() != 15 {
        return false;
    }

    let balanced = WhisperOperatingProfile::Balanced;
    if balanced.acoustic_ceiling_dba() != 32 || balanced.max_fan_rpm() != 3200 || balanced.tdp_ceiling_watts() != 22 {
        return false;
    }

    let turbo = WhisperOperatingProfile::Turbo;
    if turbo.acoustic_ceiling_dba() != 50 || turbo.max_fan_rpm() != 6500 || turbo.tdp_ceiling_watts() != 35 {
        return false;
    }

    true
}

/// Ambient room baseline noise in dBA (typical quiet bedroom: 18 dBA).
pub const AMBIENT_ACOUSTIC_BASELINE_DBA: u8 = 18;

/// Real-time dual-fan acoustic noise estimator modeling sound pressure level (dBA).
#[derive(Debug, Clone, Copy)]
pub struct AcousticNoiseEstimator {
    pub ambient_baseline_dba: u8,
    pub fan0_rpm: u16,
    pub fan1_rpm: u16,
    pub estimated_dba: u8,
}

impl AcousticNoiseEstimator {
    /// Creates a new acoustic noise estimator with 18 dBA ambient quiet baseline.
    pub const fn new() -> Self {
        Self {
            ambient_baseline_dba: AMBIENT_ACOUSTIC_BASELINE_DBA,
            fan0_rpm: 0,
            fan1_rpm: 0,
            estimated_dba: AMBIENT_ACOUSTIC_BASELINE_DBA,
        }
    }

    /// Estimates sound pressure level (dBA) from dual-fan RPM using logarithmic modeling:
    /// SPL ~ baseline + 20 * log10(RPM / 1000) for RPM > 1000.
    pub fn estimate_spl_dba(&mut self, fan0_rpm: u16, fan1_rpm: u16) -> u8 {
        self.fan0_rpm = fan0_rpm;
        self.fan1_rpm = fan1_rpm;

        let max_rpm = if fan0_rpm > fan1_rpm { fan0_rpm } else { fan1_rpm };
        if max_rpm == 0 {
            self.estimated_dba = 0; // Pure silent passive mode
            return 0;
        }

        // Low speed (< 1000 RPM): barely above ambient noise floor
        if max_rpm < 1000 {
            self.estimated_dba = self.ambient_baseline_dba + 2;
            return self.estimated_dba;
        }

        // Fan noise table approximation (logarithmic curve):
        // 1000 - 1500 RPM: 20 - 22 dBA
        // 1500 - 2000 RPM: 22 - 25 dBA
        // 2000 - 3000 RPM: 25 - 30 dBA
        // 3000 - 4500 RPM: 30 - 38 dBA
        // 4500+ RPM: 38 - 48 dBA
        let dba = if max_rpm <= 1500 {
            20 + (((max_rpm - 1000) * 2) / 500) as u8
        } else if max_rpm <= 2000 {
            22 + (((max_rpm - 1500) * 3) / 500) as u8
        } else if max_rpm <= 3000 {
            25 + (((max_rpm - 2000) * 5) / 1000) as u8
        } else if max_rpm <= 4500 {
            30 + (((max_rpm - 3000) * 8) / 1500) as u8
        } else {
            let extra = (((max_rpm - 4500) * 10) / 2000) as u8;
            (38 + extra).min(55)
        };

        // Dual-fan acoustic addition: two identical coherent/incoherent sources add approx +3 dBA
        let dual_penalty = if fan0_rpm > 1000 && fan1_rpm > 1000 { 3 } else { 0 };
        self.estimated_dba = (dba + dual_penalty).max(self.ambient_baseline_dba);
        self.estimated_dba
    }
}

/// Unit verification for real-time acoustic noise estimator.
pub fn verify_acoustic_noise_estimator() -> bool {
    let mut est = AcousticNoiseEstimator::new();

    // 0 RPM: pure silent 0 dBA
    if est.estimate_spl_dba(0, 0) != 0 {
        return false;
    }

    // 1500 RPM single fan: ~22 dBA (well below 24 dBA whisper ceiling)
    let dba_1500 = est.estimate_spl_dba(1500, 0);
    if dba_1500 < 20 || dba_1500 > 24 {
        return false;
    }

    // 1800 RPM dual fan: 23 + 3 = 26 dBA
    let dba_dual_1800 = est.estimate_spl_dba(1800, 1800);
    if dba_dual_1800 < 25 || dba_dual_1800 > 28 {
        return false;
    }

    // 5000 RPM high speed: >= 40 dBA
    let dba_5000 = est.estimate_spl_dba(5000, 5000);
    if dba_5000 < 40 {
        return false;
    }

    true
}

/// Transient thermal spike damper and PID filter.
/// Prevents sudden high-pitch fan spin-ups during short (<3s) shader compilation bursts.
#[derive(Debug, Clone, Copy)]
pub struct ThermalSpikeDamper {
    pub burst_filter_window_ms: u32,
    pub accumulated_heat_energy: i32,
    pub target_temp_c: i16,
    pub filtered_temp_c: i16,
    pub kp_q8: i32, // Proportional gain
    pub ki_q8: i32, // Integral gain
    pub kd_q8: i32, // Derivative gain
    pub integral_error: i32,
    pub last_error: i32,
    pub damped_fan_rpm: u16,
}

impl ThermalSpikeDamper {
    /// Creates a new thermal spike damper with 3000ms low-pass thermal inertia window.
    pub const fn new() -> Self {
        Self {
            burst_filter_window_ms: 3000,
            accumulated_heat_energy: 0,
            target_temp_c: 70, // 70°C target steady-state temperature
            filtered_temp_c: 45,
            kp_q8: 256, // 1.0
            ki_q8: 32,  // 0.125
            kd_q8: 64,  // 0.25
            integral_error: 0,
            last_error: 0,
            damped_fan_rpm: 0,
        }
    }

    /// Evaluates raw instantaneous APU temperature and applies PID thermal inertia damping.
    /// Returns the filtered and damped fan RPM recommendation bounded by max_rpm_limit.
    pub fn step_damping(&mut self, raw_temp_c: i16, delta_ms: u32, max_rpm_limit: u16) -> u16 {
        // First order exponential moving average for temperature (thermal capacity inertia)
        // alpha ~ delta_ms / burst_filter_window_ms
        let alpha_q8 = ((delta_ms as i32 * 256) / self.burst_filter_window_ms as i32).min(256);
        let temp_diff = raw_temp_c as i32 - self.filtered_temp_c as i32;
        self.filtered_temp_c += ((temp_diff * alpha_q8) / 256) as i16;

        // PID calculation against target_temp_c
        let error = self.filtered_temp_c as i32 - self.target_temp_c as i32;
        self.integral_error = (self.integral_error + error).clamp(-1000, 1000);
        let derivative = error - self.last_error;
        self.last_error = error;

        // Only increase RPM if above target temp or experiencing prolonged load
        if error > 0 {
            let p_term = (error * self.kp_q8) / 256;
            let i_term = (self.integral_error * self.ki_q8) / 256;
            let d_term = (derivative * self.kd_q8) / 256;
            let rpm_add = (p_term + i_term + d_term) * 50; // 50 RPM per °C error
            let target_rpm = (1200 + rpm_add).max(0) as u16;
            self.damped_fan_rpm = target_rpm.min(max_rpm_limit);
        } else {
            // Under target temperature: maintain low or silent RPM
            self.damped_fan_rpm = self.damped_fan_rpm.saturating_sub(50);
        }

        self.damped_fan_rpm
    }
}

/// Unit verification for transient thermal spike damper.
pub fn verify_thermal_spike_damper() -> bool {
    let mut damper = ThermalSpikeDamper::new();
    damper.filtered_temp_c = 45;

    // Simulate 500ms sudden temperature spike to 85°C (shader compile burst)
    let rpm_after_spike = damper.step_damping(85, 500, 1800);
    // Thermal inertia prevents jumping straight to 1800 RPM immediately
    if rpm_after_spike > 1400 {
        return false;
    }

    // Steady state below target 70°C (e.g. 60°C): fan remains quiet
    damper.filtered_temp_c = 60;
    let rpm_quiet = damper.step_damping(60, 1000, 1800);
    if rpm_quiet > 500 {
        return false;
    }

    true
}
