// Win32 Process and Thread Environment Blocks (PEB/TEB) Subsystem

use alloc::string::String;
use alloc::vec::Vec;
use crate::memory::spinlock::Spinlock;
use crate::pe::kernel32::{ERROR_INVALID_PARAMETER, ERROR_SUCCESS};
use crate::pe::ntdll::UnicodeString as UNICODE_STRING;

pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_ENVVAR_NOT_FOUND: u32 = 203;

// Standard x86-64 Win32 Client Identifier
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CLIENT_ID {
    pub unique_process: u64,
    pub unique_thread: u64,
}

// Thread Information Block header at TEB offset 0x00
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NT_TIB {
    pub exception_list: u64,
    pub stack_base: u64,
    pub stack_limit: u64,
    pub subsystem_tib: u64,
    pub fiber_data: u64,
    pub arbitrary_user_pointer: u64,
    pub self_ptr: u64,
}

impl Default for NT_TIB {
    fn default() -> Self {
        Self {
            exception_list: 0,
            stack_base: 0,
            stack_limit: 0,
            subsystem_tib: 0,
            fiber_data: 0,
            arbitrary_user_pointer: 0,
            self_ptr: 0,
        }
    }
}

// Win32 user process parameters containing paths, handles, and command line
#[repr(C)]
#[derive(Debug, Clone)]
pub struct RTL_USER_PROCESS_PARAMETERS {
    pub maximum_length: u32,
    pub length: u32,
    pub flags: u32,
    pub debug_flags: u32,
    pub console_handle: u64,
    pub console_flags: u32,
    pub standard_input: u64,
    pub standard_output: u64,
    pub standard_error: u64,
    pub current_directory_path: UNICODE_STRING,
    pub current_directory_handle: u64,
    pub dll_path: UNICODE_STRING,
    pub image_path_name: UNICODE_STRING,
    pub command_line: UNICODE_STRING,
    pub environment: u64,
    pub starting_x: u32,
    pub starting_y: u32,
    pub count_x: u32,
    pub count_y: u32,
    pub count_chars_x: u32,
    pub count_chars_y: u32,
    pub fill_attribute: u32,
    pub window_flags: u32,
    pub show_window_flags: u32,
    pub window_title: UNICODE_STRING,
    pub desktop_info: UNICODE_STRING,
    pub shell_info: UNICODE_STRING,
    pub runtime_data: UNICODE_STRING,
}

impl RTL_USER_PROCESS_PARAMETERS {
    // Construct default process parameters structure
    pub fn new() -> Self {
        Self {
            maximum_length: core::mem::size_of::<Self>() as u32,
            length: core::mem::size_of::<Self>() as u32,
            flags: 1,
            debug_flags: 0,
            console_handle: 0,
            console_flags: 0,
            standard_input: 0,
            standard_output: 0,
            standard_error: 0,
            current_directory_path: UNICODE_STRING::default(),
            current_directory_handle: 0,
            dll_path: UNICODE_STRING::default(),
            image_path_name: UNICODE_STRING::default(),
            command_line: UNICODE_STRING::default(),
            environment: 0,
            starting_x: 0,
            starting_y: 0,
            count_x: 0,
            count_y: 0,
            count_chars_x: 0,
            count_chars_y: 0,
            fill_attribute: 0,
            window_flags: 0,
            show_window_flags: 1,
            window_title: UNICODE_STRING::default(),
            desktop_info: UNICODE_STRING::default(),
            shell_info: UNICODE_STRING::default(),
            runtime_data: UNICODE_STRING::default(),
        }
    }
}

