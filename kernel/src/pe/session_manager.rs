//! Userspace Session Manager (smss) & Core Service Supervisor for KntonOS.
//!
//! Provides the primary userland supervisory subsystem responsible for:
//! 1. Multi-session lifecycle and strict Session 0 / Session 1 isolation.
//! 2. Process spawn rules, subsystem classification (Native, Win32, POSIX), and environment inheritance.
//! 3. System service registry, health heartbeats, and deterministic crash recovery auto-restart policies.
//! 4. KntPort IPC endpoints for secure inter-session and inter-process rendezvous.
//! 5. Comprehensive self-test suite verified with boot sentinel `[KNT-OS-SMSS-PASS]`.

#![allow(dead_code)]

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ============================================================================
// Feature 1: Session Manager Core Models & Session Descriptors
// ============================================================================

/// Subsystem architecture type required by a binary executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsystemType {
    /// Native NT micro-kernel subsystem (e.g. smss.exe, csrss.exe, ntoskrnl).
    Native,
    /// Win32 graphical or console subsystem (e.g. explorer.exe, gameshell.exe, steam.exe).
    Win32,
    /// POSIX compatible subsystem layer.
    Posix,
}

/// Operational state of a userland session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Session is currently initializing core services and security boundaries.
    Initializing,
    /// Session is active and executing user or system processes.
    Active,
    /// Session is locked (e.g. user lock screen, display sleep).
    Locked,
    /// Session has completed termination and all child processes are reaped.
    Terminated,
}

/// Security boundary flags enforcing session isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSecurityFlags {
    /// Whether processes in this session can allocate interactive desktop surfaces / receive user input.
    pub allow_interactive_ui: bool,
    /// Whether this session is Session 0 isolated from shatter attacks and unauthorized window messaging.
    pub isolated_session0: bool,
    /// Whether processes in this session are permitted to initiate cross-session IPC rendezvous.
    pub allow_cross_session_ipc: bool,
    /// Whether process creation requires administrator privilege token.
    pub require_elevation: bool,
}

impl SessionSecurityFlags {
    /// Security profile for Session 0 (Headless System Services):
    /// Strictly disallows interactive UI to prevent UI privilege escalation (Shatter attacks).
    pub const fn session_0() -> Self {
        Self {
            allow_interactive_ui: false,
            isolated_session0: true,
            allow_cross_session_ipc: true, // Services can receive cross-session requests on public ports
            require_elevation: true,
        }
    }

    /// Security profile for Session 1 (Interactive Handheld Gaming Session):
    /// Allows full display scanout, raw controller input, and audio buffers.
    pub const fn session_1() -> Self {
        Self {
            allow_interactive_ui: true,
            isolated_session0: false,
            allow_cross_session_ipc: true, // Can connect to public Session 0 system services
            require_elevation: false,
        }
    }
}

/// Descriptor managing the state, security, and process hierarchy of a userland session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescriptor {
    /// Unique numeric session identifier (0 = System, 1 = Primary Interactive User).
    pub session_id: u32,
    /// Human-readable session name (e.g. "Services", "Console", "HandheldUser").
    pub name: String,
    /// Current operational state of the session.
    pub state: SessionState,
    /// Active subsystems supported inside this session.
    pub supported_subsystems: Vec<SubsystemType>,
    /// List of process identifiers (PIDs) running inside this session.
    pub active_pids: Vec<u64>,
    /// Security boundary configuration.
    pub security_flags: SessionSecurityFlags,
    /// Session-specific environment variable map.
    pub environment_vars: BTreeMap<String, String>,
    /// Allocated desktop heap size in bytes for GUI objects.
    pub desktop_heap_bytes: u32,
    /// Kernel tick timestamp when session was instantiated.
    pub created_tick: u64,
}

impl SessionDescriptor {
    /// Instantiates Session 0: The isolated system background services session.
    pub fn new_session_0(created_tick: u64) -> Self {
        let mut env = BTreeMap::new();
        env.insert("SESSIONNAME".to_string(), "Services".to_string());
        env.insert("SESSION_ID".to_string(), "0".to_string());
        env.insert("SYSTEMROOT".to_string(), "C:\\Windows".to_string());

        let mut subsystems = Vec::new();
        subsystems.push(SubsystemType::Native);
        subsystems.push(SubsystemType::Win32);

        Self {
            session_id: 0,
            name: "Session0_Services".to_string(),
            state: SessionState::Initializing,
            supported_subsystems: subsystems,
            active_pids: Vec::new(),
            security_flags: SessionSecurityFlags::session_0(),
            environment_vars: env,
            desktop_heap_bytes: 512 * 1024, // Minimal 512 KiB headless desktop heap
            created_tick,
        }
    }

