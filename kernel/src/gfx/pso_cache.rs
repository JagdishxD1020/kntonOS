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

// Priority level for asynchronous shader compilation requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PsoCompilePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

// Current compilation lifecycle state for an asynchronous job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PsoCompileStatus {
    Queued,
    Compiling,
    Ready(CompiledPipelineBlob),
    Failed,
}

// Work item submitted to the asynchronous PSO compiler queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsoCompileRequest {
    // Unique request tracking identifier.
    pub request_id: u64,
    // Symbolic pipeline name.
    pub name: String,
    // Deterministic hash key.
    pub hash_key: PsoHashKey,
    // Pipeline classification kind.
    pub kind: PipelineKind,
    // Bound root signature ID.
    pub root_signature_id: u64,
    // Raw bytecode input payload.
    pub raw_bytecode: Vec<u8>,
    // Scheduled compilation priority.
    pub priority: PsoCompilePriority,
    // Current compilation status.
    pub status: PsoCompileStatus,
}

// Asynchronous background PSO compiler queue preventing main-thread draw hitching.
#[derive(Debug, Clone)]
pub struct AsyncPsoCompilerQueue {
    // List of pending and completed compile requests.
    pub requests: Vec<PsoCompileRequest>,
    // Monotonic request ID counter.
    pub next_request_id: u64,
    // Maximum concurrent worker jobs allowed per tick.
    pub max_in_flight_workers: usize,
    // Total jobs successfully compiled.
    pub total_compiled_jobs: u64,
    // Total runtime requests satisfied from cache or ready state without stutter.
    pub total_stutter_free_hits: u64,
}

impl AsyncPsoCompilerQueue {
    // Creates a new async compiler queue with specified concurrency limit.
    pub fn new(max_workers: usize) -> Self {
        Self {
            requests: Vec::new(),
            next_request_id: 1,
            max_in_flight_workers: if max_workers == 0 { 4 } else { max_workers },
            total_compiled_jobs: 0,
            total_stutter_free_hits: 0,
        }
    }

    // Submits a pipeline compilation request to background workers.
    pub fn submit_request(
        &mut self,
        name: &str,
        kind: PipelineKind,
        root_sig_id: u64,
        raw_bytecode: &[u8],
        priority: PsoCompilePriority,
    ) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let hash_key = hash_bytes_128(raw_bytecode);

        self.requests.push(PsoCompileRequest {
            request_id,
            name: String::from(name),
            hash_key,
            kind,
            root_signature_id: root_sig_id,
            raw_bytecode: raw_bytecode.to_vec(),
            priority,
            status: PsoCompileStatus::Queued,
        });

        request_id
    }

    // Steps background worker threads to compile queued requests.
    pub fn step_workers(
        &mut self,
        max_steps: usize,
        library: &mut D3D12PipelineLibrary,
    ) -> usize {
        let mut steps = 0;
        let limit = core::cmp::min(max_steps, self.max_in_flight_workers);

        while steps < limit {
            let mut best_idx = None;
            let mut highest_prio = PsoCompilePriority::Low;

            for (i, req) in self.requests.iter().enumerate() {
                if req.status == PsoCompileStatus::Queued {
                    if best_idx.is_none() || req.priority >= highest_prio {
                        highest_prio = req.priority;
                        best_idx = Some(i);
                    }
                }
            }

            let idx = match best_idx {
                Some(i) => i,
                None => break,
            };

            self.requests[idx].status = PsoCompileStatus::Compiling;
            let blob = CompiledPipelineBlob {
                hash_key: self.requests[idx].hash_key,
                kind: self.requests[idx].kind,
                bytecode: self.requests[idx].raw_bytecode.clone(),
                root_signature_id: self.requests[idx].root_signature_id,
            };

            let _ = library.store_pipeline(&self.requests[idx].name, blob.clone());
            self.requests[idx].status = PsoCompileStatus::Ready(blob);
            self.total_compiled_jobs += 1;
            steps += 1;
        }

        steps
    }

    // Queries current status of a submitted compilation request.
    pub fn poll_status(&self, request_id: u64) -> Option<&PsoCompileStatus> {
        for req in &self.requests {
            if req.request_id == request_id {
                return Some(&req.status);
            }
        }
        None
    }

    // Takes completed compiled pipeline blob if ready.
    pub fn take_completed(&mut self, request_id: u64) -> Option<CompiledPipelineBlob> {
        for req in &mut self.requests {
            if req.request_id == request_id {
                if let PsoCompileStatus::Ready(ref blob) = req.status {
                    self.total_stutter_free_hits += 1;
                    return Some(blob.clone());
                }
            }
        }
        None
    }

    // Returns count of jobs waiting in queued state.
    pub fn queued_count(&self) -> usize {
        self.requests
            .iter()
            .filter(|r| r.status == PsoCompileStatus::Queued)
            .count()
    }
}

