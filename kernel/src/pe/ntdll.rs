//! Win32 Userland Runtime & NTDLL Emulation Layer.
//! Provides NT status codes, TEB/PEB userland environments, and system service wrappers.

pub type NtStatus = u32;

pub const STATUS_SUCCESS: NtStatus = 0x0000_0000;
pub const STATUS_TIMEOUT: NtStatus = 0x0000_0102;
pub const STATUS_PENDING: NtStatus = 0x0000_0103;
pub const STATUS_UNSUCCESSFUL: NtStatus = 0xC000_0001;
pub const STATUS_NOT_IMPLEMENTED: NtStatus = 0xC000_0002;
pub const STATUS_INFO_LENGTH_MISMATCH: NtStatus = 0xC000_0004;
pub const STATUS_ACCESS_VIOLATION: NtStatus = 0xC000_0005;
pub const STATUS_INVALID_HANDLE: NtStatus = 0xC000_0008;
pub const STATUS_INVALID_PARAMETER: NtStatus = 0xC000_000D;
pub const STATUS_END_OF_FILE: NtStatus = 0xC000_0011;
pub const STATUS_NO_MEMORY: NtStatus = 0xC000_0017;
pub const STATUS_BUFFER_TOO_SMALL: NtStatus = 0xC000_0023;
pub const STATUS_OBJECT_NAME_NOT_FOUND: NtStatus = 0xC000_0034;
pub const STATUS_OBJECT_NAME_COLLISION: NtStatus = 0xC000_0035;

/// Returns true if status code indicates success.
#[inline(always)]
pub const fn nt_success(status: NtStatus) -> bool {
    status == STATUS_SUCCESS
}

/// Returns true if status code represents an error condition.
#[inline(always)]
pub const fn nt_error(status: NtStatus) -> bool {
    (status & 0xC000_0000) == 0xC000_0000
}

/// Translates an NT status code into its canonical symbolic name.
pub const fn nt_status_to_str(status: NtStatus) -> &'static str {
    match status {
        STATUS_SUCCESS => "STATUS_SUCCESS",
        STATUS_TIMEOUT => "STATUS_TIMEOUT",
        STATUS_PENDING => "STATUS_PENDING",
        STATUS_UNSUCCESSFUL => "STATUS_UNSUCCESSFUL",
        STATUS_NOT_IMPLEMENTED => "STATUS_NOT_IMPLEMENTED",
        STATUS_INFO_LENGTH_MISMATCH => "STATUS_INFO_LENGTH_MISMATCH",
        STATUS_ACCESS_VIOLATION => "STATUS_ACCESS_VIOLATION",
        STATUS_INVALID_HANDLE => "STATUS_INVALID_HANDLE",
        STATUS_INVALID_PARAMETER => "STATUS_INVALID_PARAMETER",
        STATUS_END_OF_FILE => "STATUS_END_OF_FILE",
        STATUS_NO_MEMORY => "STATUS_NO_MEMORY",
        STATUS_BUFFER_TOO_SMALL => "STATUS_BUFFER_TOO_SMALL",
        STATUS_OBJECT_NAME_NOT_FOUND => "STATUS_OBJECT_NAME_NOT_FOUND",
        STATUS_OBJECT_NAME_COLLISION => "STATUS_OBJECT_NAME_COLLISION",
        _ => "STATUS_UNKNOWN",
    }
}

/// Win32 Client Identifier containing unique process and thread IDs.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientId {
    pub unique_process: u64,
    pub unique_thread: u64,
}

/// Thread Information Block (NT_TIB) header at the base of the TEB.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NtTib {
    pub exception_list: u64,
    pub stack_base: u64,
    pub stack_limit: u64,
    pub subsystem_tib: u64,
    pub fiber_data: u64,
    pub arbitrary_user_pointer: u64,
    pub self_ptr: u64,
}