    /// Instantiates Session 1: The interactive handheld gaming session.
    pub fn new_session_1(user_name: &str, created_tick: u64) -> Self {
        let mut env = BTreeMap::new();
        env.insert("SESSIONNAME".to_string(), "Console".to_string());
        env.insert("SESSION_ID".to_string(), "1".to_string());
        env.insert("USERNAME".to_string(), user_name.to_string());
        env.insert("USERPROFILE".to_string(), "C:\\Users\\Gamer".to_string());
        env.insert("SYSTEMROOT".to_string(), "C:\\Windows".to_string());

        let mut subsystems = Vec::new();
        subsystems.push(SubsystemType::Native);
        subsystems.push(SubsystemType::Win32);
        subsystems.push(SubsystemType::Posix);

        Self {
            session_id: 1,
            name: "Session1_Console".to_string(),
            state: SessionState::Initializing,
            supported_subsystems: subsystems,
            active_pids: Vec::new(),
            security_flags: SessionSecurityFlags::session_1(),
            environment_vars: env,
            desktop_heap_bytes: 48 * 1024 * 1024, // 48 MiB high-res interactive gaming desktop heap
            created_tick,
        }
    }

    /// Adds a process to this session.
    pub fn add_process(&mut self, pid: u64) {
        if !self.active_pids.contains(&pid) {
            self.active_pids.push(pid);
        }
    }

    /// Removes a process from this session upon exit.
    pub fn remove_process(&mut self, pid: u64) -> bool {
        if let Some(pos) = self.active_pids.iter().position(|&p| p == pid) {
            self.active_pids.remove(pos);
            true
        } else {
            false
        }
    }

    /// Transitions the session to a new state.
    pub fn set_state(&mut self, new_state: SessionState) {
        self.state = new_state;
    }

    /// Inserts or updates an environment variable.
    pub fn set_env(&mut self, key: &str, value: &str) {
        self.environment_vars.insert(key.to_string(), value.to_string());
    }

    /// Retrieves an environment variable.
    pub fn get_env(&self, key: &str) -> Option<&String> {
        self.environment_vars.get(key)
    }

    /// Checks if a subsystem is supported in this session.
    pub fn supports_subsystem(&self, subsystem: SubsystemType) -> bool {
        self.supported_subsystems.contains(&subsystem)
    }
}

/// Verification test for session descriptor models and Session 0 / 1 isolation flags.
pub fn verify_session_descriptor_models() -> bool {
    // 1. Verify Session 0 setup and security constraints
    let mut sess0 = SessionDescriptor::new_session_0(100);
    if sess0.session_id != 0 || sess0.state != SessionState::Initializing {
        return false;
    }
    // Session 0 must forbid interactive UI (shatter attack prevention)
    if sess0.security_flags.allow_interactive_ui || !sess0.security_flags.isolated_session0 {
        return false;
    }
    if sess0.get_env("SESSIONNAME").map(|s| s.as_str()) != Some("Services") {
        return false;
    }

    // 2. Verify Session 1 setup and interactive gaming privileges
    let mut sess1 = SessionDescriptor::new_session_1("HandheldGamer", 150);
    if sess1.session_id != 1 || !sess1.security_flags.allow_interactive_ui {
        return false;
    }
    if sess1.security_flags.isolated_session0 {
        return false;
    }
    if sess1.desktop_heap_bytes < 10 * 1024 * 1024 {
        return false;
    }
    if sess1.get_env("USERNAME").map(|s| s.as_str()) != Some("HandheldGamer") {
        return false;
    }

    // 3. Process tracking operations
    sess0.add_process(101);
    sess0.add_process(102);
    if sess0.active_pids.len() != 2 || !sess0.active_pids.contains(&101) {
        return false;
    }
    sess0.remove_process(101);
    if sess0.active_pids.len() != 1 || sess0.active_pids.contains(&101) {
        return false;
    }

    // 4. State transitions
    sess1.set_state(SessionState::Active);
    if sess1.state != SessionState::Active {
        return false;
    }

    true
}


// ============================================================================
// Feature 2: Process Spawn Rules & Environment Block Inheritance Engine
// ============================================================================

/// Auto-restart policy applied by the supervisor when a service or process exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Never restart upon process termination.
    Never,
    /// Always restart immediately regardless of exit code.
    Always,
    /// Restart upon abnormal termination (non-zero exit code or fault) with backoff retry limit.
    OnCrashWithBackoff {
        /// Maximum restart attempts allowed within the retry window.
        max_retries: u32,
        /// Delay in milliseconds before executing next restart attempt.
        backoff_ms: u32,
    },
    /// Critical system process; crashing this process triggers emergency session lockdown.
    CriticalSessionHalt,
}

/// Rule governing how the session manager launches a specific process or subsystem daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpawnRule {
    /// Fully qualified executable path.
    pub binary_path: String,
    /// Subsystem classification required by binary.
    pub subsystem: SubsystemType,
    /// Target session into which process must be instantiated.
    pub target_session_id: u32,
    /// Default command-line parameters passed to executable.
    pub command_line: String,
    /// Execution priority class (0 = Idle, 1 = BelowNormal, 2 = Normal, 3 = AboveNormal, 4 = High, 5 = Realtime).
    pub priority_class: u8,
    /// Crash recovery and restart policy.
    pub restart_policy: RestartPolicy,
    /// Whether this daemon is automatically launched during session bootstrap.
    pub auto_start: bool,
    /// Whether the process executes with elevated system token privileges.
    pub elevated_token: bool,
}

