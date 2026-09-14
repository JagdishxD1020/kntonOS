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


use alloc::collections::BTreeMap;
use alloc::vec::Vec;

pub type BCRYPT_HANDLE = usize;
pub type BCRYPT_ALG_HANDLE = usize;
pub type BCRYPT_KEY_HANDLE = usize;
pub type BCRYPT_HASH_HANDLE = usize;

pub const BCRYPT_RNG_ALGORITHM: &str = "RNG";
pub const BCRYPT_SHA256_ALGORITHM: &str = "SHA256";
pub const BCRYPT_HMAC_SHA256_FLAG: u32 = 0x00000008;
pub const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x00000002;

use crate::pe::ntdll::{STATUS_INVALID_HANDLE, STATUS_INVALID_PARAMETER, STATUS_SUCCESS};
pub const STATUS_NOT_SUPPORTED: u32 = 0xC000_00BB;

#[derive(Debug, Clone)]
pub enum BCryptAlgType {
    Rng,
    Sha256 { hmac: bool },
}

#[derive(Debug, Clone)]
pub struct BCryptAlgorithm {
    pub alg_type: BCryptAlgType,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct BCryptHashContext {
    pub is_hmac: bool,
    pub secret: Vec<u8>,
    pub data: Vec<u8>,
}

static BCRYPT_ALG_TABLE: Spinlock<BTreeMap<BCRYPT_ALG_HANDLE, BCryptAlgorithm>> = Spinlock::new(BTreeMap::new());
static BCRYPT_HASH_TABLE: Spinlock<BTreeMap<BCRYPT_HASH_HANDLE, BCryptHashContext>> = Spinlock::new(BTreeMap::new());
static NEXT_BCRYPT_HANDLE: Spinlock<usize> = Spinlock::new(0x000C_0001);

/// Emulates Win32 BCryptOpenAlgorithmProvider creating an algorithm provider handle.
pub unsafe fn BCryptOpenAlgorithmProvider(
    ph_algorithm: *mut BCRYPT_ALG_HANDLE,
    psz_alg_id: &str,
    _psz_implementation: Option<&str>,
    dw_flags: u32,
) -> u32 {
    if ph_algorithm.is_null() {
        return STATUS_INVALID_PARAMETER;
    }

    let trimmed = psz_alg_id.trim_matches('\0').trim();
    let alg_type = if trimmed.eq_ignore_ascii_case("RNG") {
        BCryptAlgType::Rng
    } else if trimmed.eq_ignore_ascii_case("SHA256") {
        BCryptAlgType::Sha256 {
            hmac: (dw_flags & BCRYPT_HMAC_SHA256_FLAG) != 0,
        }
    } else {
        return STATUS_NOT_SUPPORTED;
    };

    let mut gen = NEXT_BCRYPT_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let mut table = BCRYPT_ALG_TABLE.lock();
    table.insert(handle, BCryptAlgorithm { alg_type, flags: dw_flags });

    *ph_algorithm = handle;
    STATUS_SUCCESS
}

/// Emulates Win32 BCryptCloseAlgorithmProvider closing an algorithm provider handle.
pub unsafe fn BCryptCloseAlgorithmProvider(h_algorithm: BCRYPT_ALG_HANDLE, _dw_flags: u32) -> u32 {
    let mut table = BCRYPT_ALG_TABLE.lock();
    if table.remove(&h_algorithm).is_some() {
        STATUS_SUCCESS
    } else {
        STATUS_INVALID_HANDLE
    }
}

/// Emulates Win32 BCryptGenRandom generating cryptographically secure random bytes.
pub unsafe fn BCryptGenRandom(
    h_algorithm: BCRYPT_ALG_HANDLE,
    pb_buffer: *mut u8,
    cb_buffer: u32,
    dw_flags: u32,
) -> u32 {
    if pb_buffer.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    if cb_buffer == 0 {
        return STATUS_SUCCESS;
    }

    let slice = core::slice::from_raw_parts_mut(pb_buffer, cb_buffer as usize);

    if (dw_flags & BCRYPT_USE_SYSTEM_PREFERRED_RNG) != 0 {
        win32_crypto_get_random(slice);
        return STATUS_SUCCESS;
    }

    let table = BCRYPT_ALG_TABLE.lock();
    match table.get(&h_algorithm) {
        Some(alg) => match alg.alg_type {
            BCryptAlgType::Rng => {
                win32_crypto_get_random(slice);
                STATUS_SUCCESS
            }
            _ => STATUS_NOT_SUPPORTED,
        },
        None => STATUS_INVALID_HANDLE,
    }
}

/// Emulates Win32 BCryptCreateHash initializing a hash or HMAC context.
pub unsafe fn BCryptCreateHash(
    h_algorithm: BCRYPT_ALG_HANDLE,
    ph_hash: *mut BCRYPT_HASH_HANDLE,
    _pb_hash_object: *mut u8,
    _cb_hash_object: u32,
    pb_secret: *const u8,
    cb_secret: u32,
    _dw_flags: u32,
) -> u32 {
    if ph_hash.is_null() {
        return STATUS_INVALID_PARAMETER;
    }

    let table = BCRYPT_ALG_TABLE.lock();
    let alg = match table.get(&h_algorithm) {
        Some(a) => a,
        None => return STATUS_INVALID_HANDLE,
    };

    let (is_hmac, secret) = match alg.alg_type {
        BCryptAlgType::Sha256 { hmac } => {
            let secret_vec = if hmac && !pb_secret.is_null() && cb_secret > 0 {
                let slice = core::slice::from_raw_parts(pb_secret, cb_secret as usize);
                slice.to_vec()
            } else {
                Vec::new()
            };
            (hmac, secret_vec)
        }
        _ => return STATUS_NOT_SUPPORTED,
    };

    let mut gen = NEXT_BCRYPT_HANDLE.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let ctx = BCryptHashContext {
        is_hmac,
        secret,
        data: Vec::new(),
    };

    let mut hash_table = BCRYPT_HASH_TABLE.lock();
    hash_table.insert(handle, ctx);

    *ph_hash = handle;
    STATUS_SUCCESS
}

/// Emulates Win32 BCryptHashData streaming input data into a hash context.
pub unsafe fn BCryptHashData(
    h_hash: BCRYPT_HASH_HANDLE,
    pb_input: *const u8,
    cb_input: u32,
    _dw_flags: u32,
) -> u32 {
    if pb_input.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    if cb_input == 0 {
        return STATUS_SUCCESS;
    }

    let mut table = BCRYPT_HASH_TABLE.lock();
    let ctx = match table.get_mut(&h_hash) {
        Some(c) => c,
        None => return STATUS_INVALID_HANDLE,
    };

    let slice = core::slice::from_raw_parts(pb_input, cb_input as usize);
    ctx.data.extend_from_slice(slice);
    STATUS_SUCCESS
}

/// Emulates Win32 BCryptFinishHash completing hash calculation and extracting digest.
pub unsafe fn BCryptFinishHash(
    h_hash: BCRYPT_HASH_HANDLE,
    pb_output: *mut u8,
    cb_output: u32,
    _dw_flags: u32,
) -> u32 {
    if pb_output.is_null() || cb_output < 32 {
        return STATUS_INVALID_PARAMETER;
    }

    let mut table = BCRYPT_HASH_TABLE.lock();
    let ctx = match table.get_mut(&h_hash) {
        Some(c) => c,
        None => return STATUS_INVALID_HANDLE,
    };

    let digest = if ctx.is_hmac {
        crate::crypto::sha256::hmac_sha256(&ctx.secret, &ctx.data)
    } else {
        crate::crypto::sha256::Sha256::digest(&ctx.data)
    };

    let out_slice = core::slice::from_raw_parts_mut(pb_output, 32);
    out_slice.copy_from_slice(&digest);
    STATUS_SUCCESS
}

/// Emulates Win32 BCryptDestroyHash releasing a hash or HMAC context.
pub unsafe fn BCryptDestroyHash(h_hash: BCRYPT_HASH_HANDLE) -> u32 {
    let mut table = BCRYPT_HASH_TABLE.lock();
    if table.remove(&h_hash).is_some() {
        STATUS_SUCCESS
    } else {
        STATUS_INVALID_HANDLE
    }
}
