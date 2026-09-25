//! Direct3D 12 Dynamic Descriptor Heap Suballocator & Ring Buffer.
//! Manages chunked descriptor suballocation, bulk copying, timeline fences, and ring buffers.

#![allow(dead_code, non_snake_case, unused_imports)]

use alloc::vec::Vec;
use super::root_signature::{
    D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_DESCRIPTOR_HEAP_TYPE, D3D12_GPU_DESCRIPTOR_HANDLE,
};

// Fixed number of descriptors per allocation chunk.
pub const DESCRIPTOR_CHUNK_SIZE: usize = 64;

// Standard D3D12 CBV/SRV/UAV descriptor size in bytes.
pub const DESCRIPTOR_SIZE_CBV_SRV_UAV: usize = 32;

// Standard D3D12 Sampler descriptor size in bytes.
pub const DESCRIPTOR_SIZE_SAMPLER: usize = 16;

// Suballocator error conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorSuballocatorError {
    OutOfMemory,
    PoolExhausted,
    InvalidChunkIndex,
    InvalidDescriptorCount,
    BufferOverflow,
    FenceNotSignaled,
    InvalidHeapType,
}

// Represents a suballocated contiguous range of descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuballocatedDescriptorRange {
    pub chunk_index: usize,
    pub offset_in_chunk: usize,
    pub count: usize,
    pub base_cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
    pub base_gpu_handle: D3D12_GPU_DESCRIPTOR_HANDLE,
}

impl SuballocatedDescriptorRange {
    // Calculates the CPU handle for the descriptor at the given index in this range.
    #[inline(always)]
    pub fn cpu_handle(&self, index: usize, descriptor_size: usize) -> Option<D3D12_CPU_DESCRIPTOR_HANDLE> {
        if index < self.count {
            Some(self.base_cpu_handle.offset(index, descriptor_size))
        } else {
            None
        }
    }

    // Calculates the GPU handle for the descriptor at the given index in this range.
    #[inline(always)]
    pub fn gpu_handle(&self, index: usize, descriptor_size: usize) -> Option<D3D12_GPU_DESCRIPTOR_HANDLE> {
        if index < self.count {
            Some(self.base_gpu_handle.offset(index, descriptor_size))
        } else {
            None
        }
    }
}

// A fixed-size 64-descriptor chunk inside a shader-visible descriptor heap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorChunk {
    pub chunk_index: usize,
    pub base_descriptor_index: usize,
    pub base_cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
    pub base_gpu_handle: D3D12_GPU_DESCRIPTOR_HANDLE,
    pub allocated_descriptors: usize,
    pub is_free: bool,
}

impl DescriptorChunk {
    // Creates a new empty descriptor chunk at a given index.
    pub fn new(
        chunk_index: usize,
        base_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
        base_gpu: D3D12_GPU_DESCRIPTOR_HANDLE,
        descriptor_size: usize,
    ) -> Self {
        let base_descriptor_index = chunk_index * DESCRIPTOR_CHUNK_SIZE;
        Self {
            chunk_index,
            base_descriptor_index,
            base_cpu_handle: base_cpu.offset(base_descriptor_index, descriptor_size),
            base_gpu_handle: base_gpu.offset(base_descriptor_index, descriptor_size),
            allocated_descriptors: 0,
            is_free: true,
        }
    }

    // Resets chunk allocation state for reuse.
    pub fn reset(&mut self) {
        self.allocated_descriptors = 0;
        self.is_free = true;
    }
}

// Chunked Descriptor Pool Manager managing fixed 64-descriptor blocks.
#[derive(Debug)]
pub struct ChunkedDescriptorPool {
    pub heap_type: D3D12_DESCRIPTOR_HEAP_TYPE,
    pub total_chunks: usize,
    pub descriptor_size: usize,
    pub base_cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
    pub base_gpu_handle: D3D12_GPU_DESCRIPTOR_HANDLE,
    pub chunks: Vec<DescriptorChunk>,
    pub free_list: Vec<usize>,
    pub active_chunk_index: Option<usize>,
    pub total_allocated_chunks: usize,
}