// Process Environment Block describing process-wide userland state
#[repr(C, align(64))]
#[derive(Debug, Clone)]
pub struct PEB {
    pub inherited_address_space: u8,
    pub read_image_file_exec_options: u8,
    pub being_debugged: u8,
    pub bit_field: u8,
    pub padding0: u32,
    pub mutant: u64,
    pub image_base_address: u64,
    pub ldr: u64,
    pub process_parameters: u64,
    pub subsystem_data: u64,
    pub process_heap: u64,
    pub fast_peb_lock: u64,
    pub atl_thunk_slist_ptr: u64,
    pub ifeo_key: u64,
    pub cross_process_flags: u32,
    pub padding1: u32,
    pub user_shared_info_ptr: u64,
    pub system_reserved: u32,
    pub atl_thunk_slist_ptr32: u32,
    pub api_set_map: u64,
    pub tls_expansion_counter: u32,
    pub padding2: u32,
    pub tls_bitmap: u64,
    pub tls_bitmap_bits: [u32; 2],
    pub read_only_shared_memory_base: u64,
    pub shared_data: u64,
    pub read_only_static_server_data: u64,
    pub ansi_code_page_data: u64,
    pub oem_code_page_data: u64,
    pub unicode_case_table_data: u64,
    pub number_of_processors: u32,
    pub nt_global_flag: u32,
    pub critical_section_timeout: i64,
    pub heap_segment_reserve: u64,
    pub heap_segment_commit: u64,
    pub heap_decommit_total_free_threshold: u64,
    pub heap_decommit_free_block_threshold: u64,
    pub number_of_heaps: u32,
    pub maximum_number_of_heaps: u32,
    pub process_heaps: u64,
    pub gdi_shared_handle_table: u64,
    pub process_starter_helper: u64,
    pub gdi_dc_attribute_list: u32,
    pub padding3: u32,
    pub loader_lock: u64,
    pub os_major_version: u32,
    pub os_minor_version: u32,
    pub os_build_number: u16,
    pub os_csd_version: u16,
    pub os_platform_id: u32,
    pub image_subsystem: u32,
    pub image_subsystem_major_version: u32,
    pub image_subsystem_minor_version: u32,
    pub active_process_affinity_mask: u64,
    pub session_id: u32,
}

impl PEB {
    // Create new PEB with initial image base and processor count
    pub fn new(image_base: u64, heap_base: u64, num_processors: u32) -> Self {
        Self {
            inherited_address_space: 0,
            read_image_file_exec_options: 0,
            being_debugged: 0,
            bit_field: 0,
            padding0: 0,
            mutant: 0,
            image_base_address: image_base,
            ldr: 0,
            process_parameters: 0,
            subsystem_data: 0,
            process_heap: heap_base,
            fast_peb_lock: 0,
            atl_thunk_slist_ptr: 0,
            ifeo_key: 0,
            cross_process_flags: 0,
            padding1: 0,
            user_shared_info_ptr: 0,
            system_reserved: 0,
            atl_thunk_slist_ptr32: 0,
            api_set_map: 0,
            tls_expansion_counter: 0,
            padding2: 0,
            tls_bitmap: 0,
            tls_bitmap_bits: [0; 2],
            read_only_shared_memory_base: 0,
            shared_data: 0,
            read_only_static_server_data: 0,
            ansi_code_page_data: 0,
            oem_code_page_data: 0,
            unicode_case_table_data: 0,
            number_of_processors: num_processors,
            nt_global_flag: 0,
            critical_section_timeout: -2500000,
            heap_segment_reserve: 0x100000,
            heap_segment_commit: 0x10000,
            heap_decommit_total_free_threshold: 0x10000,
            heap_decommit_free_block_threshold: 0x1000,
            number_of_heaps: 1,
            maximum_number_of_heaps: 16,
            process_heaps: 0,
            gdi_shared_handle_table: 0,
            process_starter_helper: 0,
            gdi_dc_attribute_list: 0,
            padding3: 0,
            loader_lock: 0,
            os_major_version: 10,
            os_minor_version: 0,
            os_build_number: 19045,
            os_csd_version: 0,
            os_platform_id: 2,
            image_subsystem: 3,
            image_subsystem_major_version: 10,
            image_subsystem_minor_version: 0,
            active_process_affinity_mask: if num_processors >= 64 { !0u64 } else { (1u64 << num_processors) - 1 },
            session_id: 1,
        }
    }
}

