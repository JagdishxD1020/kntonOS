# KntonOS

KntonOS is a high-performance bare-metal gaming micro-kernel written in Rust for the x86-64 architecture.

## Overview

KntonOS is engineered from the ground up for low-latency interactive workloads, featuring:
- **Direct Bare-Metal Boot**: Native UEFI 2.x bootloader with GOP display initialization and paging handoff.
- **Micro-Kernel Architecture**: Modular kernel design with decoupled IPC, paging, scheduler, and device drivers.
- **Low-Latency Subsystems**: Sub-millisecond IPC futex primitives, hybrid slab/buddy allocators, and hardware-accelerated sensor processing.
- **Direct Hardware Access**: High-throughput memory-mapped GPU/VRS, PCIe, NVMe, USB 3.x (xHCI), and low-latency audio pipelines.
- **Built-in Diagnostics**: Automated QEMU boot test harness and kernel sentinels.

## Building and Verification

### Prerequisites
- Rust nightly toolchain with `x86_64-unknown-uefi` and `x86_64-unknown-none` targets.
- QEMU (`qemu-system-x86_64`) with OVMF firmware.

### Build
```bash
# Compile UEFI Bootloader
cargo build --manifest-path boot/Cargo.toml --target x86_64-unknown-uefi

# Compile Kernel
cargo build --manifest-path kernel/Cargo.toml --target x86_64-unknown-none
```

### Run Native UEFI Boot Test
```bash
python scripts/boot_runner.py --timeout 120
```
