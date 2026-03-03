//! Representation of a live sub-agent process inside the supervisor.

use std::sync::Arc;
use tokio::{
    sync::{mpsc, Mutex},
    time::Instant,
};

use pince_protocol::SupervisorMessage;

/// Lifecycle state of an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    Starting,
    Ready,
    Processing,
    Dead,
}

/// All mutable state for a running agent, shared across tasks.
pub struct AgentSharedState {
    pub agent_id: String,
    pub last_heartbeat: Mutex<Instant>,
    pub status: Mutex<AgentStatus>,
}

impl AgentSharedState {
    pub fn new(agent_id: String) -> Arc<Self> {
        Arc::new(Self {
            agent_id,
            last_heartbeat: Mutex::new(Instant::now()),
            status: Mutex::new(AgentStatus::Starting),
        })
    }
}

/// Handle to a running agent — used by the supervisor to send messages and query state.
pub struct AgentHandle {
    pub shared: Arc<AgentSharedState>,
    /// Channel to the per-agent writer task.
    tx: mpsc::Sender<SupervisorMessage>,
}

impl AgentHandle {
    pub fn new(shared: Arc<AgentSharedState>, tx: mpsc::Sender<SupervisorMessage>) -> Arc<Self> {
        Arc::new(Self { shared, tx })
    }

    /// Enqueue a message to be sent to the agent.
    pub async fn send(&self, msg: SupervisorMessage) -> anyhow::Result<()> {
        self.tx
            .send(msg)
            .await
            .map_err(|e| anyhow::anyhow!("agent channel closed: {e}"))
    }

    pub async fn record_heartbeat(&self) {
        *self.shared.last_heartbeat.lock().await = Instant::now();
    }

    pub async fn set_status(&self, status: AgentStatus) {
        *self.shared.status.lock().await = status;
    }

    pub async fn heartbeat_age(&self) -> tokio::time::Duration {
        self.shared.last_heartbeat.lock().await.elapsed()
    }

    /// Non-blocking status read (falls back to Starting if locked).
    pub fn status_nonblocking(&self) -> AgentStatus {
        self.shared
            .status
            .try_lock()
            .map(|g| g.clone())
            .unwrap_or(AgentStatus::Starting)
    }
}
