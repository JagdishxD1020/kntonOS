//! Win32 DirectSound & DirectSound3D Legacy Game Audio API Emulation Layer.

#![allow(non_snake_case)]

use alloc::vec::Vec;
use core::mem::size_of;
use super::xaudio2::WaveFormatEx;

// Standard DirectSound HRESULT status codes.
pub const DS_OK: u32 = 0;
pub const DSERR_INVALIDPARAM: u32 = 0x80070057;
pub const DSERR_BUFFERLOST: u32 = 0x88780096;
pub const DSERR_UNINITIALIZED: u32 = 0x8878000A;

// Buffer capability flags.
pub const DSBCAPS_PRIMARYBUFFER: u32 = 0x00000001;
pub const DSBCAPS_STATIC: u32 = 0x00000002;
pub const DSBCAPS_LOCHARDWARE: u32 = 0x00000004;
pub const DSBCAPS_LOCSOFTWARE: u32 = 0x00000008;
pub const DSBCAPS_CTRL3D: u32 = 0x00000010;
pub const DSBCAPS_CTRLFREQUENCY: u32 = 0x00000020;
pub const DSBCAPS_CTRLPAN: u32 = 0x00000040;
pub const DSBCAPS_CTRLVOLUME: u32 = 0x00000080;
pub const DSBCAPS_CTRLPOSITIONNOTIFY: u32 = 0x00000100;
pub const DSBCAPS_GLOBALFOCUS: u32 = 0x00008000;

// Playback flags.
pub const DSBPLAY_LOOPING: u32 = 0x00000001;

// Buffer status flags.
pub const DSBSTATUS_PLAYING: u32 = 0x00000001;
pub const DSBSTATUS_BUFFERLOST: u32 = 0x00000002;
pub const DSBSTATUS_LOOPING: u32 = 0x00000004;

// Lock flags.
pub const DSBLOCK_FROMWRITECURSOR: u32 = 0x00000001;
pub const DSBLOCK_ENTIREBUFFER: u32 = 0x00000002;

/// Win32 DSBUFFERDESC structure specifying characteristics of a new DirectSound buffer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DSBUFFERDESC {
    pub dwSize: u32,
    pub dwFlags: u32,
    pub dwBufferBytes: u32,
    pub dwReserved: u32,
    pub lpwfxFormat: WaveFormatEx,
}

impl Default for DSBUFFERDESC {
    fn default() -> Self {
        Self {
            dwSize: size_of::<Self>() as u32,
            dwFlags: DSBCAPS_CTRLVOLUME | DSBCAPS_CTRLPAN | DSBCAPS_CTRLFREQUENCY,
            dwBufferBytes: 0,
            dwReserved: 0,
            lpwfxFormat: WaveFormatEx::default(),
        }
    }
}

/// Circular lock region pointers and lengths returned by IDirectSoundBuffer::Lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DSoundLockRegion {
    pub offset1: usize,
    pub len1: usize,
    pub offset2: usize,
    pub len2: usize,
}

/// DirectSound Buffer representing a circular PCM sound buffer with cursor tracking.
#[derive(Debug, Clone)]
pub struct DirectSoundBuffer {
    pub desc: DSBUFFERDESC,
    pub buffer: Vec<u8>,
    pub play_cursor: usize,
    pub write_cursor: usize,
    pub status: u32,
    pub is_locked: bool,
    pub lock_region: DSoundLockRegion,
}

impl DirectSoundBuffer {
    // Creates a new secondary or primary DirectSoundBuffer.
    pub fn new(desc: DSBUFFERDESC) -> Self {
        let size = desc.dwBufferBytes as usize;
        Self {
            desc,
            buffer: alloc::vec![0u8; size],
            play_cursor: 0,
            write_cursor: 0,
            status: 0,
            is_locked: false,
            lock_region: DSoundLockRegion {
                offset1: 0,
                len1: 0,
                offset2: 0,
                len2: 0,
            },
        }
    }