impl ProcessSpawnRule {
    /// Creates a spawn rule for a native critical system service (e.g. csrss.exe).
    pub fn new_critical_service(path: &str, session_id: u32) -> Self {
        Self {
            binary_path: path.to_string(),
            subsystem: SubsystemType::Native,
            target_session_id: session_id,
            command_line: path.to_string(),
            priority_class: 4, // High priority
            restart_policy: RestartPolicy::CriticalSessionHalt,
            auto_start: true,
            elevated_token: true,
        }
    }

    /// Creates a spawn rule for an interactive user shell (e.g. gameshell.exe).
    pub fn new_interactive_shell(path: &str, session_id: u32) -> Self {
        Self {
            binary_path: path.to_string(),
            subsystem: SubsystemType::Win32,
            target_session_id: session_id,
            command_line: path.to_string(),
            priority_class: 3, // Above Normal for low input latency
            restart_policy: RestartPolicy::OnCrashWithBackoff {
                max_retries: 5,
                backoff_ms: 1000,
            },
            auto_start: true,
            elevated_token: false,
        }
    }
}

/// Environment block supervisor managing global variables and child process inheritance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentManager {
    /// Global system-wide environment variables inherited by all sessions.
    pub system_environment: BTreeMap<String, String>,
}

impl EnvironmentManager {
    /// Initializes environment manager with default KntonOS system variables.
    pub fn new() -> Self {
        let mut sys = BTreeMap::new();
        sys.insert("OS".to_string(), "Windows_NT".to_string());
        sys.insert("KTON_KERNEL".to_string(), "MicroKernel_x86_64".to_string());
        sys.insert("PROCESSOR_ARCHITECTURE".to_string(), "AMD64".to_string());
        sys.insert("SYSTEMDRIVE".to_string(), "C:".to_string());
        sys.insert("SYSTEMROOT".to_string(), "C:\\Windows".to_string());
        sys.insert("PATH".to_string(), "C:\\Windows\\System32;C:\\Knton\\Bin".to_string());

        Self {
            system_environment: sys,
        }
    }

    /// Composes a unified child process environment by merging:
    /// 1. System global environment
    /// 2. Session-specific environment
    /// 3. Per-spawn override environment
    pub fn build_process_environment(
        &self,
        session: &SessionDescriptor,
        overrides: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut merged = self.system_environment.clone();

        // Overlay session-specific variables
        for (k, v) in &session.environment_vars {
            merged.insert(k.clone(), v.clone());
        }

        // Overlay process-specific overrides
        for (k, v) in overrides {
            merged.insert(k.clone(), v.clone());
        }

        merged
    }
}

/// Verification test for process spawn rules and environment block inheritance.
pub fn verify_environment_inheritance_and_spawn_rules() -> bool {
    let env_mgr = EnvironmentManager::new();
    let sess1 = SessionDescriptor::new_session_1("GamerPro", 200);

    // 1. Verify spawn rule configurations
    let csrss_rule = ProcessSpawnRule::new_critical_service("C:\\Windows\\System32\\csrss.exe", 0);
    if csrss_rule.subsystem != SubsystemType::Native || csrss_rule.restart_policy != RestartPolicy::CriticalSessionHalt {
        return false;
    }
    if !csrss_rule.elevated_token || csrss_rule.target_session_id != 0 {
        return false;
    }

    let shell_rule = ProcessSpawnRule::new_interactive_shell("C:\\Knton\\Shell\\gameshell.exe", 1);
    if shell_rule.subsystem != SubsystemType::Win32 || shell_rule.elevated_token {
        return false;
    }
    match shell_rule.restart_policy {
        RestartPolicy::OnCrashWithBackoff { max_retries, backoff_ms } => {
            if max_retries != 5 || backoff_ms != 1000 {
                return false;
            }
        }
        _ => return false,
    }

    // 2. Verify environment inheritance hierarchy
    let mut overrides = BTreeMap::new();
    overrides.insert("GAME_FPS_TARGET".to_string(), "120".to_string());
    overrides.insert("SYSTEMROOT".to_string(), "C:\\CustomWin".to_string()); // Override system default

    let child_env = env_mgr.build_process_environment(&sess1, &overrides);

    // Should inherit system global
    if child_env.get("OS").map(|s| s.as_str()) != Some("Windows_NT") {
        return false;
    }
    // Should inherit session 1 specific
    if child_env.get("USERNAME").map(|s| s.as_str()) != Some("GamerPro") {
        return false;
    }
    if child_env.get("SESSION_ID").map(|s| s.as_str()) != Some("1") {
        return false;
    }
    // Should respect overrides
    if child_env.get("GAME_FPS_TARGET").map(|s| s.as_str()) != Some("120") {
        return false;
    }
    if child_env.get("SYSTEMROOT").map(|s| s.as_str()) != Some("C:\\CustomWin") {
        return false;
    }

    true
}


