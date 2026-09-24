//! Handheld eGPU Hot-Plug & PCIe Bus Rebalancing Governor.

#![allow(dead_code)]

/// Standard Hot-Plug Controller (SHPC) state machine for handheld eGPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgpuHotplugState {
    /// No external GPU connected to USB4/Oculink port.
    Disconnected,
    /// Cable detected; protocol negotiation and PCIe tunnel training active.
    TunnelNegotiating,
    /// High-speed PCIe Gen4 x4 tunnel established and link trained.
    LinkReadyGen4,
    /// MMIO BARs allocated and eGPU active as primary graphics renderer.
    ActivePrimary,
    /// Sudden physical disconnect detected while active; safety fallback engaged.
    SurpriseRemoval,
}

/// PCIe Link Generation and Lane Width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcieLinkSpeed {
    None,
    Gen3x4,
    Gen4x4,
}

/// Hot-plug eGPU controller tracking connection state and PCIe link negotiation.
#[derive(Debug, Clone)]
pub struct EgpuController {
    pub state: EgpuHotplugState,
    pub link_speed: PcieLinkSpeed,
    pub link_width: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub state_transition_count: u32,
}

impl EgpuController {
    pub const fn new() -> Self {
        Self {
            state: EgpuHotplugState::Disconnected,
            link_speed: PcieLinkSpeed::None,
            link_width: 0,
            vendor_id: 0,
            device_id: 0,
            state_transition_count: 0,
        }
    }

    /// Handles physical cable insertion interrupt event.
    pub fn on_cable_insert(&mut self) -> EgpuHotplugState {
        if self.state == EgpuHotplugState::Disconnected {
            self.state = EgpuHotplugState::TunnelNegotiating;
            self.state_transition_count += 1;
        }
        self.state
    }

    /// Completes PCIe tunnel training and establishes high-speed link.
    pub fn complete_tunnel_training(&mut self, vendor_id: u16, device_id: u16) -> Result<PcieLinkSpeed, &'static str> {
        if self.state != EgpuHotplugState::TunnelNegotiating {
            return Err("Not in tunnel negotiating state");
        }

        self.vendor_id = vendor_id;
        self.device_id = device_id;
        self.link_speed = PcieLinkSpeed::Gen4x4;
        self.link_width = 4;
        self.state = EgpuHotplugState::LinkReadyGen4;
        self.state_transition_count += 1;
        Ok(self.link_speed)
    }

    /// Transitions to active primary renderer once BARs are mapped.
    pub fn mark_active_primary(&mut self) -> Result<(), &'static str> {
        if self.state != EgpuHotplugState::LinkReadyGen4 {
            return Err("Link not ready");
        }
        self.state = EgpuHotplugState::ActivePrimary;
        self.state_transition_count += 1;
        Ok(())
    }

    /// Triggers sudden surprise removal safety sequence.
    pub fn on_surprise_removal(&mut self) -> EgpuHotplugState {
        self.state = EgpuHotplugState::SurpriseRemoval;
        self.link_speed = PcieLinkSpeed::None;
        self.link_width = 0;
        self.state_transition_count += 1;
        self.state
    }

    /// Resets state machine back to Disconnected after teardown.
    pub fn complete_disconnect(&mut self) {
        self.state = EgpuHotplugState::Disconnected;
        self.vendor_id = 0;
        self.device_id = 0;
        self.state_transition_count += 1;
    }
}

/// Unit test verifying eGPU hot-plug controller state machine and link negotiation.
pub fn verify_egpu_state_machine() -> bool {
    let mut egpu = EgpuController::new();
    if egpu.state != EgpuHotplugState::Disconnected {
        return false;
    }

    // 1. Cable insertion -> TunnelNegotiating
    let s1 = egpu.on_cable_insert();
    if s1 != EgpuHotplugState::TunnelNegotiating {
        return false;
    }

    // 2. Tunnel training -> LinkReadyGen4 (e.g. AMD Radeon RX 7800 XT / NVIDIA RTX 4070)
    let res = egpu.complete_tunnel_training(0x10DE, 0x2782);
    if res != Ok(PcieLinkSpeed::Gen4x4) || egpu.state != EgpuHotplugState::LinkReadyGen4 {
        return false;
    }
    if egpu.link_width != 4 {
        return false;
    }

    // 3. Mark active primary
    if egpu.mark_active_primary().is_err() || egpu.state != EgpuHotplugState::ActivePrimary {
        return false;
    }

    // 4. Sudden disconnect -> SurpriseRemoval
    let s_drop = egpu.on_surprise_removal();
    if s_drop != EgpuHotplugState::SurpriseRemoval || egpu.link_speed != PcieLinkSpeed::None {
        return false;
    }

    // 5. Complete teardown
    egpu.complete_disconnect();
    if egpu.state != EgpuHotplugState::Disconnected {
        return false;
    }

    true
}



// 64-bit BAR Rebalancing Allocator for eGPU High-MMIO Apertures.

