//! Handheld PC Multi-Zone Thermal Governor and Dynamic Fan Controller.

/// Thermal sensor zones monitored across handheld gaming hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandheldThermalZone {
    CpuPackage,
    IgpuHotspot,
    BatteryThermistor,
    AmbientChassis,
}

/// Instantaneous temperature reading in millidegrees Celsius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThermalReading {
    pub zone: HandheldThermalZone,
    pub raw_millicelsius: i32,
    pub filtered_millicelsius: i32,
    pub timestamp_ms: u64,
}

impl ThermalReading {
    pub const fn from_celsius(zone: HandheldThermalZone, celsius: f32, timestamp_ms: u64) -> Self {
        let mc = (celsius * 1000.0) as i32;
        Self {
            zone,
            raw_millicelsius: mc,
            filtered_millicelsius: mc,
            timestamp_ms,
        }
    }

    #[inline]
    pub fn celsius(&self) -> f32 {
        self.filtered_millicelsius as f32 / 1000.0
    }
}

/// Moving average filter state for a single thermal sensor zone.
#[derive(Clone, Copy, Debug)]
pub struct ZoneFilter {
    samples: [i32; 4],
    index: usize,
    count: usize,
    last_filtered: i32,
}

impl ZoneFilter {
    pub const fn new() -> Self {
        Self {
            samples: [0; 4],
            index: 0,
            count: 0,
            last_filtered: 0,
        }
    }

    pub fn push(&mut self, sample_mc: i32) -> i32 {
        self.samples[self.index] = sample_mc;
        self.index = (self.index + 1) % 4;
        if self.count < 4 {
            self.count += 1;
        }

        let mut sum: i64 = 0;
        for i in 0..self.count {
            sum += self.samples[i] as i64;
        }
        let filtered = (sum / (self.count as i64)) as i32;
        self.last_filtered = filtered;
        filtered
    }
}

/// Multi-zone thermal sensor monitor managing telemetry across all zones.
pub struct MultiZoneThermalMonitor {
    filters: [ZoneFilter; 4],
    readings: [ThermalReading; 4],
}

impl MultiZoneThermalMonitor {
    pub const fn new() -> Self {
        Self {
            filters: [ZoneFilter::new(); 4],
            readings: [
                ThermalReading::from_celsius(HandheldThermalZone::CpuPackage, 35.0, 0),
                ThermalReading::from_celsius(HandheldThermalZone::IgpuHotspot, 35.0, 0),
                ThermalReading::from_celsius(HandheldThermalZone::BatteryThermistor, 25.0, 0),
                ThermalReading::from_celsius(HandheldThermalZone::AmbientChassis, 25.0, 0),
            ],
        }
    }

    #[inline]
    fn zone_to_index(zone: HandheldThermalZone) -> usize {
        match zone {
            HandheldThermalZone::CpuPackage => 0,
            HandheldThermalZone::IgpuHotspot => 1,
            HandheldThermalZone::BatteryThermistor => 2,
            HandheldThermalZone::AmbientChassis => 3,
        }
    }

    pub fn update_reading(&mut self, zone: HandheldThermalZone, raw_mc: i32, timestamp_ms: u64) -> f32 {
        let idx = Self::zone_to_index(zone);
        let filtered = self.filters[idx].push(raw_mc);
        self.readings[idx] = ThermalReading {
            zone,
            raw_millicelsius: raw_mc,
            filtered_millicelsius: filtered,
            timestamp_ms,
        };
        filtered as f32 / 1000.0
    }

    pub fn get_reading(&self, zone: HandheldThermalZone) -> ThermalReading {
        let idx = Self::zone_to_index(zone);
        self.readings[idx]
    }

    pub fn max_silicon_temperature(&self) -> f32 {
        let cpu_t = self.get_reading(HandheldThermalZone::CpuPackage).celsius();
        let gpu_t = self.get_reading(HandheldThermalZone::IgpuHotspot).celsius();
        if cpu_t > gpu_t { cpu_t } else { gpu_t }
    }
}

/// Verifies multi-zone thermal sensor reading and moving average smoothing pipeline.
pub fn verify_multizone_thermal_sensors() -> bool {
    let mut monitor = MultiZoneThermalMonitor::new();

    if monitor.get_reading(HandheldThermalZone::CpuPackage).celsius() < 30.0 {
        return false;
    }

    monitor.update_reading(HandheldThermalZone::CpuPackage, 60_000, 100);
    monitor.update_reading(HandheldThermalZone::CpuPackage, 62_000, 200);
    monitor.update_reading(HandheldThermalZone::CpuPackage, 64_000, 300);
    let filtered_c = monitor.update_reading(HandheldThermalZone::CpuPackage, 66_000, 400);

    if (filtered_c - 63.0).abs() > 0.05 {
        return false;
    }

    monitor.update_reading(HandheldThermalZone::IgpuHotspot, 72_000, 400);
    if (monitor.max_silicon_temperature() - 72.0).abs() > 0.1 {
        return false;
    }

    true
}
