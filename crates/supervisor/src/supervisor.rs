//! Core supervisor state machine and event loop.

use std::{
    collections::HashMap,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use tokio::{
    net::UnixListener,
    sync::mpsc,
    time,
};
use uuid::Uuid;

use pince_protocol::{
    codec,
    frontend,
    frontend_types::{
        self,
        ApprovalDecision,
        FrontendMessage,
        SupervisorFrontendMessage,
        frontend_message::Msg as FrontMsg,
        supervisor_frontend_message::Msg as SupFrontMsg,
    },
    AgentMessage,
    SupervisorMessage,
    agent_message::Msg as AgentMsg,
    supervisor_message::Msg as SupMsg,
    AgentConfig,
    Init,
    ToolCallRequest,
    ToolCallResult,
    ToolCallDenied,
    UserMessage,
};

use pince_protocol::auth;

use crate::{
    agent_handle::{AgentHandle, AgentSharedState, AgentStatus},
    audit::{AuditEntry, AuditLog, Decision},
    config::Config,
    spawn,
};

/// A pending tool call waiting for frontend approval.
struct PendingApproval {
    #[allow(dead_code)]
    agent_id: String,
    #[allow(dead_code)]
    tool_call: ToolCallRequest,
    resolve: tokio::sync::oneshot::Sender<ApprovalDecision>,
}

/// Events flowing through the supervisor's central select loop.
#[allow(dead_code)]
pub(crate) enum Event {
    FrontendConnected {
        id: String,
        tx: mpsc::Sender<SupervisorFrontendMessage>,
    },
    FrontendMessage(String, FrontendMessage),
    FrontendDisconnected(String),
    AgentMessage(String, AgentMessage),
    AgentDied(String),
    AgentConnected {
        agent_id: String,
        handle: Arc<AgentHandle>,
    },
}

/// The supervisor process.
pub struct Supervisor {
    config: Config,
    audit: Arc<AuditLog>,
    agents: HashMap<String, Arc<AgentHandle>>,
    children: HashMap<String, tokio::process::Child>,
    frontends: HashMap<String, mpsc::Sender<SupervisorFrontendMessage>>,
    pending_approvals: HashMap<String, PendingApproval>,
    auth_token: String,
}

impl Supervisor {
    pub async fn new(config: Config) -> Result<Self> {
        let auth_token = load_or_create_token(&config.auth_token_file).await?;
        let audit = Arc::new(AuditLog::new(config.audit_log.clone()));
        Ok(Self {
            config,
            audit,
            agents: HashMap::new(),
            children: HashMap::new(),
            frontends: HashMap::new(),
            pending_approvals: HashMap::new(),
            auth_token,
        })
    }

    pub async fn run(mut self) -> Result<()> {
        if let Some(parent) = self.config.frontend_socket.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let _ = tokio::fs::remove_file(&self.config.frontend_socket).await;

        let listener = UnixListener::bind(&self.config.frontend_socket)
            .context("binding frontend socket")?;
        tracing::info!(socket = ?self.config.frontend_socket, "supervisor listening");

        let (ev_tx, mut ev_rx) = mpsc::channel::<Event>(256);

        // Frontend acceptor task.
        {
            let ev_tx = ev_tx.clone();
            let heartbeat_timeout = Duration::from_secs(self.config.heartbeat_timeout_secs);
            tokio::spawn(accept_frontends(listener, ev_tx, heartbeat_timeout));
        }

        // Main event loop.
        while let Some(event) = ev_rx.recv().await {
            if let Err(e) = self.handle_event(event, &ev_tx).await {
                tracing::error!("event handler error: {e}");
            }
        }
        Ok(())
    }

    async fn handle_event(&mut self, event: Event, _ev_tx: &mpsc::Sender<Event>) -> Result<()> {
        match event {
            Event::FrontendConnected { id, tx } => {
                tracing::info!(frontend_id = %id, "frontend connected");
                self.frontends.insert(id, tx);
            }
            Event::FrontendMessage(id, msg) => {
                self.on_frontend_message(id, msg, _ev_tx).await?;
            }
            Event::FrontendDisconnected(id) => {
                tracing::info!(frontend_id = %id, "frontend disconnected");
                self.frontends.remove(&id);
            }
            Event::AgentConnected { agent_id, handle } => {
                tracing::info!(agent_id = %agent_id, "agent connected");
                self.agents.insert(agent_id.clone(), handle.clone());
                let init_msg = SupervisorMessage {
                    msg: Some(SupMsg::Init(Init {
                        config: Some(AgentConfig {
                            agent_id,
                            model: String::new(),
                            provider: String::new(),
                            system_prompt: String::new(),
                            tools: Vec::new(),
                            max_tokens: 0,
                            temperature: 0.0,
                        }),
                    })),
                };
                handle.send(init_msg).await?;
            }
            Event::AgentMessage(id, msg) => {
                self.on_agent_message(id, msg).await?;
            }
            Event::AgentDied(id) => {
                tracing::warn!(agent_id = %id, "agent died");
                self.agents.remove(&id);
                self.broadcast_frontend(SupervisorFrontendMessage {
                    msg: Some(SupFrontMsg::AgentStatusChange(
                        frontend_types::AgentStatusChange {
                            agent_id: id,
                            status: frontend_types::AgentStatus::Dead.into(),
                        },
                    )),
                })
                .await;
            }
        }
        Ok(())
    }

    // ── Frontend message handler ───────────────────────────────────────────────

    async fn on_frontend_message(
        &mut self,
        id: String,
        msg: FrontendMessage,
        _ev_tx: &mpsc::Sender<Event>,
    ) -> Result<()> {
        match msg.msg {
            Some(FrontMsg::Auth(auth)) => {
                let success = auth.token == self.auth_token;
                let resp = if success {
                    frontend::auth_ok()
                } else {
                    frontend::auth_err("invalid token")
                };
                self.send_to(&id, resp).await;
            }

            Some(FrontMsg::SendMessage(send)) => {
                if let Some((_agent_id, handle)) = self.agents.iter().next() {
                    let user_msg = SupervisorMessage {
                        msg: Some(SupMsg::UserMessage(UserMessage {
                            content: send.content,
                        })),
                    };
                    handle.send(user_msg).await?;
                } else {
                    self.send_to(
                        &id,
                        SupervisorFrontendMessage {
                            msg: Some(SupFrontMsg::Error(frontend_types::FrontendError {
                                message: "no agents available".into(),
                            })),
                        },
                    )
                    .await;
                }
            }

            Some(FrontMsg::ApprovalResponse(resp)) => {
                if let Some(pending) = self.pending_approvals.remove(&resp.request_id) {
                    let decision = ApprovalDecision::try_from(resp.decision)
                        .unwrap_or(ApprovalDecision::Deny);
                    let _ = pending.resolve.send(decision);
                }
            }

            Some(FrontMsg::ListAgents(_)) => {
                let agents: Vec<frontend_types::AgentInfo> = self
                    .agents
                    .values()
                    .map(|h| {
                        let status = match h.status_nonblocking() {
                            AgentStatus::Starting => frontend_types::AgentStatus::Initializing,
                            AgentStatus::Ready => frontend_types::AgentStatus::Ready,
                            AgentStatus::Processing => frontend_types::AgentStatus::Busy,
                            AgentStatus::Dead => frontend_types::AgentStatus::Dead,
                        };
                        frontend_types::AgentInfo {
                            agent_id: h.shared.agent_id.clone(),
                            agent_type: String::new(),
                            status: status.into(),
                        }
                    })
                    .collect();
                self.send_to(
                    &id,
                    SupervisorFrontendMessage {
                        msg: Some(SupFrontMsg::AgentList(frontend_types::AgentList { agents })),
                    },
                )
                .await;
            }

            Some(FrontMsg::SpawnAgent(_spawn)) => {
                let agent_id = Uuid::new_v4().to_string();
                let ev_tx = _ev_tx.clone();
                let heartbeat_timeout =
                    Duration::from_secs(self.config.heartbeat_timeout_secs);

                match spawn::spawn_agent(&agent_id) {
                    Ok(spawned) => {
                        tracing::info!(agent_id = %agent_id, "agent process spawned");
                        self.children.insert(agent_id.clone(), spawned.child);

                        // Authenticate and wire up the agent connection in a background task.
                        let token = spawned.auth_token;
                        tokio::spawn(async move {
                            let mut stream = spawned.stream;

                            // Validate auth token from agent.
                            if let Err(e) = auth::recv_auth_token(&mut stream, &token).await {
                                tracing::error!(agent_id = %agent_id, "agent auth failed: {e}");
                                let _ = ev_tx.send(Event::AgentDied(agent_id)).await;
                                return;
                            }
                            tracing::debug!(agent_id = %agent_id, "agent authenticated");

                            handle_agent_connection(
                                agent_id,
                                stream,
                                ev_tx,
                                heartbeat_timeout,
                            )
                            .await;
                        });
                    }
                    Err(e) => {
                        tracing::error!("failed to spawn agent: {e}");
                        self.send_to(
                            &id,
                            SupervisorFrontendMessage {
                                msg: Some(SupFrontMsg::Error(frontend_types::FrontendError {
                                    message: format!("failed to spawn agent: {e}"),
                                })),
                            },
                        )
                        .await;
                    }
                }
            }

            Some(FrontMsg::KillAgent(kill)) => {
                let agent_id = &kill.agent_id;
                if let Some(handle) = self.agents.get(agent_id) {
                    // Send Shutdown message to the agent.
                    let shutdown_msg = SupervisorMessage {
                        msg: Some(SupMsg::Shutdown(pince_protocol::Shutdown {})),
                    };
                    let _ = handle.send(shutdown_msg).await;
                    tracing::info!(agent_id = %agent_id, "sent shutdown to agent");
                }
                // Kill the child process if it's still running.
                if let Some(mut child) = self.children.remove(agent_id) {
                    let _ = child.kill().await;
                    tracing::info!(agent_id = %agent_id, "killed agent process");
                }
                self.agents.remove(agent_id);
                self.broadcast_frontend(SupervisorFrontendMessage {
                    msg: Some(SupFrontMsg::AgentStatusChange(
                        frontend_types::AgentStatusChange {
                            agent_id: agent_id.clone(),
                            status: frontend_types::AgentStatus::Dead.into(),
                        },
                    )),
                })
                .await;
            }

            None => {
                tracing::warn!(frontend_id = %id, "received empty frontend message");
            }
        }
        Ok(())
    }

    // ── Agent message handler ─────────────────────────────────────────────────

    async fn on_agent_message(&mut self, agent_id: String, msg: AgentMessage) -> Result<()> {
        match msg.msg {
            Some(AgentMsg::Ready(_)) => {
                if let Some(h) = self.agents.get(&agent_id) {
                    h.set_status(AgentStatus::Ready).await;
                }
                self.broadcast_frontend(SupervisorFrontendMessage {
                    msg: Some(SupFrontMsg::AgentStatusChange(
                        frontend_types::AgentStatusChange {
                            agent_id,
                            status: frontend_types::AgentStatus::Ready.into(),
                        },
                    )),
                })
                .await;
            }

            Some(AgentMsg::Heartbeat(_)) => {
                if let Some(h) = self.agents.get(&agent_id) {
                    h.record_heartbeat().await;
                }
            }

            Some(AgentMsg::ToolCall(tool_call)) => {
                self.handle_tool_call(agent_id, tool_call).await?;
            }

            Some(AgentMsg::Response(chunk)) => {
                if let Some(h) = self.agents.get(&agent_id) {
                    h.set_status(AgentStatus::Processing).await;
                }
                self.broadcast_frontend(SupervisorFrontendMessage {
                    msg: Some(SupFrontMsg::AgentResponse(
                        frontend_types::AgentResponseChunk {
                            agent_id,
                            content: chunk.content,
                        },
                    )),
                })
                .await;
            }

            Some(AgentMsg::ResponseDone(_)) => {
                if let Some(h) = self.agents.get(&agent_id) {
                    h.set_status(AgentStatus::Ready).await;
                }
                self.broadcast_frontend(SupervisorFrontendMessage {
                    msg: Some(SupFrontMsg::AgentResponseDone(
                        frontend_types::AgentResponseDone { agent_id },
                    )),
                })
                .await;
            }

            Some(AgentMsg::Error(err)) => {
                tracing::error!(agent_id = %agent_id, "agent error: {}", err.message);
                self.agents.remove(&agent_id);
                self.broadcast_frontend(SupervisorFrontendMessage {
                    msg: Some(SupFrontMsg::AgentStatusChange(
                        frontend_types::AgentStatusChange {
                            agent_id,
                            status: frontend_types::AgentStatus::Dead.into(),
                        },
                    )),
                })
                .await;
            }

            None => {
                tracing::warn!(agent_id = %agent_id, "received empty agent message");
            }
        }
        Ok(())
    }

    // ── Tool call handling ────────────────────────────────────────────────────

    async fn handle_tool_call(
        &mut self,
        agent_id: String,
        tool_call: ToolCallRequest,
    ) -> Result<()> {
        let request_id = tool_call.request_id.clone();
        let args_summary = String::from_utf8_lossy(&tool_call.arguments_json).to_string();

        // Broadcast approval request to all frontends.
        self.broadcast_frontend(SupervisorFrontendMessage {
            msg: Some(SupFrontMsg::ApprovalRequest(
                frontend_types::ApprovalRequest {
                    request_id: request_id.clone(),
                    agent_id: agent_id.clone(),
                    tool: tool_call.tool.clone(),
                    arguments_json: tool_call.arguments_json.clone(),
                    risk_level: 0,
                },
            )),
        })
        .await;

        // Register pending approval.
        let (resolve_tx, resolve_rx) = tokio::sync::oneshot::channel::<ApprovalDecision>();
        self.pending_approvals.insert(
            request_id.clone(),
            PendingApproval {
                agent_id: agent_id.clone(),
                tool_call,
                resolve: resolve_tx,
            },
        );

        // Spawn resolver task.
        let audit = self.audit.clone();
        let tool_name = self
            .pending_approvals
            .get(&request_id)
            .map(|p| p.tool_call.tool.clone())
            .unwrap_or_default();
        let agents = self.agents.clone();

        tokio::spawn(async move {
            let decision = resolve_rx.await.unwrap_or(ApprovalDecision::Deny);
            let approved = matches!(
                decision,
                ApprovalDecision::ApproveOnce | ApprovalDecision::ApproveSession
            );
            let (audit_decision, result_summary) = if approved {
                let result_msg = SupervisorMessage {
                    msg: Some(SupMsg::ToolResult(ToolCallResult {
                        request_id: request_id.clone(),
                        result_json: serde_json::to_vec(&serde_json::json!({"ok": true}))
                            .unwrap_or_default(),
                    })),
                };
                if let Some(h) = agents.get(&agent_id) {
                    let _ = h.send(result_msg).await;
                }
                (Decision::Ask, "approved".to_string())
            } else {
                let denied_msg = SupervisorMessage {
                    msg: Some(SupMsg::ToolDenied(ToolCallDenied {
                        request_id,
                        reason: "denied by user".into(),
                    })),
                };
                if let Some(h) = agents.get(&agent_id) {
                    let _ = h.send(denied_msg).await;
                }
                (Decision::Deny, "denied".to_string())
            };

            let entry =
                AuditEntry::new(agent_id, &tool_name, &args_summary, audit_decision, result_summary);
            if let Err(e) = audit.append(entry).await {
                tracing::error!("audit log: {e}");
            }
        });

        Ok(())
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    async fn send_to(&self, id: &str, msg: SupervisorFrontendMessage) {
        if let Some(tx) = self.frontends.get(id) {
            let _ = tx.send(msg).await;
        }
    }

    async fn broadcast_frontend(&self, msg: SupervisorFrontendMessage) {
        for tx in self.frontends.values() {
            let _ = tx.send(msg.clone()).await;
        }
    }
}

// ── Free functions for acceptor / per-connection tasks ───────────────────────

async fn accept_frontends(
    listener: UnixListener,
    ev_tx: mpsc::Sender<Event>,
    _heartbeat_timeout: Duration,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let id = Uuid::new_v4().to_string();
                let ev_tx2 = ev_tx.clone();
                tokio::spawn(handle_frontend_connection(id, stream, ev_tx2));
            }
            Err(e) => {
                tracing::error!("frontend accept error: {e}");
                break;
            }
        }
    }
}

