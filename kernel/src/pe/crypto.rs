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


pub type HCRYPTPROV = usize;
pub type HCRYPTKEY = usize;
pub type HCRYPTHASH = usize;

pub const PROV_RSA_FULL: u32 = 1;
pub const PROV_RSA_AES: u32 = 24;

pub const CRYPT_VERIFYCONTEXT: u32 = 0xF000_0000;
pub const CRYPT_NEWKEYSET: u32 = 0x0000_0008;
pub const CRYPT_DELETEKEYSET: u32 = 0x0000_0010;
pub const CRYPT_MACHINE_KEYSET: u32 = 0x0000_0020;
pub const CRYPT_SILENT: u32 = 0x0000_0040;

pub const NTE_BAD_KEYSET: u32 = 0x8009_0016;
pub const NTE_BAD_FLAGS: u32 = 0x8009_0009;
pub const NTE_BAD_PROV_TYPE: u32 = 0x8009_0014;
pub const NTE_INVALID_HANDLE: u32 = 0x8009_0026;
pub const NTE_INVALID_PARAMETER: u32 = 0x8009_0027;

#[derive(Debug, Clone)]
pub struct CryptoContext {
    pub container_name: Option<alloc::string::String>,
    pub provider_name: Option<alloc::string::String>,
    pub prov_type: u32,
    pub flags: u32,
}

static CRYPTO_PROV_TABLE: Spinlock<BTreeMap<HCRYPTPROV, CryptoContext>> = Spinlock::new(BTreeMap::new());
static NEXT_HCRYPTPROV: Spinlock<HCRYPTPROV> = Spinlock::new(0x000E_0001);

/// Emulates Win32 CryptAcquireContextA acquiring a cryptographic service provider handle.
pub unsafe fn CryptAcquireContextA(
    ph_prov: *mut HCRYPTPROV,
    psz_container: *const u8,
    psz_provider: *const u8,
    dw_prov_type: u32,
    dw_flags: u32,
) -> bool {
    if ph_prov.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    let container_name = if !psz_container.is_null() {
        let mut len = 0;
        while *psz_container.add(len) != 0 && len < 260 {
            len += 1;
        }
        core::str::from_utf8(core::slice::from_raw_parts(psz_container, len)).ok().map(alloc::string::String::from)
    } else {
        None
    };

    let provider_name = if !psz_provider.is_null() {
        let mut len = 0;
        while *psz_provider.add(len) != 0 && len < 260 {
            len += 1;
        }
        core::str::from_utf8(core::slice::from_raw_parts(psz_provider, len)).ok().map(alloc::string::String::from)
    } else {
        None
    };

    let mut gen = NEXT_HCRYPTPROV.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let ctx = CryptoContext {
        container_name,
        provider_name,
        prov_type: dw_prov_type,
        flags: dw_flags,
    };

    let mut table = CRYPTO_PROV_TABLE.lock();
    table.insert(handle, ctx);

    *ph_prov = handle;
    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Emulates Win32 CryptAcquireContextW acquiring a cryptographic provider handle (UTF-16).
pub unsafe fn CryptAcquireContextW(
    ph_prov: *mut HCRYPTPROV,
    sz_container: *const u16,
    sz_provider: *const u16,
    dw_prov_type: u32,
    dw_flags: u32,
) -> bool {
    if ph_prov.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }

    let container_name = if !sz_container.is_null() {
        let mut len = 0;
        while *sz_container.add(len) != 0 && len < 260 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(sz_container, len);
        let chars: alloc::vec::Vec<char> = slice.iter().map(|&c| c as u8 as char).collect();
        Some(chars.into_iter().collect())
    } else {
        None
    };

    let provider_name = if !sz_provider.is_null() {
        let mut len = 0;
        while *sz_provider.add(len) != 0 && len < 260 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(sz_provider, len);
        let chars: alloc::vec::Vec<char> = slice.iter().map(|&c| c as u8 as char).collect();
        Some(chars.into_iter().collect())
    } else {
        None
    };

    let mut gen = NEXT_HCRYPTPROV.lock();
    let handle = *gen;
    *gen = handle.saturating_add(1);
    drop(gen);

    let ctx = CryptoContext {
        container_name,
        provider_name,
        prov_type: dw_prov_type,
        flags: dw_flags,
    };

    let mut table = CRYPTO_PROV_TABLE.lock();
    table.insert(handle, ctx);

    *ph_prov = handle;
    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Emulates Win32 CryptGenRandom filling buffer with cryptographically secure random bytes.
pub unsafe fn CryptGenRandom(h_prov: HCRYPTPROV, dw_len: u32, pb_buffer: *mut u8) -> bool {
    if pb_buffer.is_null() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_PARAMETER);
        return false;
    }
    if dw_len == 0 {
        return true;
    }

    let table = CRYPTO_PROV_TABLE.lock();
    if !table.contains_key(&h_prov) {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        return false;
    }

    let slice = core::slice::from_raw_parts_mut(pb_buffer, dw_len as usize);
    win32_crypto_get_random(slice);

    crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
    true
}

