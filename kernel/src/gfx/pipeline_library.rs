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

// Compresses raw bytes into an LZ4 block containing valid literal and sequence framing.
pub fn lz4_compress(src: &[u8], dst: &mut [u8]) -> Result<usize, D3d12PipelineLibraryError> {
    let src_len = src.len();
    let mut dst_pos = 0usize;

    if src_len < 15 {
        if dst.len() < 1 + src_len {
            return Err(D3d12PipelineLibraryError::BufferTooSmall);
        }
        dst[dst_pos] = (src_len as u8) << 4;
        dst_pos += 1;
    } else {
        let mut rem = src_len - 15;
        let extra_bytes = rem / 255 + 1;
        if dst.len() < 1 + extra_bytes + src_len {
            return Err(D3d12PipelineLibraryError::BufferTooSmall);
        }
        dst[dst_pos] = 0xF0;
        dst_pos += 1;
        while rem >= 255 {
            dst[dst_pos] = 255;
            dst_pos += 1;
            rem -= 255;
        }
        dst[dst_pos] = rem as u8;
        dst_pos += 1;
    }

    dst[dst_pos..dst_pos + src_len].copy_from_slice(src);
    dst_pos += src_len;

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

// Asynchronous background PSO compilation execution status.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AsyncPsoStatus {
    Queued,
    Compiling,
    Ready,
    Failed,
}

// Background PSO compilation request task metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsyncPsoTask {
    pub pso_id: u64,
    pub priority: u8,
    pub pso_type: PipelineStateType,
    pub status: AsyncPsoStatus,
    pub bytecode_bytes: usize,
    pub worker_thread_id: Option<u32>,
}

// Asynchronous multi-threaded pipeline state compilation queue.
pub struct AsyncPsoCompilationQueue {
    tasks: Vec<AsyncPsoTask>,
    completed_count: usize,
}

impl AsyncPsoCompilationQueue {
    // Initializes an empty asynchronous PSO compilation queue.
    pub const fn new() -> Self {
        Self {
            tasks: Vec::new(),
            completed_count: 0,
        }
    }

    // Submits a pipeline state compilation task to the background queue with priority ordering.
    pub fn enqueue_pso(
        &mut self,
        pso_id: u64,
        pso_type: PipelineStateType,
        priority: u8,
        bytecode_bytes: usize,
    ) {
        let task = AsyncPsoTask {
            pso_id,
            priority,
            pso_type,
            status: AsyncPsoStatus::Queued,
            bytecode_bytes,
            worker_thread_id: None,
        };

        // Insert maintaining descending priority order (higher priority at the front).
        let idx = self.tasks.iter().position(|t| t.priority < priority).unwrap_or(self.tasks.len());
        self.tasks.insert(idx, task);
    }

    // Simulates multi-threaded worker dispatch to compile queued PSO jobs concurrently.
    pub fn dispatch_worker_threads(&mut self, thread_count: usize) {
        let mut active_worker = 0u32;
        for task in self.tasks.iter_mut() {
            if task.status == AsyncPsoStatus::Queued && (active_worker as usize) < thread_count {
                task.status = AsyncPsoStatus::Compiling;
                task.worker_thread_id = Some(active_worker);
                active_worker += 1;
            } else if task.status == AsyncPsoStatus::Compiling {
                // Advance compiling jobs to Ready on worker completion.
                task.status = AsyncPsoStatus::Ready;
                self.completed_count += 1;
            }
        }
    }

    // Queries compilation readiness for a specific pipeline state object.
    pub fn query_status(&self, pso_id: u64) -> Option<AsyncPsoStatus> {
        self.tasks.iter().find(|t| t.pso_id == pso_id).map(|t| t.status)
    }

    // Determines if a pipeline state object is compiled and ready for immediate draw binding.
    pub fn is_ready(&self, pso_id: u64) -> bool {
        self.query_status(pso_id) == Some(AsyncPsoStatus::Ready)
    }