async fn handle_frontend_connection(
    id: String,
    stream: tokio::net::UnixStream,
    ev_tx: mpsc::Sender<Event>,
) {
    let (tx, mut rx) = mpsc::channel::<SupervisorFrontendMessage>(64);
    let _ = ev_tx
        .send(Event::FrontendConnected { id: id.clone(), tx })
        .await;

    let (mut reader, mut writer) = stream.into_split();

    // Writer task.
    let writer_id = id.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = codec::write_message(&mut writer, &msg).await {
                tracing::warn!(frontend_id = %writer_id, "frontend writer: {e}");
                break;
            }
        }
    });

    // Reader loop (this task IS the connection).
    loop {
        match codec::read_message::<FrontendMessage, _>(&mut reader).await {
            Ok(msg) => {
                if ev_tx
                    .send(Event::FrontendMessage(id.clone(), msg))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(e) => {
                tracing::debug!(frontend_id = %id, "frontend read: {e}");
                let _ = ev_tx.send(Event::FrontendDisconnected(id)).await;
                break;
            }
        }
    }
}

/// Connect an agent socket to the supervisor event loop.
/// Called after auth succeeds on a spawned agent's socketpair.
pub(crate) async fn handle_agent_connection(
    agent_id: String,
    stream: tokio::net::UnixStream,
    ev_tx: mpsc::Sender<Event>,
    heartbeat_timeout: Duration,
) {
    let shared = AgentSharedState::new(agent_id.clone());
    let (msg_tx, mut msg_rx) = mpsc::channel::<SupervisorMessage>(64);
    let handle = AgentHandle::new(shared.clone(), msg_tx);

    // Notify the supervisor.
    let _ = ev_tx
        .send(Event::AgentConnected {
            agent_id: agent_id.clone(),
            handle,
        })
        .await;

    let (mut reader, mut writer) = stream.into_split();

    // Writer task.
    let writer_agent_id = agent_id.clone();
    tokio::spawn(async move {
        while let Some(msg) = msg_rx.recv().await {
            if let Err(e) = codec::write_message(&mut writer, &msg).await {
                tracing::warn!(agent_id = %writer_agent_id, "agent writer: {e}");
                break;
            }
        }
    });

    // Reader loop with heartbeat watchdog.
    let mut check_interval = time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            result = codec::read_message::<AgentMessage, _>(&mut reader) => {
                match result {
                    Ok(msg) => {
                        if matches!(msg.msg, Some(AgentMsg::Heartbeat(_))) {
                            *shared.last_heartbeat.lock().await = tokio::time::Instant::now();
                        }
                        if ev_tx.send(Event::AgentMessage(agent_id.clone(), msg)).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::debug!(agent_id = %agent_id, "agent read: {e}");
                        let _ = ev_tx.send(Event::AgentDied(agent_id)).await;
                        break;
                    }
                }
            }
            _ = check_interval.tick() => {
                let age = shared.last_heartbeat.lock().await.elapsed();
                if age > heartbeat_timeout {
                    tracing::warn!(agent_id = %agent_id, ?age, "heartbeat timeout");
                    let _ = ev_tx.send(Event::AgentDied(agent_id)).await;
                    break;
                }
            }
        }
    }
}

// ── Auth token helper ─────────────────────────────────────────────────────────

async fn load_or_create_token(path: &std::path::Path) -> Result<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(t) => Ok(t.trim().to_string()),
        Err(_) => {
            let token = Uuid::new_v4().to_string();
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(path, &token).await?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
            tracing::info!(path = ?path, "created auth token");
            Ok(token)
        }
    }
}
