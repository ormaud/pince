/// Round-trip serialization tests for all frontend protocol message types.
#[cfg(test)]
mod tests {
    use crate::codec::{read_message, write_message};
    use crate::generated::pince_frontend::{
        frontend_message::Msg as FMsg, supervisor_frontend_message::Msg as SMsg,
        AgentInfo, AgentList, AgentResponseChunk, AgentResponseDone, AgentStatus,
        AgentStatusChange, ApprovalDecision, ApprovalRequest, ApprovalResponse, Auth, AuthResult,
        FrontendError, FrontendMessage, KillAgent, ListAgents, SendMessage, SpawnAgent,
        SupervisorFrontendMessage, ToolCallEvent, ToolResultEvent,
    };
    use crate::generated::pince::agent::RiskLevel;
    use tokio::net::UnixStream;

    // ── helpers ──────────────────────────────────────────────────────────────

    async fn round_trip_frontend(msg: FrontendMessage) -> FrontendMessage {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        write_message(&mut a, &msg).await.unwrap();
        read_message(&mut b).await.unwrap()
    }

    async fn round_trip_supervisor(msg: SupervisorFrontendMessage) -> SupervisorFrontendMessage {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        write_message(&mut a, &msg).await.unwrap();
        read_message(&mut b).await.unwrap()
    }

    // ── FrontendMessage variants ──────────────────────────────────────────────

