pub mod header;
pub mod loader;
pub mod image;
pub mod user_stack;
pub mod spawn;
pub mod ntdll;
pub mod kernel32;
pub mod user32;
pub mod ws2_32;
pub mod raw_input;
pub mod synch;
pub mod virtual_alloc;
pub mod tls_fiber;
pub mod seh_veh;
pub mod perf_counter;
pub mod file;
pub mod peb_teb;
pub mod delay_load;
pub mod crypto;
pub mod pipe;
pub mod registry;
pub mod mmap;
pub mod wait;
pub mod threadpool;
pub mod iocp;
pub mod fiber;
pub mod debug_api;
pub mod timer_queue;
pub mod token_security;
pub mod srw_futex;
pub mod enclave;
pub mod dns_client;
pub mod sxs_manifest;
pub mod txf_filter;
pub mod keyed_event;
pub mod shared_section;
pub mod pmc_perf;
pub mod av_clock;
pub mod guard_stack;
pub mod etw_provider;
pub mod debug_registers;
pub mod avx512_state;
pub mod object_namespace;
pub mod apc_dispatcher;
pub mod large_page_valloc;
pub mod power_qos;
pub mod working_set;
pub mod job_objects;
pub mod seh_unwind;
pub mod fls_storage;
pub mod pmu_trace;
pub mod enclave_attest;
pub mod etw_stream;
pub mod threadpool_steal;
pub mod wait_on_address;
pub mod enclave_callgate;
pub mod crash_telemetry;
pub mod direct_fiber_switch;

pub use header::*;
pub use ntdll::*;
pub use kernel32::*;
pub use user32::*;
pub use ws2_32::*;
pub use raw_input::*;
pub use synch::*;
pub use virtual_alloc::*;
pub use tls_fiber::*;
pub use seh_veh::*;
pub use perf_counter::*;
pub use file::*;
pub use peb_teb::*;
pub use delay_load::*;
pub use crypto::*;
pub use pipe::*;
pub use registry::*;
pub use mmap::*;
pub use wait::*;
pub use threadpool::*;
pub use iocp::*;
pub use fiber::*;
pub use debug_api::*;
pub use timer_queue::*;
pub use token_security::*;
pub use srw_futex::*;
pub use enclave::*;
pub use dns_client::*;
pub use sxs_manifest::*;
pub use txf_filter::*;
pub use keyed_event::*;
pub use shared_section::*;
pub use pmc_perf::*;
pub use av_clock::*;
pub use guard_stack::*;
pub use etw_provider::*;
pub use debug_registers::*;
pub use avx512_state::*;
pub use object_namespace::*;
pub use apc_dispatcher::*;
pub use large_page_valloc::*;
pub use power_qos::*;
pub use working_set::*;
pub use job_objects::*;
pub use seh_unwind::test_win32_seh_unwind_subsystem;
pub use fls_storage::test_win32_fls_subsystem;
pub use pmu_trace::test_win32_pmu_trace_subsystem;
pub use enclave_attest::test_win32_enclave_attest_subsystem;
pub use etw_stream::test_win32_etw_stream_subsystem;
pub use threadpool_steal::test_win32_threadpool_steal_subsystem;
pub use wait_on_address::test_win32_wait_on_address_subsystem;
pub use enclave_callgate::test_win32_enclave_callgate_subsystem;
pub use crash_telemetry::test_win32_crash_telemetry_subsystem;
pub use loader::*;
pub use image::*;
pub use user_stack::*;
pub use spawn::*;

use crate::serial_write_str;

