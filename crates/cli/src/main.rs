//! `pince` CLI binary.
//!
//! Usage:
//!   pince                         — chat mode (default)
//!   pince secret set <name>       — read value from stdin, store secret
//!   pince secret list             — print secret names
//!   pince secret delete <name>    — remove a secret
//!   pince secret show <name>      — print secret value (human use only)

use std::{
    io::{self, BufRead, Read, Write as IoWrite},
    path::PathBuf,
};

use anyhow::{bail, Result};
use secrets::SecretStore;

use pince_protocol::{
    codec::{read_message, write_message},
    frontend::{default_auth_token_path, read_token_from_file, send_auth, recv_auth_result},
    frontend_types::{
        FrontendMessage, SupervisorFrontendMessage,
        SendMessage, ListAgents, SpawnAgent, KillAgent,
        frontend_message::Msg as FMsg,
        supervisor_frontend_message::Msg as SFMsg,
        ApprovalDecision,
        AgentStatus,
    },
};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("secret") => handle_secret(&args[2..]),
        Some(other) => bail!(
            "unknown subcommand: {other}\nUsage: pince [secret <set|list|delete|show>]"
        ),
        None => run_chat().await,
    }
}

// ── Chat mode ─────────────────────────────────────────────────────────────────

/// Messages from the stdin thread to the main loop.
enum StdinEvent {
    Line(String),
    /// Approval decision from user prompt.
    Approval(FrontendMessage),
}