    #[tokio::test]
    async fn round_trip_auth() {
        let msg = FrontendMessage {
            msg: Some(FMsg::Auth(Auth { token: "secret-token-abc".into() })),
        };
        let got = round_trip_frontend(msg).await;
        match got.msg {
            Some(FMsg::Auth(a)) => assert_eq!(a.token, "secret-token-abc"),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_send_message() {
        let msg = FrontendMessage {
            msg: Some(FMsg::SendMessage(SendMessage {
                content: "Hello agent!".into(),
                agent_id: "agent-42".into(),
            })),
        };
        let got = round_trip_frontend(msg).await;
        match got.msg {
            Some(FMsg::SendMessage(m)) => {
                assert_eq!(m.content, "Hello agent!");
                assert_eq!(m.agent_id, "agent-42");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_send_message_default_agent() {
        // agent_id = "" means "default agent"
        let msg = FrontendMessage {
            msg: Some(FMsg::SendMessage(SendMessage {
                content: "hi".into(),
                agent_id: String::new(),
            })),
        };
        let got = round_trip_frontend(msg).await;
        match got.msg {
            Some(FMsg::SendMessage(m)) => assert_eq!(m.agent_id, ""),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_approval_response_approve_once() {
        let msg = FrontendMessage {
            msg: Some(FMsg::ApprovalResponse(ApprovalResponse {
                request_id: "req-1".into(),
                decision: ApprovalDecision::ApproveOnce as i32,
            })),
        };
        let got = round_trip_frontend(msg).await;
        match got.msg {
            Some(FMsg::ApprovalResponse(r)) => {
                assert_eq!(r.request_id, "req-1");
                assert_eq!(r.decision, ApprovalDecision::ApproveOnce as i32);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_approval_response_deny() {
        let msg = FrontendMessage {
            msg: Some(FMsg::ApprovalResponse(ApprovalResponse {
                request_id: "req-2".into(),
                decision: ApprovalDecision::Deny as i32,
            })),
        };
        let got = round_trip_frontend(msg).await;
        match got.msg {
            Some(FMsg::ApprovalResponse(r)) => {
                assert_eq!(r.decision, ApprovalDecision::Deny as i32);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_list_agents() {
        let msg = FrontendMessage {
            msg: Some(FMsg::ListAgents(ListAgents {})),
        };
        let got = round_trip_frontend(msg).await;
        assert!(matches!(got.msg, Some(FMsg::ListAgents(_))));
    }

    #[tokio::test]
    async fn round_trip_spawn_agent() {
        let msg = FrontendMessage {
            msg: Some(FMsg::SpawnAgent(SpawnAgent { agent_type: "claude-3-opus".into() })),
        };
        let got = round_trip_frontend(msg).await;
        match got.msg {
            Some(FMsg::SpawnAgent(s)) => assert_eq!(s.agent_type, "claude-3-opus"),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_kill_agent() {
        let msg = FrontendMessage {
            msg: Some(FMsg::KillAgent(KillAgent { agent_id: "dead-agent".into() })),
        };
        let got = round_trip_frontend(msg).await;
        match got.msg {
            Some(FMsg::KillAgent(k)) => assert_eq!(k.agent_id, "dead-agent"),
            other => panic!("unexpected: {:?}", other),
        }
    }

    // ── SupervisorFrontendMessage variants ────────────────────────────────────

    #[tokio::test]
    async fn round_trip_auth_result_ok() {
        let msg = SupervisorFrontendMessage {
            msg: Some(SMsg::AuthResult(AuthResult { success: true, error: String::new() })),
        };
        let got = round_trip_supervisor(msg).await;
        match got.msg {
            Some(SMsg::AuthResult(r)) => assert!(r.success),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_auth_result_err() {
        let msg = SupervisorFrontendMessage {
            msg: Some(SMsg::AuthResult(AuthResult {
                success: false,
                error: "bad token".into(),
            })),
        };
        let got = round_trip_supervisor(msg).await;
        match got.msg {
            Some(SMsg::AuthResult(r)) => {
                assert!(!r.success);
                assert_eq!(r.error, "bad token");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_agent_response_chunk() {
        let msg = SupervisorFrontendMessage {
            msg: Some(SMsg::AgentResponse(AgentResponseChunk {
                agent_id: "a1".into(),
                content: "Hello, ".into(),
            })),
        };
        let got = round_trip_supervisor(msg).await;
        match got.msg {
            Some(SMsg::AgentResponse(c)) => {
                assert_eq!(c.agent_id, "a1");
                assert_eq!(c.content, "Hello, ");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_agent_response_done() {
        let msg = SupervisorFrontendMessage {
            msg: Some(SMsg::AgentResponseDone(AgentResponseDone { agent_id: "a1".into() })),
        };
        let got = round_trip_supervisor(msg).await;
        match got.msg {
            Some(SMsg::AgentResponseDone(d)) => assert_eq!(d.agent_id, "a1"),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_tool_call_event() {
        let args = br#"{"cmd":"ls"}"#.to_vec();
        let msg = SupervisorFrontendMessage {
            msg: Some(SMsg::ToolCallEvent(ToolCallEvent {
                agent_id: "a1".into(),
                request_id: "r1".into(),
                tool: "shell_exec".into(),
                arguments_json: args.clone(),
            })),
        };
        let got = round_trip_supervisor(msg).await;
        match got.msg {
            Some(SMsg::ToolCallEvent(e)) => {
                assert_eq!(e.agent_id, "a1");
                assert_eq!(e.request_id, "r1");
                assert_eq!(e.tool, "shell_exec");
                assert_eq!(e.arguments_json, args);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_tool_result_event() {
        let result = br#"{"exit":0}"#.to_vec();
        let msg = SupervisorFrontendMessage {
            msg: Some(SMsg::ToolResultEvent(ToolResultEvent {
                request_id: "r1".into(),
                result_json: result.clone(),
            })),
        };
        let got = round_trip_supervisor(msg).await;
        match got.msg {
            Some(SMsg::ToolResultEvent(e)) => {
                assert_eq!(e.request_id, "r1");
                assert_eq!(e.result_json, result);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_approval_request() {
        let args = br#"{"cmd":"rm -rf /"}"#.to_vec();
        let msg = SupervisorFrontendMessage {
            msg: Some(SMsg::ApprovalRequest(ApprovalRequest {
                request_id: "r99".into(),
                agent_id: "a1".into(),
                tool: "shell_exec".into(),
                arguments_json: args.clone(),
                risk_level: RiskLevel::Dangerous as i32,
            })),
        };
        let got = round_trip_supervisor(msg).await;
        match got.msg {
            Some(SMsg::ApprovalRequest(r)) => {
                assert_eq!(r.request_id, "r99");
                assert_eq!(r.agent_id, "a1");
                assert_eq!(r.tool, "shell_exec");
                assert_eq!(r.arguments_json, args);
                assert_eq!(r.risk_level, RiskLevel::Dangerous as i32);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_agent_list() {
        let msg = SupervisorFrontendMessage {
            msg: Some(SMsg::AgentList(AgentList {
                agents: vec![
                    AgentInfo {
                        agent_id: "a1".into(),
                        agent_type: "claude".into(),
                        status: AgentStatus::Ready as i32,
                    },
                    AgentInfo {
                        agent_id: "a2".into(),
                        agent_type: "claude".into(),
                        status: AgentStatus::Busy as i32,
                    },
                ],
            })),
        };
        let got = round_trip_supervisor(msg).await;
        match got.msg {
            Some(SMsg::AgentList(list)) => {
                assert_eq!(list.agents.len(), 2);
                assert_eq!(list.agents[0].agent_id, "a1");
                assert_eq!(list.agents[0].status, AgentStatus::Ready as i32);
                assert_eq!(list.agents[1].agent_id, "a2");
                assert_eq!(list.agents[1].status, AgentStatus::Busy as i32);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_agent_status_change() {
        for status in [
            AgentStatus::Initializing,
            AgentStatus::Ready,
            AgentStatus::Busy,
            AgentStatus::Dead,
        ] {
            let msg = SupervisorFrontendMessage {
                msg: Some(SMsg::AgentStatusChange(AgentStatusChange {
                    agent_id: "a1".into(),
                    status: status as i32,
                })),
            };
            let got = round_trip_supervisor(msg).await;
            match got.msg {
                Some(SMsg::AgentStatusChange(c)) => {
                    assert_eq!(c.agent_id, "a1");
                    assert_eq!(c.status, status as i32);
                }
                other => panic!("unexpected: {:?}", other),
            }
        }
    }

    #[tokio::test]
    async fn round_trip_frontend_error() {
        let msg = SupervisorFrontendMessage {
            msg: Some(SMsg::Error(FrontendError {
                message: "something went wrong".into(),
            })),
        };
        let got = round_trip_supervisor(msg).await;
        match got.msg {
            Some(SMsg::Error(e)) => assert_eq!(e.message, "something went wrong"),
            other => panic!("unexpected: {:?}", other),
        }
    }

    // ── multi-frontend routing simulation ─────────────────────────────────────

    /// Simulates two frontends both receiving the same broadcast agent response.
    #[tokio::test]
    async fn multi_frontend_broadcast() {
        let (mut sv1, mut fe1) = UnixStream::pair().unwrap();
        let (mut sv2, mut fe2) = UnixStream::pair().unwrap();

        let chunk = SupervisorFrontendMessage {
            msg: Some(SMsg::AgentResponse(AgentResponseChunk {
                agent_id: "a1".into(),
                content: "broadcast".into(),
            })),
        };

        // Supervisor writes to both frontends
        write_message(&mut sv1, &chunk).await.unwrap();
        write_message(&mut sv2, &chunk).await.unwrap();

        let got1: SupervisorFrontendMessage = read_message(&mut fe1).await.unwrap();
        let got2: SupervisorFrontendMessage = read_message(&mut fe2).await.unwrap();

        for got in [got1, got2] {
            match got.msg {
                Some(SMsg::AgentResponse(c)) => assert_eq!(c.content, "broadcast"),
                other => panic!("unexpected: {:?}", other),
            }
        }
    }

    /// ApprovalRequest goes only to the originating frontend (simulated).
    #[tokio::test]
    async fn approval_request_targeted() {
        let (mut sv_orig, mut fe_orig) = UnixStream::pair().unwrap();
        let (mut sv_other, mut fe_other) = UnixStream::pair().unwrap();

        let approval = SupervisorFrontendMessage {
            msg: Some(SMsg::ApprovalRequest(ApprovalRequest {
                request_id: "r1".into(),
                agent_id: "a1".into(),
                tool: "read_file".into(),
                arguments_json: b"{}".to_vec(),
                risk_level: RiskLevel::Safe as i32,
            })),
        };

        // Only send to originating frontend
        write_message(&mut sv_orig, &approval).await.unwrap();

        // Other frontend gets an unrelated agent response
        let other_msg = SupervisorFrontendMessage {
            msg: Some(SMsg::AgentResponse(AgentResponseChunk {
                agent_id: "a1".into(),
                content: "hey".into(),
            })),
        };
        write_message(&mut sv_other, &other_msg).await.unwrap();

        let got_orig: SupervisorFrontendMessage = read_message(&mut fe_orig).await.unwrap();
        let got_other: SupervisorFrontendMessage = read_message(&mut fe_other).await.unwrap();

        assert!(matches!(got_orig.msg, Some(SMsg::ApprovalRequest(_))));
        assert!(matches!(got_other.msg, Some(SMsg::AgentResponse(_))));
    }
}