// Thread Environment Block mapped per userland thread
#[repr(C, align(64))]
#[derive(Debug, Clone)]
pub struct TEB {
    pub tib: NT_TIB,
    pub environment_pointer: u64,
    pub client_id: CLIENT_ID,
    pub active_rpc_handle: u64,
    pub thread_local_storage_pointer: u64,
    pub process_environment_block: u64,
    pub last_error_value: u32,
    pub count_of_owned_critical_sections: u32,
    pub csr_client_thread: u64,
    pub win32_thread_info: u64,
    pub user32_reserved: [u32; 26],
    pub user_reserved: [u32; 5],
    pub reserved_wow64: u64,
    pub current_locale: u32,
    pub fp_software_status_register: u32,
    pub reserved_for_debugger: [u64; 54],
    pub exception_code: u32,
    pub padding0: u32,
    pub activation_context_stack_pointer: u64,
    pub instrumentation_callback_sp: u64,
    pub instrumentation_callback_previous_pc: u64,
    pub instrumentation_callback_previous_sp: u64,
    pub tx_fs_context: u8,
    pub instrumentation_callback_disabled: u8,
    pub padding1: [u8; 6],
    pub gdi_teb_batch: [u8; 1280],
    pub real_client_id: CLIENT_ID,
    pub gdi_cached_process_handle: u64,
    pub gdi_client_pid: u32,
    pub gdi_client_tid: u32,
    pub gdi_thread_local_info: u64,
    pub win32_client_info: [u64; 62],
    pub gl_dispatch_table: [u64; 233],
    pub gl_reserved1: [u64; 29],
    pub gl_reserved2: u64,
    pub gl_section_info: u64,
    pub gl_section: u64,
    pub gl_table: u64,
    pub gl_current_rc: u64,
    pub gl_context: u64,
    pub last_status_value: u32,
    pub padding2: u32,
    pub static_unicode_string: UNICODE_STRING,
    pub static_unicode_buffer: [u16; 261],
    pub deallocation_stack: u64,
    pub tls_slots: [u64; 64],
}

impl TEB {
    // Create new TEB initialized with thread IDs and stack boundaries
    pub fn new(pid: u64, tid: u64, stack_base: u64, stack_limit: u64, peb_addr: u64) -> Self {
        Self {
            tib: NT_TIB {
                exception_list: 0,
                stack_base,
                stack_limit,
                subsystem_tib: 0,
                fiber_data: 0,
                arbitrary_user_pointer: 0,
                self_ptr: 0,
            },
            environment_pointer: 0,
            client_id: CLIENT_ID {
                unique_process: pid,
                unique_thread: tid,
            },
            active_rpc_handle: 0,
            thread_local_storage_pointer: 0,
            process_environment_block: peb_addr,
            last_error_value: 0,
            count_of_owned_critical_sections: 0,
            csr_client_thread: 0,
            win32_thread_info: 0,
            user32_reserved: [0; 26],
            user_reserved: [0; 5],
            reserved_wow64: 0,
            current_locale: 0x0409,
            fp_software_status_register: 0,
            reserved_for_debugger: [0; 54],
            exception_code: 0,
            padding0: 0,
            activation_context_stack_pointer: 0,
            instrumentation_callback_sp: 0,
            instrumentation_callback_previous_pc: 0,
            instrumentation_callback_previous_sp: 0,
            tx_fs_context: 0,
            instrumentation_callback_disabled: 0,
            padding1: [0; 6],
            gdi_teb_batch: [0; 1280],
            real_client_id: CLIENT_ID {
                unique_process: pid,
                unique_thread: tid,
            },
            gdi_cached_process_handle: 0,
            gdi_client_pid: pid as u32,
            gdi_client_tid: tid as u32,
            gdi_thread_local_info: 0,
            win32_client_info: [0; 62],
            gl_dispatch_table: [0; 233],
            gl_reserved1: [0; 29],
            gl_reserved2: 0,
            gl_section_info: 0,
            gl_section: 0,
            gl_table: 0,
            gl_current_rc: 0,
            gl_context: 0,
            last_status_value: 0,
            padding2: 0,
            static_unicode_string: UNICODE_STRING::default(),
            static_unicode_buffer: [0; 261],
            deallocation_stack: stack_limit,
            tls_slots: [0; 64],
        }
    }
}

// Canonical TEB field offsets in 64-bit Windows
pub const TEB_TIB_OFFSET: usize = 0x00;
pub const TEB_STACK_BASE_OFFSET: usize = 0x08;
pub const TEB_STACK_LIMIT_OFFSET: usize = 0x10;
pub const TEB_SELF_PTR_OFFSET: usize = 0x30;
pub const TEB_CLIENT_ID_OFFSET: usize = 0x40;
pub const TEB_PEB_OFFSET: usize = 0x60;
pub const TEB_LAST_ERROR_OFFSET: usize = 0x68;

