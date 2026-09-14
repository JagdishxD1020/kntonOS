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
