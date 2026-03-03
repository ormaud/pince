//! Session-scoped in-memory approval rules.
//!
//! When the user approves a tool call with "approve for session", the supervisor
//! can insert a temporary allow rule here. These rules are checked before the
//! static policy and are NOT persisted — they are cleared on restart.

/// An entry in the session overlay.
#[derive(Debug, Clone)]
struct SessionRule {
    /// `None` means "any agent".
    agent_id: Option<String>,
    tool: String,
}

/// The in-memory collection of session-scoped allow rules.
#[derive(Debug, Default)]
pub struct SessionOverlay {
    rules: Vec<SessionRule>,
}

impl SessionOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a session-scoped allow rule.
    ///
    /// - `agent_id = None`: allow for any agent.
    /// - `agent_id = Some(id)`: allow only for the named agent.
    pub fn add_allow(&mut self, agent_id: Option<String>, tool: String) {
        // Deduplicate.
        if !self.is_allowed(agent_id.as_deref().unwrap_or("*"), &tool) {
            self.rules.push(SessionRule { agent_id, tool });
        }
    }

    /// Returns true if there is a session-scoped allow for (agent_id, tool).
    pub fn is_allowed(&self, agent_id: &str, tool: &str) -> bool {
        self.rules.iter().any(|r| {
            r.tool == tool
                && match &r.agent_id {
                    None => true,
                    Some(id) => id == agent_id,
                }
        })
    }

    /// Remove all session rules (called on supervisor restart / session end).
    pub fn clear(&mut self) {
        self.rules.clear();
    }
}