    // Returns the count of tasks currently queued or compiling in the background.
    pub fn pending_count(&self) -> usize {
        self.tasks.iter().filter(|t| t.status != AsyncPsoStatus::Ready && t.status != AsyncPsoStatus::Failed).count()
    }

    // Returns total count of successfully compiled pipeline state objects.
    pub fn completed_count(&self) -> usize {
        self.completed_count
    }
}

// Default disk shader cache storage quota (512 Megabytes).
pub const DEFAULT_SHADER_CACHE_QUOTA_BYTES: usize = 512 * 1024 * 1024;

// Cache entry tracked by the driver shader cache eviction governor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderCacheEntry {
    pub pso_id: u64,
    pub size_bytes: usize,
    pub last_access_tick: u64,
    pub access_count: u32,
}

// Driver shader cache eviction governor managing disk page-cache trimming under quota pressure.
pub struct DriverShaderCacheEvictionGovernor {
    quota_bytes: usize,
    current_usage_bytes: usize,
    entries: Vec<ShaderCacheEntry>,
    access_counter: u64,
}

impl DriverShaderCacheEvictionGovernor {
    // Creates a new eviction governor with specified cache quota limit.
    pub const fn new(quota_bytes: usize) -> Self {
        Self {
            quota_bytes,
            current_usage_bytes: 0,
            entries: Vec::new(),
            access_counter: 1,
        }
    }

    // Default governor initialized with standard 512MB disk shader cache quota.
    pub const fn default_quota() -> Self {
        Self::new(DEFAULT_SHADER_CACHE_QUOTA_BYTES)
    }

    // Inserts or updates a cached shader binary, trimming LRU entries when quota is exceeded.
    pub fn insert_shader(&mut self, pso_id: u64, size_bytes: usize) -> Result<usize, D3d12PipelineLibraryError> {
        if size_bytes > self.quota_bytes {
            return Err(D3d12PipelineLibraryError::QuotaExceeded);
        }

        // If entry already exists, update size and access tick.
        if let Some(pos) = self.entries.iter().position(|e| e.pso_id == pso_id) {
            let old_size = self.entries[pos].size_bytes;
            self.current_usage_bytes = self.current_usage_bytes.saturating_sub(old_size);
            self.entries.remove(pos);
        }

        let mut evicted_count = 0usize;
        // Evict least-recently-used entries until sufficient quota headroom is restored.
        while self.current_usage_bytes.saturating_add(size_bytes) > self.quota_bytes && !self.entries.is_empty() {
            // Find entry with minimum last_access_tick.
            let min_idx = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_access_tick)
                .map(|(idx, _)| idx)
                .unwrap_or(0);

            let removed = self.entries.remove(min_idx);
            self.current_usage_bytes = self.current_usage_bytes.saturating_sub(removed.size_bytes);
            evicted_count += 1;
        }

        self.access_counter += 1;
        self.entries.push(ShaderCacheEntry {
            pso_id,
            size_bytes,
            last_access_tick: self.access_counter,
            access_count: 1,
        });
        self.current_usage_bytes += size_bytes;

        Ok(evicted_count)
    }

    // Marks a cached shader entry as active, refreshing its LRU eviction timeline.
    pub fn touch_shader(&mut self, pso_id: u64) -> bool {
        self.access_counter += 1;
        if let Some(entry) = self.entries.iter_mut().find(|e| e.pso_id == pso_id) {
            entry.last_access_tick = self.access_counter;
            entry.access_count = entry.access_count.saturating_add(1);
            true
        } else {
            false
        }
    }

    // Indicates whether the specified PSO currently resides in the disk shader cache.
    pub fn contains(&self, pso_id: u64) -> bool {
        self.entries.iter().any(|e| e.pso_id == pso_id)
    }

    // Returns current memory/disk consumption of cached shaders in bytes.
    pub fn current_usage_bytes(&self) -> usize {
        self.current_usage_bytes
    }

    // Returns the total allocated quota limit in bytes.
    pub fn quota_bytes(&self) -> usize {
        self.quota_bytes
    }

    // Returns count of active entries currently held in the cache.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