// ============================================================================
// Feature 3: Service Registry & Crash Recovery Auto-Restart Supervisor
// ============================================================================

/// Operational state of a supervised background service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    /// Service is registered but not running.
    Stopped,
    /// Service binary has been spawned and is initializing its IPC port.
    Starting,
    /// Service is healthy and reporting periodic heartbeats.
    Running,
    /// Service terminated abnormally or raised an unhandled fault.
    Crashed,
    /// Service is within backoff delay awaiting restart.
    Recovering,
    /// Service exceeded max retry attempts and has been disabled to prevent restart loop.
    Disabled,
}

/// Classification of services managed by the supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceType {
    /// Kernel-mode or native micro-kernel driver server.
    CoreDriver,
    /// System background daemon running in Session 0.
    SystemDaemon,
    /// Interactive desktop/gaming shell running in Session 1.
    UserShell,
    /// User application or gaming runtime.
    GameRuntime,
}

/// Action to be taken by supervisor in response to a service crash event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCrashAction {
    /// Restart immediately or after backoff delay.
    ScheduleRestart {
        /// Delay in milliseconds before attempting restart.
        delay_ms: u32,
    },
    /// Halt and lock down the target session due to critical failure.
    HaltSession {
        /// Target session identifier to lockdown.
        session_id: u32,
    },
    /// Do not restart; service exceeded max attempts and is marked Disabled.
    MarkDisabled,
    /// No action needed (normal voluntary exit).
    Ignore,
}

/// Descriptor tracking the lifecycle and health metrics of a supervised service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDescriptor {
    /// Unique service identifier.
    pub service_id: u32,
    /// Human-readable service name (e.g. "AudioSrv", "Csrss", "GameShell").
    pub name: String,
    /// Service functional category.
    pub service_type: ServiceType,
    /// Process spawn configuration and restart policy.
    pub spawn_rule: ProcessSpawnRule,
    /// Current execution state.
    pub state: ServiceState,
    /// PID of currently running process instance, if active.
    pub current_pid: Option<u64>,
    /// Number of consecutive crashes without sustained healthy execution.
    pub consecutive_crashes: u32,
    /// Cumulative crash count over service lifetime.
    pub total_crashes: u32,
    /// Tick timestamp when last crash occurred.
    pub last_crash_tick: u64,
    /// Tick timestamp when next restart is scheduled to execute.
    pub next_restart_tick: u64,
    /// Tick timestamp of most recent heartbeat signal.
    pub last_heartbeat_tick: u64,
}

impl ServiceDescriptor {
    /// Creates a new service descriptor in Stopped state.
    pub fn new(service_id: u32, name: &str, service_type: ServiceType, spawn_rule: ProcessSpawnRule) -> Self {
        Self {
            service_id,
            name: name.to_string(),
            service_type,
            spawn_rule,
            state: ServiceState::Stopped,
            current_pid: None,
            consecutive_crashes: 0,
            total_crashes: 0,
            last_crash_tick: 0,
            next_restart_tick: 0,
            last_heartbeat_tick: 0,
        }
    }
}

/// Central supervisor registry managing service life-cycle, heartbeats, and fault recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRegistry {
    /// Map of service ID to service descriptor.
    pub services: BTreeMap<u32, ServiceDescriptor>,
    /// Auto-incrementing service identifier counter.
    pub next_service_id: u32,
}

impl ServiceRegistry {
    /// Initializes an empty service registry.
    pub fn new() -> Self {
        Self {
            services: BTreeMap::new(),
            next_service_id: 1,
        }
    }

    /// Registers a new service and returns its unique ID.
    pub fn register_service(&mut self, name: &str, service_type: ServiceType, spawn_rule: ProcessSpawnRule) -> u32 {
        let sid = self.next_service_id;
        self.next_service_id += 1;

        let desc = ServiceDescriptor::new(sid, name, service_type, spawn_rule);
        self.services.insert(sid, desc);
        sid
    }

    /// Marks a service as Running and records its active PID.
    pub fn notify_service_started(&mut self, service_id: u32, pid: u64, current_tick: u64) -> bool {
        if let Some(svc) = self.services.get_mut(&service_id) {
            svc.state = ServiceState::Running;
            svc.current_pid = Some(pid);
            svc.last_heartbeat_tick = current_tick;
            true
        } else {
            false
        }
    }

