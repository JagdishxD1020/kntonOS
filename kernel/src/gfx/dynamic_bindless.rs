// Direct3D 12 Agility Dynamic Resources & Bindless Sampler Feedback Optimization Engine.
// Manages bindless descriptor heap indexing, sampler feedback LOD evaluation, and memory throttling.

extern crate alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;

// Number of descriptors per heap allocation chunk (1024).
pub const BINDLESS_CHUNK_SIZE: usize = 1024;
// Maximum descriptor capacity supporting up to 1,048,576 bindless views.
pub const MAX_BINDLESS_DESCRIPTORS: usize = 1_048_576;
// Maximum number of chunks in the descriptor table.
pub const MAX_BINDLESS_CHUNKS: usize = MAX_BINDLESS_DESCRIPTORS / BINDLESS_CHUNK_SIZE;

// Extended resource descriptor view archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgilityDescriptorViewType {
    Texture2D,
    Texture2DArray,
    Texture3D,
    BufferRaw,
    BufferStructured,
    BufferTyped,
}

// Errors returned by bindless descriptor table operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindlessTableError {
    IndexOutOfBounds(u32),
    TableFull,
    SlotNotAllocated(u32),
}

// Dynamic resource descriptor layout representing fine-grained texture and buffer views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicResourceDescriptor {
    pub view_type: AgilityDescriptorViewType,
    pub gpu_va: u64,
    pub size_or_stride: u32,
    pub format_fourcc: u32,
    pub num_mips: u8,
    pub most_detailed_mip: u8,
    pub min_lod_clamp: u8,
    pub is_valid: bool,
}

impl DynamicResourceDescriptor {
    // Construct null/empty descriptor.
    pub const fn null() -> Self {
        Self {
            view_type: AgilityDescriptorViewType::Texture2D,
            gpu_va: 0,
            size_or_stride: 0,
            format_fourcc: 0,
            num_mips: 0,
            most_detailed_mip: 0,
            min_lod_clamp: 0,
            is_valid: false,
        }
    }

    // Construct a Texture2D descriptor view.
    pub fn texture_2d(gpu_va: u64, size: u32, format: u32, num_mips: u8) -> Self {
        Self {
            view_type: AgilityDescriptorViewType::Texture2D,
            gpu_va,
            size_or_stride: size,
            format_fourcc: format,
            num_mips,
            most_detailed_mip: 0,
            min_lod_clamp: 0,
            is_valid: true,
        }
    }

    // Construct a Buffer descriptor view.
    pub fn buffer(gpu_va: u64, size_bytes: u32, stride: u32, is_structured: bool) -> Self {
        Self {
            view_type: if is_structured {
                AgilityDescriptorViewType::BufferStructured
            } else {
                AgilityDescriptorViewType::BufferRaw
            },
            gpu_va,
            size_or_stride: if is_structured { stride } else { size_bytes },
            format_fourcc: 0,
            num_mips: 1,
            most_detailed_mip: 0,
            min_lod_clamp: 0,
            is_valid: true,
        }
    }
}

// Chunked 2-level bindless descriptor table supporting millions of views with sparse allocation.
pub struct BindlessDescriptorHeapTable {
    pub chunks: Vec<Option<Box<[DynamicResourceDescriptor; BINDLESS_CHUNK_SIZE]>>>,
    pub allocated_count: usize,
    pub next_free_hint: u32,
}

impl BindlessDescriptorHeapTable {
    // Construct new empty bindless descriptor table.
    pub fn new() -> Self {
        let mut chunks = Vec::with_capacity(MAX_BINDLESS_CHUNKS);
        for _ in 0..MAX_BINDLESS_CHUNKS {
            chunks.push(None);
        }
        Self {
            chunks,
            allocated_count: 0,
            next_free_hint: 0,
        }
    }

    // Allocate next available descriptor index in the table.
    pub fn allocate_index(&mut self) -> Result<u32, BindlessTableError> {
        if self.allocated_count >= MAX_BINDLESS_DESCRIPTORS {
            return Err(BindlessTableError::TableFull);
        }

        let start = self.next_free_hint as usize;
        for i in 0..MAX_BINDLESS_DESCRIPTORS {
            let idx = (start + i) % MAX_BINDLESS_DESCRIPTORS;
            let chunk_idx = idx / BINDLESS_CHUNK_SIZE;
            let slot_idx = idx % BINDLESS_CHUNK_SIZE;

            if self.chunks[chunk_idx].is_none() {
                let initial_chunk = Box::new([DynamicResourceDescriptor::null(); BINDLESS_CHUNK_SIZE]);
                self.chunks[chunk_idx] = Some(initial_chunk);
            }

            let chunk = self.chunks[chunk_idx].as_mut().unwrap();
            if !chunk[slot_idx].is_valid {
                self.allocated_count += 1;
                self.next_free_hint = (idx + 1) as u32;
                return Ok(idx as u32);
            }
        }

        Err(BindlessTableError::TableFull)
    }

    // Write descriptor into allocated table slot.
    pub fn write_descriptor(
        &mut self,
        index: u32,
        desc: DynamicResourceDescriptor,
    ) -> Result<(), BindlessTableError> {
        let idx = index as usize;
        if idx >= MAX_BINDLESS_DESCRIPTORS {
            return Err(BindlessTableError::IndexOutOfBounds(index));
        }

        let chunk_idx = idx / BINDLESS_CHUNK_SIZE;
        let slot_idx = idx % BINDLESS_CHUNK_SIZE;

        if self.chunks[chunk_idx].is_none() {
            let new_chunk = Box::new([DynamicResourceDescriptor::null(); BINDLESS_CHUNK_SIZE]);
            self.chunks[chunk_idx] = Some(new_chunk);
        }

        let chunk = self.chunks[chunk_idx].as_mut().unwrap();
        if !chunk[slot_idx].is_valid {
            self.allocated_count += 1;
        }
        chunk[slot_idx] = desc;
        Ok(())
    }

    // Read descriptor from table by global bindless index.
    pub fn read_descriptor(&self, index: u32) -> Option<&DynamicResourceDescriptor> {
        let idx = index as usize;
        if idx >= MAX_BINDLESS_DESCRIPTORS {
            return None;
        }

        let chunk_idx = idx / BINDLESS_CHUNK_SIZE;
        let slot_idx = idx % BINDLESS_CHUNK_SIZE;

        self.chunks.get(chunk_idx)?.as_ref().map(|chunk| &chunk[slot_idx])
    }

    // Free descriptor slot, marking it as invalid for recycling.
    pub fn free_descriptor(&mut self, index: u32) -> Result<(), BindlessTableError> {
        let idx = index as usize;
        if idx >= MAX_BINDLESS_DESCRIPTORS {
            return Err(BindlessTableError::IndexOutOfBounds(index));
        }

        let chunk_idx = idx / BINDLESS_CHUNK_SIZE;
        let slot_idx = idx % BINDLESS_CHUNK_SIZE;

        if let Some(ref mut chunk) = self.chunks[chunk_idx] {
            if chunk[slot_idx].is_valid {
                chunk[slot_idx] = DynamicResourceDescriptor::null();
                self.allocated_count = self.allocated_count.saturating_sub(1);
                if (index as u32) < self.next_free_hint {
                    self.next_free_hint = index;
                }
                return Ok(());
            }
        }
        Err(BindlessTableError::SlotNotAllocated(index))
    }

    // Return count of currently allocated descriptors.
    pub fn allocated_count(&self) -> usize {
        self.allocated_count
    }
}