/// Thread Environment Block (TEB) mapped per userland thread.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct ThreadEnvironmentBlock {
    pub tib: NtTib,
    pub environment_pointer: u64,
    pub client_id: ClientId,
    pub active_rpc_handle: u64,
    pub thread_local_storage_pointer: u64,
    pub process_environment_block: u64,
    pub last_error_value: u32,
    pub count_of_owned_critical_sections: u32,
    pub current_locale: u32,
    pub padding: u32,
    pub tls_slots: [u64; 64],
}

impl ThreadEnvironmentBlock {
    /// Creates a new TEB initialized with given IDs, stack bounds, and PEB reference.
    pub fn new(tid: u64, pid: u64, stack_base: u64, stack_limit: u64, peb_addr: u64) -> Self {
        Self {
            tib: NtTib {
                exception_list: 0,
                stack_base,
                stack_limit,
                subsystem_tib: 0,
                fiber_data: 0,
                arbitrary_user_pointer: 0,
                self_ptr: 0,
            },
            environment_pointer: 0,
            client_id: ClientId {
                unique_process: pid,
                unique_thread: tid,
            },
            active_rpc_handle: 0,
            thread_local_storage_pointer: 0,
            process_environment_block: peb_addr,
            last_error_value: 0,
            count_of_owned_critical_sections: 0,
            current_locale: 0x0409, // en-US
            padding: 0,
            tls_slots: [0; 64],
        }
    }
}

/// Process Environment Block (PEB) describing process-wide userland state.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct ProcessEnvironmentBlock {
    pub inherited_address_space: u8,
    pub read_image_file_exec_options: u8,
    pub being_debugged: u8,
    pub bit_field: u8,
    pub padding0: u32,
    pub mutant: u64,
    pub image_base_address: u64,
    pub loader_data: u64,
    pub process_parameters: u64,
    pub subsystem_data: u64,
    pub process_heap: u64,
    pub fast_peb_lock: u64,
    pub number_of_processors: u32,
    pub nt_global_flag: u32,
    pub session_id: u32,
    pub padding1: u32,
    pub os_major_version: u32,
    pub os_minor_version: u32,
    pub os_build_number: u16,
    pub os_csd_version: u16,
    pub os_platform_id: u32,
    pub image_subsystem: u32,
    pub active_process_affinity_mask: u64,
}

impl ProcessEnvironmentBlock {
    /// Creates a new PEB initialized with image base, heap base, and processor count.
    pub fn new(image_base: u64, heap_base: u64, num_processors: u32) -> Self {
        Self {
            inherited_address_space: 0,
            read_image_file_exec_options: 0,
            being_debugged: 0,
            bit_field: 0,
            padding0: 0,
            mutant: 0,
            image_base_address: image_base,
            loader_data: 0,
            process_parameters: 0,
            subsystem_data: 0,
            process_heap: heap_base,
            fast_peb_lock: 0,
            number_of_processors: num_processors,
            nt_global_flag: 0,
            session_id: 1,
            padding1: 0,
            os_major_version: 10,
            os_minor_version: 0,
            os_build_number: 19045,
            os_csd_version: 0,
            os_platform_id: 2, // VER_PLATFORM_WIN32_NT
            image_subsystem: 3, // IMAGE_SUBSYSTEM_WINDOWS_CUI
            active_process_affinity_mask: if num_processors >= 64 {
                !0u64
            } else {
                (1u64 << num_processors) - 1
            },
        }
    }
}

/// I/O Status Block structure holding completion status and information.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoStatusBlock {
    pub status: NtStatus,
    pub information: usize,
}

/// Unicode String structure for NT object names.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnicodeString {
    pub length: u16,
    pub maximum_length: u16,
    pub buffer: *mut u16,
}

/// Object Attributes descriptor for object creation/opening.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ObjectAttributes {
    pub length: u32,
    pub root_directory: u64,
    pub object_name: *const UnicodeString,
    pub attributes: u32,
    pub security_descriptor: *const u8,
    pub security_quality_of_service: *const u8,
}

/// System Information Class enumeration.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemInformationClass {
    SystemBasicInformation = 0,
    SystemProcessorInformation = 1,
    SystemPerformanceInformation = 2,
    SystemTimeOfDayInformation = 3,
}