    /// Records a periodic health heartbeat from a running service.
    pub fn notify_service_heartbeat(&mut self, service_id: u32, current_tick: u64) -> bool {
        if let Some(svc) = self.services.get_mut(&service_id) {
            if svc.state == ServiceState::Running {
                svc.last_heartbeat_tick = current_tick;
                // Sustained heartbeats reset consecutive crash counter
                if current_tick.saturating_sub(svc.last_crash_tick) > 10_000 {
                    svc.consecutive_crashes = 0;
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Processes a service crash event, evaluates restart policy, and returns action.
    pub fn handle_service_crash(&mut self, service_id: u32, exit_code: i32, current_tick: u64) -> ServiceCrashAction {
        let svc = match self.services.get_mut(&service_id) {
            Some(s) => s,
            None => return ServiceCrashAction::Ignore,
        };

        svc.current_pid = None;
        svc.total_crashes += 1;
        svc.consecutive_crashes += 1;
        svc.last_crash_tick = current_tick;

        // If exit code is clean 0 and policy is not Always, treat as voluntary stop
        if exit_code == 0 && svc.spawn_rule.restart_policy != RestartPolicy::Always {
            svc.state = ServiceState::Stopped;
            return ServiceCrashAction::Ignore;
        }

        match svc.spawn_rule.restart_policy {
            RestartPolicy::Never => {
                svc.state = ServiceState::Stopped;
                ServiceCrashAction::Ignore
            }
            RestartPolicy::CriticalSessionHalt => {
                svc.state = ServiceState::Crashed;
                ServiceCrashAction::HaltSession {
                    session_id: svc.spawn_rule.target_session_id,
                }
            }
            RestartPolicy::Always => {
                svc.state = ServiceState::Recovering;
                svc.next_restart_tick = current_tick + 100; // Fast 100ms restart
                ServiceCrashAction::ScheduleRestart { delay_ms: 100 }
            }
            RestartPolicy::OnCrashWithBackoff { max_retries, backoff_ms } => {
                if svc.consecutive_crashes > max_retries {
                    // Exceeded max retry budget: disable to prevent thrashing
                    svc.state = ServiceState::Disabled;
                    ServiceCrashAction::MarkDisabled
                } else {
                    svc.state = ServiceState::Recovering;
                    let delay = backoff_ms.saturating_mul(svc.consecutive_crashes);
                    svc.next_restart_tick = current_tick + (delay as u64);
                    ServiceCrashAction::ScheduleRestart { delay_ms: delay }
                }
            }
        }
    }

    /// Scans for services in Recovering state whose restart backoff timer has matured.
    pub fn poll_pending_restarts(&mut self, current_tick: u64) -> Vec<u32> {
        let mut ready = Vec::new();
        for (sid, svc) in self.services.iter_mut() {
            if svc.state == ServiceState::Recovering && current_tick >= svc.next_restart_tick {
                svc.state = ServiceState::Starting;
                ready.push(*sid);
            }
        }
        ready
    }
}

/// Verification test for service registry and crash recovery policies.
pub fn verify_service_registry_and_recovery() -> bool {
    let mut registry = ServiceRegistry::new();

    // 1. Register a critical service (csrss.exe)
    let csrss_rule = ProcessSpawnRule::new_critical_service("C:\\Windows\\System32\\csrss.exe", 0);
    let csrss_id = registry.register_service("csrss", ServiceType::SystemDaemon, csrss_rule);
    if csrss_id != 1 {
        return false;
    }

    // 2. Register an interactive shell with backoff (gameshell.exe)
    let mut shell_rule = ProcessSpawnRule::new_interactive_shell("C:\\Knton\\Shell\\gameshell.exe", 1);
    shell_rule.restart_policy = RestartPolicy::OnCrashWithBackoff {
        max_retries: 2,
        backoff_ms: 200,
    };
    let shell_id = registry.register_service("gameshell", ServiceType::UserShell, shell_rule);

    // 3. Start gameshell and test crash recovery with backoff
    registry.notify_service_started(shell_id, 501, 1000);
    if registry.services.get(&shell_id).map(|s| s.state) != Some(ServiceState::Running) {
        return false;
    }

    // Crash 1: Should schedule restart with backoff
    let action1 = registry.handle_service_crash(shell_id, -1, 1500);
    match action1 {
        ServiceCrashAction::ScheduleRestart { delay_ms } => {
            if delay_ms != 200 {
                return false;
            }
        }
        _ => return false,
    }
    if registry.services.get(&shell_id).map(|s| s.state) != Some(ServiceState::Recovering) {
        return false;
    }

    // Advance clock past backoff delay and poll restart
    let restarts = registry.poll_pending_restarts(1705);
    if !restarts.contains(&shell_id) {
        return false;
    }
    if registry.services.get(&shell_id).map(|s| s.state) != Some(ServiceState::Starting) {
        return false;
    }

    // Start again, then Crash 2
    registry.notify_service_started(shell_id, 502, 1750);
    let action2 = registry.handle_service_crash(shell_id, -1, 1800);
    match action2 {
        ServiceCrashAction::ScheduleRestart { delay_ms } => {
            if delay_ms != 400 { // 2nd consecutive crash: 200 * 2 = 400ms
                return false;
            }
        }
        _ => return false,
    }

    // Crash 3: Exceeds max_retries (2): should mark Disabled
    registry.poll_pending_restarts(2250);
    registry.notify_service_started(shell_id, 503, 2300);
    let action3 = registry.handle_service_crash(shell_id, -1, 2350);
    if action3 != ServiceCrashAction::MarkDisabled {
        return false;
    }
    if registry.services.get(&shell_id).map(|s| s.state) != Some(ServiceState::Disabled) {
        return false;
    }

    // 4. Test CriticalSessionHalt on critical service crash
    registry.notify_service_started(csrss_id, 401, 3000);
    let crit_action = registry.handle_service_crash(csrss_id, -1073741819_i32, 3500);
    match crit_action {
        ServiceCrashAction::HaltSession { session_id } => {
            if session_id != 0 {
                return false;
            }
        }
        _ => return false,
    }

    true
}


// ============================================================================
// Feature 4: KntPort IPC Endpoints & Multi-Session Rendezvous Router
// ============================================================================

/// Message packet transmitted across a KntPort LPC/ALPC endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KntPortMessage {
    /// Unique monotonically increasing message identifier.
    pub msg_id: u64,
    /// Process identifier of the sender.
    pub sender_pid: u64,
    /// Session identifier of the sender.
    pub sender_session_id: u32,
    /// Application-defined message type or LPC message code.
    pub msg_type: u32,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

/// Named IPC port endpoint facilitating client-server connection and messaging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KntPort {
    /// Unique port identifier.
    pub port_id: u32,
    /// Canonical NT path name for the port (e.g. "\\Smss\\Port", "\\Csrss\\ApiPort").
    pub name: String,
    /// Owning session that instantiated this port.
    pub owning_session_id: u32,
    /// Whether processes originating from external sessions can connect to this port.
    pub is_public_cross_session: bool,
    /// Maximum allowed payload size in bytes.
    pub max_message_size: u32,
    /// List of client PIDs currently connected to this port.
    pub connected_pids: Vec<u64>,
    /// Inbound message queue awaiting server consumption.
    pub message_queue: Vec<KntPortMessage>,
}

impl KntPort {
    /// Creates a new named KntPort endpoint.
    pub fn new(port_id: u32, name: &str, owning_session_id: u32, is_public: bool, max_message_size: u32) -> Self {
        Self {
            port_id,
            name: name.to_string(),
            owning_session_id,
            is_public_cross_session: is_public,
            max_message_size,
            connected_pids: Vec::new(),
            message_queue: Vec::new(),
        }
    }
}

/// Central IPC router enforcing session isolation and routing messages across KntPorts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRouter {
    /// Map of canonical port name to KntPort endpoint.
    pub ports: BTreeMap<String, KntPort>,
    /// Auto-incrementing port ID.
    pub next_port_id: u32,
    /// Monotonically increasing message ID.
    pub next_msg_id: u64,
}

impl PortRouter {
    /// Initializes an empty port router.
    pub fn new() -> Self {
        Self {
            ports: BTreeMap::new(),
            next_port_id: 1,
            next_msg_id: 1,
        }
    }

    /// Registers a new named port in the router namespace.
    pub fn register_port(
        &mut self,
        name: &str,
        owning_session_id: u32,
        is_public: bool,
        max_message_size: u32,
    ) -> Result<u32, &'static str> {
        if self.ports.contains_key(name) {
            return Err("PortAlreadyExists");
        }
        let pid = self.next_port_id;
        self.next_port_id += 1;

        let port = KntPort::new(pid, name, owning_session_id, is_public, max_message_size);
        self.ports.insert(name.to_string(), port);
        Ok(pid)
    }

    /// Connects a client process to a named port with cross-session authorization validation.
    pub fn connect_to_port(
        &mut self,
        port_name: &str,
        caller_pid: u64,
        caller_session_id: u32,
    ) -> Result<u32, &'static str> {
        let port = self.ports.get_mut(port_name).ok_or("PortNotFound")?;

        // Session boundary check:
        // If connecting across session boundary, the port must be explicitly designated public.
        if port.owning_session_id != caller_session_id && !port.is_public_cross_session {
            return Err("AccessDeniedCrossSessionPrivatePort");
        }

        if !port.connected_pids.contains(&caller_pid) {
            port.connected_pids.push(caller_pid);
        }

        Ok(port.port_id)
    }

