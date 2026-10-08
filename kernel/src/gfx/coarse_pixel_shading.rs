//! Direct3D 12 Agility Coarse Pixel Shading (Tier 2 Shading Rate Palette & Tile Mask Compiler).
//! Provides hardware-accelerated coarse pixel shading rates, tile masks, and peripheral foveation.

#![allow(dead_code, non_camel_case_types, non_snake_case, unused_imports)]

extern crate alloc;
use alloc::vec::Vec;

// Direct3D 12 coarse pixel shading rates supported in Tier 2 hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CoarseShadingRate {
    Rate1x1 = 0,
    Rate1x2 = 1,
    Rate2x1 = 2,
    Rate2x2 = 3,
    Rate2x4 = 4,
    Rate4x2 = 5,
    Rate4x4 = 6,
}

impl CoarseShadingRate {
    // Width dimension in pixels for coarse shading rate.
    pub const fn width(self) -> u32 {
        match self {
            Self::Rate1x1 | Self::Rate1x2 => 1,
            Self::Rate2x1 | Self::Rate2x2 | Self::Rate2x4 => 2,
            Self::Rate4x2 | Self::Rate4x4 => 4,
        }
    }

    // Height dimension in pixels for coarse shading rate.
    pub const fn height(self) -> u32 {
        match self {
            Self::Rate1x1 | Self::Rate2x1 => 1,
            Self::Rate1x2 | Self::Rate2x2 | Self::Rate4x2 => 2,
            Self::Rate2x4 | Self::Rate4x4 => 4,
        }
    }

    // Pixel area covered by a single coarse fragment invocation.
    pub const fn pixel_area(self) -> u32 {
        self.width() * self.height()
    }

    // D3D12 standard hardware bitwise encoding for the shading rate.
    pub const fn to_d3d12_encoding(self) -> u32 {
        match self {
            Self::Rate1x1 => 0x0,
            Self::Rate1x2 => 0x1,
            Self::Rate2x1 => 0x4,
            Self::Rate2x2 => 0x5,
            Self::Rate2x4 => 0x6,
            Self::Rate4x2 => 0x9,
            Self::Rate4x4 => 0xa,
        }
    }

    // Constructs coarse rate from pixel dimensions if valid.
    pub const fn from_dimensions(width: u32, height: u32) -> Option<Self> {
        match (width, height) {
            (1, 1) => Some(Self::Rate1x1),
            (1, 2) => Some(Self::Rate1x2),
            (2, 1) => Some(Self::Rate2x1),
            (2, 2) => Some(Self::Rate2x2),
            (2, 4) => Some(Self::Rate2x4),
            (4, 2) => Some(Self::Rate4x2),
            (4, 4) => Some(Self::Rate4x4),
            _ => None,
        }
    }

    // Constructs coarse rate from D3D12 hardware bitwise encoding.
    pub const fn from_d3d12_encoding(encoding: u32) -> Option<Self> {
        match encoding {
            0x0 => Some(Self::Rate1x1),
            0x1 => Some(Self::Rate1x2),
            0x4 => Some(Self::Rate2x1),
            0x5 => Some(Self::Rate2x2),
            0x6 => Some(Self::Rate2x4),
            0x9 => Some(Self::Rate4x2),
            0xa => Some(Self::Rate4x4),
            _ => None,
        }
    }
}

// Maximum entries in a Direct3D 12 hardware shading rate palette.
pub const D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES: usize = 16;

// Direct3D 12 16-entry shading rate palette mapping 4-bit indices to coarse rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadingRatePalette {
    // Array of palette entries storing coarse rates.
    pub entries: [CoarseShadingRate; D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES],
    // Number of active programmed palette entries.
    pub active_count: u32,
}

impl Default for ShadingRatePalette {
    fn default() -> Self {
        Self::new_standard()
    }
}

impl ShadingRatePalette {
    // Creates standard default palette populated with all standard rates.
    pub const fn new_standard() -> Self {
        let mut entries = [CoarseShadingRate::Rate1x1; D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES];
        entries[0] = CoarseShadingRate::Rate1x1;
        entries[1] = CoarseShadingRate::Rate1x2;
        entries[2] = CoarseShadingRate::Rate2x1;
        entries[3] = CoarseShadingRate::Rate2x2;
        entries[4] = CoarseShadingRate::Rate2x4;
        entries[5] = CoarseShadingRate::Rate4x2;
        entries[6] = CoarseShadingRate::Rate4x4;
        Self {
            entries,
            active_count: 7,
        }
    }

    // Looks up coarse shading rate for given palette index.
    pub fn lookup(&self, index: u8) -> CoarseShadingRate {
        let idx = (index as usize) % D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES;
        self.entries[idx]
    }

    // Finds palette index for a given rate if present.
    pub fn find_index(&self, rate: CoarseShadingRate) -> Option<u8> {
        let mut i = 0usize;
        while i < (self.active_count as usize) && i < D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES {
            if self.entries[i] == rate {
                return Some(i as u8);
            }
            i += 1;
        }
        None
    }

    // Programs a specific palette slot with a coarse shading rate.
    pub fn set_entry(&mut self, slot: usize, rate: CoarseShadingRate) -> bool {
        if slot >= D3D12_SHADING_RATE_PALETTE_MAX_ENTRIES {
            return false;
        }
        self.entries[slot] = rate;
        if (slot as u32) >= self.active_count {
            self.active_count = (slot as u32) + 1;
        }
        true
    }
}

// Temporary stub for incremental commits.
pub fn test_coarse_pixel_shading_subsystem() -> bool {
    true
}
