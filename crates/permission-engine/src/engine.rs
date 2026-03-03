//! The core `PolicyEngine` that evaluates tool calls against a merged policy.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::policy::{Action, PolicyFile, merge_policies};
use crate::session::SessionOverlay;

/// A snapshot of the current merged policy plus a session overlay.
///
/// Cheaply cloneable via `Arc`.
#[derive(Clone)]
pub struct PolicyEngine {
    inner: Arc<RwLock<Inner>>,
    session: Arc<RwLock<SessionOverlay>>,
}

struct Inner {
    policy: PolicyFile,
}

impl PolicyEngine {
    /// Load the engine from the user-global policy and an optional project-local policy.
    ///
    /// If `global_path` does not exist, an empty (deny-all) policy is used.
    /// If `project_path` does not exist, only the global policy is used.
    pub fn load(global_path: &Path, project_path: Option<&Path>) -> Result<Self> {
        let global = if global_path.exists() {
            PolicyFile::load(global_path)?
        } else {
            PolicyFile::default()
        };

        let project = match project_path {
            Some(p) if p.exists() => {
                let proj = PolicyFile::load(p)?;
                proj.validate_project_local()?;
                Some(proj)
            }
            _ => None,
        };

        let merged = merge_policies(global, project);
        Ok(Self {
            inner: Arc::new(RwLock::new(Inner { policy: merged })),
            session: Arc::new(RwLock::new(SessionOverlay::new())),
        })
    }

    /// Create an engine from an already-merged `PolicyFile` (for testing).
    pub fn from_policy(policy: PolicyFile) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner { policy })),
            session: Arc::new(RwLock::new(SessionOverlay::new())),
        }
    }

    /// Evaluate a tool call and return the decision.
    ///
    /// Evaluation order:
    /// 1. Session overlay (temporary `allow` rules set by the user during this run)
    /// 2. Explicit `deny` rules (with condition matching)
    /// 3. Explicit `allow` rules (with condition matching)
    /// 4. Explicit `ask` rules (with condition matching)
    /// 5. Default action
    pub async fn evaluate(
        &self,
        agent_id: &str,
        tool: &str,
        args: &serde_json::Value,
    ) -> Action {
        // Check session overlay first.
        {
            let overlay = self.session.read().await;
            if overlay.is_allowed(agent_id, tool) {
                tracing::debug!(agent=%agent_id, tool=%tool, "session overlay: allow");
                return Action::Allow;
            }
        }

        let inner = self.inner.read().await;
        self.evaluate_against_policy(&inner.policy, agent_id, tool, args)
    }

    /// Synchronous evaluation for use in non-async contexts (e.g., unit tests).
    pub fn evaluate_sync(
        &self,
        agent_id: &str,
        tool: &str,
        args: &serde_json::Value,
    ) -> Action {
        // Note: this blocks; only appropriate in tests or non-async call sites.
        let rt = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            rt.block_on(self.evaluate(agent_id, tool, args))
        })
    }

    /// Add a session-scoped allow rule (user approved "for this session").
    pub async fn add_session_allow(&self, agent_id: Option<String>, tool: String) {
        let mut overlay = self.session.write().await;
        overlay.add_allow(agent_id, tool.clone());
        tracing::info!("session overlay: added allow for tool={tool}");
    }

    /// Hot-reload: atomically replace the current policy from updated files.
    pub async fn reload(
        &self,
        global_path: &Path,
        project_path: Option<&Path>,
    ) -> Result<()> {
        let global = if global_path.exists() {
            PolicyFile::load(global_path)?
        } else {
            PolicyFile::default()
        };

        let project = match project_path {
            Some(p) if p.exists() => {
                let proj = PolicyFile::load(p)?;
                proj.validate_project_local()?;
                Some(proj)
            }
            _ => None,
        };

        let merged = merge_policies(global, project);
        let mut inner = self.inner.write().await;
        inner.policy = merged;
        tracing::info!("policy engine: reloaded policy");
        Ok(())
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn evaluate_against_policy(
        &self,
        policy: &PolicyFile,
        agent_id: &str,
        tool: &str,
        args: &serde_json::Value,
    ) -> Action {
        // Phase 1: explicit deny rules.
        for rule in &policy.rules {
            if rule.action == Action::Deny
                && rule.agent.matches(agent_id)
                && rule.tool == tool
                && rule.conditions.as_ref().is_none_or(|c| c.matches(args))
            {
                tracing::debug!(agent=%agent_id, tool=%tool, "policy: deny");
                return Action::Deny;
            }
        }

        // Phase 2: explicit allow rules.
        for rule in &policy.rules {
            if rule.action == Action::Allow
                && rule.agent.matches(agent_id)
                && rule.tool == tool
                && rule.conditions.as_ref().is_none_or(|c| c.matches(args))
            {
                tracing::debug!(agent=%agent_id, tool=%tool, "policy: allow");
                return Action::Allow;
            }
        }

        // Phase 3: explicit ask rules.
        for rule in &policy.rules {
            if rule.action == Action::Ask
                && rule.agent.matches(agent_id)
                && rule.tool == tool
                && rule.conditions.as_ref().is_none_or(|c| c.matches(args))
            {
                tracing::debug!(agent=%agent_id, tool=%tool, "policy: ask");
                return Action::Ask;
            }
        }

        // Phase 4: fall through to default.
        tracing::debug!(agent=%agent_id, tool=%tool, action=?policy.defaults.action, "policy: default");
        policy.defaults.action
    }
}