// Convert null-terminated UTF-16 pointer to Rust String
pub unsafe fn utf16_ptr_to_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    Some(String::from_utf16_lossy(slice))
}

// Convert Rust &str to UTF-16 vector with null terminator
pub fn str_to_utf16_null(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

// Case-insensitive ASCII comparison for environment variable names
fn env_name_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).all(|(x, y)| x.to_ascii_uppercase() == y.to_ascii_uppercase())
}

// Global process environment state tracking variables and serialized UTF-16 block
pub struct ProcessEnvironment {
    vars: Vec<(String, String)>,
    serialized_cache: Vec<u16>,
    initialized: bool,
}

impl ProcessEnvironment {
    // Construct empty process environment block
    pub const fn empty() -> Self {
        Self {
            vars: Vec::new(),
            serialized_cache: Vec::new(),
            initialized: false,
        }
    }

    // Initialize default environment variables if not yet populated
    pub fn ensure_init(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        self.set("SystemRoot", "C:\\Windows");
        self.set("COMPUTERNAME", "KNTON-PC");
        self.set("USERNAME", "Administrator");
        self.set("OS", "Windows_NT");
        self.set("PROCESSOR_ARCHITECTURE", "AMD64");
        self.set("NUMBER_OF_PROCESSORS", "4");
        self.set("PATH", "C:\\Windows\\System32;C:\\Windows");
        self.rebuild_cache();
    }

    // Lookup variable value by case-insensitive name
    pub fn get(&mut self, name: &str) -> Option<&str> {
        self.ensure_init();
        self.vars.iter().find(|(k, _)| env_name_eq(k, name)).map(|(_, v)| v.as_str())
    }

    // Insert or update variable value
    pub fn set(&mut self, name: &str, value: &str) {
        if let Some(pos) = self.vars.iter().position(|(k, _)| env_name_eq(k, name)) {
            self.vars[pos].1 = String::from(value);
        } else {
            self.vars.push((String::from(name), String::from(value)));
        }
        self.rebuild_cache();
    }

    // Remove variable by case-insensitive name
    pub fn remove(&mut self, name: &str) -> bool {
        self.ensure_init();
        if let Some(pos) = self.vars.iter().position(|(k, _)| env_name_eq(k, name)) {
            self.vars.remove(pos);
            self.rebuild_cache();
            true
        } else {
            false
        }
    }

    // Rebuild UTF-16 serialized double-null terminated environment block
    pub fn rebuild_cache(&mut self) {
        let mut buf = Vec::new();
        for (k, v) in &self.vars {
            for ch in k.encode_utf16() {
                buf.push(ch);
            }
            buf.push('=' as u16);
            for ch in v.encode_utf16() {
                buf.push(ch);
            }
            buf.push(0);
        }
        buf.push(0);
        self.serialized_cache = buf;
    }

    // Parse raw double-null terminated UTF-16 block into key-value pairs
    pub fn parse_block_w(block: &[u16]) -> Vec<(String, String)> {
        let mut result = Vec::new();
        let mut idx = 0;
        while idx < block.len() {
            if block[idx] == 0 {
                break;
            }
            let start = idx;
            while idx < block.len() && block[idx] != 0 {
                idx += 1;
            }
            let entry = String::from_utf16_lossy(&block[start..idx]);
            if let Some(eq_pos) = entry.find('=') {
                let (k, v) = entry.split_at(eq_pos);
                result.push((String::from(k), String::from(&v[1..])));
            }
            idx += 1;
        }
        result
    }
}

pub static PROCESS_ENV: Spinlock<ProcessEnvironment> = Spinlock::new(ProcessEnvironment::empty());

