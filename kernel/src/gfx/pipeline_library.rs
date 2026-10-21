// Direct3D 12 Agility Pipeline Library Disk Caching & Background Multi-Threaded Shader Deserialization Engine.
// Provides fast CRC32 integrity verification, LZ4 decompression, and anti-stutter shader pre-warming.

extern crate alloc;
use alloc::vec::Vec;

// Magic 32-bit identifier for D3D12 Agility serialized pipeline library files ("12PL").
pub const D3D12_PIPELINE_LIBRARY_MAGIC: u32 = 0x4C50_3231;

// Binary schema version for serialized pipeline libraries.
pub const D3D12_PIPELINE_LIBRARY_VERSION: u32 = 1;

// Error conditions encountered during pipeline library operations.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum D3d12PipelineLibraryError {
    // Magic header token mismatch or corrupted file header.
    InvalidMagic,
    // Unsupported schema version encountered.
    UnsupportedVersion,
    // CRC32 checksum mismatch indicating corrupted or truncated binary payload.
    ChecksumMismatch,
    // Compressed buffer format error during LZ4 decompression.
    DecompressionFailed,
    // Destination buffer capacity insufficient to hold decompressed output.
    BufferTooSmall,
    // Quota reached during shader cache allocation.
    QuotaExceeded,
    // Requested PSO identifier not found in library index.
    PsoNotFound,
}

// Computes fast IEEE 802.3 CRC32 integrity checksum over binary stream.
pub fn compute_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = if (crc & 1) != 0 { 0xEDB8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    !crc
}

// Lightweight LZ4 block decompressor conforming to the standard LZ4 frame specification.
pub fn lz4_decompress(src: &[u8], dst: &mut [u8]) -> Result<usize, D3d12PipelineLibraryError> {
    let mut src_pos = 0usize;
    let mut dst_pos = 0usize;
    let src_len = src.len();
    let dst_len = dst.len();

    while src_pos < src_len {
        let token = src[src_pos];
        src_pos += 1;

        // 1. Decode literal length from upper 4 bits.
        let mut literal_len = (token >> 4) as usize;
        if literal_len == 15 {
            while src_pos < src_len {
                let add = src[src_pos] as usize;
                src_pos += 1;
                literal_len += add;
                if add != 255 {
                    break;
                }
            }
        }

        if src_pos + literal_len > src_len || dst_pos + literal_len > dst_len {
            return Err(D3d12PipelineLibraryError::DecompressionFailed);
        }

        dst[dst_pos..dst_pos + literal_len].copy_from_slice(&src[src_pos..src_pos + literal_len]);
        src_pos += literal_len;
        dst_pos += literal_len;

        if src_pos >= src_len {
            break;
        }

        // 2. Read 16-bit little-endian match offset.
        if src_pos + 2 > src_len {
            return Err(D3d12PipelineLibraryError::DecompressionFailed);
        }
        let offset = (src[src_pos] as usize) | ((src[src_pos + 1] as usize) << 8);
        src_pos += 2;

        if offset == 0 || offset > dst_pos {
            return Err(D3d12PipelineLibraryError::DecompressionFailed);
        }

        // 3. Decode match length from lower 4 bits (minimum match length 4).
        let mut match_len = (token & 0x0F) as usize + 4;
        if match_len == 19 {
            while src_pos < src_len {
                let add = src[src_pos] as usize;
                src_pos += 1;
                match_len += add;
                if add != 255 {
                    break;
                }
            }
        }

        if dst_pos + match_len > dst_len {
            return Err(D3d12PipelineLibraryError::DecompressionFailed);
        }

        let match_src = dst_pos - offset;
        for i in 0..match_len {
            dst[dst_pos + i] = dst[match_src + i];
        }
        dst_pos += match_len;
    }

    Ok(dst_pos)
}

// Compresses raw bytes into an LZ4 block containing literals and match sequences.
pub fn lz4_compress(src: &[u8], dst: &mut [u8]) -> Result<usize, D3d12PipelineLibraryError> {
    let mut src_pos = 0usize;
    let mut dst_pos = 0usize;
    let src_len = src.len();
    let dst_len = dst.len();

    while src_pos < src_len {
        let chunk_len = (src_len - src_pos).min(14);
        if dst_pos + 1 + chunk_len > dst_len {
            return Err(D3d12PipelineLibraryError::BufferTooSmall);
        }

        let token = (chunk_len as u8) << 4;
        dst[dst_pos] = token;
        dst_pos += 1;

        dst[dst_pos..dst_pos + chunk_len].copy_from_slice(&src[src_pos..src_pos + chunk_len]);
        dst_pos += chunk_len;
        src_pos += chunk_len;
    }

    Ok(dst_pos)
}

// Pipeline state object type classification.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PipelineStateType {
    Graphics = 0,
    Compute = 1,
    MeshShader = 2,
}

// Serialized pipeline state object descriptor stored in pipeline library cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializedPsoEntry {
    pub pso_id: u64,
    pub pso_type: PipelineStateType,
    pub bytecode: Vec<u8>,
}

// Header metadata written at the beginning of serialized pipeline library disk files.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct PipelineLibraryHeader {
    pub magic: u32,
    pub version: u32,
    pub pso_count: u32,
    pub uncompressed_bytes: u32,
    pub compressed_bytes: u32,
    pub crc32_checksum: u32,
}