async fn run_chat() -> Result<()> {
    // Read auth token.
    let token_path = auth_token_path();
    let token = read_token_from_file(&token_path).map_err(|e| {
        anyhow::anyhow!(
            "cannot read auth token from {}: {e}\nIs the pince supervisor running?",
            token_path.display()
        )
    })?;

    // Connect to supervisor socket.
    let sock_path = supervisor_socket_path();
    let mut stream = tokio::net::UnixStream::connect(&sock_path).await.map_err(|e| {
        anyhow::anyhow!(
            "cannot connect to supervisor at {}: {e}\nIs the pince supervisor running?",
            sock_path.display()
        )
    })?;

    // Authenticate.
    send_auth(&mut stream, token).await?;
    recv_auth_result(&mut stream).await.map_err(|e| anyhow::anyhow!("auth failed: {e}"))?;

    eprintln!("Connected to pince supervisor. Type /quit to exit.");
    eprintln!("Slash commands: /agents  /spawn <type>  /kill <id>  /quit");
    eprintln!();

    // Split stream for concurrent reading/writing.
    let (mut reader, mut writer) = stream.into_split();

    // Channel from stdin thread -> main loop.
    let (input_tx, mut input_rx) = tokio::sync::mpsc::channel::<StdinEvent>(16);

    // Channel for approval requests: reader task -> stdin thread.
    let (approval_req_tx, approval_req_rx) =
        std::sync::mpsc::channel::<pince_protocol::frontend_types::ApprovalRequest>();

    // Spawn the stdin thread — handles both normal input and approval prompts.
    let input_tx_clone = input_tx.clone();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut lines = stdin.lock().lines();
        loop {
            // Check for pending approval requests before blocking on stdin.
            if let Ok(req) = approval_req_rx.try_recv() {
                handle_approval_prompt(&req, &input_tx_clone);
                continue;
            }

            // Print prompt.
            print!("pince> ");
            io::stdout().flush().ok();

            match lines.next() {
                Some(Ok(l)) => {
                    if input_tx_clone.blocking_send(StdinEvent::Line(l)).is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    });

    // Spawn a task to read supervisor messages.
    let reader_task = tokio::spawn(async move {
        loop {
            match read_message::<SupervisorFrontendMessage, _>(&mut reader).await {
                Ok(msg) => {
                    if !print_supervisor_message(msg, &approval_req_tx) {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("\r[disconnected: {e}]");
                    break;
                }
            }
        }
    });

    loop {
        tokio::select! {
            line = input_rx.recv() => {
                let event = match line {
                    Some(e) => e,
                    None => break,
                };
                match event {
                    StdinEvent::Approval(msg) => {
                        write_message(&mut writer, &msg).await?;
                    }
                    StdinEvent::Line(line) => {
                        let line = line.trim().to_string();
                        if line.is_empty() {
                            continue;
                        }
                        if line.starts_with('/') {
                            let msg = match parse_slash_command(&line) {
                                Ok(Some(m)) => m,
                                Ok(None) => break, // /quit
                                Err(e) => {
                                    eprintln!("[error] {e}");
                                    continue;
                                }
                            };
                            write_message(&mut writer, &msg).await?;
                        } else {
                            let msg = FrontendMessage {
                                msg: Some(FMsg::SendMessage(SendMessage {
                                    content: line,
                                    agent_id: String::new(),
                                })),
                            };
                            write_message(&mut writer, &msg).await?;
                        }
                    }
                }
            }
        }
    }

    // Clean shutdown.
    drop(writer);
    reader_task.abort();
    Ok(())
}

/// Handle an approval prompt on the stdin thread — reads user input synchronously.
fn handle_approval_prompt(
    req: &pince_protocol::frontend_types::ApprovalRequest,
    tx: &tokio::sync::mpsc::Sender<StdinEvent>,
) {
    let risk = match req.risk_level {
        2 => "dangerous",
        1 => "sensitive",
        _ => "safe",
    };
    let args = String::from_utf8_lossy(&req.arguments_json);
    println!();
    println!("[APPROVAL REQUIRED]");
    println!(
        "Agent \"{}\" wants to execute: {}",
        short_id(&req.agent_id),
        req.tool
    );
    println!("Arguments: {args}");
    println!("Risk level: {risk}");
    print!("Allow? [y]es once / [a]lways this session / [n]o: ");
    io::stdout().flush().ok();

    let mut buf = [0u8; 1];
    let decision = if io::stdin().read(&mut buf).is_ok() {
        match buf[0] {
            b'y' | b'Y' => ApprovalDecision::ApproveOnce,
            b'a' | b'A' => ApprovalDecision::ApproveSession,
            _ => ApprovalDecision::Deny,
        }
    } else {
        ApprovalDecision::Deny
    };
    println!();

    let label = match decision {
        ApprovalDecision::ApproveOnce => "approved (once)",
        ApprovalDecision::ApproveSession => "approved (session)",
        ApprovalDecision::Deny => "denied",
    };
    println!("[approval] {label}");

    let resp_msg = FrontendMessage {
        msg: Some(FMsg::ApprovalResponse(
            pince_protocol::frontend_types::ApprovalResponse {
                request_id: req.request_id.clone(),
                decision: decision as i32,
            },
        )),
    };
    if tx.blocking_send(StdinEvent::Approval(resp_msg)).is_err() {
        eprintln!("[error] failed to send approval response — channel closed");
    }
}

/// Print a message received from the supervisor.
/// Returns false if the session should end.
fn print_supervisor_message(
    msg: SupervisorFrontendMessage,
    approval_req_tx: &std::sync::mpsc::Sender<pince_protocol::frontend_types::ApprovalRequest>,
) -> bool {
    match msg.msg {
        Some(SFMsg::AgentResponse(chunk)) => {
            let agent = short_id(&chunk.agent_id);
            print!("\r[agent:{agent}] {}", chunk.content);
            io::stdout().flush().ok();
        }

        Some(SFMsg::AgentResponseDone(_)) => {
            println!();
        }

        Some(SFMsg::ToolCallEvent(ev)) => {
            let args = String::from_utf8_lossy(&ev.arguments_json);
            println!("\r[tool:{}] args: {args}", ev.tool);
        }

        Some(SFMsg::ToolResultEvent(ev)) => {
            let result = String::from_utf8_lossy(&ev.result_json);
            println!("[tool-result:{}] {result}", short_id(&ev.request_id));
        }

        Some(SFMsg::ApprovalRequest(req)) => {
            // Send to the stdin thread for synchronous prompting.
            if approval_req_tx.send(req).is_err() {
                eprintln!("[error] stdin thread gone — cannot prompt for approval");
            }
        }

        Some(SFMsg::AgentList(list)) => {
            if list.agents.is_empty() {
                println!("[agents] (none)");
            } else {
                for a in &list.agents {
                    let status = match a.status {
                        s if s == AgentStatus::Ready as i32 => "ready",
                        s if s == AgentStatus::Busy as i32 => "busy",
                        s if s == AgentStatus::Dead as i32 => "dead",
                        _ => "initializing",
                    };
                    println!(
                        "[agents] {} ({}) — {status}",
                        short_id(&a.agent_id),
                        a.agent_type
                    );
                }
            }
        }

        Some(SFMsg::AgentStatusChange(change)) => {
            let status = match change.status {
                s if s == AgentStatus::Ready as i32 => "ready",
                s if s == AgentStatus::Busy as i32 => "busy",
                s if s == AgentStatus::Dead as i32 => "dead (exited)",
                _ => "initializing",
            };
            println!(
                "[status] agent {} is now {status}",
                short_id(&change.agent_id)
            );
        }

        Some(SFMsg::Error(err)) => {
            eprintln!("[error] {}", err.message);
        }

        Some(SFMsg::AuthResult(_)) => {
            // Should not appear after handshake — ignore.
        }

        None => {}
    }
    true
}

fn parse_slash_command(line: &str) -> Result<Option<FrontendMessage>> {
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    match parts[0] {
        "/quit" | "/exit" | "/q" => Ok(None),

        "/agents" => Ok(Some(FrontendMessage {
            msg: Some(FMsg::ListAgents(ListAgents {})),
        })),

        "/spawn" => {
            let agent_type = parts.get(1).copied().unwrap_or("default");
            Ok(Some(FrontendMessage {
                msg: Some(FMsg::SpawnAgent(SpawnAgent {
                    agent_type: agent_type.to_string(),
                })),
            }))
        }

        "/kill" => {
            let id = parts
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("Usage: /kill <agent-id>"))?;
            Ok(Some(FrontendMessage {
                msg: Some(FMsg::KillAgent(KillAgent {
                    agent_id: id.to_string(),
                })),
            }))
        }

        other => bail!(
            "unknown command: {other}. Try /agents, /spawn <type>, /kill <id>, /quit"
        ),
    }
}

/// Return the first 8 chars of an ID for display (safe for non-ASCII).
fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

// ── Path helpers ──────────────────────────────────────────────────────────────

fn auth_token_path() -> PathBuf {
    if let Ok(val) = std::env::var("PINCE_AUTH_TOKEN_FILE") {
        return PathBuf::from(val);
    }
    default_auth_token_path()
}

fn supervisor_socket_path() -> PathBuf {
    if let Ok(val) = std::env::var("PINCE_FRONTEND_SOCKET") {
        return PathBuf::from(val);
    }
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(base).join("pince").join("supervisor.sock")
}

// ── Secret management ─────────────────────────────────────────────────────────

fn handle_secret(args: &[String]) -> Result<()> {
    let store = open_store()?;
    match args.first().map(|s| s.as_str()) {
        Some("set") => {
            let name =
                args.get(1).ok_or_else(|| anyhow::anyhow!("Usage: pince secret set <name>"))?;
            let mut value = String::new();
            io::stdin().read_to_string(&mut value)?;
            let value = value.trim_end_matches('\n');
            store.set(name, value.as_bytes())?;
            eprintln!("Secret '{name}' stored.");
        }
        Some("list") => {
            let names = store.list()?;
            if names.is_empty() {
                println!("(no secrets stored)");
            } else {
                for name in names {
                    println!("{name}");
                }
            }
        }
        Some("delete") => {
            let name = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("Usage: pince secret delete <name>"))?;
            store.delete(name)?;
            eprintln!("Secret '{name}' deleted.");
        }
        Some("show") => {
            let name = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("Usage: pince secret show <name>"))?;
            eprintln!("Warning: displaying secret value for '{name}'.");
            let val = store.resolve(name)?;
            let s = val.expose_str().unwrap_or("<binary value>");
            println!("{s}");
        }
        Some(other) => bail!(
            "unknown secret subcommand: {other}\nUsage: pince secret <set|list|delete|show>"
        ),
        None => bail!("Usage: pince secret <set|list|delete|show>"),
    }
    Ok(())
}

fn open_store() -> Result<SecretStore> {
    let dir = secrets_dir()?;
    SecretStore::new(dir)
}

fn secrets_dir() -> Result<PathBuf> {
    if let Ok(val) = std::env::var("PINCE_SECRETS_DIR") {
        return Ok(PathBuf::from(val));
    }
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".config")
        });
    Ok(config.join("pince").join("secrets"))
}