    /// Sends a message into the specified port queue.
    pub fn send_message(
        &mut self,
        port_name: &str,
        sender_pid: u64,
        sender_session_id: u32,
        msg_type: u32,
        data: &[u8],
    ) -> Result<u64, &'static str> {
        let port = self.ports.get_mut(port_name).ok_or("PortNotFound")?;

        // Cross-session check
        if port.owning_session_id != sender_session_id && !port.is_public_cross_session {
            return Err("AccessDeniedCrossSession");
        }

        if data.len() > port.max_message_size as usize {
            return Err("MessageTooLarge");
        }

        let mid = self.next_msg_id;
        self.next_msg_id += 1;

        let msg = KntPortMessage {
            msg_id: mid,
            sender_pid,
            sender_session_id,
            msg_type,
            payload: data.to_vec(),
        };

        port.message_queue.push(msg);
        Ok(mid)
    }

    /// Receives and removes the oldest message from a port queue for the owning session.
    pub fn receive_message(&mut self, port_name: &str, receiver_session_id: u32) -> Option<KntPortMessage> {
        let port = self.ports.get_mut(port_name)?;
        // Only processes belonging to the owning session can dequeue messages
        if port.owning_session_id != receiver_session_id {
            return None;
        }

        if !port.message_queue.is_empty() {
            Some(port.message_queue.remove(0))
        } else {
            None
        }
    }
}