    // Obtains pointers/ranges into circular buffer for writing data (handles circular wraparound).
    pub fn lock(&mut self, offset: usize, bytes: usize, flags: u32) -> Result<DSoundLockRegion, u32> {
        if self.buffer.is_empty() {
            return Err(DSERR_INVALIDPARAM);
        }

        let start_offset = if (flags & DSBLOCK_FROMWRITECURSOR) != 0 {
            self.write_cursor
        } else {
            offset % self.buffer.len()
        };

        let lock_bytes = if (flags & DSBLOCK_ENTIREBUFFER) != 0 {
            self.buffer.len()
        } else {
            bytes
        };

        let cap = self.buffer.len();
        let region = if start_offset + lock_bytes <= cap {
            DSoundLockRegion {
                offset1: start_offset,
                len1: lock_bytes,
                offset2: 0,
                len2: 0,
            }
        } else {
            let len1 = cap - start_offset;
            let len2 = (lock_bytes - len1).min(start_offset);
            DSoundLockRegion {
                offset1: start_offset,
                len1,
                offset2: 0,
                len2,
            }
        };

        self.is_locked = true;
        self.lock_region = region;
        Ok(region)
    }

    // Commits written data to circular buffer and advances write cursor.
    pub fn unlock(&mut self, region: DSoundLockRegion, data1: &[u8], data2: &[u8]) -> u32 {
        if !self.is_locked {
            return DSERR_INVALIDPARAM;
        }

        let l1 = region.len1.min(data1.len());
        if l1 > 0 && region.offset1 + l1 <= self.buffer.len() {
            self.buffer[region.offset1..region.offset1 + l1].copy_from_slice(&data1[..l1]);
        }

        let l2 = region.len2.min(data2.len());
        if l2 > 0 && region.offset2 + l2 <= self.buffer.len() {
            self.buffer[region.offset2..region.offset2 + l2].copy_from_slice(&data2[..l2]);
        }

        self.write_cursor = if region.len2 > 0 {
            region.offset2 + l2
        } else {
            (region.offset1 + l1) % self.buffer.len()
        };

        self.is_locked = false;
        DS_OK
    }

    // Queries current play and write cursors.
    pub fn get_current_position(&self) -> (usize, usize) {
        (self.play_cursor, self.write_cursor)
    }

    // Sets current playback cursor position.
    pub fn set_current_position(&mut self, pos: usize) -> u32 {
        if self.buffer.is_empty() {
            return DSERR_INVALIDPARAM;
        }
        self.play_cursor = pos % self.buffer.len();
        DS_OK
    }
}

/// Verifies memory layout, capability flags, and circular lock/unlock split ranges.
pub fn verify_dsound_buffer_descriptors() -> bool {
    let mut desc = DSBUFFERDESC::default();
    desc.dwBufferBytes = 1000;
    let mut buf = DirectSoundBuffer::new(desc);

    // Lock non-wrapping region: 200 bytes at offset 100
    let r1 = match buf.lock(100, 200, 0) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if r1.offset1 != 100 || r1.len1 != 200 || r1.len2 != 0 {
        return false;
    }

    let dummy_data = alloc::vec![0xAAu8; 200];
    if buf.unlock(r1, &dummy_data, &[]) != DS_OK {
        return false;
    }
    if buf.buffer[100] != 0xAA || buf.buffer[299] != 0xAA {
        return false;
    }

    // Lock wrapping region: 300 bytes at offset 850 (850..1000 = 150, 0..150 = 150)
    let r2 = match buf.lock(850, 300, 0) {
        Ok(r) => r,
        Err(_) => return false,
    };
    if r2.offset1 != 850 || r2.len1 != 150 || r2.offset2 != 0 || r2.len2 != 150 {
        return false;
    }

    let d1 = alloc::vec![0xBBu8; 150];
    let d2 = alloc::vec![0xCCu8; 150];
    if buf.unlock(r2, &d1, &d2) != DS_OK {
        return false;
    }
    if buf.buffer[850] != 0xBB || buf.buffer[999] != 0xBB || buf.buffer[0] != 0xCC || buf.buffer[149] != 0xCC {
        return false;
    }

    true
}