/// Allocates or commits a region of virtual memory within the user address space.
pub unsafe fn nt_allocate_virtual_memory(
    process_handle: u64,
    base_address: *mut usize,
    zero_bits: usize,
    region_size: *mut usize,
    allocation_type: u32,
    protect: u32,
) -> NtStatus {
    let _ = (zero_bits, allocation_type);
    crate::syscall::table::dispatch(
        crate::syscall::table::NT_ALLOCATE_VIRTUAL_MEMORY,
        process_handle,
        base_address as u64,
        region_size as u64,
        protect as u64,
    )
}

/// Frees or decommits a region of virtual memory within the user address space.
pub unsafe fn nt_free_virtual_memory(
    process_handle: u64,
    base_address: *mut usize,
    region_size: *mut usize,
    free_type: u32,
) -> NtStatus {
    crate::syscall::table::dispatch(
        crate::syscall::table::NT_FREE_VIRTUAL_MEMORY,
        process_handle,
        base_address as u64,
        region_size as u64,
        free_type as u64,
    )
}

/// Creates a new file or opens an existing file or device object.
pub unsafe fn nt_create_file(
    file_handle: *mut u64,
    desired_access: u32,
    object_attributes: *const ObjectAttributes,
    io_status_block: *mut IoStatusBlock,
    allocation_size: *const u64,
    file_attributes: u32,
    share_access: u32,
    create_disposition: u32,
    create_options: u32,
    ea_buffer: *const u8,
    ea_length: u32,
) -> NtStatus {
    let _ = (
        allocation_size,
        file_attributes,
        share_access,
        create_disposition,
        create_options,
        ea_buffer,
        ea_length,
    );
    let name_ptr = if !object_attributes.is_null() && !(*object_attributes).object_name.is_null() {
        (*(*object_attributes).object_name).buffer as u64
    } else {
        0
    };

    crate::syscall::table::dispatch(
        crate::syscall::table::NT_CREATE_FILE,
        file_handle as u64,
        desired_access as u64,
        name_ptr,
        io_status_block as u64,
    )
}

/// Reads data from an open file or device into a userland buffer.
pub unsafe fn nt_read_file(
    file_handle: u64,
    event: u64,
    apc_routine: u64,
    apc_context: u64,
    io_status_block: *mut IoStatusBlock,
    buffer: *mut u8,
    length: u32,
    byte_offset: *const u64,
    key: *const u32,
) -> NtStatus {
    let _ = (apc_routine, apc_context, byte_offset, key);
    let status = crate::syscall::table::dispatch(
        crate::syscall::table::NT_READ_FILE,
        file_handle,
        event,
        buffer as u64,
        length as u64,
    );
    if !io_status_block.is_null() {
        (*io_status_block).status = status;
        if status == STATUS_SUCCESS {
            (*io_status_block).information = length as usize;
        }
    }
    status
}

/// Writes data from a userland buffer to an open file or device.
pub unsafe fn nt_write_file(
    file_handle: u64,
    event: u64,
    apc_routine: u64,
    apc_context: u64,
    io_status_block: *mut IoStatusBlock,
    buffer: *const u8,
    length: u32,
    byte_offset: *const u64,
    key: *const u32,
) -> NtStatus {
    let _ = (apc_routine, apc_context, byte_offset, key);
    let status = crate::syscall::table::dispatch(
        crate::syscall::table::NT_WRITE_FILE,
        file_handle,
        event,
        buffer as u64,
        length as u64,
    );
    if !io_status_block.is_null() {
        (*io_status_block).status = status;
        if status == STATUS_SUCCESS {
            (*io_status_block).information = length as usize;
        }
    }
    status
}

/// Closes an open handle to a kernel object, file, or device.
pub unsafe fn nt_close(handle: u64) -> NtStatus {
    crate::syscall::table::dispatch(
        crate::syscall::table::NT_CLOSE,
        handle,
        0,
        0,
        0,
    )
}