/// Verification test for KntPort IPC rendezvous and session boundary isolation.
pub fn verify_knt_port_ipc_rendezvous() -> bool {
    let mut router = PortRouter::new();

    // 1. Register a public system port in Session 0 (\Smss\PublicApi)
    let pub_port_id = router.register_port("\\Smss\\PublicApi", 0, true, 4096).unwrap();
    if pub_port_id != 1 {
        return false;
    }

    // 2. Register a private internal port in Session 0 (\Kernel\InternalLsa)
    let priv_port_id = router.register_port("\\Kernel\\InternalLsa", 0, false, 4096).unwrap();
    if priv_port_id != 2 {
        return false;
    }

    // 3. Client in Session 1 connects to public Session 0 port: Should SUCCEED
    let conn_ok = router.connect_to_port("\\Smss\\PublicApi", 601, 1);
    if conn_ok.is_err() {
        return false;
    }

    // 4. Client in Session 1 connects to private Session 0 port: Must FAIL with AccessDenied
    let conn_err = router.connect_to_port("\\Kernel\\InternalLsa", 601, 1);
    if conn_err != Err("AccessDeniedCrossSessionPrivatePort") {
        return false;
    }

    // 5. Client in Session 0 connects to private Session 0 port: Should SUCCEED
    let conn_sess0 = router.connect_to_port("\\Kernel\\InternalLsa", 402, 0);
    if conn_sess0.is_err() {
        return false;
    }

    // 6. Test message delivery through public port from Session 1 to Session 0
    let payload = b"CONNECT_SESSION_1_HANDHELD";
    let send_res = router.send_message("\\Smss\\PublicApi", 601, 1, 0x101, payload);
    if send_res.is_err() {
        return false;
    }

    // 7. Session 0 receives the message
    let recv_msg = router.receive_message("\\Smss\\PublicApi", 0);
    match recv_msg {
        Some(m) => {
            if m.sender_pid != 601 || m.sender_session_id != 1 || m.payload != payload {
                return false;
            }
        }
        None => return false,
    }

    // 8. Session 1 tries to unauthorized dequeue from Session 0 port: Must return None
    let unauthorized_recv = router.receive_message("\\Smss\\PublicApi", 1);
    if unauthorized_recv.is_some() {
        return false;
    }

    true
}


// ============================================================================
// Feature 5: Userspace Session Manager Daemon (smss) Orchestrator
// ============================================================================

/// Central supervisory daemon managing multi-session orchestration and service monitoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionManager {
    /// Active sessions indexed by session ID (0 = System, 1 = Interactive User).
    pub sessions: BTreeMap<u32, SessionDescriptor>,
    /// Background service registry and crash recovery supervisor.
    pub service_registry: ServiceRegistry,
    /// Multi-session LPC/ALPC port router.
    pub port_router: PortRouter,
    /// System and session environment block manager.
    pub environment_mgr: EnvironmentManager,
    /// Currently focused interactive session mapped to the display scanout.
    pub active_session_id: u32,
    /// Monotonically increasing synthetic process ID generator for test harness.
    pub next_pid: u64,
    /// Kernel tick counter.
    pub current_tick: u64,
}

