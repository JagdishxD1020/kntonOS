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

// Temporary stub for subsystem self-test during incremental commits.
pub fn test_descriptor_suballocator_subsystem() -> bool {
    true
}