/// Emulates Win32 CryptReleaseContext releasing a cryptographic service provider handle.
pub unsafe fn CryptReleaseContext(h_prov: HCRYPTPROV, _dw_flags: u32) -> bool {
    let mut table = CRYPTO_PROV_TABLE.lock();
    if table.remove(&h_prov).is_some() {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_SUCCESS);
        true
    } else {
        crate::pe::kernel32::win32_set_last_error(crate::pe::kernel32::ERROR_INVALID_HANDLE);
        false
    }
}

/// Anti-cheat session verification token containing HMAC signature and cycle nonce.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AntiCheatToken {
    pub signature: [u8; 32],
    pub timestamp: u64,
    pub nonce: u64,
}

/// Computes SHA-256 digest over in-memory PE section or memory region.
pub fn win32_compute_pe_section_digest(section_bytes: &[u8]) -> [u8; 32] {
    crate::crypto::sha256::Sha256::digest(section_bytes)
}

/// Verifies memory region matches expected SHA-256 digest.
pub fn win32_verify_module_integrity(module_bytes: &[u8], expected_digest: &[u8; 32]) -> bool {
    let digest = crate::crypto::sha256::Sha256::digest(module_bytes);
    &digest == expected_digest
}

/// Generates signed anti-cheat integrity token bound to module state and timestamp.
pub unsafe fn win32_generate_anticheat_token(
    session_seed: &[u8],
    module_base: *const u8,
    module_len: usize,
    nonce: u64,
) -> AntiCheatToken {
    let timestamp = crate::crypto::rng::rdtsc();
    let module_hash = if !module_base.is_null() && module_len > 0 {
        let slice = core::slice::from_raw_parts(module_base, module_len);
        crate::crypto::sha256::Sha256::digest(slice)
    } else {
        crate::crypto::sha256::Sha256::digest(&[])
    };

    let mut msg = [0u8; 48];
    msg[0..8].copy_from_slice(&timestamp.to_le_bytes());
    msg[8..16].copy_from_slice(&nonce.to_le_bytes());
    msg[16..48].copy_from_slice(&module_hash);

    let signature = crate::crypto::sha256::hmac_sha256(session_seed, &msg);
    AntiCheatToken {
        signature,
        timestamp,
        nonce,
    }
}

/// Validates signed anti-cheat integrity token against module state and session seed.
pub unsafe fn win32_validate_anticheat_token(
    token: &AntiCheatToken,
    session_seed: &[u8],
    module_base: *const u8,
    module_len: usize,
) -> bool {
    let module_hash = if !module_base.is_null() && module_len > 0 {
        let slice = core::slice::from_raw_parts(module_base, module_len);
        crate::crypto::sha256::Sha256::digest(slice)
    } else {
        crate::crypto::sha256::Sha256::digest(&[])
    };

    let mut msg = [0u8; 48];
    msg[0..8].copy_from_slice(&token.timestamp.to_le_bytes());
    msg[8..16].copy_from_slice(&token.nonce.to_le_bytes());
    msg[16..48].copy_from_slice(&module_hash);

    let expected_sig = crate::crypto::sha256::hmac_sha256(session_seed, &msg);
    token.signature == expected_sig
}

