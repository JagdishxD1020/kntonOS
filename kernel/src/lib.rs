#![no_std]

extern crate alloc;

pub mod arch;
pub mod memory;
pub mod display;
pub mod pe;
pub mod fs;
pub mod process;
pub mod syscall;
pub mod drivers;
pub mod compositor;
pub mod crypto;
pub mod net;
pub mod shell;
pub mod ipc;
pub mod loader;
pub mod graphics;
pub mod gfx;
pub mod terminal;
pub mod power;
pub mod mm;
pub mod mem;
pub mod trace;
pub mod bpf;
pub mod audio;
pub mod time;
pub mod video;
pub mod sys;
pub mod ui;

pub use memory::ALLOCATOR;

// 16550A UART COM1 port
pub const COM1_PORT: u16 = 0x3F8;

#[inline(always)]
pub unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!(
        "in al, dx",
        out("al") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}

#[inline(always)]
pub unsafe fn outl(port: u16, val: u32) {
    core::arch::asm!(
        "out dx, eax",
        in("dx") port,
        in("eax") val,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline(always)]
pub unsafe fn inl(port: u16) -> u32 {
    let val: u32;
    core::arch::asm!(
        "in eax, dx",
        out("eax") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}

pub fn serial_init() {
    unsafe {
        outb(COM1_PORT + 1, 0x00); // Disable interrupts
        outb(COM1_PORT + 3, 0x80); // Enable DLAB (set baud rate divisor)
        outb(COM1_PORT + 0, 0x01); // Set divisor to 1 (115200 baud) lo byte
        outb(COM1_PORT + 1, 0x00); //                               hi byte
        outb(COM1_PORT + 3, 0x03); // 8 bits, no parity, one stop bit
        outb(COM1_PORT + 2, 0xC7); // Enable FIFO, clear them, with 14-byte threshold
        outb(COM1_PORT + 4, 0x0B); // IRQs enabled, RTS/DSR set
    }
}

pub fn serial_write_byte(b: u8) {
    unsafe {
        // Wait for Line Status Register (0x3FD) empty transmitter holding register (bit 5)
        while (inb(COM1_PORT + 5) & 0x20) == 0 {
            core::arch::asm!("pause", options(nomem, nostack, preserves_flags));
        }
        outb(COM1_PORT, b);
    }
}

pub fn serial_write_str(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            serial_write_byte(b'\r');
        }
        serial_write_byte(b);
    }
}