impl SessionManager {
    /// Initializes a new SessionManager supervisor.
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            service_registry: ServiceRegistry::new(),
            port_router: PortRouter::new(),
            environment_mgr: EnvironmentManager::new(),
            active_session_id: 0,
            next_pid: 1000,
            current_tick: 0,
        }
    }

    /// Executes the standard operating system boot sequence:
    /// 1. Initializes Session 0 (Headless background services)
    /// 2. Registers public system rendezvous port (\\Smss\\Port)
    /// 3. Registers critical system daemon (csrss.exe) in Session 0
    /// 4. Initializes Session 1 (Interactive handheld user console)
    /// 5. Registers interactive user shell (gameshell.exe) in Session 1
    pub fn bootstrap_system_sessions(&mut self, start_tick: u64) {
        self.current_tick = start_tick;

        // 1. Session 0 (System Services)
        let mut sess0 = SessionDescriptor::new_session_0(start_tick);
        sess0.set_state(SessionState::Active);
        self.sessions.insert(0, sess0);

        // Register default cross-session SMSS port
        let _ = self.port_router.register_port("\\Smss\\Port", 0, true, 4096);

        // Register critical csrss service
        let csrss_rule = ProcessSpawnRule::new_critical_service("C:\\Windows\\System32\\csrss.exe", 0);
        let csrss_id = self.service_registry.register_service("csrss", ServiceType::SystemDaemon, csrss_rule);
        let _ = self.spawn_service_process(csrss_id);

        // 2. Session 1 (Interactive Handheld Gaming)
        let mut sess1 = SessionDescriptor::new_session_1("HandheldGamer", start_tick + 50);
        sess1.set_state(SessionState::Active);
        self.sessions.insert(1, sess1);

        // Register interactive shell
        let shell_rule = ProcessSpawnRule::new_interactive_shell("C:\\Knton\\Shell\\gameshell.exe", 1);
        let shell_id = self.service_registry.register_service("gameshell", ServiceType::UserShell, shell_rule);
        let _ = self.spawn_service_process(shell_id);

        self.active_session_id = 1;
    }

    /// Spawns a process instance for a registered service and binds it to its target session.
    pub fn spawn_service_process(&mut self, service_id: u32) -> Result<u64, &'static str> {
        let (target_session, _name) = {
            let svc = self.service_registry.services.get(&service_id).ok_or("ServiceNotFound")?;
            (svc.spawn_rule.target_session_id, svc.name.clone())
        };

        let session = self.sessions.get_mut(&target_session).ok_or("TargetSessionNotFound")?;
        if session.state != SessionState::Active {
            return Err("TargetSessionNotActive");
        }

        let pid = self.next_pid;
        self.next_pid += 1;

        session.add_process(pid);
        self.service_registry.notify_service_started(service_id, pid, self.current_tick);

        Ok(pid)
    }

    /// Handles process termination/crash, updates session tracking, and executes recovery policy.
    pub fn handle_process_exit(&mut self, pid: u64, exit_code: i32, current_tick: u64) -> ServiceCrashAction {
        self.current_tick = current_tick;

        // Remove process from its hosting session
        for session in self.sessions.values_mut() {
            session.remove_process(pid);
        }

        // Find service associated with this PID
        let matched_service_id = self.service_registry.services.iter()
            .find(|(_, s)| s.current_pid == Some(pid))
            .map(|(id, _)| *id);

        if let Some(sid) = matched_service_id {
            let action = self.service_registry.handle_service_crash(sid, exit_code, current_tick);
            if let ServiceCrashAction::HaltSession { session_id } = action {
                if let Some(sess) = self.sessions.get_mut(&session_id) {
                    sess.set_state(SessionState::Locked);
                }
            }
            action
        } else {
            ServiceCrashAction::Ignore
        }
    }

    /// Advances the supervisor tick, checks pending service restarts, and spawns recovered processes.
    pub fn tick(&mut self, current_tick: u64) -> Vec<u64> {
        self.current_tick = current_tick;
        let pending = self.service_registry.poll_pending_restarts(current_tick);
        let mut spawned = Vec::new();

        for sid in pending {
            if let Ok(pid) = self.spawn_service_process(sid) {
                spawned.push(pid);
            }
        }

        spawned
    }
}

/// Verification test for full session manager daemon orchestrator pipeline.
pub fn verify_session_manager_daemon_pipeline() -> bool {
    let mut sm = SessionManager::new();

    // 1. Bootstrap system sessions
    sm.bootstrap_system_sessions(100);

    // Verify Session 0 and Session 1 exist and are Active
    if sm.sessions.len() != 2 || sm.active_session_id != 1 {
        return false;
    }
    let sess0 = sm.sessions.get(&0).unwrap();
    let sess1 = sm.sessions.get(&1).unwrap();
    if sess0.state != SessionState::Active || sess1.state != SessionState::Active {
        return false;
    }
    if sess0.active_pids.is_empty() || sess1.active_pids.is_empty() {
        return false;
    }

    // 2. Identify the spawned gameshell PID in Session 1
    let shell_pid = *sess1.active_pids.first().unwrap();

    // 3. Simulate gameshell crash: Should trigger auto-restart backoff
    let crash_action = sm.handle_process_exit(shell_pid, -1, 500);
    match crash_action {
        ServiceCrashAction::ScheduleRestart { delay_ms } => {
            if delay_ms == 0 {
                return false;
            }
        }
        _ => return false,
    }

    // Process should be removed from Session 1 active list while recovering
    if sm.sessions.get(&1).unwrap().active_pids.contains(&shell_pid) {
        return false;
    }

    // 4. Advance time past backoff: tick should restart gameshell with a new PID
    let new_pids = sm.tick(2000);
    if new_pids.len() != 1 {
        return false;
    }
    let new_shell_pid = new_pids[0];
    if new_shell_pid == shell_pid {
        return false;
    }
    if !sm.sessions.get(&1).unwrap().active_pids.contains(&new_shell_pid) {
        return false;
    }

    // 5. Test inter-session communication from new shell to SMSS public port
    let payload = b"GET_SESSION_TOPOLOGY";
    let send_res = sm.port_router.send_message("\\Smss\\Port", new_shell_pid, 1, 0x10, payload);
    if send_res.is_err() {
        return false;
    }
    let msg = sm.port_router.receive_message("\\Smss\\Port", 0).unwrap();
    if msg.sender_pid != new_shell_pid || msg.sender_session_id != 1 {
        return false;
    }

    true
}