// Configuration for handheld DirectStorage persistent PSO cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectStoragePsoCacheConfig {
    // Disk sector alignment in bytes (e.g. 4096).
    pub block_size_bytes: usize,
    // Maximum persistent cache storage capacity in bytes.
    pub max_cache_file_size_bytes: usize,
}

impl Default for DirectStoragePsoCacheConfig {
    fn default() -> Self {
        Self {
            block_size_bytes: 4096,
            max_cache_file_size_bytes: 32 * 1024 * 1024,
        }
    }
}

// Handheld DirectStorage persistent cache backing for zero-stutter runtime PSO lookups.
#[derive(Debug, Clone)]
pub struct DirectStoragePsoCache {
    // Cache configuration parameters.
    pub config: DirectStoragePsoCacheConfig,
    // Persistent NVMe storage byte backing.
    pub disk_storage: Vec<u8>,
    // Successful cache hit counter.
    pub cache_hits: u64,
    // Cache miss counter requiring runtime fallback.
    pub cache_misses: u64,
    // Cumulative bytes streamed from NVMe.
    pub bytes_read_from_disk: u64,
    // Cumulative bytes written to NVMe.
    pub bytes_written_to_disk: u64,
    // Total pipelines loaded during pre-warming phase.
    pub prewarmed_pipeline_count: u64,
}

impl DirectStoragePsoCache {
    // Creates a new DirectStorage cache backing with specified configuration.
    pub fn new(config: DirectStoragePsoCacheConfig) -> Self {
        Self {
            config,
            disk_storage: Vec::new(),
            cache_hits: 0,
            cache_misses: 0,
            bytes_read_from_disk: 0,
            bytes_written_to_disk: 0,
            prewarmed_pipeline_count: 0,
        }
    }

    // Persists the in-memory pipeline library into DirectStorage aligned disk backing.
    pub fn write_library_to_disk(
        &mut self,
        library: &D3D12PipelineLibrary,
    ) -> Result<usize, PipelineLibraryError> {
        let serialized = library.serialize();
        let block_sz = self.config.block_size_bytes;
        let padded_size = if block_sz > 0 {
            ((serialized.len() + block_sz - 1) / block_sz) * block_sz
        } else {
            serialized.len()
        };

        if padded_size > self.config.max_cache_file_size_bytes {
            return Err(PipelineLibraryError::BufferTooSmall {
                required: padded_size,
                provided: self.config.max_cache_file_size_bytes,
            });
        }

        self.disk_storage.clear();
        self.disk_storage.extend_from_slice(&serialized);
        if self.disk_storage.len() < padded_size {
            self.disk_storage.resize(padded_size, 0);
        }

        self.bytes_written_to_disk += padded_size as u64;
        Ok(padded_size)
    }

    // Pre-warms the in-memory pipeline library by streaming cache from DirectStorage.
    pub fn prewarm_library_from_disk(
        &mut self,
        library: &mut D3D12PipelineLibrary,
    ) -> Result<usize, PipelineLibraryError> {
        if self.disk_storage.is_empty() {
            return Ok(0);
        }

        let loaded_lib = D3D12PipelineLibrary::deserialize(&self.disk_storage)?;
        let mut loaded_count = 0;

        for entry in loaded_lib.entries {
            if library.store_pipeline(&entry.name, entry.blob).is_ok() {
                loaded_count += 1;
            }
        }

        self.bytes_read_from_disk += self.disk_storage.len() as u64;
        self.prewarmed_pipeline_count += loaded_count as u64;
        Ok(loaded_count)
    }

    // Queries a pipeline from the pre-warmed library, tracking cache hit/miss telemetry.
    pub fn query_pipeline(
        &mut self,
        name: &str,
        library: &D3D12PipelineLibrary,
    ) -> Option<CompiledPipelineBlob> {
        match library.load_pipeline(name) {
            Ok(blob) => {
                self.cache_hits += 1;
                Some(blob)
            }
            Err(_) => {
                self.cache_misses += 1;
                None
            }
        }
    }

    // Calculates current runtime cache hit ratio percentage (0 to 100).
    pub fn hit_ratio_percentage(&self) -> u32 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0
        } else {
            ((self.cache_hits * 100) / total) as u32
        }
    }
}