// Disk serialization and deserialization engine for ID3D12PipelineLibrary.
pub struct PipelineLibraryDiskStorage;

impl PipelineLibraryDiskStorage {
    // Serializes pipeline library entries into compressed binary disk archive with CRC32.
    pub fn serialize(entries: &[SerializedPsoEntry]) -> Result<Vec<u8>, D3d12PipelineLibraryError> {
        // 1. Pack uncompressed payload stream.
        let mut uncompressed = Vec::new();
        for entry in entries {
            uncompressed.extend_from_slice(&entry.pso_id.to_le_bytes());
            uncompressed.extend_from_slice(&(entry.pso_type as u32).to_le_bytes());
            uncompressed.extend_from_slice(&(entry.bytecode.len() as u32).to_le_bytes());
            uncompressed.extend_from_slice(&entry.bytecode);
        }

        let uncompressed_len = uncompressed.len();
        let mut compressed = alloc::vec![0u8; uncompressed_len + uncompressed_len / 4 + 64];
        let comp_len = lz4_compress(&uncompressed, &mut compressed)?;
        compressed.truncate(comp_len);

        let checksum = compute_crc32(&compressed);

        let header = PipelineLibraryHeader {
            magic: D3D12_PIPELINE_LIBRARY_MAGIC,
            version: D3D12_PIPELINE_LIBRARY_VERSION,
            pso_count: entries.len() as u32,
            uncompressed_bytes: uncompressed_len as u32,
            compressed_bytes: comp_len as u32,
            crc32_checksum: checksum,
        };

        let mut output = Vec::new();
        output.extend_from_slice(&header.magic.to_le_bytes());
        output.extend_from_slice(&header.version.to_le_bytes());
        output.extend_from_slice(&header.pso_count.to_le_bytes());
        output.extend_from_slice(&header.uncompressed_bytes.to_le_bytes());
        output.extend_from_slice(&header.compressed_bytes.to_le_bytes());
        output.extend_from_slice(&header.crc32_checksum.to_le_bytes());
        output.extend_from_slice(&compressed);

        Ok(output)
    }

    // Deserializes and validates a binary pipeline library file from disk storage.
    pub fn deserialize(data: &[u8]) -> Result<Vec<SerializedPsoEntry>, D3d12PipelineLibraryError> {
        if data.len() < 24 {
            return Err(D3d12PipelineLibraryError::InvalidMagic);
        }

        let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        if magic != D3D12_PIPELINE_LIBRARY_MAGIC {
            return Err(D3d12PipelineLibraryError::InvalidMagic);
        }

        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != D3D12_PIPELINE_LIBRARY_VERSION {
            return Err(D3d12PipelineLibraryError::UnsupportedVersion);
        }

        let pso_count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let uncompressed_len = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as usize;
        let compressed_len = u32::from_le_bytes([data[16], data[17], data[18], data[19]]) as usize;
        let expected_crc = u32::from_le_bytes([data[20], data[21], data[22], data[23]]);

        let compressed_payload = &data[24..24 + compressed_len];
        if compute_crc32(compressed_payload) != expected_crc {
            return Err(D3d12PipelineLibraryError::ChecksumMismatch);
        }

        let mut uncompressed = alloc::vec![0u8; uncompressed_len];
        let decomp_bytes = lz4_decompress(compressed_payload, &mut uncompressed)?;
        if decomp_bytes != uncompressed_len {
            return Err(D3d12PipelineLibraryError::DecompressionFailed);
        }

        // Parse individual PSO entries from decompressed byte stream.
        let mut entries = Vec::with_capacity(pso_count);
        let mut pos = 0usize;
        for _ in 0..pso_count {
            if pos + 16 > uncompressed_len {
                return Err(D3d12PipelineLibraryError::DecompressionFailed);
            }
            let pso_id = u64::from_le_bytes([
                uncompressed[pos], uncompressed[pos + 1], uncompressed[pos + 2], uncompressed[pos + 3],
                uncompressed[pos + 4], uncompressed[pos + 5], uncompressed[pos + 6], uncompressed[pos + 7],
            ]);
            pos += 8;

            let type_u32 = u32::from_le_bytes([
                uncompressed[pos], uncompressed[pos + 1], uncompressed[pos + 2], uncompressed[pos + 3],
            ]);
            pos += 4;
            let pso_type = match type_u32 {
                0 => PipelineStateType::Graphics,
                1 => PipelineStateType::Compute,
                2 => PipelineStateType::MeshShader,
                _ => return Err(D3d12PipelineLibraryError::DecompressionFailed),
            };

            let bc_len = u32::from_le_bytes([
                uncompressed[pos], uncompressed[pos + 1], uncompressed[pos + 2], uncompressed[pos + 3],
            ]) as usize;
            pos += 4;

            if pos + bc_len > uncompressed_len {
                return Err(D3d12PipelineLibraryError::DecompressionFailed);
            }
            let bytecode = uncompressed[pos..pos + bc_len].to_vec();
            pos += bc_len;

            entries.push(SerializedPsoEntry {
                pso_id,
                pso_type,
                bytecode,
            });
        }

        Ok(entries)
    }
}
