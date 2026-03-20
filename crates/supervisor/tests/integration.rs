//! Integration tests: start a real supervisor, connect mock frontends and
//! verify the full authentication + routing loop.

use std::time::Duration;
use tempfile::TempDir;
use tokio::net::UnixStream;
use uuid::Uuid;

use pince_protocol::{
    codec,
    frontend_types::{
        self,
        FrontendMessage,
        SupervisorFrontendMessage,
        frontend_message::Msg as FrontMsg,
        supervisor_frontend_message::Msg as SupFrontMsg,
    },
};
use supervisor_lib::{config::{Config, PermissionsConfig}, supervisor::Supervisor};

/// Spawn a supervisor in a background task. Returns (socket_path, auth_token).
async fn start_supervisor(dir: &TempDir) -> (std::path::PathBuf, String) {
    let socket = dir.path().join("supervisor.sock");
    let token_file = dir.path().join("auth_token");
    let audit_log = dir.path().join("audit.jsonl");
    let agent_socket_dir = dir.path().join("agents");

    let token = Uuid::new_v4().to_string();
    tokio::fs::write(&token_file, &token).await.unwrap();

    let cfg = Config {
        frontend_socket: socket.clone(),
        agent_socket_dir,
        auth_token_file: token_file,
        audit_log,
        heartbeat_timeout_secs: 10,
        config_file: dir.path().join("supervisor.toml"),
        cron_jobs: Vec::new(),
        permissions: PermissionsConfig {
            global_policy: dir.path().join("policy.toml"),
            project_policy: None,
            hot_reload: false,
        },
    };

    tokio::spawn(async move {
        let sup = Supervisor::new(cfg).await.unwrap();
        sup.run().await.unwrap();
    });

    // Give the supervisor a moment to bind its socket.
    tokio::time::sleep(Duration::from_millis(50)).await;

    (socket, token)
}

/// Authenticate a frontend connection and return the ready stream.
async fn auth_frontend(socket: &std::path::Path, token: &str) -> UnixStream {
    let mut stream = UnixStream::connect(socket).await.unwrap();
    let auth_msg = FrontendMessage {
        msg: Some(FrontMsg::Auth(frontend_types::Auth {
            token: token.to_string(),
        })),
    };
    codec::write_message(&mut stream, &auth_msg).await.unwrap();

    let resp: SupervisorFrontendMessage = codec::read_message(&mut stream).await.unwrap();
    match &resp.msg {
        Some(SupFrontMsg::AuthResult(result)) => {
            assert!(result.success, "auth failed: {:?}", result);
        }
        other => panic!("expected AuthResult, got {:?}", other),
    }
    stream
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn frontend_auth_success() {
    let dir = TempDir::new().unwrap();
    let (socket, token) = start_supervisor(&dir).await;
    let _stream = auth_frontend(&socket, &token).await;
}

#[tokio::test]
async fn frontend_auth_failure() {
    let dir = TempDir::new().unwrap();
    let (socket, _) = start_supervisor(&dir).await;

    let mut stream = UnixStream::connect(&socket).await.unwrap();
    let auth_msg = FrontendMessage {
        msg: Some(FrontMsg::Auth(frontend_types::Auth {
            token: "bad-token".into(),
        })),
    };
    codec::write_message(&mut stream, &auth_msg).await.unwrap();

    let resp: SupervisorFrontendMessage = codec::read_message(&mut stream).await.unwrap();
    match &resp.msg {
        Some(SupFrontMsg::AuthResult(result)) => {
            assert!(!result.success, "expected failure, got {:?}", result);
        }
        other => panic!("expected AuthResult, got {:?}", other),
    }
}

#[tokio::test]
async fn list_agents_empty() {
    let dir = TempDir::new().unwrap();
    let (socket, token) = start_supervisor(&dir).await;
    let mut stream = auth_frontend(&socket, &token).await;

    let list_msg = FrontendMessage {
        msg: Some(FrontMsg::ListAgents(frontend_types::ListAgents {})),
    };
    codec::write_message(&mut stream, &list_msg).await.unwrap();

    let resp: SupervisorFrontendMessage = codec::read_message(&mut stream).await.unwrap();
    match &resp.msg {
        Some(SupFrontMsg::AgentList(list)) => {
            assert!(list.agents.is_empty(), "expected empty list");
        }
        other => panic!("expected AgentList, got {:?}", other),
    }
}

#[tokio::test]
async fn send_message_no_agents_returns_error() {
    let dir = TempDir::new().unwrap();
    let (socket, token) = start_supervisor(&dir).await;
    let mut stream = auth_frontend(&socket, &token).await;

    let send_msg = FrontendMessage {
        msg: Some(FrontMsg::SendMessage(frontend_types::SendMessage {
            content: "hello".into(),
            agent_id: String::new(),
        })),
    };
    codec::write_message(&mut stream, &send_msg).await.unwrap();

    let resp: SupervisorFrontendMessage = codec::read_message(&mut stream).await.unwrap();
    assert!(
        matches!(&resp.msg, Some(SupFrontMsg::Error(_))),
        "expected Error when no agents, got {:?}",
        resp
    );
}

/// Multiple frontends can connect simultaneously.
#[tokio::test]
async fn multiple_frontends_connect() {
    let dir = TempDir::new().unwrap();
    let (socket, token) = start_supervisor(&dir).await;

    let mut f1 = auth_frontend(&socket, &token).await;
    let mut f2 = auth_frontend(&socket, &token).await;

    // Both can list agents.
    let list_msg = FrontendMessage {
        msg: Some(FrontMsg::ListAgents(frontend_types::ListAgents {})),
    };
    codec::write_message(&mut f1, &list_msg).await.unwrap();
    codec::write_message(&mut f2, &list_msg).await.unwrap();

    let r1: SupervisorFrontendMessage = codec::read_message(&mut f1).await.unwrap();
    let r2: SupervisorFrontendMessage = codec::read_message(&mut f2).await.unwrap();

    assert!(matches!(&r1.msg, Some(SupFrontMsg::AgentList(_))));
    assert!(matches!(&r2.msg, Some(SupFrontMsg::AgentList(_))));
}
