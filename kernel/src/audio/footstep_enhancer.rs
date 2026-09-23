//! Handheld Directional Footstep Audio Enhancer & Headphone Amp.

#![allow(dead_code)]

/// Headphone load impedance classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadphoneImpedanceClass {
    /// In-Ear Monitors (IEMs) & low-impedance earbuds (16 .. 32 ohms).
    LowImpedanceIem,
    /// Standard consumer headphones (33 .. 150 ohms).
    MidImpedance,
    /// High-impedance studio audiophile headphones (151 .. 600 ohms).
    HighImpedanceStudio,
}

impl HeadphoneImpedanceClass {
    /// Returns recommended analog DAC gain boost in decibels.
    pub const fn gain_boost_db(self) -> f32 {
        match self {
            Self::LowImpedanceIem => 0.0,
            Self::MidImpedance => 6.0,
            Self::HighImpedanceStudio => 12.0,
        }
    }

    /// Linear voltage gain multiplier corresponding to gain_boost_db.
    pub fn linear_gain(self) -> f32 {
        match self {
            Self::LowImpedanceIem => 1.0,
            Self::MidImpedance => 1.9952623, // 10^(6/20) ~ 2.0
            Self::HighImpedanceStudio => 3.9810717, // 10^(12/20) ~ 4.0
        }
    }
}

/// Headphone jack impedance auto-detector and analog output amplifier controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadphoneAmpDetector {
    /// Detected load impedance in ohms.
    pub detected_ohms: f32,
    /// Active impedance classification.
    pub impedance_class: HeadphoneImpedanceClass,
    /// Current output gain multiplier.
    pub active_gain: f32,
    /// Headphone jack plug inserted.
    pub is_plugged: bool,
}

impl HeadphoneAmpDetector {
    pub const fn new() -> Self {
        Self {
            detected_ohms: 32.0,
            impedance_class: HeadphoneImpedanceClass::LowImpedanceIem,
            active_gain: 1.0,
            is_plugged: false,
        }
    }

    /// Senses jack insertion and performs test-tone load impedance measurement.
    pub fn detect_load(&mut self, sensed_ohms: f32) -> HeadphoneImpedanceClass {
        self.is_plugged = true;
        self.detected_ohms = sensed_ohms.max(1.0);

        if self.detected_ohms <= 32.5 {
            self.impedance_class = HeadphoneImpedanceClass::LowImpedanceIem;
        } else if self.detected_ohms <= 150.0 {
            self.impedance_class = HeadphoneImpedanceClass::MidImpedance;
        } else {
            self.impedance_class = HeadphoneImpedanceClass::HighImpedanceStudio;
        }

        self.active_gain = self.impedance_class.linear_gain();
        self.impedance_class
    }

    /// Senses headphone jack unplug event.
    pub fn on_unplug(&mut self) {
        self.is_plugged = false;
        self.detected_ohms = 32.0;
        self.impedance_class = HeadphoneImpedanceClass::LowImpedanceIem;
        self.active_gain = 1.0;
    }
}

/// Unit test verifying headphone load impedance detection and DAC gain stage selection.
pub fn verify_impedance_detection_and_gain() -> bool {
    let mut amp = HeadphoneAmpDetector::new();
    if amp.is_plugged {
        return false;
    }

    // 1. 16-ohm IEM insertion (e.g. Moondrop Chu) -> LowImpedanceIem (0 dB, 1.0x gain)
    let c1 = amp.detect_load(16.0);
    if c1 != HeadphoneImpedanceClass::LowImpedanceIem || (amp.active_gain - 1.0).abs() > 0.001 {
        return false;
    }

    // 2. 80-ohm consumer headphones (e.g. DT 770 80 ohm) -> MidImpedance (+6 dB, ~2.0x gain)
    let c2 = amp.detect_load(80.0);
    if c2 != HeadphoneImpedanceClass::MidImpedance || (amp.active_gain - 2.0).abs() > 0.05 {
        return false;
    }

    // 3. 250-ohm studio headphones (e.g. DT 990 Pro 250 ohm) -> HighImpedanceStudio (+12 dB, ~4.0x gain)
    let c3 = amp.detect_load(250.0);
    if c3 != HeadphoneImpedanceClass::HighImpedanceStudio || (amp.active_gain - 4.0).abs() > 0.05 {
        return false;
    }

    // 4. Unplug event
    amp.on_unplug();
    if amp.is_plugged || amp.active_gain != 1.0 {
        return false;
    }

    true
}
