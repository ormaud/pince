//! Core supervisor state machine and event loop.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use permission_engine::{Action, PolicyEngine};
use permission_engine::reload::{watch_and_reload, ReloadGuard};

use anyhow::{Context, Result};
use tokio::{
    net::UnixListener,
    sync::{mpsc, Mutex},
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

use pince_sandbox::{SandboxConfig, SandboxRunner};
use pince_scheduler::{DueJob, Scheduler};
use tool_registry::{
    builtin::{register_all, ProtectedPaths},
    schema::RiskLevel as RegistryRiskLevel,
    ToolRegistry,
};

use crate::{
    agent_handle::{AgentHandle, AgentSharedState, AgentStatus},
    audit::{AuditEntry, AuditLog, Decision},
    config::Config,
    health,
    secrets_injection,
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
    /// A cron job is due — spawn a headless agent and send it the prompt.
    ScheduledJob(DueJob),
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
    /// Agents spawned by the scheduler: agent_id → job name.
    headless_agents: HashMap<String, String>,
    /// Monotonic start time for uptime calculation.
    started_at: Instant,
    /// Number of registered (enabled) cron jobs.
    scheduler_jobs: usize,
    /// Permission policy engine for evaluating tool calls.
    policy_engine: Arc<PolicyEngine>,
    /// Hot-reload guard — kept alive for the supervisor's lifetime.
    #[allow(dead_code)]
    reload_guard: Option<ReloadGuard>,
    /// Registry of all tools known to the supervisor.
    tool_registry: Arc<ToolRegistry>,
    /// Sandbox backend for executing tools in isolation.
    sandbox: Arc<Mutex<SandboxRunner>>,
}

impl Supervisor {
    pub async fn new(config: Config) -> Result<Self> {
        let auth_token = load_or_create_token(&config.auth_token_file).await?;
        let audit = Arc::new(AuditLog::new(config.audit_log.clone()));
        let scheduler_jobs = config.cron_jobs.len();

        // Load the permission policy engine.
        let policy_engine = Arc::new(
            PolicyEngine::load(
                &config.permissions.global_policy,
                config.permissions.project_policy.as_deref(),
            )
            .context("loading permission policy")?,
        );

        // Build tool registry with all built-in tools.
        let mut registry = ToolRegistry::new();
        let protected = ProtectedPaths::default_protected();
        register_all(&mut registry, protected);
        let tool_registry = Arc::new(registry);

        // Build sandbox backend (mock for CI/tests, real for production).
        let sandbox_runner = build_sandbox_runner(&config.sandbox);
        let sandbox = Arc::new(Mutex::new(sandbox_runner));

        Ok(Self {
            config,
            audit,
            agents: HashMap::new(),
            children: HashMap::new(),
            frontends: HashMap::new(),
            pending_approvals: HashMap::new(),
            auth_token,
            headless_agents: HashMap::new(),
            started_at: Instant::now(),
            scheduler_jobs,
            policy_engine,
            reload_guard: None,
            tool_registry,
            sandbox,
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

        // Start hot-reload watcher for policy files (if enabled).
        if self.config.permissions.hot_reload {
            match watch_and_reload(
                self.policy_engine.clone(),
                self.config.permissions.global_policy.clone(),
                self.config.permissions.project_policy.clone(),
            ) {
                Ok(guard) => {
                    self.reload_guard = Some(guard);
                    tracing::info!("policy hot-reload watcher started");
                }
                Err(e) => {
                    tracing::warn!("could not start policy hot-reload watcher: {e}");
                }
            }
        }

        let (ev_tx, mut ev_rx) = mpsc::channel::<Event>(256);

        // Frontend acceptor task.
        {
            let ev_tx = ev_tx.clone();
            let heartbeat_timeout = Duration::from_secs(self.config.heartbeat_timeout_secs);
            tokio::spawn(accept_frontends(listener, ev_tx, heartbeat_timeout));
        }

        // Scheduler task: only start if there are cron jobs configured.
        if !self.config.cron_jobs.is_empty() {
            let (sched_tx, mut sched_rx) = mpsc::channel::<DueJob>(32);
            let jobs = self.config.cron_jobs.clone();
            tokio::spawn(async move {
                let mut scheduler = Scheduler::new(jobs, sched_tx);
                scheduler.run().await;
            });

            // Bridge: forward DueJob → Event::ScheduledJob.
            let ev_tx_bridge = ev_tx.clone();
            tokio::spawn(async move {
                while let Some(job) = sched_rx.recv().await {
                    if ev_tx_bridge.send(Event::ScheduledJob(job)).await.is_err() {
                        break;
                    }
                }
            });

            tracing::info!(jobs = self.config.cron_jobs.len(), "scheduler started");
        }

        // Main event loop.
        while let Some(event) = ev_rx.recv().await {
            if let Err(e) = self.handle_event(event, &ev_tx).await {
                tracing::error!("event handler error: {e}");
            }
        }
        Ok(())
    }

    async fn handle_event(&mut self, event: Event, ev_tx: &mpsc::Sender<Event>) -> Result<()> {
        match event {
            Event::FrontendConnected { id, tx } => {
                tracing::info!(frontend_id = %id, "frontend connected");
                self.frontends.insert(id, tx);
            }
            Event::FrontendMessage(id, msg) => {
                self.on_frontend_message(id, msg, ev_tx).await?;
            }
            Event::FrontendDisconnected(id) => {
                tracing::info!(frontend_id = %id, "frontend disconnected");
                self.frontends.remove(&id);
            }
            Event::AgentConnected { agent_id, handle } => {
                tracing::info!(agent_id = %agent_id, "agent connected");
                self.agents.insert(agent_id.clone(), handle.clone());

                // Spawn a sandbox for this agent.
                let mut sandbox = self.sandbox.lock().await;
                if let Err(e) = sandbox.spawn(&agent_id).await {
                    tracing::error!(agent_id = %agent_id, "failed to spawn sandbox: {e}");
                    // Proceed without sandbox — tool calls will fail gracefully.
                }
                drop(sandbox);

                // Build the tool list visible to this agent: all registered tools
                // except those unconditionally denied by policy.
                let visible_tools = build_visible_tools(
                    &self.tool_registry,
                    &self.policy_engine,
                    &agent_id,
                )
                .await;

                let agent_cfg = &self.config.agent;
                let init_msg = SupervisorMessage {
                    msg: Some(SupMsg::Init(Init {
                        config: Some(AgentConfig {
                            agent_id,
                            model: agent_cfg.default_model.clone(),
                            provider: agent_cfg.default_provider.clone(),
                            system_prompt: agent_cfg.system_prompt.clone(),
                            tools: visible_tools,
                            max_tokens: agent_cfg.max_tokens,
                            temperature: agent_cfg.temperature,
                        }),
                    })),
                };
                handle.send(init_msg).await?;
            }
            Event::AgentMessage(id, msg) => {
                self.on_agent_message(id, msg).await?;
            }
            Event::AgentDied(id) => {
                let job_name = self.headless_agents.remove(&id);
                tracing::warn!(agent_id = %id, job = ?job_name, "agent died");
                if let Some(name) = &job_name {
                    self.log_scheduled_outcome(&id, name, "died").await;
                }
                self.agents.remove(&id);
                // Destroy the agent's sandbox (no workspace cleanup on crash).
                let mut sandbox = self.sandbox.lock().await;
                if let Err(e) = sandbox.destroy(&id, false).await {
                    tracing::debug!(agent_id = %id, "sandbox destroy on agent death: {e}");
                }
                drop(sandbox);
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
            Event::ScheduledJob(job) => {
                self.on_scheduled_job(job, ev_tx).await?;
            }
        }
        Ok(())
    }

    // ── Scheduled job handler ─────────────────────────────────────────────────

    async fn on_scheduled_job(&mut self, job: DueJob, ev_tx: &mpsc::Sender<Event>) -> Result<()> {
        tracing::info!(job = %job.name, "cron job triggered, spawning agent");

        let agent_id = Uuid::new_v4().to_string();
        let heartbeat_timeout = Duration::from_secs(self.config.heartbeat_timeout_secs);

        match spawn::spawn_agent(&agent_id) {
            Ok(spawned) => {
                tracing::info!(agent_id = %agent_id, job = %job.name, "headless agent spawned");
                self.children.insert(agent_id.clone(), spawned.child);
                self.headless_agents.insert(agent_id.clone(), job.name.clone());

                // Authenticate and wire up the agent.
                let token = spawned.auth_token;
                let ev_tx2 = ev_tx.clone();
                let agent_id2 = agent_id.clone();
                let prompt = job.prompt.clone();
                tokio::spawn(async move {
                    let mut stream = spawned.stream;
                    if let Err(e) = auth::recv_auth_token(&mut stream, &token).await {
                        tracing::error!(agent_id = %agent_id2, "headless agent auth failed: {e}");
                        let _ = ev_tx2.send(Event::AgentDied(agent_id2)).await;
                        return;
                    }
                    // Wire up the connection, then send the initial prompt via AgentReady.
                    handle_agent_connection_with_prompt(
                        agent_id2,
                        stream,
                        ev_tx2,
                        heartbeat_timeout,
                        Some(prompt),
                    )
                    .await;
                });

                // Watchdog: kill the agent if it runs too long.
                let timeout_secs = job.timeout_secs;
                let ev_tx3 = ev_tx.clone();
                let agent_id3 = agent_id.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
                    tracing::warn!(agent_id = %agent_id3, timeout = timeout_secs, "headless agent timed out");
                    let _ = ev_tx3.send(Event::AgentDied(agent_id3)).await;
                });
            }
            Err(e) => {
                tracing::error!(job = %job.name, "failed to spawn headless agent: {e}");
            }
        }
        Ok(())
    }

    async fn log_scheduled_outcome(&self, agent_id: &str, job_name: &str, outcome: &str) {
        let entry = AuditEntry::new(
            agent_id.to_string(),
            "scheduled_job",
            format!("job={job_name}"),
            Decision::Allow,
            outcome.to_string(),
        );
        if let Err(e) = self.audit.append(entry).await {
            tracing::error!("audit log: {e}");
        }
    }

    // ── Frontend message handler ───────────────────────────────────────────────

    async fn on_frontend_message(
        &mut self,
        id: String,
        msg: FrontendMessage,
        ev_tx: &mpsc::Sender<Event>,
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
                    // If approved for the session, add a session overlay rule.
                    if decision == ApprovalDecision::ApproveSession {
                        self.policy_engine
                            .add_session_allow(
                                Some(pending.agent_id.clone()),
                                pending.tool_call.tool.clone(),
                            )
                            .await;
                    }
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
                let ev_tx = ev_tx.clone();
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
                self.headless_agents.remove(agent_id);
                // Destroy the agent's sandbox.
                let mut sandbox = self.sandbox.lock().await;
                if let Err(e) = sandbox.destroy(agent_id, false).await {
                    tracing::debug!(agent_id = %agent_id, "sandbox destroy on kill: {e}");
                }
                drop(sandbox);
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
                // Handle supervisor_health inline — no frontend approval needed.
                if tool_call.tool == health::TOOL_NAME {
                    let report = health::HealthReport {
                        uptime_secs: self.started_at.elapsed().as_secs(),
                        active_agents: self.agents.len(),
                        connected_frontends: self.frontends.len(),
                        scheduler_jobs: self.scheduler_jobs,
                        scheduler_enabled: self.scheduler_jobs > 0,
                    };
                    let result_json = serde_json::to_vec(&report).unwrap_or_default();
                    if let Some(h) = self.agents.get(&agent_id) {
                        let _ = h
                            .send(SupervisorMessage {
                                msg: Some(SupMsg::ToolResult(ToolCallResult {
                                    request_id: tool_call.request_id,
                                    result_json,
                                })),
                            })
                            .await;
                    }
                } else {
                    self.handle_tool_call(agent_id, tool_call).await?;
                }
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
                let is_headless = self.headless_agents.contains_key(&agent_id);
                if let Some(h) = self.agents.get(&agent_id) {
                    h.set_status(AgentStatus::Ready).await;
                }
                if is_headless {
                    // Log completion and clean up.
                    let job_name = self.headless_agents.remove(&agent_id).unwrap_or_default();
                    tracing::info!(agent_id = %agent_id, job = %job_name, "scheduled agent finished");
                    self.log_scheduled_outcome(&agent_id, &job_name, "completed").await;
                } else {
                    self.broadcast_frontend(SupervisorFrontendMessage {
                        msg: Some(SupFrontMsg::AgentResponseDone(
                            frontend_types::AgentResponseDone { agent_id },
                        )),
                    })
                    .await;
                }
            }

            Some(AgentMsg::Error(err)) => {
                tracing::error!(agent_id = %agent_id, "agent error: {}", err.message);
                let job_name = self.headless_agents.remove(&agent_id);
                if let Some(name) = &job_name {
                    self.log_scheduled_outcome(&agent_id, name, "error").await;
                }
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
        let tool_name = tool_call.tool.clone();
        let args_summary = String::from_utf8_lossy(&tool_call.arguments_json).to_string();

        // 1. Parse and validate arguments.
        let args_value: serde_json::Value =
            serde_json::from_slice(&tool_call.arguments_json).unwrap_or(serde_json::Value::Null);

        // 2. Evaluate policy.
        let policy_action = self
            .policy_engine
            .evaluate(&agent_id, &tool_name, &args_value)
            .await;

        match policy_action {
            Action::Allow => {
                tracing::info!(agent_id = %agent_id, tool = %tool_name, "policy: allow");

                // 3. Secret injection (stubbed — no-op for MVP).
                let resolved_args = secrets_injection::inject_secrets(args_value);

                // 4. Execute in sandbox.
                let exec_result = self
                    .sandbox
                    .lock()
                    .await
                    .execute(&agent_id, &tool_name, resolved_args)
                    .await;

                // 5. Send result back to agent and audit.
                let (result_msg, audit_decision, audit_note) = match exec_result {
                    Ok(result_json) => {
                        let msg = SupervisorMessage {
                            msg: Some(SupMsg::ToolResult(ToolCallResult {
                                request_id: request_id.clone(),
                                result_json: serde_json::to_vec(&result_json).unwrap_or_default(),
                            })),
                        };
                        (msg, Decision::Allow, "policy: allow; executed in sandbox")
                    }
                    Err(sandbox_err) => {
                        tracing::error!(
                            agent_id = %agent_id,
                            tool = %tool_name,
                            "sandbox execution failed: {sandbox_err}",
                        );
                        let msg = SupervisorMessage {
                            msg: Some(SupMsg::ToolResult(ToolCallResult {
                                request_id: request_id.clone(),
                                result_json: serde_json::to_vec(
                                    &serde_json::json!({"error": sandbox_err.to_string()}),
                                )
                                .unwrap_or_default(),
                            })),
                        };
                        (msg, Decision::Allow, "policy: allow; sandbox error")
                    }
                };
                if let Some(h) = self.agents.get(&agent_id) {
                    let _ = h.send(result_msg).await;
                }
                let entry = AuditEntry::new(
                    &agent_id, &tool_name, &args_summary, audit_decision, audit_note,
                );
                if let Err(e) = self.audit.append(entry).await {
                    tracing::error!("audit log: {e}");
                }
            }

            Action::Deny => {
                // Denied by policy — reject immediately, no user prompt.
                tracing::info!(agent_id = %agent_id, tool = %tool_name, "policy: deny");
                let denied_msg = SupervisorMessage {
                    msg: Some(SupMsg::ToolDenied(ToolCallDenied {
                        request_id: request_id.clone(),
                        reason: "denied by policy".into(),
                    })),
                };
                if let Some(h) = self.agents.get(&agent_id) {
                    let _ = h.send(denied_msg).await;
                }
                let entry = AuditEntry::new(
                    &agent_id, &tool_name, &args_summary, Decision::Deny, "policy: deny",
                );
                if let Err(e) = self.audit.append(entry).await {
                    tracing::error!("audit log: {e}");
                }
            }

            Action::Ask => {
                // Policy says ask the user — broadcast approval request to frontends.
                tracing::info!(agent_id = %agent_id, tool = %tool_name, "policy: ask (prompting user)");
                self.broadcast_frontend(SupervisorFrontendMessage {
                    msg: Some(SupFrontMsg::ApprovalRequest(
                        frontend_types::ApprovalRequest {
                            request_id: request_id.clone(),
                            agent_id: agent_id.clone(),
                            tool: tool_name.clone(),
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

                // Spawn resolver task — executes after user decision.
                let audit = self.audit.clone();
                let agents = self.agents.clone();
                let sandbox = self.sandbox.clone();

                tokio::spawn(async move {
                    let decision = resolve_rx.await.unwrap_or(ApprovalDecision::Deny);
                    let approved = matches!(
                        decision,
                        ApprovalDecision::ApproveOnce | ApprovalDecision::ApproveSession
                    );

                    let (audit_decision, result_summary) = if approved {
                        // Secret injection (stubbed — no-op for MVP).
                        let resolved_args = secrets_injection::inject_secrets(args_value);

                        // Execute in sandbox after approval.
                        let exec_result = sandbox
                            .lock()
                            .await
                            .execute(&agent_id, &tool_name, resolved_args)
                            .await;

                        match exec_result {
                            Ok(result_json) => {
                                let result_msg = SupervisorMessage {
                                    msg: Some(SupMsg::ToolResult(ToolCallResult {
                                        request_id: request_id.clone(),
                                        result_json: serde_json::to_vec(&result_json)
                                            .unwrap_or_default(),
                                    })),
                                };
                                if let Some(h) = agents.get(&agent_id) {
                                    let _ = h.send(result_msg).await;
                                }
                                (Decision::Ask, "approved by user; executed in sandbox".to_string())
                            }
                            Err(sandbox_err) => {
                                tracing::error!(
                                    agent_id = %agent_id,
                                    tool = %tool_name,
                                    "sandbox execution failed after approval: {sandbox_err}",
                                );
                                let result_msg = SupervisorMessage {
                                    msg: Some(SupMsg::ToolResult(ToolCallResult {
                                        request_id: request_id.clone(),
                                        result_json: serde_json::to_vec(
                                            &serde_json::json!({"error": sandbox_err.to_string()}),
                                        )
                                        .unwrap_or_default(),
                                    })),
                                };
                                if let Some(h) = agents.get(&agent_id) {
                                    let _ = h.send(result_msg).await;
                                }
                                (Decision::Ask, format!("approved; sandbox error: {sandbox_err}"))
                            }
                        }
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
                        (Decision::Deny, "denied by user".to_string())
                    };

                    let entry = AuditEntry::new(
                        agent_id, &tool_name, &args_summary, audit_decision, result_summary,
                    );
                    if let Err(e) = audit.append(entry).await {
                        tracing::error!("audit log: {e}");
                    }
                });
            }
        }

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
    handle_agent_connection_with_prompt(agent_id, stream, ev_tx, heartbeat_timeout, None).await;
}

/// Like `handle_agent_connection` but sends an initial prompt once the agent is ready.
async fn handle_agent_connection_with_prompt(
    agent_id: String,
    stream: tokio::net::UnixStream,
    ev_tx: mpsc::Sender<Event>,
    heartbeat_timeout: Duration,
    initial_prompt: Option<String>,
) {
    let shared = AgentSharedState::new(agent_id.clone());
    let (msg_tx, mut msg_rx) = mpsc::channel::<SupervisorMessage>(64);
    let handle = AgentHandle::new(shared.clone(), msg_tx.clone());

    let _ = ev_tx
        .send(Event::AgentConnected {
            agent_id: agent_id.clone(),
            handle,
        })
        .await;

    let (mut reader, mut writer) = stream.into_split();

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
    let mut prompt_sent = initial_prompt.is_none();
    let mut check_interval = time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            result = codec::read_message::<AgentMessage, _>(&mut reader) => {
                match result {
                    Ok(msg) => {
                        // When the agent becomes ready, send the initial prompt (headless mode).
                        if !prompt_sent {
                            if let Some(AgentMsg::Ready(_)) = &msg.msg {
                                if let Some(ref prompt) = initial_prompt {
                                    let user_msg = SupervisorMessage {
                                        msg: Some(SupMsg::UserMessage(UserMessage {
                                            content: prompt.clone(),
                                        })),
                                    };
                                    let _ = msg_tx.send(user_msg).await;
                                    prompt_sent = true;
                                }
                            }
                        }
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

// ── Sandbox / tool registry helpers ──────────────────────────────────────────

/// Choose the sandbox backend based on config and runtime environment.
///
/// Uses the mock backend if `/dev/kvm` is unavailable (CI/containers),
/// or if the Firecracker binary doesn't exist at the configured path.
fn build_sandbox_runner(config: &SandboxConfig) -> SandboxRunner {
    use std::path::PathBuf;
    let kvm_available = std::path::Path::new("/dev/kvm").exists();
    let fc_available = config.firecracker_binary.exists();
    if kvm_available && fc_available {
        let run_dir = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
            .join("pince")
            .join("sandboxes");
        tracing::info!("using real Firecracker sandbox backend");
        SandboxRunner::real(config.clone(), run_dir)
    } else {
        tracing::info!(
            kvm = kvm_available,
            fc = fc_available,
            "using mock sandbox backend (Firecracker not available)"
        );
        SandboxRunner::mock(config)
    }
}

/// Build the list of tool schemas to send to an agent at init time.
///
/// Filters out tools that are unconditionally denied by the policy engine.
async fn build_visible_tools(
    registry: &ToolRegistry,
    policy_engine: &PolicyEngine,
    agent_id: &str,
) -> Vec<pince_protocol::ToolSchema> {
    let mut visible = Vec::new();
    for schema in registry.schemas() {
        // Send a null-args probe to check whether this tool is unconditionally denied.
        let action = policy_engine
            .evaluate(agent_id, &schema.name, &serde_json::Value::Null)
            .await;
        if matches!(action, Action::Deny) {
            continue;
        }
        let risk_level = match schema.risk_level {
            RegistryRiskLevel::Safe => 0,      // ProtoRiskLevel::Safe
            RegistryRiskLevel::Sensitive => 1, // ProtoRiskLevel::Sensitive
            RegistryRiskLevel::Dangerous => 2, // ProtoRiskLevel::Dangerous
        };
        visible.push(pince_protocol::ToolSchema {
            name: schema.name,
            description: schema.description,
            input_schema_json: serde_json::to_vec(&schema.input_schema).unwrap_or_default(),
            risk_level,
        });
    }
    visible
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use permission_engine::{Action, PolicyEngine, PolicyFile};

    use crate::config::{Config, PermissionsConfig};
    use super::{build_sandbox_runner, build_visible_tools, Supervisor};

    /// Build a minimal test Config with a given PolicyEngine.
    fn test_config(dir: &TempDir) -> Config {
        use crate::config::AgentDefaults;
        use pince_sandbox::SandboxConfig;
        Config {
            frontend_socket: dir.path().join("supervisor.sock"),
            agent_socket_dir: dir.path().join("agents"),
            auth_token_file: dir.path().join("auth_token"),
            audit_log: dir.path().join("audit.jsonl"),
            heartbeat_timeout_secs: 30,
            config_file: dir.path().join("supervisor.toml"),
            cron_jobs: vec![],
            permissions: PermissionsConfig {
                global_policy: dir.path().join("policy.toml"),
                project_policy: None,
                hot_reload: false,
            },
            agent: AgentDefaults {
                default_model: "test-model".into(),
                default_provider: "test-provider".into(),
                system_prompt: "You are a test assistant.".into(),
                max_tokens: 1024,
                temperature: 0.0,
            },
            sandbox: SandboxConfig {
                workspace_base: dir.path().join("workspaces"),
                ..SandboxConfig::default()
            },
        }
    }

    #[tokio::test]
    async fn policy_allow_action() {
        let policy = PolicyFile::parse(r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "read_file"
action = "allow"
"#)
        .unwrap();
        let engine = PolicyEngine::from_policy(policy);
        let action = engine
            .evaluate("test-agent", "read_file", &serde_json::Value::Null)
            .await;
        assert_eq!(action, Action::Allow);
    }

    #[tokio::test]
    async fn policy_deny_action() {
        let policy = PolicyFile::parse(r#"
[defaults]
action = "ask"

[[rules]]
agent = "*"
tool = "shell_exec"
action = "deny"
"#)
        .unwrap();
        let engine = PolicyEngine::from_policy(policy);
        let action = engine
            .evaluate("test-agent", "shell_exec", &serde_json::Value::Null)
            .await;
        assert_eq!(action, Action::Deny);
    }

    #[tokio::test]
    async fn policy_ask_action() {
        let policy = PolicyFile::parse(r#"
[defaults]
action = "allow"

[[rules]]
agent = "*"
tool = "write_file"
action = "ask"
"#)
        .unwrap();
        let engine = PolicyEngine::from_policy(policy);
        let action = engine
            .evaluate("test-agent", "write_file", &serde_json::Value::Null)
            .await;
        assert_eq!(action, Action::Ask);
    }

    #[tokio::test]
    async fn policy_default_deny_unknown_tool() {
        let policy = PolicyFile::parse(r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "read_file"
action = "allow"
"#)
        .unwrap();
        let engine = PolicyEngine::from_policy(policy);
        // shell_exec has no rule → hits default deny
        let action = engine
            .evaluate("test-agent", "shell_exec", &serde_json::Value::Null)
            .await;
        assert_eq!(action, Action::Deny);
    }

    #[tokio::test]
    async fn session_overlay_overrides_deny() {
        let policy = PolicyFile::parse(r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "shell_exec"
action = "deny"
"#)
        .unwrap();
        let engine = PolicyEngine::from_policy(policy);
        // Normally denied…
        let before = engine
            .evaluate("test-agent", "shell_exec", &serde_json::Value::Null)
            .await;
        assert_eq!(before, Action::Deny);

        // Add a session overlay allow.
        engine
            .add_session_allow(Some("test-agent".to_string()), "shell_exec".to_string())
            .await;

        // Now it should be allowed.
        let after = engine
            .evaluate("test-agent", "shell_exec", &serde_json::Value::Null)
            .await;
        assert_eq!(after, Action::Allow);
    }

    #[tokio::test]
    async fn policy_condition_path_glob() {
        let policy = PolicyFile::parse(r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "write_file"
action = "allow"
conditions = { path_glob = "/workspace/**" }

[[rules]]
agent = "*"
tool = "write_file"
action = "deny"
conditions = { path_glob = "/workspace/.env" }
"#)
        .unwrap();
        let engine = PolicyEngine::from_policy(policy);

        // Allowed: inside workspace
        let args_ok = serde_json::json!({"path": "/workspace/src/main.rs"});
        let action = engine.evaluate("agent", "write_file", &args_ok).await;
        assert_eq!(action, Action::Allow);

        // Denied by default (no matching rule): outside workspace
        let args_outside = serde_json::json!({"path": "/etc/passwd"});
        let action = engine.evaluate("agent", "write_file", &args_outside).await;
        assert_eq!(action, Action::Deny);
    }

    #[tokio::test]
    async fn supervisor_loads_empty_policy_file() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        // global_policy path does not exist → PolicyEngine::load uses empty default
        let sup = Supervisor::new(config).await.unwrap();
        // Verify the engine starts with default (deny-all) when no file exists.
        let action = sup
            .policy_engine
            .evaluate("agent", "any_tool", &serde_json::Value::Null)
            .await;
        assert_eq!(action, Action::Deny);
    }

    #[tokio::test]
    async fn supervisor_initialises_tool_registry() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let sup = Supervisor::new(config).await.unwrap();
        // At least the built-in tools should be registered.
        assert!(!sup.tool_registry.is_empty(), "tool registry should not be empty");
        assert!(sup.tool_registry.contains("read_file"));
        assert!(sup.tool_registry.contains("write_file"));
        assert!(sup.tool_registry.contains("shell_exec"));
    }

    #[tokio::test]
    async fn build_visible_tools_excludes_denied() {
        use permission_engine::PolicyFile;
        // Policy: deny shell_exec for everyone, allow everything else.
        let policy = PolicyFile::parse(r#"
[defaults]
action = "allow"

[[rules]]
agent = "*"
tool = "shell_exec"
action = "deny"
"#)
        .unwrap();
        let engine = PolicyEngine::from_policy(policy);

        let mut registry = tool_registry::ToolRegistry::new();
        let protected = tool_registry::builtin::ProtectedPaths::default_protected();
        tool_registry::builtin::register_all(&mut registry, protected);

        let visible: Vec<pince_protocol::ToolSchema> =
            build_visible_tools(&registry, &engine, "test-agent").await;
        let names: Vec<&str> = visible.iter().map(|t| t.name.as_str()).collect();

        // shell_exec is denied → should be excluded.
        assert!(!names.contains(&"shell_exec"), "shell_exec should be filtered out");
        // Other tools should be present.
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
    }

    #[tokio::test]
    async fn build_visible_tools_includes_all_when_allow_all() {
        use permission_engine::PolicyFile;
        let policy = PolicyFile::parse(r#"
[defaults]
action = "allow"
"#)
        .unwrap();
        let engine = PolicyEngine::from_policy(policy);

        let mut registry = tool_registry::ToolRegistry::new();
        let protected = tool_registry::builtin::ProtectedPaths::default_protected();
        tool_registry::builtin::register_all(&mut registry, protected);
        let total = registry.len();

        let visible: Vec<pince_protocol::ToolSchema> =
            build_visible_tools(&registry, &engine, "test-agent").await;
        assert_eq!(visible.len(), total, "all tools should be visible when policy allows all");
    }

    #[tokio::test]
    async fn build_sandbox_runner_returns_mock_without_kvm() {
        use pince_sandbox::SandboxConfig;
        // /dev/kvm is almost certainly absent in the test container; if it is,
        // this test doesn't make a strong claim — it just checks that the
        // function doesn't panic.
        let dir = TempDir::new().unwrap();
        let cfg = SandboxConfig {
            workspace_base: dir.path().join("workspaces"),
            ..SandboxConfig::default()
        };
        let _runner = build_sandbox_runner(&cfg);
        // No panic → pass.
    }
}