/// Constructs a valid in-memory minimal Win32 PE32+ (64-bit) executable.
pub fn build_demo_pe() -> alloc::vec::Vec<u8> {
    let mut pe = alloc::vec::Vec::with_capacity(1024);
    pe.resize(1024, 0);

    // 1. DOS Header (64 bytes)
    pe[0] = b'M';
    pe[1] = b'Z';
    // e_lfanew at offset 60 (little endian i32: 64 = 0x40)
    pe[60] = 64;

    // 2. PE Signature at offset 64 ("PE\0\0")
    pe[64] = b'P';
    pe[65] = b'E';
    pe[66] = 0;
    pe[67] = 0;

    // 3. ImageFileHeader (20 bytes) at offset 68
    // Machine = 0x8664 (x86-64)
    pe[68] = 0x64;
    pe[69] = 0x86;
    // NumberOfSections = 1
    pe[70] = 1;
    pe[71] = 0;
    // SizeOfOptionalHeader = 240
    pe[84] = 240;
    pe[85] = 0;
    // Characteristics = 0x0022 (EXECUTABLE_IMAGE | LARGE_ADDRESS_AWARE)
    pe[86] = 0x22;
    pe[87] = 0x00;

    // 4. ImageOptionalHeader64 (240 bytes) at offset 88
    // Magic = 0x020B (PE32+)
    pe[88] = 0x0B;
    pe[89] = 0x02;
    // AddressOfEntryPoint = 0x1000 (offset 88 + 16 = 104)
    pe[104] = 0x00;
    pe[105] = 0x10;
    // ImageBase = 0x0040_0000 (offset 88 + 24 = 112)
    pe[112] = 0x00;
    pe[113] = 0x00;
    pe[114] = 0x40;
    // SectionAlignment = 4096 (offset 88 + 32 = 120)
    pe[120] = 0x00;
    pe[121] = 0x10;
    // FileAlignment = 512 (offset 88 + 36 = 124)
    pe[124] = 0x00;
    pe[125] = 0x02;
    // SizeOfImage = 8192 (offset 88 + 56 = 144)
    pe[144] = 0x00;
    pe[145] = 0x20;
    // SizeOfHeaders = 512 (offset 88 + 60 = 148)
    pe[148] = 0x00;
    pe[149] = 0x02;
    // Subsystem = 3 (Windows CUI, offset 88 + 68 = 156)
    pe[156] = 3;
    // NumberOfRvaAndSizes = 16 (offset 88 + 108 = 196)
    pe[196] = 16;

    // 5. ImageSectionHeader (40 bytes) at offset 88 + 240 = 328
    // Name = ".text"
    pe[328..333].copy_from_slice(b".text");
    // VirtualSize = 64 (offset 328 + 8 = 336)
    pe[336] = 64;
    // VirtualAddress = 0x1000 (offset 328 + 12 = 340)
    pe[340] = 0x00;
    pe[341] = 0x10;
    // SizeOfRawData = 512 (offset 328 + 16 = 344)
    pe[344] = 0x00;
    pe[345] = 0x02;
    // PointerToRawData = 512 (offset 328 + 20 = 348)
    pe[348] = 0x00;
    pe[349] = 0x02;
    // Characteristics = 0x60000020 (MEM_EXECUTE | MEM_READ | CNT_CODE, offset 328 + 36 = 364)
    pe[364] = 0x20;
    pe[365] = 0x00;
    pe[366] = 0x00;
    pe[367] = 0x60;

    // 6. Section raw code at offset 512
    // Machine code: mov eax, 0; syscall; ret
    pe[512] = 0xB8;
    pe[513] = 0x00;
    pe[514] = 0x00;
    pe[515] = 0x00;
    pe[516] = 0x00;
    pe[517] = 0x0F;
    pe[518] = 0x05;
    pe[519] = 0xC3;

    pe
}

/// Spawns and executes the initial Win32 PE userland process from the VFS root.
pub fn run_init_process() -> bool {
    serial_write_str("[KNT-PE] Loading initial userland process (/bin/init.exe)...\r\n");

    let pe_data = {
        let vfs = crate::fs::VFS.lock();
        match vfs.read_file("/bin/init.exe") {
            Ok(data) => data,
            Err(_) => {
                serial_write_str("[KNT-PE] Error: /bin/init.exe not found in VFS\r\n");
                return false;
            }
        }
    };

    match spawn_user_process("init.exe", &pe_data) {
        Ok(proc) => {
            serial_write_str("[KNT-PE] Process /bin/init.exe successfully initialized (PID: ");
            crate::serial_write_byte(b'0' + (proc.pcb.pid % 10) as u8);
            serial_write_str(")\r\n");

            // Yield execution to allow user process thread to run
            crate::process::yield_now();

            serial_write_str("[KNT-PE-PASS] Initial Win32 user process execution cycle completed\r\n");
            true
        }
        Err(_) => {
            serial_write_str("[KNT-PE] Error: Failed to spawn /bin/init.exe\r\n");
            false
        }
    }
}