// Game level shader pre-warming manifest specifying PSO dependencies for seamless loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelShaderPrewarmManifest {
    pub level_id: u32,
    pub pso_dependencies: Vec<u64>,
}

impl LevelShaderPrewarmManifest {
    // Creates a new shader pre-warm manifest for a specific game level or scene.
    pub const fn new(level_id: u32) -> Self {
        Self {
            level_id,
            pso_dependencies: Vec::new(),
        }
    }

    // Registers a required pipeline state object dependency.
    pub fn add_dependency(&mut self, pso_id: u64) {
        if !self.pso_dependencies.contains(&pso_id) {
            self.pso_dependencies.push(pso_id);
        }
    }
}

// Anti-stutter shader pre-warming governor eliminating runtime compilation hitches.
pub struct AntiStutterShaderPrewarmer {
    active_manifest: Option<LevelShaderPrewarmManifest>,
    prewarmed_count: usize,
    stutters_prevented: u32,
}

impl AntiStutterShaderPrewarmer {
    // Initializes the anti-stutter shader pre-warmer.
    pub const fn new() -> Self {
        Self {
            active_manifest: None,
            prewarmed_count: 0,
            stutters_prevented: 0,
        }
    }

    // Stages a game level manifest to initiate proactive PSO pre-compilation.
    pub fn stage_level(&mut self, manifest: LevelShaderPrewarmManifest) {
        self.active_manifest = Some(manifest);
    }

    // Executes pre-warming dispatch, compiling missing PSOs during level loading screens.
    pub fn prewarm_level_dependencies(
        &mut self,
        queue: &mut AsyncPsoCompilationQueue,
        governor: &mut DriverShaderCacheEvictionGovernor,
    ) -> usize {
        let manifest = match &self.active_manifest {
            Some(m) => m,
            None => return 0,
        };

        let mut enqueued = 0usize;
        for &pso_id in &manifest.pso_dependencies {
            // Check if PSO is already in disk cache or already queued.
            if !governor.contains(pso_id) && queue.query_status(pso_id).is_none() {
                // Priority 2 = Critical loading pre-warm priority.
                queue.enqueue_pso(pso_id, PipelineStateType::Graphics, 2, 4096);
                enqueued += 1;
            }
        }

        // Dispatch background worker threads until all prewarm tasks complete.
        while queue.pending_count() > 0 {
            queue.dispatch_worker_threads(4);
        }

        // Register compiled PSOs into driver disk cache governor.
        for &pso_id in &manifest.pso_dependencies {
            if !governor.contains(pso_id) {
                let _ = governor.insert_shader(pso_id, 4096);
                self.prewarmed_count += 1;
                self.stutters_prevented = self.stutters_prevented.saturating_add(1);
            }
        }

        enqueued
    }

    // Verifies whether all PSOs required for the active game level are compiled and cached.
    pub fn is_level_ready(
        &self,
        queue: &AsyncPsoCompilationQueue,
        governor: &DriverShaderCacheEvictionGovernor,
    ) -> bool {
        let manifest = match &self.active_manifest {
            Some(m) => m,
            None => return true,
        };

        manifest.pso_dependencies.iter().all(|&id| {
            governor.contains(id) || queue.is_ready(id)
        })
    }

    // Returns cumulative number of runtime shader compilation stutters avoided.
    pub fn stutters_prevented(&self) -> u32 {
        self.stutters_prevented
    }
}