/// Comprehensive self-test verifying Win32 crypto subsystem, RNG entropy, CNG, CryptoAPI, and anti-cheat engine.
pub fn test_win32_crypto_subsystem() -> bool {
    let mut rng_buf = [0u8; 128];
    win32_crypto_get_random(&mut rng_buf);
    let mut ones = 0usize;
    for &b in &rng_buf {
        ones += b.count_ones() as usize;
    }
    if ones < 300 || ones > 724 {
        return false;
    }

    let mut h_alg: BCRYPT_ALG_HANDLE = 0;
    let status = unsafe {
        BCryptOpenAlgorithmProvider(&mut h_alg, BCRYPT_RNG_ALGORITHM, None, 0)
    };
    if status != STATUS_SUCCESS {
        return false;
    }
    let mut cng_buf = [0u8; 32];
    let status = unsafe { BCryptGenRandom(h_alg, cng_buf.as_mut_ptr(), 32, 0) };
    if status != STATUS_SUCCESS {
        return false;
    }
    let status = unsafe { BCryptCloseAlgorithmProvider(h_alg, 0) };
    if status != STATUS_SUCCESS {
        return false;
    }

    let mut pref_buf = [0u8; 16];
    let status = unsafe {
        BCryptGenRandom(
            0,
            pref_buf.as_mut_ptr(),
            16,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != STATUS_SUCCESS {
        return false;
    }

    let mut h_hash_alg: BCRYPT_ALG_HANDLE = 0;
    let status = unsafe {
        BCryptOpenAlgorithmProvider(
            &mut h_hash_alg,
            BCRYPT_SHA256_ALGORITHM,
            None,
            BCRYPT_HMAC_SHA256_FLAG,
        )
    };
    if status != STATUS_SUCCESS {
        return false;
    }

    let key = b"key";
    let mut h_hash: BCRYPT_HASH_HANDLE = 0;
    let status = unsafe {
        BCryptCreateHash(
            h_hash_alg,
            &mut h_hash,
            core::ptr::null_mut(),
            0,
            key.as_ptr() as *mut u8,
            key.len() as u32,
            0,
        )
    };
    if status != STATUS_SUCCESS {
        return false;
    }

    let msg = b"The quick brown fox jumps over the lazy dog";
    let status = unsafe {
        BCryptHashData(
            h_hash,
            msg.as_ptr() as *mut u8,
            msg.len() as u32,
            0,
        )
    };
    if status != STATUS_SUCCESS {
        return false;
    }

    let mut out_hmac = [0u8; 32];
    let status = unsafe { BCryptFinishHash(h_hash, out_hmac.as_mut_ptr(), 32, 0) };
    if status != STATUS_SUCCESS {
        return false;
    }

    let expected_hmac: [u8; 32] = [
        0xf7, 0xbc, 0x83, 0xf4, 0x30, 0x53, 0x84, 0x24,
        0xb1, 0x32, 0x98, 0xe6, 0xaa, 0x6f, 0xb1, 0x43,
        0xef, 0x4d, 0x59, 0xa1, 0x49, 0x46, 0x17, 0x59,
        0x97, 0x47, 0x9d, 0xbc, 0x2d, 0x1a, 0x3c, 0xd8,
    ];
    if out_hmac != expected_hmac {
        return false;
    }

    let _ = unsafe { BCryptDestroyHash(h_hash) };
    let _ = unsafe { BCryptCloseAlgorithmProvider(h_hash_alg, 0) };

    let mut h_prov: HCRYPTPROV = 0;
    let ok = unsafe {
        CryptAcquireContextA(
            &mut h_prov,
            core::ptr::null(),
            core::ptr::null(),
            PROV_RSA_FULL,
            CRYPT_VERIFYCONTEXT,
        )
    };
    if !ok || h_prov == 0 {
        return false;
    }

    let mut capi_buf = [0u8; 64];
    let ok = unsafe { CryptGenRandom(h_prov, 64, capi_buf.as_mut_ptr()) };
    if !ok {
        return false;
    }

    let ok = unsafe { CryptReleaseContext(h_prov, 0) };
    if !ok {
        return false;
    }

    let session_seed = b"knton_anticheat_session_seed_test";
    let module_code = [0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0xc3];
    let token = unsafe {
        win32_generate_anticheat_token(session_seed, module_code.as_ptr(), module_code.len(), 0xdeadbeef)
    };
    let valid = unsafe {
        win32_validate_anticheat_token(&token, session_seed, module_code.as_ptr(), module_code.len())
    };
    if !valid {
        return false;
    }

    let mut tampered = module_code;
    tampered[0] ^= 0x90;
    let invalid = unsafe {
        win32_validate_anticheat_token(&token, session_seed, tampered.as_ptr(), tampered.len())
    };
    if invalid {
        return false;
    }

    let digest = win32_compute_pe_section_digest(&module_code);
    if !win32_verify_module_integrity(&module_code, &digest) {
        return false;
    }

    crate::serial_write_str("[KNT-WIN32-CRYPTO-OK] Win32 crypto and anti-cheat entropy provider verified
");
    true
}
