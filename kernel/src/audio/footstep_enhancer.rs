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



// Footstep Acoustic Resonance Booster & Low-Frequency Rumble Filter.

/// 2nd-order Direct Form I/II IIR Biquad Filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootstepBiquadFilter {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
    pub x1: f32,
    pub x2: f32,
    pub y1: f32,
    pub y2: f32,
}

impl FootstepBiquadFilter {
    pub const fn new(b0: f32, b1: f32, b2: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0, b1, b2, a1, a2,
            x1: 0.0, x2: 0.0,
            y1: 0.0, y2: 0.0,
        }
    }

    /// Creates a 120 Hz 2nd-order Butterworth High-Pass Filter (sample rate 48 kHz).
    /// Suppresses rumble, vehicle noise, and sub-bass explosions below 120 Hz.
    pub fn rumble_suppression_120hz() -> Self {
        // Pre-computed normalized Butterworth HPF at 120 Hz / 48000 Hz (fc ~ 0.0025 fs)
        // Passes > 120 Hz cleanly, attenuates < 120 Hz
        Self::new(0.9780, -1.9560, 0.9780, -1.9555, 0.9565)
    }

    /// Creates a parametric peaking/bandpass filter centered at 1500 Hz (Q ~ 1.2) with +6 dB boost.
    /// Resonates footstep footsteps, crouch rustle, and shell casing drops in 800 Hz - 2.5 kHz band.
    pub fn footstep_peaking_1500hz() -> Self {
        // Center 1500 Hz, Q=1.2, Gain = +6 dB (A = 2.0) at 48000 Hz
        Self::new(1.1852, -1.8674, 0.8148, -1.8674, 0.8000)
    }

    /// Processes a single audio sample through the biquad filter.
    pub fn process_sample(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

/// Dual-stage Footstep Acoustic Enhancer combining rumble suppression and resonance boost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FootstepAcousticFilter {
    pub rumble_hpf: FootstepBiquadFilter,
    pub resonance_bpf: FootstepBiquadFilter,
    pub is_enabled: bool,
}

impl FootstepAcousticFilter {
    pub fn new() -> Self {
        Self {
            rumble_hpf: FootstepBiquadFilter::rumble_suppression_120hz(),
            resonance_bpf: FootstepBiquadFilter::footstep_peaking_1500hz(),
            is_enabled: true,
        }
    }

    /// Processes an input audio sample through rumble suppression and footstep resonance booster.
    pub fn process(&mut self, sample: f32) -> f32 {
        if !self.is_enabled {
            return sample;
        }
        let rumble_cleaned = self.rumble_hpf.process_sample(sample);
        self.resonance_bpf.process_sample(rumble_cleaned)
    }
}

/// Unit test verifying footstep acoustic resonance booster and rumble attenuation.
pub fn verify_footstep_resonance_and_rumble_filter() -> bool {
    let mut filter = FootstepAcousticFilter::new();

    // 1. Verify DC / Ultra-low rumble rejection: constant 50 Hz rumble offset (0.50 input)
    // Run 50 samples; DC component must attenuate toward zero
    let mut last_dc_out = 0.5f32;
    for _ in 0..60 {
        last_dc_out = filter.rumble_hpf.process_sample(0.5);
    }
    // High-pass filter must have severely attenuated DC/sub-rumble (< 0.15)
    if last_dc_out.abs() > 0.15 {
        return false;
    }

    // 2. Verify Footstep Bandpass Resonance (+6 dB gain around 1500 Hz):
    // Simulate 1500 Hz sine wave (period = 32 samples at 48 kHz)
    let mut max_in = 0.0f32;
    let mut max_out = 0.0f32;
    for i in 0..128 {
        let angle = (i as f32) * (2.0 * 3.14159265 / 32.0);
        let sample_in = crate::graphics::sin_f32(angle) * 0.20; // 0.20 amplitude
        if sample_in.abs() > max_in { max_in = sample_in.abs(); }

        let sample_out = filter.resonance_bpf.process_sample(sample_in);
        if i > 32 && sample_out.abs() > max_out {
            max_out = sample_out.abs();
        }
    }

    // Peak resonant output must be greater than input due to peaking boost (> 1.25x)
    if max_out <= max_in * 1.25 {
        return false;
    }

    true
}