// Comprehensive verification self-test suite for D3D12 Agility PSO binary library cache.
pub fn test_pso_library_cache_subsystem() -> bool {
    // 1. Verify 128-bit fast hash key indexing and determinism.
    let h1 = hash_bytes_128(b"vs_main_dxbc");
    let h2 = hash_bytes_128(b"vs_main_dxbc");
    let h3 = hash_bytes_128(b"ps_main_dxbc");

    if h1 != h2 || h1 == h3 || h1 == PsoHashKey::ZERO {
        return false;
    }

    // 2. Verify D3D12 pipeline library serialization and deserialization.
    let mut lib = D3D12PipelineLibrary::new(*b"KntonGpuDrv-RDNA");
    let blob_gfx = CompiledPipelineBlob {
        hash_key: h1,
        kind: PipelineKind::Graphics,
        bytecode: alloc::vec![10, 20, 30, 40],
        root_signature_id: 100,
    };
    let blob_comp = CompiledPipelineBlob {
        hash_key: h3,
        kind: PipelineKind::Compute,
        bytecode: alloc::vec![50, 60, 70, 80],
        root_signature_id: 200,
    };

    if lib.store_pipeline("OpaqueDraw", blob_gfx.clone()).is_err() {
        return false;
    }
    if lib.store_pipeline("PostProcessCS", blob_comp.clone()).is_err() {
        return false;
    }
    // Reject duplicate pipeline name.
    if lib.store_pipeline("OpaqueDraw", blob_gfx.clone()) != Err(PipelineLibraryError::DuplicateName) {
        return false;
    }

    let serialized = lib.serialize();
    if serialized.len() != lib.get_serialized_size() {
        return false;
    }

    let deserialized = match D3D12PipelineLibrary::deserialize(&serialized) {
        Ok(l) => l,
        Err(_) => return false,
    };
    if deserialized.pipeline_count() != 2 {
        return false;
    }
    let loaded = match deserialized.load_pipeline("OpaqueDraw") {
        Ok(b) => b,
        Err(_) => return false,
    };
    if loaded.bytecode != blob_gfx.bytecode || loaded.root_signature_id != 100 {
        return false;
    }
    if deserialized.load_pipeline("MissingPSO") != Err(PipelineLibraryError::NotFound) {
        return false;
    }

    // 3. Verify asynchronous background PSO compiler queue.
    let mut queue = AsyncPsoCompilerQueue::new(4);
    let _req1 = queue.submit_request("ShadowPass", PipelineKind::Graphics, 10, &[1, 2], PsoCompilePriority::Normal);
    let req2 = queue.submit_request("UIPass", PipelineKind::Graphics, 10, &[3, 4], PsoCompilePriority::Critical);
    let _req3 = queue.submit_request("BloomPass", PipelineKind::Compute, 20, &[5, 6], PsoCompilePriority::Low);

    if queue.queued_count() != 3 {
        return false;
    }

    let mut worker_lib = D3D12PipelineLibrary::new(*b"KntonGpuDrv-RDNA");
    let processed = queue.step_workers(2, &mut worker_lib);
    if processed != 2 || queue.queued_count() != 1 {
        return false;
    }

    // Critical priority job must be ready.
    let taken = match queue.take_completed(req2) {
        Some(b) => b,
        None => return false,
    };
    if taken.bytecode != alloc::vec![3, 4] {
        return false;
    }

    // 4. Verify handheld DirectStorage persistent cache backing and pre-warming.
    let mut ds_cache = DirectStoragePsoCache::new(DirectStoragePsoCacheConfig::default());
    let write_size = match ds_cache.write_library_to_disk(&lib) {
        Ok(sz) => sz,
        Err(_) => return false,
    };
    if write_size < 4096 || write_size % 4096 != 0 {
        return false;
    }

    let mut fresh_lib = D3D12PipelineLibrary::new(*b"KntonGpuDrv-RDNA");
    let prewarmed = match ds_cache.prewarm_library_from_disk(&mut fresh_lib) {
        Ok(cnt) => cnt,
        Err(_) => return false,
    };
    if prewarmed != 2 || fresh_lib.pipeline_count() != 2 {
        return false;
    }

    let hit_blob = ds_cache.query_pipeline("OpaqueDraw", &fresh_lib);
    if hit_blob.is_none() {
        return false;
    }
    let miss_blob = ds_cache.query_pipeline("MissingPSO", &fresh_lib);
    if miss_blob.is_some() {
        return false;
    }

    if ds_cache.cache_hits != 1 || ds_cache.cache_misses != 1 || ds_cache.hit_ratio_percentage() != 50 {
        return false;
    }

    // 5. Emit deterministic boot sentinel.
    crate::serial_write_str(
        "[KNT-D3D12-PSO-CACHE-PASS] Direct3D 12 agility PSO binary library caching verified\r\n",
    );
    true
}