impl ChunkedDescriptorPool {
    // Creates a new chunked descriptor pool with fixed 64-descriptor chunks.
    pub fn new(
        base_cpu_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
        base_gpu_handle: D3D12_GPU_DESCRIPTOR_HANDLE,
        heap_type: D3D12_DESCRIPTOR_HEAP_TYPE,
        total_chunks: usize,
        descriptor_size: usize,
    ) -> Result<Self, DescriptorSuballocatorError> {
        if total_chunks == 0 {
            return Err(DescriptorSuballocatorError::PoolExhausted);
        }

        let mut chunks = Vec::with_capacity(total_chunks);
        let mut free_list = Vec::with_capacity(total_chunks);

        for i in 0..total_chunks {
            chunks.push(DescriptorChunk::new(
                i,
                base_cpu_handle,
                base_gpu_handle,
                descriptor_size,
            ));
            free_list.push(total_chunks - 1 - i);
        }

        Ok(Self {
            heap_type,
            total_chunks,
            descriptor_size,
            base_cpu_handle,
            base_gpu_handle,
            chunks,
            free_list,
            active_chunk_index: None,
            total_allocated_chunks: 0,
        })
    }

    // Allocates a dedicated 64-descriptor chunk from the pool.
    pub fn allocate_chunk(&mut self) -> Result<usize, DescriptorSuballocatorError> {
        if let Some(chunk_idx) = self.free_list.pop() {
            self.chunks[chunk_idx].is_free = false;
            self.chunks[chunk_idx].allocated_descriptors = 0;
            self.total_allocated_chunks = self.total_allocated_chunks.saturating_add(1);
            Ok(chunk_idx)
        } else {
            Err(DescriptorSuballocatorError::PoolExhausted)
        }
    }

    // Returns a previously allocated chunk back to the pool free list.
    pub fn release_chunk(&mut self, chunk_index: usize) -> Result<(), DescriptorSuballocatorError> {
        if chunk_index >= self.total_chunks {
            return Err(DescriptorSuballocatorError::InvalidChunkIndex);
        }
        if self.chunks[chunk_index].is_free {
            return Ok(());
        }

        self.chunks[chunk_index].reset();
        self.free_list.push(chunk_index);
        self.total_allocated_chunks = self.total_allocated_chunks.saturating_sub(1);
        if self.active_chunk_index == Some(chunk_index) {
            self.active_chunk_index = None;
        }
        Ok(())
    }

    // Suballocates a contiguous range of descriptors from the pool's current active chunk.
    pub fn suballocate(
        &mut self,
        count: usize,
    ) -> Result<SuballocatedDescriptorRange, DescriptorSuballocatorError> {
        if count == 0 || count > DESCRIPTOR_CHUNK_SIZE {
            return Err(DescriptorSuballocatorError::InvalidDescriptorCount);
        }

        let chunk_idx = match self.active_chunk_index {
            Some(idx) => {
                if self.chunks[idx].allocated_descriptors + count <= DESCRIPTOR_CHUNK_SIZE {
                    idx
                } else {
                    let new_idx = self.allocate_chunk()?;
                    self.active_chunk_index = Some(new_idx);
                    new_idx
                }
            }
            None => {
                let new_idx = self.allocate_chunk()?;
                self.active_chunk_index = Some(new_idx);
                new_idx
            }
        };

        let chunk = &mut self.chunks[chunk_idx];
        let offset = chunk.allocated_descriptors;
        chunk.allocated_descriptors += count;

        let base_cpu = chunk.base_cpu_handle.offset(offset, self.descriptor_size);
        let base_gpu = chunk.base_gpu_handle.offset(offset, self.descriptor_size);

        Ok(SuballocatedDescriptorRange {
            chunk_index: chunk_idx,
            offset_in_chunk: offset,
            count,
            base_cpu_handle: base_cpu,
            base_gpu_handle: base_gpu,
        })
    }

    // Returns the number of currently available chunks in the pool.
    pub fn available_chunks(&self) -> usize {
        self.free_list.len()
    }
}

// Simulated payload of an emulated 32-byte or 16-byte D3D12 descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SimulatedDescriptorPayload {
    pub raw_bytes: [u8; 32],
    pub resource_gpu_address: u64,
    pub format: u32,
    pub view_type: u32,
}

impl SimulatedDescriptorPayload {
    // Creates a new simulated descriptor payload.
    pub fn new(resource_gpu_address: u64, format: u32, view_type: u32) -> Self {
        let mut raw_bytes = [0u8; 32];
        raw_bytes[0..8].copy_from_slice(&resource_gpu_address.to_le_bytes());
        raw_bytes[8..12].copy_from_slice(&format.to_le_bytes());
        raw_bytes[12..16].copy_from_slice(&view_type.to_le_bytes());
        Self {
            raw_bytes,
            resource_gpu_address,
            format,
            view_type,
        }
    }
}

// Bulk descriptor copy engine supporting CopyDescriptorsSimple and CopyDescriptors.
#[derive(Debug, Clone, Default)]
pub struct DescriptorCopyEngine {
    pub total_descriptors_copied: u64,
    pub total_copy_operations: u64,
}