/// Sends a device I/O control code (IOCTL) directly to a device driver.
pub unsafe fn nt_device_io_control_file(
    file_handle: u64,
    event: u64,
    apc_routine: u64,
    apc_context: u64,
    io_status_block: *mut IoStatusBlock,
    io_control_code: u32,
    input_buffer: *const u8,
    input_buffer_length: u32,
    output_buffer: *mut u8,
    output_buffer_length: u32,
) -> NtStatus {
    let _ = (
        apc_routine,
        apc_context,
        input_buffer,
        input_buffer_length,
        output_buffer_length,
    );
    let status = crate::syscall::table::dispatch(
        crate::syscall::table::NT_DEVICE_IO_CONTROL_FILE,
        file_handle,
        event,
        io_control_code as u64,
        output_buffer as u64,
    );
    if !io_status_block.is_null() {
        (*io_status_block).status = status;
        if status == STATUS_SUCCESS {
            (*io_status_block).information = output_buffer_length as usize;
        }
    }
    status
}

/// Queries internal kernel telemetry, memory limits, and hardware information.
pub unsafe fn nt_query_system_information(
    system_information_class: SystemInformationClass,
    system_information: *mut u8,
    system_information_length: u32,
    return_length: *mut u32,
) -> NtStatus {
    crate::syscall::table::dispatch(
        crate::syscall::table::NT_QUERY_SYSTEM_INFORMATION,
        system_information_class as u64,
        system_information as u64,
        system_information_length as u64,
        return_length as u64,
    )
}

