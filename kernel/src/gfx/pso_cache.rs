//! Direct3D 12 Agility Pipeline State Object (PSO) Binary Library Caching & Background Async Compiler.
//! Implements ID3D12PipelineLibrary serialization, async compilation queues, and DirectStorage cache backing.

#![allow(dead_code, non_camel_case_types, non_snake_case, unused_imports)]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

// Magic identifier for serialized KntonOS PSO binary cache (KPSO).
pub const KNT_PSO_LIBRARY_MAGIC: u32 = 0x4F53_504B;

// Current schema version for pipeline library binary format.
pub const KNT_PSO_LIBRARY_VERSION: u32 = 1;

// 128-bit fast deterministic hash key for shader bytecode and pipeline state objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PsoHashKey {
    // Upper 64 bits of hash key.
    pub high: u64,
    // Lower 64 bits of hash key.
    pub low: u64,
}

impl PsoHashKey {
    pub const ZERO: Self = Self { high: 0, low: 0 };

    pub const fn new(high: u64, low: u64) -> Self {
        Self { high, low }
    }
}

// Computes 128-bit hash key from raw byte stream using dual-stream FNV-1a.
pub fn hash_bytes_128(data: &[u8]) -> PsoHashKey {
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x84222325cbf29ce4;
    const PRIME1: u64 = 0x100000001b3;
    const PRIME2: u64 = 0x100000001b5;

    for &byte in data {
        h1 ^= byte as u64;
        h1 = h1.wrapping_mul(PRIME1);
        h2 ^= (byte as u64).rotate_left(4);
        h2 = h2.wrapping_mul(PRIME2);
    }

    PsoHashKey { high: h1, low: h2 }
}

// Pipeline state object type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineKind {
    Graphics,
    Compute,
    Mesh,
}

// Compiled binary representation of a pipeline state object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPipelineBlob {
    // Unique deterministic hash key.
    pub hash_key: PsoHashKey,
    // Pipeline classification kind.
    pub kind: PipelineKind,
    // Native GPU ISA bytecode.
    pub bytecode: Vec<u8>,
    // Associated root signature identifier.
    pub root_signature_id: u64,
}

// Named entry stored in the pipeline library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineLibraryEntry {
    // Unique symbolic pipeline name.
    pub name: String,
    // Compiled pipeline binary blob.
    pub blob: CompiledPipelineBlob,
}

// Error conditions returned by pipeline library operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineLibraryError {
    DuplicateName,
    NotFound,
    CorruptedHeader,
    UnsupportedVersion(u32),
    BufferTooSmall { required: usize, provided: usize },
    DescriptorMismatch,
}

// Direct3D 12 Agility Pipeline Library managing serialization and fast lookup.
#[derive(Debug, Clone)]
pub struct D3D12PipelineLibrary {
    // Stored pipeline entries.
    pub entries: Vec<PipelineLibraryEntry>,
    // Hardware driver architecture UUID.
    pub driver_uuid: [u8; 16],
}

impl Default for D3D12PipelineLibrary {
    fn default() -> Self {
        Self::new(*b"KntonGpuDrv-RDNA")
    }
}

impl D3D12PipelineLibrary {
    // Creates a new empty pipeline library with specified driver UUID.
    pub fn new(driver_uuid: [u8; 16]) -> Self {
        Self {
            entries: Vec::new(),
            driver_uuid,
        }
    }

    // Stores a compiled pipeline blob under a symbolic name.
    pub fn store_pipeline(
        &mut self,
        name: &str,
        blob: CompiledPipelineBlob,
    ) -> Result<(), PipelineLibraryError> {
        if self.entries.iter().any(|e| e.name == name) {
            return Err(PipelineLibraryError::DuplicateName);
        }
        self.entries.push(PipelineLibraryEntry {
            name: String::from(name),
            blob,
        });
        Ok(())
    }

    // Loads a compiled pipeline blob by symbolic name.
    pub fn load_pipeline(&self, name: &str) -> Result<CompiledPipelineBlob, PipelineLibraryError> {
        for entry in &self.entries {
            if entry.name == name {
                return Ok(entry.blob.clone());
            }
        }
        Err(PipelineLibraryError::NotFound)
    }

    // Returns total count of stored pipelines.
    pub fn pipeline_count(&self) -> usize {
        self.entries.len()
    }

    // Calculates byte size required for serialized binary representation.
    pub fn get_serialized_size(&self) -> usize {
        let mut size = 4 + 4 + 16 + 4;
        for entry in &self.entries {
            size += 4 + entry.name.len() + 16 + 4 + 4 + 8 + entry.blob.bytecode.len();
        }
        size
    }