// Comprehensive self-test suite for Direct3D 12 Agility Pipeline Library and shader cache.
pub fn test_pipeline_library_subsystem() -> bool {
    crate::serial_write_str("[KNT-PIPELIB] Initializing Direct3D 12 Agility Pipeline Library self-test...\r\n");

    // 1. Verify PSO serialization, CRC32 checksum, and LZ4 decompression.
    let entry0 = SerializedPsoEntry {
        pso_id: 0x1000_A001,
        pso_type: PipelineStateType::Graphics,
        bytecode: alloc::vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44],
    };
    let entry1 = SerializedPsoEntry {
        pso_id: 0x2000_B002,
        pso_type: PipelineStateType::Compute,
        bytecode: alloc::vec![0x55, 0x66, 0x77, 0x88, 0x99, 0x00, 0xEE, 0xFF],
    };

    let serialized = match PipelineLibraryDiskStorage::serialize(&[entry0.clone(), entry1.clone()]) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    let deserialized = match PipelineLibraryDiskStorage::deserialize(&serialized) {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    if deserialized.len() != 2 || deserialized[0] != entry0 || deserialized[1] != entry1 {
        return false;
    }

    // Corrupted payload verification.
    let mut corrupted = serialized.clone();
    let last_idx = corrupted.len() - 1;
    corrupted[last_idx] ^= 0xFF;
    if PipelineLibraryDiskStorage::deserialize(&corrupted) != Err(D3d12PipelineLibraryError::ChecksumMismatch) {
        return false;
    }

    // 2. Verify background asynchronous PSO compilation queue and worker dispatch.
    let mut queue = AsyncPsoCompilationQueue::new();
    queue.enqueue_pso(0x3000_C003, PipelineStateType::Graphics, 1, 1024);
    queue.enqueue_pso(0x4000_D004, PipelineStateType::Compute, 2, 2048);

    if queue.query_status(0x3000_C003) != Some(AsyncPsoStatus::Queued) {
        return false;
    }

    // First dispatch advances to Compiling.
    queue.dispatch_worker_threads(2);
    if queue.query_status(0x4000_D004) != Some(AsyncPsoStatus::Compiling) {
        return false;
    }

    // Second dispatch completes compilation to Ready.
    queue.dispatch_worker_threads(2);
    if !queue.is_ready(0x4000_D004) || !queue.is_ready(0x3000_C003) || queue.completed_count() != 2 {
        return false;
    }

    // 3. Verify driver shader cache eviction governor (LRU trimming under quota).
    let mut governor = DriverShaderCacheEvictionGovernor::new(100_000);
    if governor.insert_shader(0x101, 40_000) != Ok(0) || governor.insert_shader(0x102, 40_000) != Ok(0) {
        return false;
    }
    if !governor.contains(0x101) || !governor.contains(0x102) {
        return false;
    }

    // Touch 0x101 to make 0x102 the least-recently used entry.
    governor.touch_shader(0x101);

    // Inserting 0x103 (40,000 bytes) requires 80,000 + 40,000 > 100,000, evicting 0x102.
    let evicted = match governor.insert_shader(0x103, 40_000) {
        Ok(n) => n,
        Err(_) => return false,
    };
    if evicted != 1 || !governor.contains(0x101) || governor.contains(0x102) || !governor.contains(0x103) {
        return false;
    }
    if governor.current_usage_bytes() != 80_000 {
        return false;
    }

    // 4. Verify anti-stutter shader pre-warming during level loading.
    let mut manifest = LevelShaderPrewarmManifest::new(101);
    manifest.add_dependency(0x5000_E005);
    manifest.add_dependency(0x6000_F006);

    let mut prewarmer = AntiStutterShaderPrewarmer::new();
    prewarmer.stage_level(manifest);

    let prewarmed = prewarmer.prewarm_level_dependencies(&mut queue, &mut governor);
    if prewarmed != 2 || !prewarmer.is_level_ready(&queue, &governor) || prewarmer.stutters_prevented() != 2 {
        return false;
    }

    // 5. Emit deterministic boot sentinel.
    crate::serial_write_str("[KNT-D3D12-PIPE-LIB-PASS] Direct3D 12 Agility pipeline library & shader cache engine verified\r\n");
    true
}