#[no_mangle]
pub extern "C" fn kmain() -> ! {
    serial_init();
    serial_write_str("\r\n[KNT-KERNEL-INIT] KntonOS micro-kernel bootstrap reached\r\n");

    // Initialize low-level CPU infrastructure (GDT, TSS, IDT, Syscall MSRs)
    arch::init();

    // Initialize foundational memory management subsystems (PMM, VMM, Heap)
    memory::init();

    // Execute memory subsystem verification self-test
    if !memory::test_memory_subsystem() {
        serial_write_str("[KNT-PANIC] Memory subsystem self-test failed!\r\n");
    }

    // Initialize and verify Demand Paging & Copy-on-Write (COW) Fork Engine
    if !memory::test_cow_and_demand_paging() {
        serial_write_str("[KNT-PANIC] Demand paging and COW fork self-test failed!\r\n");
    }

    // Initialize and verify Transparent Huge Pages (THP 2MB) & AMD 3D V-Cache NUMA Topology
    if !mm::test_thp_and_numa_subsystem() {
        serial_write_str("[KNT-PANIC] THP and NUMA V-Cache self-test failed!\r\n");
    }

    // Initialize and verify Dynamic VMM Memory Compactor & Transparent Page Defragmentation
    if !mm::test_memory_compaction_subsystem() {
        serial_write_str("[KNT-PANIC] Memory compactor and defragmentation self-test failed!\r\n");
    }

    // Initialize and verify Micro-Kernel Lock-Free Ring Buffer Object Pool & Slab Cache Allocator
    if !mem::test_slab_allocator_subsystem() {
        serial_write_str("[KNT-PANIC] Slab cache and magazine allocator self-test failed!\r\n");
    }

    // Initialize SMP multi-core discovery and Local APIC timer
    arch::init_smp();

    // Verify SMP multi-core topology and Local APIC timer operations
    if !arch::test_smp_and_timer_subsystem() {
        serial_write_str("[KNT-PANIC] SMP and Local APIC timer self-test failed!\r\n");
    }

    // Initialize ACPI Power Management Subsystem (FADT/DSDT S3/S5, CPU Throttling, Reset & Shutdown)
    arch::init_power_management();

    // Verify ACPI Power Management Subsystem
    if !arch::test_power_subsystem() {
        serial_write_str("[KNT-PANIC] ACPI Power Management self-test failed!\r\n");
    }

    // Initialize and verify HPET & Real-Time Monotonic Nanosecond Clock Subsystem
    if !arch::test_hpet_subsystem() {
        serial_write_str("[KNT-PANIC] HPET and monotonic clock self-test failed!\r\n");
    }
    // Initialize and verify High-Resolution Timers (hrtimer) & Red-Black Tree Wheel Engine
    if !time::test_hrtimer_subsystem() {
        serial_write_str("[KNT-PANIC] High-resolution timer and timing wheel self-test failed!\r\n");
    }

    // Initialize and verify MMIO Watchpoint & Kernel Debug Trap Engine
    if !arch::test_debug_trap_subsystem() {
        serial_write_str("[KNT-PANIC] MMIO watchpoint and debug trap self-test failed!\r\n");
    }

    // Initialize and verify Zero-Overhead Kernel Trace Ring Buffer & Performance Profiling
    if !trace::test_trace_subsystem() {
        serial_write_str("[KNT-PANIC] Kernel trace ring buffer self-test failed!\r\n");
    }

    // Initialize and verify Dynamic Instrumentation Probes (kprobes / ftrace Style) & Instruction Detours
    if !trace::test_kprobes_subsystem() {
        serial_write_str("[KNT-PANIC] Dynamic kprobes instrumentation self-test failed!\r\n");
    }

    // Initialize and verify Dynamic Power Governor & Thermal Energy Meter
    if !power::test_power_governor_subsystem() {
        serial_write_str("[KNT-PANIC] Dynamic power governor self-test failed!\r\n");
    }

    // Initialize and verify Dynamic Core Parking, C-State Governor, & EPB/EPP Manager
    if !power::test_core_parking_and_cstate_subsystem() {
        serial_write_str("[KNT-PANIC] Core parking and C-state governor self-test failed!\r\n");
    }

    // Initialize and verify High-Performance S0i3 / S3 Instant-Resume Freeze & Wakeup Controller
    if !power::test_s0i3_s3_suspend_resume_subsystem() {
        serial_write_str("[KNT-PANIC] S0i3/S3 suspend-resume subsystem self-test failed!\r\n");
    }

    // Initialize and verify Handheld Thermal & Fan Profile Controller
    if !power::test_handheld_thermal_and_fan_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld thermal and fan profile subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct-to-Disk Kernel Hibernation (S4 State) & LZ4 Fast-Restore Engine
    if !power::test_power_hibernate_subsystem() {
        serial_write_str("[KNT-PANIC] Kernel hibernation and LZ4 fast-restore self-test failed!\r\n");
    }

    // Initialize and verify Handheld Battery Fuel Gauge, Coulomb Counter & Smart Charging State Machine
    if !power::test_battery_and_charger_subsystem() {
        serial_write_str("[KNT-PANIC] Battery fuel gauge and smart charger self-test failed!\r\n");
    }

    // Initialize and verify Unified Kernel Dynamic Frequency Scaling (DVFS) & Micro-Voltage Stepping Governor
    if !power::test_dvfs_governor_subsystem() {
        serial_write_str("[KNT-PANIC] DVFS governor and voltage stepping self-test failed!\r\n");
    }

    // Initialize and verify Kernel-Direct Hardware Video Decode & Zero-Copy Presentation Streamer
    if !video::test_video_decode_subsystem() {
        serial_write_str("[KNT-PANIC] Hardware video decode and presentation streamer self-test failed!\r\n");
    }

    // Initialize and verify Real-Time In-Game Frame Capture HDR Tonemapper & Ultra-Low-Latency Display Mirroring Streamer (TASK-KNT-127)
    if !video::test_video_hdr_stream_subsystem() {
        serial_write_str("[KNT-PANIC] Video frame capture HDR tonemapper and display mirroring streamer self-test failed!\r\n");
    }

    // Initialize Virtual File System & mount demo initramfs
    fs::init();
    if !fs::test_vfs_subsystem() {
        serial_write_str("[KNT-PANIC] VFS subsystem self-test failed!\r\n");
    }

    // Initialize Process & Thread Scheduler subsystem
    process::init();
    if !process::test_process_subsystem() {
        serial_write_str("[KNT-PANIC] Process subsystem self-test failed!\r\n");
    }

    // Initialize Userland Syscall Dispatch Engine
    syscall::init();
    if !syscall::test_syscall_subsystem() {
        serial_write_str("[KNT-PANIC] Syscall subsystem self-test failed!\r\n");
    }

    // Verify POSIX Signal Handling & Process Exception Dispatch Engine
    if !process::test_signals_subsystem() {
        serial_write_str("[KNT-PANIC] POSIX signals subsystem self-test failed!\r\n");
    }

    // Initialize and verify State-Externalized Dual-Ring Resilient Partitions & Crash Recovery Engine
    if !process::test_resilience_subsystem() {
        serial_write_str("[KNT-PANIC] Resilient partitions & crash recovery self-test failed!\r\n");
    }

    // Initialize Inter-Process Communication (IPC) Shared Memory Rings & Fast Futex Engine
    ipc::init();
    if !ipc::test_ipc_subsystem() {
        serial_write_str("[KNT-PANIC] IPC subsystem self-test failed!\r\n");
    }

    // Initialize VirtIO Driver Subsystem (PCI discovery, split virtqueue, block device)
    drivers::virtio::init();
    if !drivers::virtio::test_virtio_subsystem() {
        serial_write_str("[KNT-PANIC] VirtIO subsystem self-test failed!\r\n");
    }

    // Initialize and verify FAT32 Persistent File System Driver & VirtIO Block Mount
    if !fs::test_fat32_subsystem() {
        serial_write_str("[KNT-PANIC] FAT32 subsystem self-test failed!\r\n");
    }

    // Verify VFS Unified Inode Cache, Lock Manager, & ACL Access Controls
    if !fs::test_vfs_acl_and_cache() {
        serial_write_str("[KNT-PANIC] VFS ACL and Inode cache self-test failed!\r\n");
    }

    // Initialize PCI Device Subsystem & USB xHCI Host Controller Discovery
    drivers::init_pci();
    if !drivers::test_pci_subsystem() {
        serial_write_str("[KNT-PANIC] PCI subsystem self-test failed!\r\n");
    }

    // Initialize and verify VirtIO GPU 2D/3D Hardware Acceleration & Display Scanout Driver
    if !drivers::test_gpu_subsystem() {
        serial_write_str("[KNT-PANIC] VirtIO GPU subsystem self-test failed!\r\n");
    }

    // Initialize and verify Software 3D Graphics Pipeline & Barycentric Triangle Rasterizer
    if !graphics::test_graphics_subsystem() {
        serial_write_str("[KNT-PANIC] Software 3D graphics subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct-to-Display Scanout & Atomic DRM/KMS Modesetting Presentation Layer
    if !drivers::gpu::test_drm_kms_subsystem() {
        serial_write_str("[KNT-PANIC] DRM/KMS modesetting subsystem self-test failed!\r\n");
    }

    // Initialize and verify Hardware Variable Refresh Rate (VRR) & Dynamic VBLANK Presentation Timing
    if !drivers::gpu::test_vrr_subsystem() {
        serial_write_str("[KNT-PANIC] Hardware VRR and dynamic timing self-test failed!\r\n");
    }

    // Initialize and verify Native Hardware Display Scanout Rotation & Panel Orientation Subsystem
    if !drivers::gpu::test_display_rotation_and_panel_subsystem() {
        serial_write_str("[KNT-PANIC] Display scanout rotation and panel orientation self-test failed!\r\n");
    }

    // Initialize and verify PCIe Function-Level Reset (FLR) & GPU Timeout Detection and Recovery (TDR)
    if !drivers::gpu::test_pcie_flr_and_gpu_tdr_subsystem() {
        serial_write_str("[KNT-PANIC] PCIe FLR and GPU TDR subsystem self-test failed!\r\n");
    }

    // Initialize and verify PCIe Native Hot-Plug & External eGPU Hot-Connect Controller
    if !drivers::pci::test_pci_hotplug_subsystem() {
        serial_write_str("[KNT-PANIC] PCIe hot-plug and external eGPU self-test failed!\r\n");
    }

    // Initialize and verify Direct3D / Vulkan-Style Command Buffer Recording & Hardware Queue Submission Engine
    if !gfx::test_gpu_command_buffer_and_queue_subsystem() {
        serial_write_str("[KNT-PANIC] GPU command buffer and queue submission self-test failed!\r\n");
    }

    // Initialize and verify Zero-Copy GPU Asynchronous Compute Dispatch & Shader Execution Fence Engine
    if !gfx::test_async_compute_subsystem() {
        serial_write_str("[KNT-PANIC] Asynchronous compute and GPU timeline fence self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Dynamic Constant Buffer Pool & Ring Allocator Engine
    if !gfx::test_constant_buffer_pool_subsystem() {
        serial_write_str("[KNT-PANIC] Constant buffer pool and ring allocator self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Raytracing Acceleration Structure (BVH) Memory Manager & DispatchRays Engine
    if !gfx::test_dxr_acceleration_structure_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 raytracing acceleration structure self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Variable Rate Shading (VRS Tier 2) & Eye-Tracked Foveated Rasterizer
    if !gfx::test_vrs_foveated_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 VRS and foveated rasterizer self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Mesh Shader & Amplification Shader Primitive Pipeline Engine
    if !gfx::test_mesh_shader_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 mesh shader pipeline self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Mesh Shader & Amplification Shader Pipeline (TASK-KNT-162)
    if !gfx::test_d3d12_mesh_shader_pipeline() {
        serial_write_str("[KNT-PANIC] Direct3D 12 mesh and amplification shader pipeline self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Work Graphs & Autonomous GPU Shader Execution Engine
    if !gfx::test_work_graphs_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 work graphs subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Work Graphs & Autonomous GPU Execution Engine (TASK-KNT-171)
    if !gfx::test_d3d12_work_graphs_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 work graphs autonomous GPU execution engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Sampler Feedback & Dynamic Texture LOD Governor (TASK-KNT-236)
    if !gfx::test_sampler_feedback_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 sampler feedback engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Enhanced Barriers & Split-Barrier Synchronized GPU Transition Engine
    if !gfx::test_enhanced_barriers_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 enhanced barriers subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Shader Execution Reordering (SER) & Raytracing Hit-Sort Accelerator Engine
    if !gfx::test_ser_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 SER hit-sort accelerator subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Shader Execution Reordering (SER) & Hit Object Engine (TASK-KNT-174)
    if !gfx::test_d3d12_ser_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 shader execution reordering engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Command Signature & GPU ExecuteIndirect Engine (TASK-KNT-177)
    if !gfx::test_execute_indirect_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 command signature and execute indirect self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Agility Dynamic Depth Bounds & Stencil Optimizer (TASK-KNT-280)
    if !gfx::test_depth_bounds_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility dynamic depth bounds self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Conservative Rasterization & Pixel Coverage Engine (TASK-KNT-179)
    if !gfx::test_conservative_raster_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 conservative rasterization engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Low-Latency Frame Pacing & Latency Marker Engine
    if !gfx::test_frame_pacing_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 low-latency frame pacing self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Low-Latency Frame Pacing & Dynamic Resolution Scaling (DRS) Governor (TASK-KNT-186)
    if !gfx::test_drs_governor_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 DRS governor self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Dynamic Primitive Culling & Mesh-Cluster LOD Streamer Engine
    if !gfx::test_cluster_cull_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 cluster cull & LOD streamer self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Screen-Space Motion Vectors & Optical Flow Generator (TASK-KNT-202)
    if !gfx::test_optical_flow_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 optical flow generator self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Dynamic Tile Pool Memory Compactor & Sparse Virtual Texture Eviction Engine
    if !gfx::test_tile_pool_compactor_and_eviction_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 tile pool memory compactor & eviction self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Tiled Resources & Sparse Virtual Texture Engine (TASK-KNT-183)
    if !gfx::test_tiled_resources_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 tiled resources engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Render Passes & Subpass Optimization Engine (TASK-KNT-188)
    if !gfx::test_render_pass_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 render pass engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Predication & GPU Occlusion Query Skipping Engine (TASK-KNT-191)
    if !gfx::test_predication_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 predication engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 DXR 1.1 Inline Ray Query & Procedural Intersection Engine (TASK-KNT-199)
    if !gfx::test_ray_query_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 DXR 1.1 inline ray query self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Dynamic Micro-Mesh Displacement & Sub-Triangle Geometry LOD Compressor
    if !gfx::test_micromesh_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 micro-mesh displacement & sub-triangle self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Variable-Rate Shading (VRS) Tier 2 Screen-Space Shading Rate Image Generator
    if !gfx::test_vrs_tier2_shading_rate_image_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 VRS Tier 2 shading rate image generator self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 VRS Tier 2 Shading Rate Image & Combiner Pipeline (TASK-KNT-205)
    if !gfx::test_shading_rate_image_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 VRS Tier 2 shading rate image and combiner self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 VRS Tier 2 Combiners & Per-Draw Rate Multiplier (TASK-KNT-214)
    if !gfx::test_vrs_combiners_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 VRS combiners self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Enhanced Memory Pools & Dedicated VRAM Heap Residency Governor (TASK-KNT-217)
    if !gfx::test_residency_manager_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 residency manager self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Root Signature Version 1.2 & Local Shader Records (TASK-KNT-219)
    if !gfx::test_root_signature_v1_2_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 root signature 1.2 self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Work Graphs Node Allocator & Suballocator (TASK-KNT-223)
    if !gfx::test_work_graphs_allocator_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 work graphs allocator self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 SM 6.8 Extended Wave MMA Engine (TASK-KNT-225)
    if !gfx::test_wave_mma_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 wave MMA self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 GPU-Driven Multi-Draw & Meshlet Culling (TASK-KNT-228)
    if !gfx::test_gpu_multi_draw_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 GPU multi-draw self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Enhanced Barrier Discard & Alias Optimizer (TASK-KNT-232)
    if !gfx::test_barrier_discard_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 barrier discard self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 SM 6.8 Extended Descriptors & Bindless Heap Service (TASK-KNT-239)
    if !gfx::test_bindless_heap_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 bindless heap engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Mesh Shader Primitive Amplification & Payload Distributor (TASK-KNT-241)
    if !gfx::test_mesh_shader_payload_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 mesh shader amplification payload self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Command Signature Suballocation & Indirect Pool (TASK-KNT-243)
    if !gfx::test_command_signature_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility command signature self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Coarse Pixel Shading Engine (TASK-KNT-247)
    if !gfx::test_coarse_pixel_shading_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility coarse pixel shading self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility PSO Binary Library Caching Engine (TASK-KNT-250)
    if !gfx::test_pso_library_cache_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility PSO binary library cache self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Dynamic Depth Bias & Conservative Raster (TASK-KNT-254)
    if !gfx::test_depth_bias_conservative_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility depth bias & conservative raster self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility GPU Hardware Virtual Memory Page Remapping (TASK-KNT-258)
    if !gfx::test_gpu_page_remapping_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility GPU page remapping self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Work Graphs Tier 1.0 (TASK-KNT-261)
    if !gfx::test_work_graphs_tier1_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility work graphs tier 1.0 self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Command Allocator Pool & Multi-Queue Engine (TASK-KNT-265)
    if !gfx::test_command_allocator_pool_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility command allocator pool self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Enhanced Sync Intervals & VRR Frame Pacing (TASK-KNT-268)
    if !gfx::test_sync_intervals_vrr_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility sync intervals & VRR frame pacing self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Dynamic Resources & Bindless Engine (TASK-KNT-271)
    if !gfx::test_dynamic_bindless_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility dynamic resources & bindless engine self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Command List Markers & PIX Subsystem (TASK-KNT-274)
    if !gfx::test_gpu_event_markers_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility execution markers & PIX self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Advanced Color Space & HDR10+ Engine (TASK-KNT-277)
    if !gfx::test_hdr_colorspace_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility HDR color space & tonemapping self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Pipeline Library & Shader Cache Engine (TASK-KNT-284)
    if !gfx::test_pipeline_library_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility pipeline library self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Sampler Feedback & Tessellation (TASK-KNT-288)
    if !gfx::test_sampler_tessellation_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility sampler feedback & tessellation self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility BVH Compaction & Dynamic Refit (TASK-KNT-291)
    if !gfx::test_raytracing_bvh_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility BVH compaction & dynamic refit self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Barrier Batching & Elimination (TASK-KNT-294)
    if !gfx::test_barrier_batcher_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility barrier batching & elimination self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Dynamic Depth/Stencil & Depth Bounds Optimizer (TASK-KNT-297)
    if !gfx::test_depth_bounds_optimizer_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility depth bounds optimizer self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Dynamic Occlusion Query Streamer (TASK-KNT-299)
    if !gfx::test_occlusion_query_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility occlusion query streamer self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Dynamic Viewport & Precision Rasterizer (TASK-KNT-302)
    if !gfx::test_viewport_rasterizer_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility viewport rasterizer self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Dynamic Sampler LOD Bias & Anisotropy Governor (TASK-KNT-304)
    if !gfx::test_sampler_anisotropy_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility dynamic sampler anisotropy self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Dynamic Blend State Hashing & Optimization (TASK-KNT-307)
    if !gfx::test_blend_optimizer_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 agility blend optimizer self-test failed!\r\n");
    }
    // Initialize and verify Direct3D 12 Agility Dynamic Multisample Anti-Aliasing (MSAA) Resolve Governor (TASK-KNT-309)
    gfx::test_msaa_resolve_subsystem();
    // Initialize and verify Handheld Game Mode UI Compositor & Hardware HUD Overlay (TASK-KNT-316)
    if !gfx::test_deck_shell_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld UI compositor and HUD overlay self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Variable Rate Shading (VRS) & Tier 2 Shading Rate Engine (TASK-KNT-159)
    if !gfx::test_vrs_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 variable rate shading engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 User-Mode Driver (UMD) Command Streamer & Swapchain Service (TASK-KNT-139)
    if !gfx::test_swapchain_service_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 UMD command streamer and swapchain service self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Root Signature & Descriptor Heap Binding Table Engine (TASK-KNT-142)
    if !gfx::test_root_signature_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 root signature subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Dynamic Descriptor Heap Suballocator & Ring Buffer (TASK-KNT-208)
    if !gfx::test_descriptor_suballocator_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 descriptor suballocator self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Async Compute & High-Priority Graphics Queue Scheduler (TASK-KNT-211)
    if !gfx::test_d3d12_async_compute_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 async compute scheduler self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Graphics PSO & Shader Bytecode Cache Engine (TASK-KNT-145)
    if !gfx::test_pso_cache_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 graphics PSO cache subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Resource Barrier State Tracker & Implicit Transitions (TASK-KNT-148)
    if !gfx::test_resource_barrier_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 resource barrier state tracker self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Bundle Recording & Fast Pre-Compilation Engine (TASK-KNT-151)
    if !gfx::test_bundle_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 bundle pre-compilation subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Query Heap, GPU Timestamp & Occlusion Engine (TASK-KNT-153)
    if !gfx::test_query_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 query heap subsystem self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 12 Raytracing (DXR) Acceleration Structure & DispatchRays Engine (TASK-KNT-156)
    if !gfx::test_dxr_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 12 raytracing acceleration structure engine self-test failed!\r\n");
    }

    // Initialize and verify Direct3D 11-on-12 (D3D11On12) Compatibility Translation Layer (TASK-KNT-165)
    if !gfx::test_d3d11on12_subsystem() {
        serial_write_str("[KNT-PANIC] Direct3D 11-on-12 compatibility translation layer self-test failed!\r\n");
    }

    // Initialize and verify AHCI SATA Persistent Storage Subsystem
    if !drivers::storage::test_ahci_sata_subsystem() {
        serial_write_str("[KNT-PANIC] AHCI SATA storage subsystem self-test failed!\r\n");
    }

    // Initialize and verify NVMe Storage Controller Subsystem
    if !drivers::storage::test_nvme_subsystem() {
        serial_write_str("[KNT-PANIC] NVMe storage subsystem self-test failed!\r\n");
    }

    // Initialize and verify High-Speed NVMe Overlapped Async I/O (io_uring Style) Completion Engine
    if !drivers::storage::test_nvme_uring_subsystem() {
        serial_write_str("[KNT-PANIC] NVMe overlapped async I/O completion engine self-test failed!\r\n");
    }

    // Initialize and verify Zero-Copy DirectStorage NVMe-to-GPU Streaming & Compressed Texture Pipeline
    if !drivers::storage::test_direct_storage_subsystem() {
        serial_write_str("[KNT-PANIC] DirectStorage NVMe-to-GPU streaming self-test failed!\r\n");
    }

    // Initialize and verify Kernel-Direct NVMe ZNS (Zoned Namespaces) & Copy Offload Engine
    if !drivers::storage::test_nvme_zns_subsystem() {
        serial_write_str("[KNT-PANIC] NVMe ZNS and copy offload self-test failed!\r\n");
    }

    // Initialize and verify Handheld Direct NVMe Host Memory Buffer (HMB) & DirectStorage IO Ring (TASK-KNT-220)
    if !drivers::storage::test_nvme_hmb_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld direct NVMe HMB and DirectStorage IO ring self-test failed!\r\n");
    }

    // Initialize Embedded Network Protocol Stack (ARP, IPv4, ICMP, UDP, TCP)
    net::init();
    if !net::test_network_stack() {
        serial_write_str("[KNT-PANIC] Network stack self-test failed!\r\n");
    }

    // Initialize and verify in-kernel eBPF Packet & Event Filter Subsystem
    if !bpf::test_bpf_filter_subsystem() {
        serial_write_str("[KNT-PANIC] eBPF packet and event filter self-test failed!\r\n");
    }

    // Initialize and verify Multi-Queue Network Packet Transmission & Zero-Copy DMA RX/TX Ring Engine
    if !net::test_net_multiqueue_subsystem() {
        serial_write_str("[KNT-PANIC] Multi-queue network packet engine self-test failed!\r\n");
    }

    // Verify Unix Domain Sockets (AF_UNIX) & Local IPC Engine
    if !net::test_unix_domain_sockets() {
        serial_write_str("[KNT-PANIC] Unix domain sockets self-test failed!\r\n");
    }

    // Verify Low-Latency In-Game Bluetooth LE Gamepad & Wireless Headset Audio Streamer (TASK-KNT-106)
    if !net::test_wireless_ble_subsystem() {
        serial_write_str("[KNT-PANIC] Wireless BLE gamepad and audio streaming self-test failed!\r\n");
    }
    // Initialize and verify Handheld Bluetooth Low Energy (BLE) Gamepad & Low-Latency Audio Transceiver Engine (TASK-KNT-173)
    if !drivers::net::test_ble_gamepad_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld BLE gamepad and LE Audio transceiver self-test failed!\r\n");
    }

    // Initialize and verify Handheld Wi-Fi 6E/7 Low-Latency Streaming & Packet Pacer (TASK-KNT-201)
    if !drivers::net::test_wifi_gaming_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld Wi-Fi 6E/7 streaming and packet pacer self-test failed!\r\n");
    }

    // Initialize and verify Handheld Wi-Fi 7 Multi-Link Operation (MLO) & Deterministic Game Packet Pacer (TASK-KNT-234)
    if !drivers::net::test_wifi_mlo_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld Wi-Fi 7 MLO and game packet pacer self-test failed!\r\n");
    }

    // Initialize and verify Handheld Wi-Fi 7 Punctured Channel Pacing & Ultra-Low-Latency Beacon Synchronization (TASK-KNT-253)
    if !drivers::net::test_wifi_punctured_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld Wi-Fi 7 punctured channel pacing and beacon sync self-test failed!\r\n");
    }

    // Verify Unified Handheld PC Wi-Fi 6E/7 Latency-Optimized Packet Scheduler & Roaming Daemon (TASK-KNT-110)
    if !net::test_wifi_scheduler_and_roaming_subsystem() {
        serial_write_str("[KNT-PANIC] Wi-Fi 6E/7 packet scheduler and roaming daemon self-test failed!\r\n");
    }

    // Verify Unified Game State Overlay Compositor & Zero-Copy Hardware Cursor Plane Subsystem (TASK-KNT-109)
    if !ui::test_game_overlay_and_cursor_subsystem() {
        serial_write_str("[KNT-PANIC] Game state overlay compositor and cursor plane self-test failed!\r\n");
    }

    // Verify Unified Handheld PC Virtual Keyboard & Predictive Swipe IME Gesture Engine (TASK-KNT-112)
    if !ui::test_virtual_keyboard_and_swipe_ime_subsystem() {
        serial_write_str("[KNT-PANIC] Virtual keyboard and swipe IME self-test failed!\r\n");
    }

    // Initialize Desktop Shell Compositor & Window Manager
    compositor::init(320, 240);
    if !compositor::test_compositor_subsystem() {
        serial_write_str("[KNT-PANIC] Compositor subsystem self-test failed!\r\n");
    }

    // Initialize and verify Hardware-Accelerated Frame Capture & Direct DRM Video Streamer Engine
    if !compositor::test_frame_streamer_subsystem() {
        serial_write_str("[KNT-PANIC] Frame capture and video streamer self-test failed!\r\n");
    }

    // Initialize and verify Handheld Gaming Shell, Game Launcher & Performance HUD Overlay (TASK-KNT-136)
    if !compositor::test_game_shell_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld gaming shell self-test failed!\r\n");
    }

    // Initialize Interactive Virtual Terminal (VT) Shell & Key-Event Engine
    shell::init(34, 9);
    if !shell::test_shell_subsystem() {
        serial_write_str("[KNT-PANIC] Shell subsystem self-test failed!\r\n");
    }

    // Verify Virtual Terminal ANSI 24-bit TrueColor Terminal Emulator & PTY Subsystem
    if !terminal::test_pty_and_ansi_subsystem() {
        serial_write_str("[KNT-PANIC] Terminal PTY and TrueColor subsystem self-test failed!\r\n");
    }

    // Initialize and verify USB 3.0 xHCI & HID Keyboard/Mouse Input Pipeline
    if !drivers::usb::test_usb_hid_subsystem() {
        serial_write_str("[KNT-PANIC] USB HID subsystem self-test failed!\r\n");
    }

    // Initialize and verify 6-Axis IMU Sensor Fusion & 1000Hz Gamepad Input Pipeline
    if !drivers::input::test_imu_and_gamepad_subsystem() {
        serial_write_str("[KNT-PANIC] 6-Axis IMU sensor fusion and 1000Hz gamepad self-test failed!\r\n");
    }

    // Initialize and verify Gamepad Haptic / Dual-Rumble Driver & Waveform Synthesizer
    if !drivers::input::test_gamepad_haptic_subsystem() {
        serial_write_str("[KNT-PANIC] Gamepad haptic and rumble self-test failed!\r\n");
    }

    // Initialize and verify Handheld Multi-Zone LRA Haptics & Resonant Frequency Tracking Governor
    if !drivers::input::test_lra_haptics_subsystem() {
        serial_write_str("[KNT-PANIC] Multi-zone LRA haptics self-test failed!\r\n");
    }

    // Initialize and verify Zero-Latency Fast Input Polling Ring & Micro-Jitter Smoothing Filter
    if !drivers::input::test_fast_input_ring_and_jitter_subsystem() {
        serial_write_str("[KNT-PANIC] Fast input ring and micro-jitter smoothing self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC Gyro Aiming & Motion Assist Dynamic Angle Governor
    if !drivers::input::test_gyro_aiming_and_motion_assist_subsystem() {
        serial_write_str("[KNT-PANIC] Gyro aiming and motion assist governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld Gyro-to-Mouse & Dynamic Flick Aiming Translator (TASK-KNT-198)
    if !drivers::input::test_gyro_mouse_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld gyro-to-mouse translator self-test failed!\r\n");
    }

    // Initialize and verify Handheld Sensor Fusion IMU 6-DoF Zero-Drift Kalman Filter & Motion-Assisted Aiming Engine (TASK-KNT-269)
    if !drivers::input::test_motion_aiming_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld sensor fusion IMU 6-DoF Kalman filter and motion aiming self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC Capacitive Touch Controller & Multi-Finger Gesture Engine
    if !drivers::input::test_touch_and_gesture_subsystem() {
        serial_write_str("[KNT-PANIC] Touch and gesture subsystem self-test failed!\r\n");
    }
    // Initialize and verify Handheld Capacitive Touchscreen Multi-Touch & Gesture Engine (TASK-KNT-182)
    if !drivers::input::test_touchscreen_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld capacitive touchscreen and gesture engine self-test failed!\r\n");
    }
    // Initialize and verify Handheld On-Screen Virtual Keyboard & Dual-Trackpad Typing Engine (TASK-KNT-184)
    if !drivers::input::test_virtual_keyboard_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld virtual keyboard and dual-trackpad typing self-test failed!\r\n");
    }



    // Initialize and verify Handheld PC Capacitive Touchpad & Virtual Trackball Emulation Engine
    if !drivers::input::test_touchpad_trackball_subsystem() {
        serial_write_str("[KNT-PANIC] Touchpad and virtual trackball emulation self-test failed!\r\n");
    }
    // Initialize and verify Handheld PC Capacitive Touchpad & Multi-Finger Haptic Gesture Filter Engine (TASK-KNT-128)
    if !drivers::input::test_touchpad_gesture_filter_subsystem() {
        serial_write_str("[KNT-PANIC] Capacitive touchpad and multi-finger gesture filter self-test failed!\r\n");
    }

    // Initialize and verify Handheld Dual Trackpad Haptic Surface & Mouse Emulation Engine (TASK-KNT-195)
    if !drivers::input::test_trackpad_surface_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld dual trackpad surface self-test failed!\r\n");
    }
    // Initialize and verify Handheld PC Hall Effect Magnetic Joystick & Trigger Calibration Engine
    if !drivers::input::test_hall_effect_subsystem() {
        serial_write_str("[KNT-PANIC] Hall effect magnetic joystick and trigger calibration self-test failed!\r\n");
    }
    // Initialize and verify Handheld PC Multi-Zone Haptic Force-Feedback Trigger & Active Resistance Motor Governor (TASK-KNT-125)
    if !drivers::input::test_trigger_motor_subsystem() {
        serial_write_str("[KNT-PANIC] Trigger motor force-feedback governor self-test failed!\r\n");
    }
    // Initialize and verify Handheld PC Fingerprint Sensor & Biometric Windows Hello Token Engine
    if !drivers::input::test_biometric_fingerprint_subsystem() {
        serial_write_str("[KNT-PANIC] Biometric fingerprint and Windows Hello token self-test failed!\r\n");
    }
    // Initialize and verify Userland Game Controller & Win32 XInput Runtime Service (TASK-KNT-135)
    if !drivers::input::test_xinput_service_subsystem() {
        serial_write_str("[KNT-PANIC] Userland game controller and XInput runtime service self-test failed!\r\n");
    }
    // Initialize and verify Handheld PC Haptic Waveform Library & Audio-to-Haptics Transducer Service (TASK-KNT-150)
    if !drivers::input::test_haptic_dsp_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld haptic waveform and audio transducer engine self-test failed!\r\n");
    }
    // Initialize and verify Handheld PC Gyroscope & Motion-Assisted Aiming Translation Subsystem (TASK-KNT-154)
    if !drivers::input::test_gyro_motion_translation_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld gyroscope and motion-assisted aiming engine self-test failed!\r\n");
    }

    // Initialize and verify Handheld Controller Profile Configurator & Remapping Daemon (TASK-KNT-164)
    if !drivers::input::test_controller_profile_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld controller profile and remapping daemon self-test failed!\r\n");
    }

    // Initialize and verify Handheld Sub-Millisecond Input Aggregator & High-Poll-Rate HID Pacer (TASK-KNT-215)
    if !drivers::input::test_input_pacer_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld input aggregator and high-poll-rate HID pacer self-test failed!\r\n");
    }

    // Initialize and verify Handheld Gamepad Dynamic Button Remapping, Macro Sequencer & Virtual Radial Menu (TASK-KNT-218)
    if !drivers::input::test_radial_macro_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld dynamic remapping, macro sequencer and radial menu self-test failed!\r\n");
    }

    // Initialize and verify Handheld Direct Haptic Waveform Synthesizer & VCA Force Profiler (TASK-KNT-231)
    if !drivers::input::test_haptic_synth_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld haptic waveform synthesizer and VCA force profiler self-test failed!\r\n");
    }

    // Initialize and verify Handheld Gyroscope-Assisted Predictive Touchpad Smoothing & Anti-Jitter Filter (TASK-KNT-246)
    if !drivers::input::test_touchpad_jitter_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld touchpad smoothing and anti-jitter self-test failed!\r\n");
    }

    // Initialize and verify Handheld Ultra-Low-Latency Bluetooth 5.4 LE Audio & Dual-Controller Low-Jitter Input Multiplexer (TASK-KNT-264)
    if !drivers::input::test_ble_audio_controller_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld BLE 5.4 LE Audio and dual-controller input multiplexer self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC Multi-Zone RGB Lighting Controller & Aura Sync Engine
    if !drivers::led::test_rgb_lighting_subsystem() {
        serial_write_str("[KNT-PANIC] RGB lighting and Aura Sync subsystem self-test failed!\r\n");
    }

    // Initialize and verify Handheld Ambient Light Sensor (ALS) & HDR Auto-Brightness Governor
    if !drivers::sensor::test_als_and_brightness_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld ALS and auto-brightness governor self-test failed!\r\n");
    }
    // Initialize and verify Handheld Ambient Light Sensor (ALS) Dynamic Screen Dimming & Color Governor (TASK-KNT-170)
    if !drivers::sensor::test_als_governor_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld ALS dynamic screen dimming and color governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC Dual-IMU Dead Reckoning & Motion Drift Eliminator (TASK-KNT-113)
    if !drivers::sensor::test_dual_imu_dead_reckoning_subsystem() {
        serial_write_str("[KNT-PANIC] Dual-IMU dead reckoning subsystem self-test failed!\r\n");
    }

    // Initialize and verify Handheld IMU 6-DoF Sensor Fusion & Thermal Drift Governor (TASK-KNT-192)
    if !drivers::sensor::test_imu_fusion_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld IMU 6-DoF sensor fusion and thermal drift governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld Precision Gyroscope Calibration & Complementary Orientation Fusion (TASK-KNT-224)
    if !drivers::sensor::test_gyro_calibrator_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld gyro calibrator and complementary orientation fusion self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC Backlight PWM Modulator & Flicker-Free DC Dimming Governor
    if !drivers::display::test_backlight_pwm_subsystem() {
        serial_write_str("[KNT-PANIC] Backlight PWM and DC dimming governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC USB-C DisplayPort Alternate Mode & Hot-Plug Modeset Daemon
    if !drivers::display::test_dp_alt_mode_subsystem() {
        serial_write_str("[KNT-PANIC] USB-C DisplayPort Alternate Mode and hot-plug daemon self-test failed!\r\n");
    }

    // Initialize and verify Handheld Display OLED Variable Refresh Rate (VRR) & LFC Frame Pacer Service (TASK-KNT-168)
    if !display::test_vrr_pacer_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld OLED VRR and LFC frame pacer self-test failed!\r\n");
    }
    // Initialize and verify Handheld USB-C Docking Station & DisplayPort MST Manager (TASK-KNT-181)
    if !display::test_dock_manager_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld USB-C docking station and DP MST manager self-test failed!\r\n");
    }

    // Initialize and verify Handheld Dynamic Display Panel Overdrive & Motion Blur Reduction Strobe Governor (TASK-KNT-282)
    if !display::test_panel_overdrive_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld dynamic display panel overdrive and strobe governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld Display DSC & Adaptive Refresh Rate (VRR) Fast-Sync Engine (TASK-KNT-238)
    if !display::test_dsc_vrr_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld display DSC compression and VRR fast-sync self-test failed!\r\n");
    }

    // Initialize and verify Handheld USB-C DisplayPort Alternate Mode & eDP Hot-Plug Re-Clocking Governor (TASK-KNT-257)
    if !display::test_dp_alt_mode_governor_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld USB-C DP Alt Mode and eDP hot-plug governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld Dynamic Display Refresh Rate Switching (DRRS) & Seamless Pixel Clock Re-PLL Engine (TASK-KNT-287)
    if !display::test_drrs_clock_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld dynamic display refresh rate switching and seamless clock re-PLL self-test failed!\r\n");
    }



    // Initialize and verify Handheld PC Dual-Battery Balance Manager & Fast USB-PD 3.1 EPR Engine (TASK-KNT-116)
    if !drivers::power::test_dual_battery_balance_subsystem() {
        serial_write_str("[KNT-PANIC] Dual-battery balance manager and USB-PD 3.1 EPR engine self-test failed!\r\n");
    }
    // Initialize and verify Handheld Battery Gauge Fuel Coulomb Counter & Dynamic Charge Limiter Service (TASK-KNT-172)
    if !drivers::power::test_battery_gauge_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld battery fuel gauge and dynamic charge limiter self-test failed!\r\n");
    }
    // Initialize and verify Handheld Fanless Whisper Mode & Acoustic Governor (TASK-KNT-176)
    if !drivers::power::test_whisper_mode_governor() {
        serial_write_str("[KNT-PANIC] Handheld fanless whisper mode and acoustic governor self-test failed!\r\n");
    }
    // Initialize and verify Handheld Instant Suspend & Low-Power Fast Resume Engine (TASK-KNT-178)
    if !drivers::power::test_quick_suspend_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld instant suspend and low-power fast resume self-test failed!\r\n");
    }

    // Initialize and verify Handheld RGB Lighting & Per-Game Aura Controller (TASK-KNT-187)
    if !drivers::power::test_rgb_aura_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld RGB lighting and per-game aura controller self-test failed!\r\n");
    }

    // Initialize and verify Handheld Power Supply (PD 3.1) Passthrough Charging & Thermal Bypass Governor (TASK-KNT-209)
    if !drivers::power::test_bypass_charging_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld PD 3.1 passthrough charging and thermal bypass governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld Real-Time Battery Discharge Curve Profiler & Micro-Watt Coulomb Counter (TASK-KNT-222)
    if !drivers::power::test_coulomb_counter_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld coulomb counter and battery discharge profiler self-test failed!\r\n");
    }




    // Initialize and verify Handheld Active Fan & Liquid-Metal Cooling Pump Governor (TASK-KNT-242)
    if !drivers::power::test_cooling_pump_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld cooling pump governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld Dual-Battery Hot-Swap Power Balancing & Charge Current Multiplexer (TASK-KNT-251)
    if !drivers::power::test_dual_battery_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld dual-battery hot-swap power balancer self-test failed!\r\n");
    }

    // Initialize and verify Handheld USB-PD 3.1 Extended Power Range (EPR 240W) Fast Charge & Real-Time Impedance Tracker (TASK-KNT-262)
    if !drivers::power::test_power_usb_pd_epr_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld USB-PD 3.1 EPR 240W and impedance tracker self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC Fan & Thermal Governor Service (TASK-KNT-160)
    if !drivers::power::test_thermal_governor_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld thermal governor and dynamic fan curve engine self-test failed!\r\n");
    }

    // Initialize and verify Handheld Thermal Chamber Calibrated Fan Tachometer & Sub-Acoustic Resonance Anti-Vibration Governor (TASK-KNT-279)
    if !drivers::power::test_fan_resonance_subsystem() {
        serial_write_str("[KNT-PANIC] Fan tachometer and anti-resonance governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld Power & Battery: Dynamic Fast Charge Dual-Cell Balance & Impedance Degradation Compensator (TASK-KNT-296)
    if !drivers::power::test_fast_charge_balance_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld dynamic fast charge dual-cell balance self-test failed!\r\n");
    }

    // Initialize and verify Handheld Peltier Active Cooling & Heat Pipe Phase-Change Vapor Chamber Governor (TASK-KNT-311)
    if !drivers::power::test_vapor_chamber_subsystem() {
        serial_write_str("[KNT-PANIC] Peltier cooling and vapor chamber governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC Multi-Zone Thermocouple & Peltier Active Cooler Module (TASK-KNT-119)
    if !drivers::thermal::test_peltier_active_cooler_subsystem() {
        serial_write_str("[KNT-PANIC] Peltier active cooler and thermocouple subsystem self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC USB4/Thunderbolt 4 Dynamic PCIe Tunneling & Hot-Plug eGPU Governor (TASK-KNT-122)
    if !drivers::bus::test_usb4_egpu_governor_subsystem() {
        serial_write_str("[KNT-PANIC] USB4 PCIe tunneling and eGPU governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld eGPU Hot-Plug & PCIe Bus Rebalancing Governor (TASK-KNT-206)
    if !drivers::bus::test_egpu_manager_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld eGPU hot-plug and PCIe rebalancing governor self-test failed!\r\n");
    }

    // Initialize and verify Handheld PCIe 5.0 Link Health Monitor & Dynamic Lane Width / Speed Re-Tuning Engine (TASK-KNT-272)
    if !drivers::bus::test_pcie_health_subsystem() {
        serial_write_str("[KNT-PANIC] PCIe 5.0 link health and dynamic retuning self-test failed!\r\n");
    }

    // Initialize and verify Handheld USB-C Billboard Device & DisplayPort Alternate Mode Firmware Handshake Driver (TASK-KNT-276)
    if !drivers::bus::test_usb_billboard_subsystem() {
        serial_write_str("[KNT-PANIC] USB-C Billboard and DP Alt Mode driver self-test failed!\r\n");
    }

    // Initialize and verify Handheld USB4 / Thunderbolt 4 PCIe Tunneling & Dynamic DMA Hot-Plug Arbiter (TASK-KNT-305)
    if !drivers::bus::test_usb4_pcie_tunnel_subsystem() {
        serial_write_str("[KNT-PANIC] USB4 PCIe tunneling and DMA hot-plug arbiter self-test failed!\r\n");
    }

    // Initialize and verify USB Mass Storage Class (MSC) Bulk-Only Transport (BOT) Driver
    if !drivers::usb::test_usb_msc_subsystem() {
        serial_write_str("[KNT-PANIC] USB MSC BOT storage subsystem self-test failed!\r\n");
    }

    // Initialize and verify Handheld PC MicroSD Card UHS-I/II Host Controller & Fast Game Storage Driver
    if !drivers::storage::test_sdhci_and_microsd_subsystem() {
        serial_write_str("[KNT-PANIC] MicroSD host controller and storage driver self-test failed!\r\n");
    }

    // Initialize and verify Handheld MicroSD UHS-I/II Game Storage & Auto-Mounter (TASK-KNT-190)
    if !drivers::storage::test_sdcard_storage_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld MicroSD game storage and auto-mounter self-test failed!\r\n");
    }

    // Initialize and verify Handheld High-Speed SD/MicroSD UHS-II SD Express Bus Driver & Zero-Copy DMA Controller (TASK-KNT-245)
    if !drivers::storage::test_sd_express_subsystem() {
        serial_write_str("[KNT-PANIC] SD Express and UHS-II storage subsystem self-test failed!\r\n");
    }

    // Initialize and verify Handheld DirectStorage 1.2 GPU Decompression Ring & NVMe Namespace Multiplexer (TASK-KNT-267)
    if !drivers::storage::test_directstorage_decomp_subsystem() {
        serial_write_str("[KNT-PANIC] DirectStorage 1.2 GPU decompression and NVMe multiplexer self-test failed!\r\n");
    }

    // Initialize and verify Handheld DirectStorage 1.2 GPU Decompression Queue & Hardware Copy Engine Orchestrator (TASK-KNT-300)
    if !drivers::storage::test_gpu_decomp_queue_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld DirectStorage 1.2 GPU decompression queue and copy orchestrator self-test failed!\r\n");
    }

    // Initialize and verify Real-Time Audio Subsystem (Intel HDA / AC97 Codec & PCM Mixer)
    if !drivers::audio::test_audio_subsystem() {
        serial_write_str("[KNT-PANIC] Audio subsystem self-test failed!\r\n");
    }

    // Verify Zero-Latency DMA Audio Ring Buffer & Multi-Stream Real-Time Mixer Engine
    if !drivers::audio::test_realtime_audio_mixer_subsystem() {
        serial_write_str("[KNT-PANIC] Real-time DMA audio mixer subsystem self-test failed!\r\n");
    }

    // Initialize and verify Real-Time 3D Audio Spatializer & Binaural HRTF DSP Pipeline
    if !audio::test_audio_spatializer_subsystem() {
        serial_write_str("[KNT-PANIC] 3D audio spatializer self-test failed!\r\n");
    }

    // Initialize and verify Real-Time Game Audio Equalizer & Dynamic Bass Booster
    if !audio::test_audio_eq_and_bass_subsystem() {
        serial_write_str("[KNT-PANIC] Game audio equalizer and bass booster self-test failed!\r\n");
    }

    // Initialize and verify Zero-Copy Shared-Memory Audio Client Protocol & Real-Time Waveform Mixer Ring
    if !audio::test_audio_client_protocol_subsystem() {
        serial_write_str("[KNT-PANIC] Audio client protocol and mixer ring self-test failed!\r\n");
    }

    // Initialize and verify Real-Time Game Voice Chat & Acoustic Noise Cancellation Filter Engine
    if !audio::test_voice_dsp_subsystem() {
        serial_write_str("[KNT-PANIC] Game voice DSP and noise cancellation self-test failed!\r\n");
    }

    // Initialize and verify Low-Latency Spatial Audio Virtualizer & Ear-Canal Impedance Compensator (TASK-KNT-118)
    if !audio::test_spatial_audio_virtualizer_subsystem() {
        serial_write_str("[KNT-PANIC] Spatial audio virtualizer and ear-canal compensator self-test failed!\r\n");
    }

    // Initialize and verify Real-Time In-Game Audio Dynamic Range Compressor & Limiter (TASK-KNT-124)
    if !audio::test_audio_compressor_subsystem() {
        serial_write_str("[KNT-PANIC] Audio dynamic range compressor and limiter self-test failed!\r\n");
    }

    // Initialize and verify Real-Time In-Game 3D Audio Positional Occlusion & Reflection Engine (TASK-KNT-130)
    if !audio::test_audio_occlusion_subsystem() {
        serial_write_str("[KNT-PANIC] Audio positional occlusion and reflection engine self-test failed!\r\n");
    }

    // Initialize and verify Ultra-Low-Latency Voice Activity Detector (VAD) & Spatial Echo Canceller (TASK-KNT-133)
    if !audio::test_audio_vad_subsystem() {
        serial_write_str("[KNT-PANIC] Audio VAD and acoustic echo canceler self-test failed!\r\n");
    }
    // Initialize and verify Userland XAudio2 Game Audio Service & Voice Graph (TASK-KNT-138)
    if !audio::test_xaudio2_service_subsystem() {
        serial_write_str("[KNT-PANIC] Userland XAudio2 game audio service self-test failed!\r\n");
    }
    // Initialize and verify DirectSound & DirectSound3D Legacy Game Audio API Emulation (TASK-KNT-140)
    if !audio::test_dsound_service_subsystem() {
        serial_write_str("[KNT-PANIC] DirectSound and DirectSound3D legacy audio service self-test failed!\r\n");
    }
    // Initialize and verify Win32 WinMM Legacy WaveOut & MIDI Synthesizer Engine (TASK-KNT-144)
    if !audio::test_winmm_service_subsystem() {
        serial_write_str("[KNT-PANIC] WinMM legacy WaveOut audio and MIDI synthesizer self-test failed!\r\n");
    }
    // Initialize and verify Win32 OpenAL 1.1 3D Spatial Audio & EAX Reverb Engine (TASK-KNT-147)
    if !audio::test_openal_service_subsystem() {
        serial_write_str("[KNT-PANIC] OpenAL 1.1 audio service self-test failed!\r\n");
    }

    // Initialize and verify DirectSound 3D Doppler Shift & Binaural HRTF Spatialization Filter (TASK-KNT-157)
    if !audio::test_hrtf_doppler_subsystem() {
        serial_write_str("[KNT-PANIC] Binaural HRTF spatial audio filter and Doppler engine self-test failed!\r\n");
    }

    // Initialize and verify Handheld Directional Footstep Audio Enhancer & Headphone Amp (TASK-KNT-203)
    if !audio::test_footstep_enhancer_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld footstep audio enhancer self-test failed!\r\n");
    }

    // Initialize and verify Handheld Sub-Millisecond Voice Chat Noise Gate, AEC & Dual-Mic Beamformer (TASK-KNT-227)
    if !audio::test_voice_beamformer_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld voice chat beamformer, noise gate and AEC self-test failed!\r\n");
    }

    // Initialize and verify Handheld Dynamic Variable Rate Audio Resampling & Sub-Millisecond Spatial Panning Engine (TASK-KNT-260)
    if !audio::test_audio_spatial_resampler_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld audio dynamic resampler and spatial panner self-test failed!\r\n");
    }

    // Initialize and verify Handheld Audio DSP Dynamic Range Compressor & Psychoacoustic Bass Booster Engine (TASK-KNT-292)
    if !audio::test_audio_dsp_dynamics_subsystem() {
        serial_write_str("[KNT-PANIC] Handheld audio DSP dynamics compressor and psychoacoustic bass booster self-test failed!\r\n");
    }

    // Initialize and verify Hardware RNG & ChaCha20 Cryptographic Entropy Pool
    crypto::init_kernel_rng();
    if !crypto::test_crypto_rng_subsystem() {
        serial_write_str("[KNT-PANIC] Hardware RNG and ChaCha20 CSPRNG self-test failed!\r\n");
    }

    // Initialize and verify ELF64 Executable Loader Subsystem
    loader::init();
    if !loader::test_loader_subsystem() {
        serial_write_str("[KNT-PANIC] ELF64 loader subsystem self-test failed!\r\n");
    }

    // Milestone 100: Unified Gaming Subsystem Health Matrix & Benchmark Supervisor
    if !sys::test_milestone_100_health_matrix() {
        serial_write_str("[KNT-PANIC] Milestone 100 health matrix and benchmark supervisor self-test failed!\r\n");
    }

    // Verify Kernel-Direct Crash Telemetry Core Dump & Instant Mini-Dump Restorer Engine (TASK-KNT-115)
    if !sys::test_minidump_subsystem() {
        serial_write_str("[KNT-PANIC] Crash telemetry core dump and minidump restorer self-test failed!\r\n");
    }

    // Verify Kernel-Direct Micro-Checkpoint Fast State Snapshot & Instant Resumption Engine (TASK-KNT-121)
    if !sys::test_micro_checkpoint_subsystem() {
        serial_write_str("[KNT-PANIC] Micro-checkpoint state snapshot and instant resumption self-test failed!\r\n");
    }

    // Initialize and verify Native Package Manager (kpkg) Manifest, Merkle Tree CAS & Profile Switcher (TASK-KNT-166)
    if !sys::test_kpkg_subsystem() {
        serial_write_str("[KNT-PANIC] Native kpkg package manager and CAS profile switcher self-test failed!\r\n");
    }

    // Verify Win32 Userland Runtime & NTDLL Emulation Layer (TASK-KNT-134)
    if !pe::test_ntdll_emulation_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 NTDLL emulation layer self-test failed!\r\n");
    }

    // Verify Win32 Kernel32 API & Dynamic Library Symbol Resolution Emulation (TASK-KNT-137)
    if !pe::test_kernel32_emulation_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Kernel32 emulation layer self-test failed!\r\n");
    }

    // Verify Win32 User32 & GDI32 Window Message Loop and Graphics Emulation (TASK-KNT-141)
    if !pe::test_user32_emulation_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 User32 & GDI32 emulation layer self-test failed!\r\n");
    }

    // Verify Win32 WinSock2 Game Multiplayer Network Socket Emulation (TASK-KNT-143)
    if !pe::test_winsock_emulation_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 WinSock2 emulation layer self-test failed!\r\n");
    }

    // Verify Win32 Raw Input High-DPI Mouse & Keyboard Gaming Input Engine (TASK-KNT-146)
    if !pe::test_raw_input_emulation_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Raw Input emulation layer self-test failed!\r\n");
    }

    // Verify Win32 Synchronization Primitives & Waitable Handles Engine (TASK-KNT-149)
    if !pe::test_synch_emulation_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 synchronization primitives self-test failed!\r\n");
    }

    // Verify Win32 Virtual Memory & Dynamic Heap Allocation Subsystem (TASK-KNT-152)
    if !pe::test_virtual_alloc_emulation_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 virtual memory and heap subsystem self-test failed!\r\n");
    }

    // Verify Win32 Thread Local Storage (TLS) & Fiber Cooperative Multitasking Subsystem (TASK-KNT-155)
    if !pe::test_tls_fiber_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 TLS and fiber emulation layer self-test failed!\r\n");
    }

    // Verify Win32 Structured Exception Handling (SEH) & Vectored Exception Handling (VEH) Subsystem (TASK-KNT-158)
    if !pe::test_seh_veh_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 SEH and VEH exception handling engine self-test failed!\r\n");
    }

    // Verify Win32 Performance Counters & High-Resolution QueryPerformanceCounter Engine (TASK-KNT-161)
    if !pe::test_perf_counter_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 high-resolution performance counter engine self-test failed!\r\n");
    }

    // Verify Win32 File I/O & VFS DOS Path Translation Subsystem (TASK-KNT-163)
    if !pe::test_win32_file_io_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 file I/O and VFS path translation self-test failed!\r\n");
    }

    // Verify Win32 PEB, TEB and Environment Variable Subsystem (TASK-KNT-167)
    if !pe::test_peb_teb_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 PEB, TEB and environment variable subsystem self-test failed!\r\n");
    }

    // Verify Win32 DLL Delay-Load & API Set Forwarder Engine (TASK-KNT-169)
    if !pe::test_delay_load_and_forwarder_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 DLL delay-load and forwarder engine self-test failed!\r\n");
    }

    // Verify Win32 Cryptographic Primitives & Game Anti-Cheat Entropy Provider (TASK-KNT-175)
    if !pe::test_win32_crypto_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 crypto subsystem self-test failed!\r\n");
    }

    // Verify Win32 Named Pipes & Anonymous IPC Engine (TASK-KNT-180)
    if !pe::test_win32_pipe_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 named pipes and anonymous IPC engine self-test failed!\r\n");
    }

    // Verify Win32 Windows Registry Subsystem & Game Configuration Hive (TASK-KNT-185)
    if !pe::test_win32_registry_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Windows registry subsystem self-test failed!\r\n");
    }

    // Verify Win32 Memory-Mapped Files & File Mapping Shared Cache Subsystem (TASK-KNT-189)
    unsafe {
        if !pe::test_win32_mmap_subsystem() {
            serial_write_str("[KNT-PANIC] Win32 memory-mapped files and shared cache self-test failed!\r\n");
        }
    }

    // Verify Win32 Waitable Objects & Multi-Thread Synchronization Dispatcher (TASK-KNT-196)
    unsafe {
        if !pe::test_win32_wait_subsystem() {
            serial_write_str("[KNT-PANIC] Win32 waitable objects and multi-thread sync dispatcher self-test failed!\r\n");
        }
    }

    // Verify Win32 Threadpool API & Parallel Game Job Scheduler (TASK-KNT-204)
    if !pe::test_win32_threadpool_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 threadpool and job scheduler self-test failed!\r\n");
    }

    // Verify Win32 I/O Completion Ports (IOCP) & Async Packet Dispatcher (TASK-KNT-207)
    if !pe::test_win32_iocp_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 IOCP and async packet dispatcher self-test failed!\r\n");
    }

    // Verify Win32 Fibers & Lightweight User-Mode Scheduling Engine (TASK-KNT-210)
    if !pe::test_win32_fibers_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 fibers and lightweight UMS engine self-test failed!\r\n");
    }

    // Verify Win32 Debugging & Exception Instrumentation API Subsystem (TASK-KNT-213)
    if !pe::test_win32_debug_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 debugging and exception instrumentation self-test failed!\r\n");
    }

    // Verify Win32 Process & Thread Power / Thermal QoS Policy Engine (TASK-KNT-270)
    if !pe::test_win32_power_qos_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Power & Thermal QoS self-test failed!\r\n");
    }
    // Verify Win32 Working Set Trimming, Watch & Fault Clustering Engine (TASK-KNT-273)
    if !pe::test_win32_working_set_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Working Set subsystem self-test failed!\r\n");
    }
    // Verify Win32 Process & Thread Job Objects & CPU Rate Limiting Engine (TASK-KNT-275)
    if !pe::test_win32_job_objects_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Job Objects subsystem self-test failed!\r\n");
    }
    // Verify Win32 SEH Vector Unwinding & Anti-Debugging Trap Virtualization (TASK-KNT-278)
    if !pe::test_win32_seh_unwind_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 SEH Unwind subsystem self-test failed!\r\n");
    }
    // Verify Win32 Fiber Local Storage & Callback Destruction Governor (TASK-KNT-281)
    if !pe::test_win32_fls_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 FLS subsystem self-test failed!\r\n");
    }
    // Verify Win32 Kernel Hardware PMU & Processor Execution Trace (TASK-KNT-283)
    if !pe::test_win32_pmu_trace_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 PMU Trace subsystem self-test failed!\r\n");
    }
    // Verify Win32 Enclave Dynamic Page Revocation & Attestation (TASK-KNT-285)
    if !pe::test_win32_enclave_attest_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Enclave Attestation subsystem self-test failed!\r\n");
    }
    // Verify Win32 ETW Stream Real-Time Compression & Ring Buffer (TASK-KNT-286)
    if !pe::test_win32_etw_stream_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 ETW Stream subsystem self-test failed!\r\n");
    }
    // Verify Win32 Thread Pool Game Worker Queues & Work-Stealing Scheduler (TASK-KNT-289)
    if !pe::test_win32_threadpool_steal_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Thread Pool Steal subsystem self-test failed!\r\n");
    }
    // Verify Win32 WaitOnAddress & Lightweight User-Mode Synchronization Governor (TASK-KNT-290)
    if !pe::test_win32_wait_on_address_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 WaitOnAddress subsystem self-test failed!\r\n");
    }
    // Verify Win32 Enclave Dynamic Call Gate & Exception Handler (TASK-KNT-293)
    if !pe::test_win32_enclave_callgate_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Enclave Call Gate subsystem self-test failed!\r\n");
    }
    // Verify Win32 Dynamic Exception Routing & Minidump Crash Telemetry Ring Buffer (TASK-KNT-295)
    if !pe::test_win32_crash_telemetry_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Crash Telemetry subsystem self-test failed!\r\n");
    }
    // Verify Win32 Direct Fiber Context Switch & State Machine (TASK-KNT-298)
    if !pe::test_win32_direct_fiber_switch_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Direct Fiber Switch subsystem self-test failed!\r\n");
    }
    // Verify Win32 Working Set Page Trimming, Page Replacement & Cold Eviction (TASK-KNT-301)
    if !pe::test_win32_cold_page_evict_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Cold Page Eviction subsystem self-test failed!\r\n");
    }
    // Verify Win32 Page Fault Clustering & Heuristic Prefetch Engine (TASK-KNT-303)
    if !pe::test_win32_prefault_cluster_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Page Fault Clustering subsystem self-test failed!\r\n");
    }
    // Verify Win32 Large Page Virtual Free & Region Compactor (TASK-KNT-306)
    if !pe::test_win32_large_page_free_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Large Page Virtual Free subsystem self-test failed!\r\n");
    }
    // Verify Win32 PMU Telemetry Sampling & Anomaly Detector (TASK-KNT-308)
    if !pe::test_win32_pmu_sampler_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 PMU Sampler subsystem self-test failed!\r\n");
    }
    // Verify Win32 SafeSEH Validation & Stack Pivoting Guard (TASK-KNT-310)
    if !pe::test_win32_safe_seh_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 SafeSEH Guard subsystem self-test failed!\r\n");
    }
    // Verify Win32 CFI Forward-Edge Target Verifier & Vector Unwinder (TASK-KNT-313)
    if !pe::test_win32_cfi_unwind_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 CFI Unwind subsystem self-test failed!\r\n");
    }
    // Verify User-Mode Runtime & High-Performance Syscall Callgate (TASK-KNT-314)
    if !pe::test_win32_user_runtime_subsystem() {
        serial_write_str("[KNT-PANIC] User-mode runtime subsystem self-test failed!\r\n");
    }

    // Verify kntonOS User-Mode Memory Allocator & Dynamic Syscall Stubs (TASK-KNT-317)
    if knton_os::verify_usermode_allocator_subsystem() {
        serial_write_str("[KNT-OS-ALLOC-PASS] kntonOS user-mode memory allocator and dynamic syscall stubs verified\r\n");
    } else {
        serial_write_str("[KNT-PANIC] kntonOS user-mode memory allocator self-test failed!\r\n");
    }

    // Verify Win32 Large Page (2MB/1GB) Allocations & VirtualAlloc2 Engine (TASK-KNT-266)
    if !pe::test_win32_large_page_valloc_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Large Page VirtualAlloc2 self-test failed!\r\n");
    }

    // Verify Win32 Kernel & User-Mode APCs, Alertable Waits & QueueUserAPC Engine (TASK-KNT-263)
    if !pe::test_win32_apc_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 APC subsystem self-test failed!\r\n");
    }

    // Verify Win32 Kernel Object Namespace Directory, Symbolic Link Engine & Object Manager Security (TASK-KNT-259)
    if !pe::test_win32_object_namespace_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Object Namespace Directory self-test failed!\r\n");
    }

    // Verify Win32 Thread Context Switch & AVX-512 / AMX Matrix State Lazy Restore Governor (TASK-KNT-256)
    if !pe::test_win32_avx512_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 AVX-512 lazy state governor self-test failed!\r\n");
    }

    // Verify Win32 Hardware Watchpoint Breakpoint Virtualization & Debug Registers (TASK-KNT-255)
    if !pe::test_win32_debug_registers_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 debug registers subsystem self-test failed!\r\n");
    }

    // Verify Win32 Performance Telemetry Ring Buffer & Real-Time ETW Provider (TASK-KNT-252)
    if !pe::test_win32_etw_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 ETW telemetry subsystem self-test failed!\r\n");
    }

    // Verify Win32 Dynamic Thread Stack Expansion & Guard Page Fault Trap Handler (TASK-KNT-249)
    if !pe::test_win32_guard_stack_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 guard stack subsystem self-test failed!\r\n");
    }

    // Verify Win32 High-Resolution Audio/Video Timeline & Media Foundation Clock Engine (TASK-KNT-248)
    if !pe::test_win32_av_clock_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 AV clock subsystem self-test failed!\r\n");
    }

    // Verify Win32 Hardware Performance Monitor Counters & Low-Overhead Frame Telemetry (TASK-KNT-244)
    if !pe::test_win32_pmc_perf_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 PMC performance subsystem self-test failed!\r\n");
    }

    // Verify Win32 Named Shared Memory Sections, Page Fault Interception & Anti-Cheat Memory Mirror (TASK-KNT-240)
    if !pe::test_win32_shared_section_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 shared section self-test failed!\r\n");
    }

    // Verify Win32 Critical Sections & Keyed Events (TASK-KNT-237)
    if !pe::test_win32_keyed_event_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 Keyed Event self-test failed!\r\n");
    }

    // Verify Win32 Transactional NTFS & Mini-Filter Layer (TASK-KNT-235)
    if !pe::test_win32_txf_filter_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 TxF filter self-test failed!\r\n");
    }

    // Verify Win32 Side-by-Side (SxS) Manifest Activation Engine (TASK-KNT-233)
    if !pe::test_win32_sxs_manifest_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 SxS manifest activation self-test failed!\r\n");
    }

    // Verify Win32 Asynchronous DNS Resolution Service (TASK-KNT-230)
    if !pe::test_win32_dns_client_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 DNS client self-test failed!\r\n");
    }

    // Verify Win32 Enclave Memory Isolation & Anti-Tamper Engine (TASK-KNT-229)
    if !pe::test_win32_enclave_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 enclave isolation self-test failed!\r\n");
    }

    // Verify Win32 SRW Locks, Condition Variables & Futex (TASK-KNT-226)
    if !pe::test_win32_srw_futex_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 SRW locks and futex self-test failed!\r\n");
    }

    // Verify Win32 Access Tokens & Security Descriptors (TASK-KNT-221)
    if !pe::test_win32_token_security_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 token security self-test failed!\r\n");
    }

    // Verify Win32 Timer Queues & Multimedia Synchronization (TASK-KNT-216)
    if !pe::test_win32_timer_queue_subsystem() {
        serial_write_str("[KNT-PANIC] Win32 timer queue self-test failed!\r\n");
    }

    // Verify Milestone 200 Unified Gaming Runtime Supervisor & Health Matrix (TASK-KNT-200)
    if !sys::test_milestone_200_supervisor() {
        serial_write_str("[KNT-PANIC] Milestone 200 supervisor self-test failed!\r\n");
    }

    // Verify Userspace Session Manager (smss) & Core Service Supervisor (TASK-KNT-315)
    if !pe::test_session_manager_subsystem() {
        serial_write_str("[KNT-PANIC] Userspace session manager & core service supervisor self-test failed!\r\n");
    }

    // Verify kntonOS Device Hardware Daemon & Input Event Dispatcher (TASK-KNT-318)
    if !knton_os::service::verify_device_daemon_subsystem() {
        serial_write_str("[KNT-PANIC] Device hardware daemon self-test failed!\r\n");
    } else {
        serial_write_str("[KNT-OS-DEVICE-DAEMON-PASS] kntonOS device hardware daemon and input event dispatcher verified\r\n");
    }

    // Verify kntonOS Userspace Power Management & Thermal Profile Daemon (TASK-KNT-321)
    if !knton_os::service::verify_power_daemon_subsystem() {
        serial_write_str("[KNT-PANIC] Power and thermal daemon self-test failed!\r\n");
    } else {
        serial_write_str("[KNT-OS-PWR-PASS] kntonOS power management & thermal daemon verified\r\n");
    }

    // Verify kntonOS Userland IPC Port Service & Message Dispatcher (TASK-KNT-320)
    if knton_os::ipc::verify_ipc_dispatcher_subsystem() {
        serial_write_str("[KNT-OS-IPC-PASS] kntonOS userland IPC port service and message dispatcher verified\r\n");
    } else {
        serial_write_str("[KNT-PANIC] kntonOS userland IPC dispatcher self-test failed!\r\n");
    }

    // Verify kntonOS Userspace Audio & Acoustic Governor Service (TASK-KNT-323)
    if !knton_os::service::verify_audio_daemon_subsystem() {
        serial_write_str("[KNT-PANIC] Audio and acoustic daemon self-test failed!\r\n");
    } else {
        serial_write_str("[KNT-OS-AUDIO-PASS] kntonOS audio and acoustic governor service verified\r\n");
    }

    // Verify kntonOS Userspace Display Compositor & Direct Scanout Manager (TASK-KNT-324)
    if knton_os::service::verify_display_daemon_subsystem() {
        serial_write_str("[KNT-OS-DISPLAY-PASS] kntonOS userspace display compositor and direct scanout manager verified\r\n");
    } else {
        serial_write_str("[KNT-PANIC] kntonOS display compositor self-test failed!\r\n");
    }

    // Verify kntonOS SteamDeck Gamepad Mapper & Virtual Input Driver (TASK-KNT-325)
    if knton_os::service::verify_gamepad_mapper_subsystem() {
        serial_write_str("[KNT-OS-PADMAP-PASS] kntonOS SteamDeck gamepad mapper and virtual input driver verified\r\n");
    } else {
        serial_write_str("[KNT-PANIC] SteamDeck gamepad mapper self-test failed!\r\n");
    }

    // Verify kntonOS Userspace Network Manager & Wi-Fi Station Daemon (TASK-KNT-326)
    if !knton_os::service::verify_network_daemon_subsystem() {
        serial_write_str("[KNT-PANIC] Network manager daemon self-test failed!\r\n");
    } else {
        serial_write_str("[KNT-OS-NET-PASS] kntonOS userspace network manager & wi-fi daemon verified\r\n");
    }

    // Verify kntonOS SteamDeck Storage & SD Card Automount Daemon (TASK-KNT-327)
    if knton_os::service::verify_storage_daemon_subsystem() {
        serial_write_str("[KNT-OS-STORAGE-PASS] kntonOS SteamDeck storage and sd card automount daemon verified\r\n");
    } else {
        serial_write_str("[KNT-PANIC] Storage automount daemon self-test failed!\r\n");
    }

    // Verify kntonOS Userspace Bluetooth HID & Peripheral Manager (TASK-KNT-328)
    if !knton_os::service::verify_bluetooth_daemon_subsystem() {
        serial_write_str("[KNT-PANIC] Bluetooth peripheral daemon self-test failed!\r\n");
    } else {
        serial_write_str("[KNT-OS-BT-PASS] kntonOS userspace bluetooth peripheral manager verified\r\n");
    }

    // Verify kntonOS Gamescope-Style Performance Overlay & HUD Telemetry Service (TASK-KNT-329)
    if knton_os::service::verify_perf_overlay_subsystem() {
        serial_write_str("[KNT-OS-PERF-PASS] kntonOS gamescope performance overlay & HUD telemetry verified\r\n");
    } else {
        serial_write_str("[KNT-PANIC] Performance overlay daemon self-test failed!\r\n");
    }

    // Verify kntonOS SteamDeck USB-C PD & DisplayPort Alt-Mode Dock Daemon (TASK-KNT-331)
    if !knton_os::service::verify_dock_daemon_subsystem() {
        serial_write_str("[KNT-PANIC] SteamDeck dock daemon self-test failed!\r\n");
    } else {
        serial_write_str("[KNT-OS-DOCK-PASS] kntonOS SteamDeck USB-C PD & DP Alt-Mode dock daemon verified\r\n");
    }

    // Spawn and execute initial Win32 PE64 userland process (/bin/init.exe)
    if !pe::run_init_process() {
        serial_write_str("[KNT-PANIC] Initial Win32 user process execution failed!\r\n");
    }

    serial_write_str("[KNT-SUBSYSTEM] Display & Win32 PE64 loader modules active\r\n");
    serial_write_str("[KNT-BOOT-SUCCESS] Native UEFI-to-kernel boot verified\r\n");

    loop {
        arch::cpu_idle();
    }
}