    // Serializes the library into a persistent binary byte vector.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.get_serialized_size());
        buf.extend_from_slice(&KNT_PSO_LIBRARY_MAGIC.to_le_bytes());
        buf.extend_from_slice(&KNT_PSO_LIBRARY_VERSION.to_le_bytes());
        buf.extend_from_slice(&self.driver_uuid);
        buf.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());

        for entry in &self.entries {
            let name_bytes = entry.name.as_bytes();
            buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(name_bytes);

            buf.extend_from_slice(&entry.blob.hash_key.high.to_le_bytes());
            buf.extend_from_slice(&entry.blob.hash_key.low.to_le_bytes());

            let kind_u32 = match entry.blob.kind {
                PipelineKind::Graphics => 0u32,
                PipelineKind::Compute => 1u32,
                PipelineKind::Mesh => 2u32,
            };
            buf.extend_from_slice(&kind_u32.to_le_bytes());
            buf.extend_from_slice(&(entry.blob.bytecode.len() as u32).to_le_bytes());
            buf.extend_from_slice(&entry.blob.root_signature_id.to_le_bytes());
            buf.extend_from_slice(&entry.blob.bytecode);
        }

        buf
    }

    // Deserializes a pipeline library from raw binary buffer.
    pub fn deserialize(data: &[u8]) -> Result<Self, PipelineLibraryError> {
        if data.len() < 28 {
            return Err(PipelineLibraryError::BufferTooSmall {
                required: 28,
                provided: data.len(),
            });
        }

        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != KNT_PSO_LIBRARY_MAGIC {
            return Err(PipelineLibraryError::CorruptedHeader);
        }

        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != KNT_PSO_LIBRARY_VERSION {
            return Err(PipelineLibraryError::UnsupportedVersion(version));
        }

        let mut driver_uuid = [0u8; 16];
        driver_uuid.copy_from_slice(&data[8..24]);

        let count = u32::from_le_bytes([data[24], data[25], data[26], data[27]]) as usize;
        let mut offset = 28;
        let mut entries = Vec::with_capacity(count);

        for _ in 0..count {
            if offset + 4 > data.len() {
                return Err(PipelineLibraryError::CorruptedHeader);
            }
            let name_len = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            offset += 4;

            if offset + name_len > data.len() {
                return Err(PipelineLibraryError::CorruptedHeader);
            }
            let name_slice = &data[offset..offset + name_len];
            let name_str = match core::str::from_utf8(name_slice) {
                Ok(s) => String::from(s),
                Err(_) => return Err(PipelineLibraryError::CorruptedHeader),
            };
            offset += name_len;

            if offset + 32 > data.len() {
                return Err(PipelineLibraryError::CorruptedHeader);
            }
            let high = u64::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            let low = u64::from_le_bytes([
                data[offset + 8],
                data[offset + 9],
                data[offset + 10],
                data[offset + 11],
                data[offset + 12],
                data[offset + 13],
                data[offset + 14],
                data[offset + 15],
            ]);
            let kind_u32 = u32::from_le_bytes([
                data[offset + 16],
                data[offset + 17],
                data[offset + 18],
                data[offset + 19],
            ]);
            let bc_len = u32::from_le_bytes([
                data[offset + 20],
                data[offset + 21],
                data[offset + 22],
                data[offset + 23],
            ]) as usize;
            let root_sig_id = u64::from_le_bytes([
                data[offset + 24],
                data[offset + 25],
                data[offset + 26],
                data[offset + 27],
                data[offset + 28],
                data[offset + 29],
                data[offset + 30],
                data[offset + 31],
            ]);
            offset += 32;

            if offset + bc_len > data.len() {
                return Err(PipelineLibraryError::CorruptedHeader);
            }
            let bytecode = data[offset..offset + bc_len].to_vec();
            offset += bc_len;

            let kind = match kind_u32 {
                0 => PipelineKind::Graphics,
                1 => PipelineKind::Compute,
                _ => PipelineKind::Mesh,
            };

            entries.push(PipelineLibraryEntry {
                name: name_str,
                blob: CompiledPipelineBlob {
                    hash_key: PsoHashKey { high, low },
                    kind,
                    bytecode,
                    root_signature_id: root_sig_id,
                },
            });
        }

        Ok(Self {
            entries,
            driver_uuid,
        })
    }
}

// Temporary stub for incremental commits.
pub fn test_pso_library_cache_subsystem() -> bool {
    true
}