/// Comprehensive self-test suite verifying the Win32 userland runtime and NTDLL emulation layer.
pub fn test_ntdll_emulation_subsystem() -> bool {
    crate::serial_write_str("[KNT-NTDLL-TEST] Executing Win32 NTDLL userland runtime verification...\r\n");

    // 1. Verify Status Code translation and error bounds handling
    {
        if !nt_success(STATUS_SUCCESS) || nt_error(STATUS_SUCCESS) {
            return false;
        }
        if !nt_error(STATUS_UNSUCCESSFUL) || !nt_error(STATUS_INVALID_PARAMETER) || !nt_error(STATUS_NO_MEMORY) {
            return false;
        }
        if nt_status_to_str(STATUS_SUCCESS) != "STATUS_SUCCESS" {
            return false;
        }
        if nt_status_to_str(STATUS_INVALID_PARAMETER) != "STATUS_INVALID_PARAMETER" {
            return false;
        }
        if nt_status_to_str(STATUS_NO_MEMORY) != "STATUS_NO_MEMORY" {
            return false;
        }
        if nt_status_to_str(STATUS_BUFFER_TOO_SMALL) != "STATUS_BUFFER_TOO_SMALL" {
            return false;
        }
    }

    // 2. Verify TEB and PEB structure alignment and initial setup
    {
        if core::mem::align_of::<ThreadEnvironmentBlock>() < 64 {
            return false;
        }
        if core::mem::align_of::<ProcessEnvironmentBlock>() < 64 {
            return false;
        }

        let peb = ProcessEnvironmentBlock::new(0x0040_0000, 0x0000_1000_0000, 4);
        if peb.image_base_address != 0x0040_0000 || peb.process_heap != 0x0000_1000_0000 {
            return false;
        }
        if peb.number_of_processors != 4 || peb.os_major_version != 10 || peb.os_platform_id != 2 {
            return false;
        }

        let teb = ThreadEnvironmentBlock::new(101, 10, 0x0000_7FFF_FFFF_0000, 0x0000_7FFF_FFF0_0000, 0x0000_7FFF_FFE0_0000);
        if teb.client_id.unique_thread != 101 || teb.client_id.unique_process != 10 {
            return false;
        }
        if teb.tib.stack_base != 0x0000_7FFF_FFFF_0000 || teb.tib.stack_limit != 0x0000_7FFF_FFF0_0000 {
            return false;
        }
        if teb.process_environment_block != 0x0000_7FFF_FFE0_0000 || teb.current_locale != 0x0409 {
            return false;
        }
    }

    // 3. Verify userland virtual memory allocation and release dispatch
    unsafe {
        let base_slot = (crate::syscall::validate::USER_SPACE_START + 0x4000) as *mut usize;
        let size_slot = (crate::syscall::validate::USER_SPACE_START + 0x4008) as *mut usize;
        *base_slot = 0;
        *size_slot = 8192;

        let alloc_status = nt_allocate_virtual_memory(
            crate::syscall::NT_CURRENT_PROCESS,
            base_slot,
            0,
            size_slot,
            0x1000,
            0x04,
        );
        if !nt_success(alloc_status) || *base_slot == 0 || *size_slot != 8192 {
            return false;
        }

        let free_status = nt_free_virtual_memory(
            crate::syscall::NT_CURRENT_PROCESS,
            base_slot,
            size_slot,
            0x8000,
        );
        if !nt_success(free_status) || *size_slot != 0 {
            return false;
        }

        let err_status = nt_allocate_virtual_memory(
            crate::syscall::NT_CURRENT_PROCESS,
            base_slot,
            0,
            core::ptr::null_mut(),
            0x1000,
            0x04,
        );
        if !nt_error(err_status) {
            return false;
        }
    }

    // 4. Verify userland file I/O and device I/O control dispatch
    unsafe {
        let handle_slot = (crate::syscall::validate::USER_SPACE_START + 0x4010) as *mut u64;
        let iosb_slot = (crate::syscall::validate::USER_SPACE_START + 0x4020) as *mut IoStatusBlock;
        *handle_slot = 0;

        let create_status = nt_create_file(
            handle_slot,
            0x8000_0000,
            core::ptr::null(),
            iosb_slot,
            core::ptr::null(),
            0,
            0,
            1,
            0,
            core::ptr::null(),
            0,
        );
        if !nt_success(create_status) || *handle_slot == 0 {
            return false;
        }

        let msg = b"[KNT-NTDLL-TEST] Userland NTDLL write verified\r\n";
        let write_buf = (crate::syscall::validate::USER_SPACE_START + 0x4040) as *mut u8;
        core::ptr::copy_nonoverlapping(msg.as_ptr(), write_buf, msg.len());

        let write_status = nt_write_file(
            *handle_slot,
            0,
            0,
            0,
            iosb_slot,
            write_buf,
            msg.len() as u32,
            core::ptr::null(),
            core::ptr::null(),
        );
        if !nt_success(write_status) || (*iosb_slot).information != msg.len() {
            return false;
        }

        let read_buf = (crate::syscall::validate::USER_SPACE_START + 0x4080) as *mut u8;
        let read_status = nt_read_file(
            *handle_slot,
            0,
            0,
            0,
            iosb_slot,
            read_buf,
            16,
            core::ptr::null(),
            core::ptr::null(),
        );
        if !nt_success(read_status) {
            return false;
        }

        let ioctl_buf = (crate::syscall::validate::USER_SPACE_START + 0x40C0) as *mut u8;
        *ioctl_buf = 0;
        let ioctl_status = nt_device_io_control_file(
            *handle_slot,
            0,
            0,
            0,
            iosb_slot,
            0x0001,
            core::ptr::null(),
            0,
            ioctl_buf,
            8,
        );
        if !nt_success(ioctl_status) {
            return false;
        }

        let sys_info_buf = (crate::syscall::validate::USER_SPACE_START + 0x4100) as *mut u8;
        let ret_len_slot = (crate::syscall::validate::USER_SPACE_START + 0x4140) as *mut u32;
        *ret_len_slot = 0;
        let query_status = nt_query_system_information(
            SystemInformationClass::SystemBasicInformation,
            sys_info_buf,
            32,
            ret_len_slot,
        );
        if !nt_success(query_status) || *ret_len_slot != 32 {
            return false;
        }

        let close_status = nt_close(*handle_slot);
        if !nt_success(close_status) {
            return false;
        }
    }

    crate::serial_write_str("[KNT-NTDLL-EMU-PASS] Win32 NTDLL userland runtime and syscall dispatch verified\r\n");
    true
}