// Win32 GetEnvironmentVariableW retrieves variable value as UTF-16
pub unsafe fn win32_get_environment_variable_w(
    lp_name: *const u16,
    lp_buffer: *mut u16,
    n_size: u32,
) -> u32 {
    let name_str = match utf16_ptr_to_string(lp_name) {
        Some(s) if !s.is_empty() => s,
        _ => {
            crate::pe::kernel32::win32_set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    };

    let mut env = PROCESS_ENV.lock();
    match env.get(&name_str) {
        Some(val) => {
            let val_utf16: Vec<u16> = val.encode_utf16().collect();
            let val_len = val_utf16.len() as u32;
            if lp_buffer.is_null() || n_size <= val_len {
                return val_len + 1;
            }
            for (i, &ch) in val_utf16.iter().enumerate() {
                *lp_buffer.add(i) = ch;
            }
            *lp_buffer.add(val_len as usize) = 0;
            crate::pe::kernel32::win32_set_last_error(ERROR_SUCCESS);
            val_len
        }
        None => {
            crate::pe::kernel32::win32_set_last_error(ERROR_ENVVAR_NOT_FOUND);
            0
        }
    }
}

// Win32 SetEnvironmentVariableW creates, updates, or deletes environment variable
pub unsafe fn win32_set_environment_variable_w(
    lp_name: *const u16,
    lp_value: *const u16,
) -> bool {
    let name_str = match utf16_ptr_to_string(lp_name) {
        Some(s) if !s.is_empty() => s,
        _ => {
            crate::pe::kernel32::win32_set_last_error(ERROR_INVALID_PARAMETER);
            return false;
        }
    };

    let mut env = PROCESS_ENV.lock();
    env.ensure_init();
    if lp_value.is_null() || *lp_value == 0 {
        env.remove(&name_str);
    } else {
        match utf16_ptr_to_string(lp_value) {
            Some(val_str) => env.set(&name_str, &val_str),
            None => {
                crate::pe::kernel32::win32_set_last_error(ERROR_INVALID_PARAMETER);
                return false;
            }
        }
    }
    crate::pe::kernel32::win32_set_last_error(ERROR_SUCCESS);
    true
}

// Win32 GetEnvironmentStringsW returns pointer to serialized UTF-16 environment block
pub fn win32_get_environment_strings_w() -> *mut u16 {
    let mut env = PROCESS_ENV.lock();
    env.ensure_init();
    if env.serialized_cache.is_empty() {
        env.rebuild_cache();
    }
    env.serialized_cache.as_mut_ptr()
}

// Win32 FreeEnvironmentStringsW releases environment strings handle
pub fn win32_free_environment_strings_w(_penv: *mut u16) -> bool {
    true
}

// Global storage for process command line in ANSI/UTF-8 and UTF-16
pub struct ProcessCommandLine {
    cmdline_a: Vec<u8>,
    cmdline_w: Vec<u16>,
    initialized: bool,
}

impl ProcessCommandLine {
    // Construct empty command line state
    pub const fn empty() -> Self {
        Self {
            cmdline_a: Vec::new(),
            cmdline_w: Vec::new(),
            initialized: false,
        }
    }

    // Ensure initial default command line is set
    pub fn ensure_init(&mut self) {
        if self.initialized {
            return;
        }
        self.initialized = true;
        self.set("init.exe");
    }

    // Update command line string in both UTF-8 and UTF-16 representations
    pub fn set(&mut self, cmd: &str) {
        self.initialized = true;
        let mut a = Vec::from(cmd.as_bytes());
        a.push(0);
        self.cmdline_a = a;

        let mut w: Vec<u16> = cmd.encode_utf16().collect();
        w.push(0);
        self.cmdline_w = w;
    }
}

pub static PROCESS_CMDLINE: Spinlock<ProcessCommandLine> = Spinlock::new(ProcessCommandLine::empty());

// Win32 GetCommandLineW returns pointer to null-terminated UTF-16 command line
pub fn win32_get_command_line_w() -> *const u16 {
    let mut cmd = PROCESS_CMDLINE.lock();
    cmd.ensure_init();
    cmd.cmdline_w.as_ptr()
}

// Win32 GetCommandLineA returns pointer to null-terminated UTF-8/ANSI command line
pub fn win32_get_command_line_a() -> *const u8 {
    let mut cmd = PROCESS_CMDLINE.lock();
    cmd.ensure_init();
    cmd.cmdline_a.as_ptr()
}

// Set process command line parameters
pub fn win32_set_command_line(cmd: &str) {
    let mut cl = PROCESS_CMDLINE.lock();
    cl.set(cmd);
}

// Retrieve current command line as a Rust String
pub fn win32_get_command_line_str() -> String {
    let mut cmd = PROCESS_CMDLINE.lock();
    cmd.ensure_init();
    if cmd.cmdline_a.is_empty() {
        return String::new();
    }
    let len = cmd.cmdline_a.len().saturating_sub(1);
    String::from_utf8_lossy(&cmd.cmdline_a[..len]).into_owned()
}
