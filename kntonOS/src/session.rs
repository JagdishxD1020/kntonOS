//! Userspace Session Manager architecture (Session 0 services vs Session 1 interactive console).

/// Session identity classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionType {
    /// Non-interactive session reserved for core drivers and daemons.
    Session0Services,
    /// Primary handheld gaming console session.
    Session1GamingConsole,
}

/// Active userland session state.
#[derive(Debug, Clone, Copy)]
pub struct SessionDescriptor {
    pub session_id: u32,
    pub session_type: SessionType,
    pub active_processes: u32,
    pub is_locked: bool,
}

impl SessionDescriptor {
    pub const fn new(session_id: u32, session_type: SessionType) -> Self {
        Self {
            session_id,
            session_type,
            active_processes: 0,
            is_locked: false,
        }
    }
}

/// Verification test for userspace session manager.
pub fn verify_session_manager() -> bool {
    let s0 = SessionDescriptor::new(0, SessionType::Session0Services);
    let s1 = SessionDescriptor::new(1, SessionType::Session1GamingConsole);

    if s0.session_type != SessionType::Session0Services {
        return false;
    }
    if s1.session_type != SessionType::Session1GamingConsole {
        return false;
    }
    if s0.session_id == s1.session_id {
        return false;
    }

    true
}