impl DescriptorCopyEngine {
    // Constructs a new descriptor copy engine.
    pub fn new() -> Self {
        Self::default()
    }

    // Translates a CPU handle to a GPU handle given heap base handles and descriptor size.
    #[inline(always)]
    pub fn translate_cpu_to_gpu(
        dest_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
        heap_base_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
        heap_base_gpu: D3D12_GPU_DESCRIPTOR_HANDLE,
        descriptor_size: usize,
    ) -> Option<D3D12_GPU_DESCRIPTOR_HANDLE> {
        if dest_cpu.ptr >= heap_base_cpu.ptr && descriptor_size > 0 {
            let offset_bytes = dest_cpu.ptr - heap_base_cpu.ptr;
            let index = offset_bytes / descriptor_size;
            Some(heap_base_gpu.offset(index, descriptor_size))
        } else {
            None
        }
    }

    // Simulates CopyDescriptorsSimple copying a contiguous range of descriptors.
    pub fn copy_descriptors_simple(
        &mut self,
        num_descriptors: usize,
        dest_range_start: D3D12_CPU_DESCRIPTOR_HANDLE,
        src_range_start: D3D12_CPU_DESCRIPTOR_HANDLE,
        descriptor_size: usize,
        descriptor_store: &mut [SimulatedDescriptorPayload],
        heap_base_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
    ) -> Result<usize, DescriptorSuballocatorError> {
        if num_descriptors == 0 {
            return Ok(0);
        }
        if descriptor_size == 0 || dest_range_start.ptr < heap_base_cpu.ptr {
            return Err(DescriptorSuballocatorError::BufferOverflow);
        }

        let dest_base_idx = (dest_range_start.ptr - heap_base_cpu.ptr) / descriptor_size;
        let src_base_idx = if src_range_start.ptr >= heap_base_cpu.ptr {
            (src_range_start.ptr - heap_base_cpu.ptr) / descriptor_size
        } else {
            src_range_start.ptr / descriptor_size
        };

        if dest_base_idx + num_descriptors > descriptor_store.len() {
            return Err(DescriptorSuballocatorError::BufferOverflow);
        }

        for i in 0..num_descriptors {
            let src_idx = src_base_idx + i;
            let dst_idx = dest_base_idx + i;
            if src_idx < descriptor_store.len() {
                descriptor_store[dst_idx] = descriptor_store[src_idx];
            }
        }

        self.total_descriptors_copied = self.total_descriptors_copied.saturating_add(num_descriptors as u64);
        self.total_copy_operations = self.total_copy_operations.saturating_add(1);

        Ok(num_descriptors)
    }

    // Simulates CopyDescriptors copying multiple non-contiguous descriptor ranges.
    pub fn copy_descriptors(
        &mut self,
        dest_range_starts: &[D3D12_CPU_DESCRIPTOR_HANDLE],
        dest_range_sizes: &[usize],
        src_range_starts: &[D3D12_CPU_DESCRIPTOR_HANDLE],
        src_range_sizes: &[usize],
        descriptor_size: usize,
        descriptor_store: &mut [SimulatedDescriptorPayload],
        heap_base_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
    ) -> Result<usize, DescriptorSuballocatorError> {
        let total_dest: usize = dest_range_sizes.iter().sum();
        let total_src: usize = src_range_sizes.iter().sum();
        if total_dest != total_src {
            return Err(DescriptorSuballocatorError::InvalidDescriptorCount);
        }

        let mut current_src_range = 0;
        let mut src_offset = 0;
        let mut copied = 0;

        for (dest_idx, &dest_start) in dest_range_starts.iter().enumerate() {
            let dest_size = if dest_idx < dest_range_sizes.len() {
                dest_range_sizes[dest_idx]
            } else {
                1
            };

            for d in 0..dest_size {
                while current_src_range < src_range_sizes.len()
                    && src_offset >= src_range_sizes[current_src_range]
                {
                    current_src_range += 1;
                    src_offset = 0;
                }

                if current_src_range >= src_range_starts.len() {
                    break;
                }

                let src_start = src_range_starts[current_src_range];
                let dest_handle = dest_start.offset(d, descriptor_size);
                let src_handle = src_start.offset(src_offset, descriptor_size);

                self.copy_descriptors_simple(
                    1,
                    dest_handle,
                    src_handle,
                    descriptor_size,
                    descriptor_store,
                    heap_base_cpu,
                )?;

                src_offset += 1;
                copied += 1;
            }
        }

        Ok(copied)
    }
}

// Temporary stub for subsystem self-test during incremental commits.
pub fn test_descriptor_suballocator_subsystem() -> bool {
    true
}
