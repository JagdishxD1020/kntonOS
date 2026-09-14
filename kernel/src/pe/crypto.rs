//! Win32 Cryptographic Primitives & Game Anti-Cheat Entropy Provider.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use crate::crypto::chacha20::ChaCha20State;
use crate::crypto::rng::{
    cpuid, rdrand64, rdseed64, rdtsc, HardwareRngCapabilities,
};
use crate::memory::spinlock::Spinlock;

/// Win32 entropy harvester and CSPRNG state engine.
pub struct Win32EntropyEngine {
    caps: HardwareRngCapabilities,
    state: ChaCha20State,
    counter: u64,
    reseed_counter: usize,
}

impl Win32EntropyEngine {
    pub const RESEED_INTERVAL: usize = 1024;

    /// Initializes entropy engine querying CPUID hardware capabilities and harvesting initial seeds.
    pub fn new() -> Self {
        let caps = HardwareRngCapabilities::detect();
        let mut engine = Self {
            caps,
            state: ChaCha20State::new(&[0u8; 32], &[0u8; 12], 0),
            counter: 0,
            reseed_counter: 0,
        };
        engine.reseed();
        engine
    }

    /// Harvests entropy from hardware RDRAND/RDSEED and TSC jitter to seed ChaCha20 state.
    pub fn reseed(&mut self) {
        let mut seed = [0u8; 32];
        let mut nonce = [0u8; 12];

        for chunk in seed.chunks_mut(8) {
            let hw = self.harvest_u64();
            chunk.copy_from_slice(&hw.to_le_bytes()[..chunk.len()]);
        }
        for chunk in nonce.chunks_mut(8) {
            let hw = self.harvest_u64();
            chunk.copy_from_slice(&hw.to_le_bytes()[..chunk.len()]);
        }

        self.state = ChaCha20State::new(&seed, &nonce, 0);
        self.counter = 0;
        self.reseed_counter = 0;
    }

    /// Harvests a single 64-bit entropy word prioritizing RDSEED, RDRAND, and TSC jitter.
    pub fn harvest_u64(&self) -> u64 {
        if self.caps.has_rdseed {
            if let Some(val) = rdseed64() {
                return val;
            }
        }
        if self.caps.has_rdrand {
            if let Some(val) = rdrand64() {
                return val;
            }
        }
        self.harvest_tsc_jitter()
    }

    /// Fallback entropy collector harvesting execution jitter across RDTSC cycles.
    pub fn harvest_tsc_jitter(&self) -> u64 {
        let mut acc = rdtsc();
        for _ in 0..16 {
            let t1 = rdtsc();
            let _ = cpuid(0, 0);
            let t2 = rdtsc();
            let delta = t2.wrapping_sub(t1);
            acc = acc.rotate_left(7) ^ delta;
        }
        acc
    }

    /// Fills the output buffer with cryptographically secure random bytes.
    pub fn fill_bytes(&mut self, dest: &mut [u8]) {
        if self.reseed_counter >= Self::RESEED_INTERVAL {
            self.reseed();
        }

        let mut offset = 0;
        while offset < dest.len() {
            self.state.set_counter(self.counter as u32);
            self.counter = self.counter.wrapping_add(1);
            let block = self.state.generate_block();

            let to_copy = core::cmp::min(dest.len() - offset, 64);
            dest[offset..offset + to_copy].copy_from_slice(&block[..to_copy]);
            offset += to_copy;
        }

        self.reseed_counter += 1;
    }
}

static WIN32_RNG: Spinlock<Option<Win32EntropyEngine>> = Spinlock::new(None);

/// Initializes or retrieves the global Win32 cryptographic entropy engine.
pub fn win32_crypto_get_random(dest: &mut [u8]) {
    let mut lock = WIN32_RNG.lock();
    if lock.is_none() {
        *lock = Some(Win32EntropyEngine::new());
    }
    if let Some(ref mut engine) = *lock {
        engine.fill_bytes(dest);
    }
}

/// Generates a single cryptographically secure 64-bit random integer.
pub fn win32_crypto_get_random_u64() -> u64 {
    let mut buf = [0u8; 8];
    win32_crypto_get_random(&mut buf);
    u64::from_le_bytes(buf)
}

/// Generates a single cryptographically secure 32-bit random integer.
pub fn win32_crypto_get_random_u32() -> u32 {
    let mut buf = [0u8; 4];
    win32_crypto_get_random(&mut buf);
    u32::from_le_bytes(buf)
}