/// Allocated MMIO BAR aperture range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarAllocation {
    pub bar_index: u8,
    pub base_addr: u64,
    pub size_bytes: u64,
    pub is_64bit: bool,
    pub is_prefetchable: bool,
}

impl BarAllocation {
    pub const fn empty() -> Self {
        Self {
            bar_index: 0,
            base_addr: 0,
            size_bytes: 0,
            is_64bit: false,
            is_prefetchable: false,
        }
    }
}

/// Dynamic 64-bit High MMIO Aperture Allocator above APU Unified Memory.
#[derive(Debug, Clone)]
pub struct BarRebalancingAllocator {
    /// Starting boundary of high 64-bit MMIO window (e.g. 0x0000_3800_0000_0000).
    pub aperture_base: u64,
    /// Total aperture size in bytes (e.g. 256 GB).
    pub aperture_size: u64,
    /// Current bump allocation pointer within aperture.
    pub current_alloc_ptr: u64,
    /// Active BAR allocations.
    pub allocations: [BarAllocation; 6],
    pub allocation_count: usize,
}

impl BarRebalancingAllocator {
    pub const fn new(aperture_base: u64, aperture_size: u64) -> Self {
        Self {
            aperture_base,
            aperture_size,
            current_alloc_ptr: aperture_base,
            allocations: [BarAllocation::empty(); 6],
            allocation_count: 0,
        }
    }

    /// Default high 64-bit aperture starting well above 64GB APU unified RAM.
    pub const fn default_handheld() -> Self {
        // Base at 0x3800_0000_0000 (~56 TB), size 256 GB (0x40_0000_0000)
        Self::new(0x0000_3800_0000_0000, 0x0000_0040_0000_0000)
    }

    /// Allocates an aligned high-MMIO BAR window for eGPU (e.g. Resizable BAR VRAM).
    pub fn allocate_bar(
        &mut self,
        bar_index: u8,
        size_bytes: u64,
        is_prefetchable: bool,
    ) -> Result<BarAllocation, &'static str> {
        if self.allocation_count >= self.allocations.len() {
            return Err("Maximum BAR allocations reached");
        }
        if size_bytes == 0 {
            return Err("Invalid BAR size");
        }

        // Align current_alloc_ptr to requested size (standard PCIe requirement)
        let alignment = size_bytes;
        let aligned_base = (self.current_alloc_ptr + (alignment - 1)) & !(alignment - 1);

        if (aligned_base + size_bytes) > (self.aperture_base + self.aperture_size) {
            return Err("Aperture space exhausted");
        }

        let alloc = BarAllocation {
            bar_index,
            base_addr: aligned_base,
            size_bytes,
            is_64bit: true,
            is_prefetchable,
        };

        self.allocations[self.allocation_count] = alloc;
        self.allocation_count += 1;
        self.current_alloc_ptr = aligned_base + size_bytes;
        Ok(alloc)
    }

    /// Verifies that an allocated address does not collide with APU unified memory (0 .. 64 GB).
    pub fn is_free_of_apu_collision(&self, base_addr: u64, size_bytes: u64) -> bool {
        let apu_max_ram = 0x0000_0010_0000_0000; // 64 GB
        let alloc_end = base_addr + size_bytes;
        base_addr >= apu_max_ram && alloc_end <= (self.aperture_base + self.aperture_size)
    }

    /// Resets all allocations on eGPU detachment.
    pub fn reset(&mut self) {
        self.current_alloc_ptr = self.aperture_base;
        self.allocation_count = 0;
        self.allocations = [BarAllocation::empty(); 6];
    }
}

/// Unit test verifying BAR rebalancing allocator and collision avoidance with APU RAM.
pub fn verify_bar_rebalancing_allocator() -> bool {
    let mut allocator = BarRebalancingAllocator::default_handheld();

    // 1. Allocate 16GB Resizable BAR (VRAM aperture) at BAR 0
    let vram_16gb = 16 * 1024 * 1024 * 1024u64;
    let alloc0 = allocator.allocate_bar(0, vram_16gb, true);
    if alloc0.is_err() {
        return false;
    }
    let bar0 = alloc0.unwrap();
    if bar0.base_addr != allocator.aperture_base || bar0.size_bytes != vram_16gb {
        return false;
    }
    // Must be completely free of APU collision
    if !allocator.is_free_of_apu_collision(bar0.base_addr, bar0.size_bytes) {
        return false;
    }

    // 2. Allocate 64MB Control MMIO register window at BAR 2
    let mmio_64mb = 64 * 1024 * 1024u64;
    let alloc2 = allocator.allocate_bar(2, mmio_64mb, false);
    if alloc2.is_err() {
        return false;
    }
    let bar2 = alloc2.unwrap();
    // Base must be aligned and placed immediately following BAR 0
    if bar2.base_addr != (bar0.base_addr + vram_16gb) || bar2.size_bytes != mmio_64mb {
        return false;
    }

    // 3. Reset check
    allocator.reset();
    if allocator.allocation_count != 0 || allocator.current_alloc_ptr != allocator.aperture_base {
        return false;
    }

    true
}
