#[tokio::test]
async fn send_discord_message_uses_canonical_dm_reply_context_for_user_targets() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.gateway.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    engine.init_gateway().await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    engine.set_gateway_ipc_sender(Some(tx)).await;

    {
        let mut gw_guard = engine.gateway_state.lock().await;
        let gw = gw_guard.as_mut().expect("gateway state should exist");
        gw.discord_dm_channels_by_user
            .insert("user:123456789".to_string(), "DM123".to_string());
        gw.reply_contexts.insert(
            "Discord:DM123".to_string(),
            crate::agent::gateway::ThreadContext {
                discord_message_id: Some("987654321".to_string()),
                ..Default::default()
            },
        );
    }

    let send_engine = engine.clone();
    let send_task = tokio::spawn(async move {
        execute_gateway_message(
            "send_discord_message",
            &serde_json::json!({
                "user_id": "123456789",
                "message": "discord reply"
            }),
            &send_engine,
            &reqwest::Client::new(),
        )
        .await
    });

    let request = match timeout(Duration::from_millis(250), rx.recv())
        .await
        .expect("gateway send request should be emitted")
        .expect("gateway send request should exist")
    {
        DaemonMessage::GatewaySendRequest { request } => request,
        other => panic!("expected GatewaySendRequest, got {other:?}"),
    };
    assert_eq!(request.platform, "discord");
    assert_eq!(request.channel_id, "user:123456789");
    assert_eq!(request.thread_id.as_deref(), Some("987654321"));

    engine
        .complete_gateway_send_result(GatewaySendResult {
            correlation_id: request.correlation_id.clone(),
            platform: "discord".to_string(),
            channel_id: "DM123".to_string(),
            requested_channel_id: Some("user:123456789".to_string()),
            delivery_id: Some("delivery-2".to_string()),
            ok: true,
            error: None,
            completed_at_ms: 1,
        })
        .await;

    let result = send_task
        .await
        .expect("send task should join")
        .expect("send should succeed");
    assert_eq!(result, "Discord message sent to user:123456789");
}

async fn spawn_scripted_tool_call_server_for_tool_executor(
    script: Vec<Option<(String, String)>>,
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind scripted tool call server");
    let addr = listener
        .local_addr()
        .expect("scripted tool call server local addr");
    let script = Arc::new(script);
    let next_response = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let script = Arc::clone(&script);
            let next_response = Arc::clone(&next_response);
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 65536];
                let read = socket
                    .read(&mut buffer)
                    .await
                    .expect("read scripted tool call request");
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let body = request
                    .split("\r\n\r\n")
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                let response = if body.contains("internal_hidden") {
                    concat!(
	                        "HTTP/1.1 200 OK\r\n",
	                        "content-type: text/event-stream\r\n",
	                        "cache-control: no-cache\r\n",
	                        "connection: close\r\n",
	                        "\r\n",
	                        "data: {\"choices\":[{\"delta\":{\"content\":\"Acknowledged.\"}}]}\n\n",
	                        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
	                        "data: [DONE]\n\n"
	                    )
	                    .to_string()
                } else {
                    let index = next_response.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    match script.get(index).and_then(|entry| entry.as_ref()) {
	                        Some((tool_name, arguments)) => {
	                            let chunk = serde_json::json!({
	                                "choices": [{
	                                    "delta": {
	                                        "tool_calls": [{
	                                            "index": 0,
	                                            "id": format!("call_scripted_{index}"),
	                                            "function": {
	                                                "name": tool_name,
	                                                "arguments": arguments,
	                                            }
	                                        }]
	                                    }
	                                }],
	                                "usage": {
	                                    "prompt_tokens": 7,
	                                    "completion_tokens": 3
	                                }
	                            })
	                            .to_string();
	                            format!(
	                                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\ndata: {chunk}\n\ndata: [DONE]\n\n",
	                                chunk = chunk
	                            )
	                        }
	                        None => concat!(
	                            "HTTP/1.1 200 OK\r\n",
	                            "content-type: text/event-stream\r\n",
	                            "cache-control: no-cache\r\n",
	                            "connection: close\r\n",
	                            "\r\n",
	                            "data: {\"choices\":[{\"delta\":{\"content\":\"Acknowledged.\"}}]}\n\n",
	                            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
	                            "data: [DONE]\n\n"
	                        )
	                        .to_string(),
	                    }
                };
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write scripted tool call response");
            });
        }
    });

    format!("http://{addr}/v1")
}

#[test]
fn visible_thread_continuation_chain_does_not_overflow_small_worker_stack() {
    let runtime_stack_size = 8 * 1024 * 1024;
    let join = std::thread::Builder::new()
        .name("continuation-stack-regression".to_string())
        .stack_size(runtime_stack_size)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .thread_stack_size(runtime_stack_size)
                .build()
                .expect("build runtime");
            runtime.block_on(async {
                let chain_len = 96usize;
                let mut script = Vec::with_capacity((chain_len - 1) * 2);
                for index in 0..(chain_len - 1) {
                    let target = if index % 2 == 0 { "svarog" } else { "weles" };
                    script.push(Some((
                        "message_agent".to_string(),
                        serde_json::json!({
                            "target": target,
                            "message": format!("continue visible chain step {}", index + 1),
                            "request_visible_thread_continuation": true
                        })
                        .to_string(),
                    )));
                    script.push(None);
                }

                let root = tempdir().expect("tempdir should succeed");
                let manager = SessionManager::new_test(root.path()).await;
                let mut config = AgentConfig::default();
                config.provider = "openai".to_string();
                config.base_url = spawn_scripted_tool_call_server_for_tool_executor(script).await;
                config.model = "gpt-4o-mini".to_string();
                config.api_key = "test-key".to_string();
                config.api_transport = crate::agent::types::ApiTransport::ChatCompletions;
                config.auto_retry = false;
                config.max_retries = 0;
                config.max_tool_loops = 1;
                let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
                let (event_tx, _) = broadcast::channel(8);
                let thread_id = "thread-visible-continuation-recursion";

                {
                    let mut threads = engine.threads.write().await;
                    threads.insert(
                        thread_id.to_string(),
                        crate::agent::types::AgentThread {
                            id: thread_id.to_string(),
                            agent_name: Some(
                                crate::agent::agent_identity::MAIN_AGENT_NAME.to_string(),
                            ),
                            title: "Visible continuation recursion".to_string(),
                            messages: vec![crate::agent::types::AgentMessage::user(
                                "Keep the visible thread moving forward",
                                1,
                            )],
                            pinned: false,
                            upstream_thread_id: None,
                            upstream_transport: None,
                            upstream_provider: None,
                            upstream_model: None,
                            upstream_assistant_id: None,
                            total_input_tokens: 0,
                            total_output_tokens: 0,
                            created_at: 1,
                            updated_at: 1,
                        },
                    );
                }

                let tool_call = ToolCall::with_default_weles_review(
                    "tool-message-agent-visible-recursion".to_string(),
                    ToolFunction {
                        name: "message_agent".to_string(),
                        arguments: serde_json::json!({
                            "target": "weles",
                            "message": "Start the visible continuation chain.",
                            "request_visible_thread_continuation": true
                        })
                        .to_string(),
                    },
                );

                let result = execute_tool(
                    &tool_call,
                    &engine,
                    thread_id,
                    None,
                    &manager,
                    None,
                    &event_tx,
                    root.path(),
                    &engine.http_client,
                    None,
                )
                .await;

                assert!(
                    !result.is_error,
                    "initial visible continuation request should succeed: {}",
                    result.content
                );

                engine
                    .flush_deferred_visible_thread_continuations(thread_id)
                    .await
                    .expect("visible continuation chain should flush");

                let assistant_count = engine
                    .threads
                    .read()
                    .await
                    .get(thread_id)
                    .expect("visible thread should exist")
                    .messages
                    .iter()
                    .filter(|message| message.role == crate::agent::MessageRole::Assistant)
                    .count();
                assert!(
                    assistant_count >= 1,
                    "expected at least one visible assistant turn, got {assistant_count}"
                );
            });
        })
        .expect("spawn regression thread");

    join.join()
        .expect("continuation chain should complete without stack overflow");
}

#[tokio::test]
async fn compaction_trims_participant_playground_threads_for_visible_thread() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.max_context_messages = 4;
    config.keep_recent_on_compact = 2;
    let assistant_message = |content: &str, timestamp: u64| crate::agent::types::AgentMessage {
        id: crate::agent::types::generate_message_id(),
        role: crate::agent::MessageRole::Assistant,
        content: content.to_string(),
        tool_calls: None,
        tool_call_id: None,
        tool_name: None,
        tool_arguments: None,
        tool_status: None,
        weles_review: None,
        input_tokens: 0,
        output_tokens: 0,
        cost: None,
        provider: None,
        model: None,
        api_transport: None,
        response_id: None,
        upstream_message: None,
        provider_final_result: None,
        author_agent_id: None,
        author_agent_name: None,
        reasoning: None,
        message_kind: crate::agent::types::AgentMessageKind::Normal,
        compaction_strategy: None,
        compaction_payload: None,
        offloaded_payload_id: None,
        structural_refs: Vec::new(),
        timestamp,
    };
    let provider_config = {
        let engine = AgentEngine::new_test(manager.clone(), config.clone(), root.path()).await;
        engine
            .resolve_provider_config(&config)
            .expect("provider config")
    };
    let engine = AgentEngine::new_test(manager, config.clone(), root.path()).await;
    let thread_id = "thread-participant-compaction-reset";
    let playground_thread_id =
        crate::agent::agent_identity::participant_playground_thread_id(thread_id, "weles");

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            crate::agent::types::AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
                title: "Participant compaction reset".to_string(),
                messages: vec![
                    crate::agent::types::AgentMessage::user("hello", 1),
                    assistant_message("step 1", 2),
                    assistant_message("step 2", 3),
                    assistant_message("step 3", 4),
                    assistant_message("step 4", 5),
                ],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                created_at: 1,
                updated_at: 5,
            },
        );
        threads.insert(
            playground_thread_id.clone(),
            crate::agent::types::AgentThread {
                id: playground_thread_id.clone(),
                agent_name: Some("Weles".to_string()),
                title: "Participant Playground".to_string(),
                messages: vec![
                    crate::agent::types::AgentMessage::user("draft prompt 1", 1),
                    assistant_message("draft reply 1", 2),
                    crate::agent::types::AgentMessage::user("draft prompt 2", 3),
                    assistant_message("draft reply 2", 4),
                    crate::agent::types::AgentMessage::user("draft prompt 3", 5),
                    assistant_message("draft reply 3", 6),
                ],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 11,
                total_output_tokens: 7,
                created_at: 1,
                updated_at: 6,
            },
        );
    }

    let compacted = engine
        .maybe_persist_compaction_artifact(thread_id, None, &config, &provider_config)
        .await
        .expect("compaction should succeed");
    assert!(compacted, "expected compaction artifact to be persisted");

    let threads = engine.threads.read().await;
    let playground = threads
        .get(&playground_thread_id)
        .expect("playground thread should still exist after compaction");
    assert_eq!(
	        playground.messages.len(),
	        4,
	        "compaction should retain a bounded recent playground tail instead of clearing hidden context"
	    );
    assert_eq!(
        playground.messages[0].content, "draft prompt 2",
        "compaction should drop the oldest playground turns first"
    );
    assert_eq!(playground.messages[3].content, "draft reply 3");
}

#[tokio::test]
async fn execute_managed_command_auto_approves_learned_git_category() {
    let recorded_bodies = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_approval_learning = true;
    config.provider = "openai".to_string();
    config.base_url = part4::spawn_stub_assistant_server_for_tool_executor(
        recorded_bodies,
        serde_json::json!({
            "verdict": "allow",
            "reasons": ["runtime approved managed command test"],
            "audit_id": "audit-weles-managed-approve"
        })
        .to_string(),
    )
    .await;
    config.model = "gpt-4o-mini".to_string();
    config.api_key = "test-key".to_string();
    config.api_transport = crate::agent::types::ApiTransport::ChatCompletions;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 1;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let repo_dir = root.path().join("git-repo");
    std::fs::create_dir_all(&repo_dir).expect("create repo dir");
    std::process::Command::new("git")
        .arg("init")
        .current_dir(&repo_dir)
        .output()
        .expect("git init should succeed");
    let (session_id, _rx) = manager
        .spawn(Some("/bin/bash".to_string()), None, None, None, 80, 24)
        .await
        .expect("spawn session");

    {
        let mut model = engine.operator_model.write().await;
        model.risk_fingerprint.approvals = 3;
        model
            .risk_fingerprint
            .category_requests
            .insert("git".to_string(), 3);
        model
            .risk_fingerprint
            .category_approvals
            .insert("git".to_string(), 3);
        model.risk_fingerprint.auto_approve_categories = vec!["git".to_string()];
    }

    let tool_call = ToolCall::with_default_weles_review(
        "tool-managed-auto-approve".to_string(),
        ToolFunction {
            name: "execute_managed_command".to_string(),
            arguments: serde_json::json!({
                "session": session_id.to_string(),
                "command": "git status --short",
                "rationale": "Check repo status",
                "cwd": repo_dir,
                "allow_network": true,
                "timeout_seconds": 5
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-managed-auto-approve",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "learned auto-approve should execute successfully: {}",
        result.content
    );
    assert!(result.pending_approval.is_none());
    assert!(result.content.contains("Managed command finished"));

    let model = engine.operator_model.read().await;
    assert_eq!(model.risk_fingerprint.approval_requests, 1);
    assert_eq!(model.risk_fingerprint.approvals, 4);
    assert_eq!(
        model.risk_fingerprint.auto_approve_categories,
        vec!["git".to_string()]
    );
}

#[tokio::test]
async fn message_agent_rejects_self_target_for_active_responder() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-self-message";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            crate::agent::types::AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
                title: "Sticky Svarog".to_string(),
                messages: vec![crate::agent::types::AgentMessage::user(
                    "Operator wants Svarog",
                    1,
                )],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                created_at: 1,
                updated_at: 1,
            },
        );
    }
    engine
        .set_thread_handoff_state(
            thread_id,
            crate::agent::ThreadHandoffState {
                origin_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                active_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                responder_stack: vec![crate::agent::ThreadResponderFrame {
                    agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                    agent_name: crate::agent::agent_identity::MAIN_AGENT_NAME.to_string(),
                    entered_at: 1,
                    entered_via_handoff_event_id: None,
                    linked_thread_id: None,
                }],
                events: Vec::new(),
                pending_approval_id: None,
            },
        )
        .await;

    let tool_call = ToolCall::with_default_weles_review(
        "tool-message-self".to_string(),
        ToolFunction {
            name: "message_agent".to_string(),
            arguments: serde_json::json!({
                "target": "svarog",
                "message": "Talk to yourself"
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        result.is_error,
        "self-targeted message_agent should be rejected"
    );
    assert!(
        result
            .content
            .contains("cannot target the current active responder")
            || result.content.contains("cannot target the same agent"),
        "unexpected self-target error: {}",
        result.content
    );
}

#[tokio::test]
async fn message_agent_can_request_visible_thread_continuation() {
    let recorded_bodies = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = "openai".to_string();
    config.base_url = part4::spawn_stub_assistant_server_for_tool_executor(
        recorded_bodies.clone(),
        serde_json::json!({
            "verdict": "allow",
            "reasons": ["unused for visible thread continuation test"],
            "audit_id": "audit-message-agent-visible-thread"
        })
        .to_string(),
    )
    .await;
    config.model = "gpt-4o-mini".to_string();
    config.api_key = "test-key".to_string();
    config.api_transport = crate::agent::types::ApiTransport::ChatCompletions;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 1;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-message-agent-visible-thread";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            crate::agent::types::AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
                title: "Thread-scoped internal DM".to_string(),
                messages: vec![crate::agent::types::AgentMessage::user(
                    "Please continue this work",
                    1,
                )],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                created_at: 1,
                updated_at: 1,
            },
        );
    }

    let tool_call = ToolCall::with_default_weles_review(
        "tool-message-agent-visible-thread".to_string(),
        ToolFunction {
            name: "message_agent".to_string(),
            arguments: serde_json::json!({
                "target": "weles",
                "message": "Please pick up the work from governance scope.",
                "request_visible_thread_continuation": true
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "thread continuation via message_agent should succeed: {}",
        result.content
    );
    assert!(
        result
            .content
            .contains("\"visible_thread_continuation_requested\": true"),
        "tool result should report that visible-thread continuation was requested: {}",
        result.content
    );
    engine
        .flush_deferred_visible_thread_continuations(thread_id)
        .await
        .expect("deferred visible-thread continuation should flush");

    let dm_thread_id = crate::agent::agent_identity::internal_dm_thread_id(
        crate::agent::agent_identity::MAIN_AGENT_ID,
        crate::agent::agent_identity::WELES_AGENT_ID,
    );
    let threads = engine.threads.read().await;
    assert!(
        threads
            .get(&dm_thread_id)
            .expect("message_agent should still use internal DM for coordination")
            .messages
            .iter()
            .any(|message| message.role == crate::agent::MessageRole::Assistant),
        "internal DM thread should contain the discussion reply"
    );
    let visible_reply = threads
        .get(thread_id)
        .expect("visible thread should remain present")
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.role == crate::agent::MessageRole::Assistant
                && message.author_agent_id.as_deref() == Some("weles")
        })
        .expect("visible thread should receive the Weles continuation");
    assert!(
        !visible_reply.content.trim().is_empty(),
        "visible-thread continuation should append a non-empty assistant reply"
    );
}

#[tokio::test]
async fn execute_managed_command_auto_denies_learned_destructive_category() {
    let recorded_bodies = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_approval_learning = true;
    config.provider = "openai".to_string();
    config.base_url = part4::spawn_stub_assistant_server_for_tool_executor(
        recorded_bodies,
        serde_json::json!({
            "verdict": "allow",
            "reasons": ["runtime approved managed command test"],
            "audit_id": "audit-weles-managed-deny"
        })
        .to_string(),
    )
    .await;
    config.model = "gpt-4o-mini".to_string();
    config.api_key = "test-key".to_string();
    config.api_transport = crate::agent::types::ApiTransport::ChatCompletions;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 1;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let delete_dir = root.path().join("delete-me");
    std::fs::create_dir_all(&delete_dir).expect("create delete target");
    let (session_id, _rx) = manager
        .spawn(Some("/bin/bash".to_string()), None, None, None, 80, 24)
        .await
        .expect("spawn session");

    {
        let mut model = engine.operator_model.write().await;
        model.risk_fingerprint.denials = 3;
        model
            .risk_fingerprint
            .category_requests
            .insert("destructive_delete".to_string(), 3);
        model.risk_fingerprint.auto_deny_categories = vec!["destructive_delete".to_string()];
    }

    let tool_call = ToolCall::with_default_weles_review(
        "tool-managed-auto-deny".to_string(),
        ToolFunction {
            name: "execute_managed_command".to_string(),
            arguments: serde_json::json!({
                "session": session_id.to_string(),
                "command": format!("rm -rf {}", delete_dir.display()),
                "rationale": "Clean up generated scratch directory",
                "timeout_seconds": 5
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-managed-auto-deny",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "learned auto-deny should return a policy message: {}",
        result.content
    );
    assert!(result.pending_approval.is_none());
    assert!(result.content.contains("auto-denied"));
    assert!(
        delete_dir.exists(),
        "the destructive command should not execute after learned denial"
    );

    let model = engine.operator_model.read().await;
    assert_eq!(model.risk_fingerprint.approval_requests, 1);
    assert_eq!(model.risk_fingerprint.denials, 4);
    assert_eq!(
        model.risk_fingerprint.auto_deny_categories,
        vec!["destructive_delete".to_string()]
    );
}

#[tokio::test]
async fn auto_backgrounded_bash_command_returns_handle_and_status() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let (session_id, _rx) = manager
        .spawn(Some("/bin/bash".to_string()), None, None, None, 80, 24)
        .await
        .expect("spawn session");

    let tool_call = ToolCall::with_default_weles_review(
        "tool-bash-auto-background".to_string(),
        ToolFunction {
            name: "bash_command".to_string(),
            arguments: serde_json::json!({
                "session": session_id.to_string(),
                "command": "sleep 0.2 && printf done",
                "timeout_seconds": 1200,
                "sandbox_enabled": true,
                "allow_network": false,
                "security_level": "moderate"
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-bash-auto-background",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "auto-backgrounded bash command should queue cleanly: {}",
        result.content
    );

    let background_task_id = result
        .content
        .lines()
        .find_map(|line| line.strip_prefix("background_task_id: "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("auto-background response should expose a background_task_id handle");

    let status_call = ToolCall::with_default_weles_review(
        "tool-background-status".to_string(),
        ToolFunction {
            name: "get_background_task_status".to_string(),
            arguments: serde_json::json!({
                "background_task_id": background_task_id,
            })
            .to_string(),
        },
    );

    let mut payload = serde_json::Value::Null;
    let mut completed = false;
    for _ in 0..20 {
        let status = execute_tool(
            &status_call,
            &engine,
            "thread-bash-auto-background",
            None,
            &manager,
            None,
            &event_tx,
            root.path(),
            &engine.http_client,
            None,
        )
        .await;

        assert!(
            !status.is_error,
            "background task status lookup should succeed: {}",
            status.content
        );

        payload =
            serde_json::from_str(&status.content).expect("status payload should be valid JSON");
        if payload["state"] == "completed" {
            completed = true;
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert!(
        completed,
        "background task should complete within the polling window: {}",
        payload
    );
    assert_eq!(payload["background_task_id"], background_task_id);
    assert_eq!(payload["state"], "completed");
    assert_eq!(payload["exit_code"], 0);
    assert_eq!(payload["command"], "sleep 0.2 && printf done");
}

#[tokio::test]
async fn tui_bash_command_wait_false_returns_immediate_operation_handle() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let marker = root.path().join("tui-headless-background.txt");
    let thread_id = "thread-tui-bash-wait-false";
    engine
        .set_thread_client_surface(thread_id, amux_protocol::ClientSurface::Tui)
        .await;

    let command = format!(
            "python3 -c \"import pathlib, time; time.sleep(1); pathlib.Path(r'{}').write_text('done')\"",
            marker.display()
        );
    let tool_call = ToolCall::with_default_weles_review(
        "tool-tui-bash-wait-false".to_string(),
        ToolFunction {
            name: "bash_command".to_string(),
            arguments: serde_json::json!({
                "command": command,
                "wait_for_completion": false,
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "non-blocking TUI bash command should return immediately: {}",
        result.content
    );
    assert!(
        result.content.contains("operation_id: "),
        "non-blocking TUI bash command should return an operation handle: {}",
        result.content
    );
    assert!(
        !marker.exists(),
        "backgrounded headless TUI command should not have finished before returning"
    );

    let operation_id = result
        .content
        .lines()
        .find_map(|line| line.strip_prefix("operation_id: "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("backgrounded TUI bash command should expose an operation_id");

    let status_call = ToolCall::with_default_weles_review(
        "tool-tui-bash-operation-status".to_string(),
        ToolFunction {
            name: "get_operation_status".to_string(),
            arguments: serde_json::json!({
                "operation_id": operation_id,
            })
            .to_string(),
        },
    );

    let mut payload = serde_json::Value::Null;
    let mut completed = false;
    for _ in 0..20 {
        let status = execute_tool(
            &status_call,
            &engine,
            thread_id,
            None,
            &manager,
            None,
            &event_tx,
            root.path(),
            &engine.http_client,
            None,
        )
        .await;

        assert!(
            !status.is_error,
            "operation status lookup should succeed: {}",
            status.content
        );

        payload = serde_json::from_str(&status.content)
            .expect("operation status payload should be valid JSON");
        if payload["state"] == "completed" {
            completed = true;
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert!(
        completed,
        "backgrounded headless TUI command should complete within the polling window: {}",
        payload
    );
    assert_eq!(payload["operation_id"], operation_id);
    assert_eq!(payload["state"], "completed");
    assert!(
        marker.exists(),
        "backgrounded headless TUI command should finish after polling"
    );
}

#[tokio::test]
async fn tui_bash_command_wait_false_exposes_failure_payload_via_operation_status() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-tui-bash-wait-false-failure";
    engine
        .set_thread_client_surface(thread_id, amux_protocol::ClientSurface::Tui)
        .await;

    let tool_call = ToolCall::with_default_weles_review(
            "tool-tui-bash-wait-false-failure".to_string(),
            ToolFunction {
                name: "bash_command".to_string(),
                arguments: serde_json::json!({
                    "command": "python3 -c \"import sys, time; print('stdout-line'); print('stderr-line', file=sys.stderr); time.sleep(0.2); sys.exit(7)\"",
                    "wait_for_completion": false,
                })
                .to_string(),
            },
        );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "non-blocking TUI bash failure should still return an operation handle: {}",
        result.content
    );

    let operation_id = result
        .content
        .lines()
        .find_map(|line| line.strip_prefix("operation_id: "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("backgrounded TUI bash failure should expose an operation_id");

    let status_call = ToolCall::with_default_weles_review(
        "tool-tui-bash-operation-status-failure".to_string(),
        ToolFunction {
            name: "get_operation_status".to_string(),
            arguments: serde_json::json!({
                "operation_id": operation_id,
            })
            .to_string(),
        },
    );

    let mut payload = serde_json::Value::Null;
    let mut failed = false;
    for _ in 0..20 {
        let status = execute_tool(
            &status_call,
            &engine,
            thread_id,
            None,
            &manager,
            None,
            &event_tx,
            root.path(),
            &engine.http_client,
            None,
        )
        .await;

        assert!(
            !status.is_error,
            "operation status lookup should succeed: {}",
            status.content
        );

        payload = serde_json::from_str(&status.content)
            .expect("operation status payload should be valid JSON");
        if payload["state"] == "failed" {
            failed = true;
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert!(
        failed,
        "backgrounded headless TUI failure should reach failed state within the polling window: {}",
        payload
    );
    assert_eq!(payload["operation_id"], operation_id);
    assert_eq!(payload["state"], "failed");
    assert_eq!(payload["exit_code"], 7);
    assert!(
        payload["terminal_result"]["stdout"]
            .as_str()
            .is_some_and(|value| value.contains("stdout-line")),
        "failed background status should include stdout payload: {}",
        payload
    );
    assert!(
        payload["terminal_result"]["stderr"]
            .as_str()
            .is_some_and(|value| value.contains("stderr-line")),
        "failed background status should include stderr payload: {}",
        payload
    );
}

#[tokio::test]
async fn get_operation_status_returns_server_operation_snapshot() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let record = crate::server::operation_registry()
        .accept_operation("skill_publish", Some("tool-test:skill-publish".to_string()));
    crate::server::operation_registry().mark_started(&record.operation_id);
    crate::server::operation_registry().mark_completed(&record.operation_id);

    let tool_call = ToolCall::with_default_weles_review(
        "tool-operation-status".to_string(),
        ToolFunction {
            name: "get_operation_status".to_string(),
            arguments: serde_json::json!({
                "operation_id": record.operation_id,
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-operation-status",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "get_operation_status should return server operation snapshots: {}",
        result.content
    );

    let payload: serde_json::Value =
        serde_json::from_str(&result.content).expect("status payload should be valid JSON");
    assert_eq!(payload["operation_id"], record.operation_id);
    assert_eq!(payload["kind"], "skill_publish");
    assert_eq!(payload["state"], "completed");
}

#[tokio::test]
async fn send_telegram_message_emits_gateway_ipc_request() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.gateway.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    engine.set_gateway_ipc_sender(Some(tx)).await;

    let send_engine = engine.clone();
    let send_task = tokio::spawn(async move {
        execute_gateway_message(
            "send_telegram_message",
            &serde_json::json!({
                "chat_id": "777",
                "message": "telegram reply",
                "reply_to_message_id": 42
            }),
            &send_engine,
            &reqwest::Client::new(),
        )
        .await
    });

    let request = match timeout(Duration::from_millis(250), rx.recv())
        .await
        .expect("gateway send request should be emitted")
        .expect("gateway send request should exist")
    {
        DaemonMessage::GatewaySendRequest { request } => request,
        other => panic!("expected GatewaySendRequest, got {other:?}"),
    };
    assert_eq!(request.platform, "telegram");
    assert_eq!(request.channel_id, "777");
    assert_eq!(request.thread_id.as_deref(), Some("42"));
    assert_eq!(request.content, "telegram reply");

    engine
        .complete_gateway_send_result(GatewaySendResult {
            correlation_id: request.correlation_id.clone(),
            platform: "telegram".to_string(),
            channel_id: "777".to_string(),
            requested_channel_id: Some("777".to_string()),
            delivery_id: Some("99".to_string()),
            ok: true,
            error: None,
            completed_at_ms: 1,
        })
        .await;

    let result = send_task
        .await
        .expect("send task should join")
        .expect("send should succeed");
    assert_eq!(result, "Telegram message sent to 777");
}

// -----------------------------------------------------------------------
// Source authority classification tests (UNCR-03)
// -----------------------------------------------------------------------

use super::{classify_freshness, classify_source_authority, format_result_with_authority};

#[test]
fn classify_source_authority_official_rust_docs() {
    assert_eq!(
        classify_source_authority("https://docs.rust-lang.org/book/"),
        "official"
    );
}

#[test]
fn classify_source_authority_community_stackoverflow() {
    assert_eq!(
        classify_source_authority("https://stackoverflow.com/questions/123"),
        "community"
    );
}

#[test]
fn classify_source_authority_unknown_random_site() {
    assert_eq!(
        classify_source_authority("https://random-site.example.com"),
        "unknown"
    );
}

#[test]
fn classify_source_authority_official_mdn() {
    assert_eq!(
        classify_source_authority("https://developer.mozilla.org/en-US/docs"),
        "official"
    );
}

#[test]
fn classify_source_authority_community_reddit() {
    assert_eq!(
        classify_source_authority("https://reddit.com/r/rust"),
        "community"
    );
}

#[test]
fn classify_source_authority_community_medium() {
    assert_eq!(
        classify_source_authority("https://medium.com/@author/article"),
        "community"
    );
}

#[test]
fn classify_source_authority_official_cppreference() {
    assert_eq!(
        classify_source_authority("https://cppreference.com/w/cpp"),
        "official"
    );
}

#[test]
fn classify_source_authority_empty_string_no_panic() {
    // Should return "unknown" without panicking.
    assert_eq!(classify_source_authority(""), "unknown");
}

#[test]
fn format_result_with_authority_prepends_official_tag() {
    let result = format_result_with_authority(
        "Rust Book",
        "https://docs.rust-lang.org/book/",
        "The Rust Programming Language",
    );
    assert!(result.starts_with("- [official]"));
    assert!(result.contains("**Rust Book**"));
    assert!(result.contains("https://docs.rust-lang.org/book/"));
    assert!(result.contains("The Rust Programming Language"));
    assert!(
        result.contains("freshness:"),
        "research result formatting should expose freshness alongside source authority"
    );
}

#[test]
fn classify_freshness_labels_recent_stale_and_old_dates() {
    assert_eq!(classify_freshness(Some("2026-03-20")), "recent");
    assert_eq!(classify_freshness(Some("2025-12-01T14:00:00Z")), "stale");
    assert_eq!(classify_freshness(Some("2024-01-01")), "old");
    assert_eq!(classify_freshness(Some("not-a-date")), "unknown");
    assert_eq!(classify_freshness(None), "unknown");
}

#[tokio::test]
async fn spawn_subagent_does_not_require_todo_bootstrap_for_chat_threads() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-planned-subagents";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
                thread_id.to_string(),
                crate::agent::types::AgentThread {
                    id: thread_id.to_string(),
                    agent_name: None,
                    title: "Parallel skill work".to_string(),
                    messages: vec![crate::agent::types::AgentMessage::user(
                        "Investigate the failing tests, then update the parser, and finally rerun the suite.",
                        1,
                    )],
                    pinned: false,
                    upstream_thread_id: None,
                    upstream_transport: None,
                    upstream_provider: None,
                    upstream_model: None,
                    upstream_assistant_id: None,
                    total_input_tokens: 0,
                    total_output_tokens: 0,
                    created_at: 1,
                    updated_at: 1,
                },
            );
    }

    let tool_call = ToolCall::with_default_weles_review(
            "tool-spawn-subagent-bootstrap".to_string(),
            ToolFunction {
                name: "spawn_subagent".to_string(),
                arguments: serde_json::json!({
                    "title": "Write foundational skill files",
                    "description": "Create the foundational skill files in parallel so the parent can integrate the batches."
                })
                .to_string(),
            },
        );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "spawn_subagent should run from a chat thread without forcing plan bootstrap: {}",
        result.content
    );
    assert!(result.content.contains("Spawned subagent"));

    let todos = engine.get_todos(thread_id).await;
    assert!(
        todos.is_empty(),
        "chat-thread subagent spawning should not auto-bootstrap todos"
    );

    let tasks = engine.list_tasks().await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].source, "subagent");
    assert_eq!(tasks[0].parent_thread_id.as_deref(), Some(thread_id));
}

#[tokio::test]
async fn read_peer_memory_honors_explicit_parent_task_id_without_current_task() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.collaboration.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let parent = engine
        .enqueue_task(
            "Parent coordinator".to_string(),
            "Coordinate the child work".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "user",
            None,
            None,
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    let child_a = engine
        .enqueue_task(
            "Research child".to_string(),
            "Inspect deployment risks".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    let child_b = engine
        .enqueue_task(
            "Review child".to_string(),
            "Cross-check deployment risks".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;

    engine
        .register_subagent_collaboration(&parent.id, &child_a)
        .await;
    engine
        .register_subagent_collaboration(&parent.id, &child_b)
        .await;
    engine
        .record_collaboration_contribution(
            &parent.id,
            &child_a.id,
            "deploy",
            "recommend staged rollout",
            vec!["smoke tests are stable".to_string()],
            0.82,
        )
        .await
        .expect("seed contribution should succeed");

    let result = super::execute_read_peer_memory(
        &serde_json::json!({ "parent_task_id": parent.id }),
        &engine,
        None,
    )
    .await
    .expect("explicit parent scope should work without a current task");

    let report: serde_json::Value =
        serde_json::from_str(&result).expect("result should be valid json");
    assert_eq!(
        report["shared_context"]
            .as_array()
            .expect("shared_context should be an array")
            .len(),
        1
    );
    assert_eq!(
        report["shared_context"][0]["task_id"],
        serde_json::Value::String(child_a.id.clone())
    );
    assert_eq!(
        report["peers"]
            .as_array()
            .expect("peers should be an array")
            .len(),
        2
    );
}

#[tokio::test]
async fn broadcast_contribution_honors_explicit_parent_task_id_without_current_task() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.collaboration.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let parent = engine
        .enqueue_task(
            "Parent coordinator".to_string(),
            "Coordinate the child work".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "user",
            None,
            None,
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    let child = engine
        .enqueue_task(
            "Research child".to_string(),
            "Inspect deployment risks".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;

    engine
        .register_subagent_collaboration(&parent.id, &child)
        .await;

    let result = super::execute_broadcast_contribution(
        &serde_json::json!({
            "parent_task_id": parent.id,
            "topic": "deploy",
            "position": "hold until the migration completes",
            "evidence": ["operator wants to preserve retrieval visibility"],
            "confidence": 0.95
        }),
        &engine,
        "thread-parent",
        None,
    )
    .await
    .expect("explicit parent scope should allow parent-context contributions");

    let report: serde_json::Value =
        serde_json::from_str(&result).expect("result should be valid json");
    assert_eq!(report["contribution"]["task_id"], "operator");
    assert_eq!(report["contribution"]["topic"], "deploy");

    let session = engine
        .collaboration_sessions_json(Some(&parent.id))
        .await
        .expect("session lookup should succeed");
    assert_eq!(
        session["contributions"]
            .as_array()
            .unwrap_or(&Vec::new())
            .len(),
        1
    );
}

#[tokio::test]
async fn dispatch_via_bid_protocol_tool_routes_through_collaboration_runtime() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.collaboration.enabled = true;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let parent = engine
        .enqueue_task(
            "Parent coordinator".to_string(),
            "Choose the best owner for the next workstream".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "user",
            None,
            None,
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    let child_a = engine
        .enqueue_task(
            "Research child".to_string(),
            "Prepare a bid for implementation ownership".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    let child_b = engine
        .enqueue_task(
            "Review child".to_string(),
            "Prepare a bid for review ownership".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;

    engine
        .register_subagent_collaboration(&parent.id, &child_a)
        .await;
    engine
        .register_subagent_collaboration(&parent.id, &child_b)
        .await;

    let tool_call = ToolCall::with_default_weles_review(
        "tool-dispatch-via-bid-protocol".to_string(),
        ToolFunction {
            name: "dispatch_via_bid_protocol".to_string(),
            arguments: serde_json::json!({
                "parent_task_id": parent.id,
                "bids": [
                    {
                        "task_id": child_b.id,
                        "confidence": 0.93,
                        "availability": "busy"
                    },
                    {
                        "task_id": child_a.id,
                        "confidence": 0.74,
                        "availability": "available"
                    }
                ]
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-parent",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "dispatch_via_bid_protocol tool should succeed: {}",
        result.content
    );
    let payload: serde_json::Value =
        serde_json::from_str(&result.content).expect("tool result should be valid json");
    assert_eq!(payload["primary_task_id"], child_a.id);
    assert_eq!(payload["reviewer_task_id"], child_b.id);

    let session = engine
        .collaboration_sessions_json(Some(&parent.id))
        .await
        .expect("session lookup should succeed");
    assert_eq!(session["role_assignment"]["primary_task_id"], child_a.id);
    assert_eq!(session["role_assignment"]["reviewer_task_id"], child_b.id);
}

#[tokio::test]
async fn dispatch_via_bid_protocol_tool_bootstraps_collaboration_agents_before_role_assignment() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.collaboration.enabled = true;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let parent = engine
        .enqueue_task(
            "Parent coordinator".to_string(),
            "Choose the best owner for the next workstream".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "user",
            None,
            None,
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    let child_a = engine
        .enqueue_task(
            "Research child".to_string(),
            "Prepare a bid for implementation ownership".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    let child_b = engine
        .enqueue_task(
            "Review child".to_string(),
            "Prepare a bid for review ownership".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;

    let tool_call = ToolCall::with_default_weles_review(
        "tool-dispatch-via-bid-protocol-bootstrap".to_string(),
        ToolFunction {
            name: "dispatch_via_bid_protocol".to_string(),
            arguments: serde_json::json!({
                "parent_task_id": parent.id,
                "bids": [
                    {
                        "task_id": child_b.id,
                        "confidence": 0.63,
                        "availability": "available"
                    },
                    {
                        "task_id": child_a.id,
                        "confidence": 0.88,
                        "availability": "available"
                    }
                ]
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-parent",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "dispatch_via_bid_protocol should bootstrap collaboration agents: {}",
        result.content
    );

    let payload: serde_json::Value =
        serde_json::from_str(&result.content).expect("tool result should be valid json");
    assert_eq!(payload["primary_task_id"], child_a.id);
    assert_eq!(payload["reviewer_task_id"], child_b.id);

    let session = engine
        .collaboration_sessions_json(Some(&parent.id))
        .await
        .expect("session lookup should succeed");
    assert_eq!(
        session["agents"].as_array().map(|items| items.len()),
        Some(2),
        "eligible subagents should be bootstrapped into the collaboration session"
    );
    assert_eq!(session["role_assignment"]["primary_task_id"], child_a.id);
    assert_eq!(session["role_assignment"]["reviewer_task_id"], child_b.id);
}

async fn spawn_model_fetch_server(models_body: serde_json::Value) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind model fetch server");
    let address = listener
        .local_addr()
        .expect("read model fetch listener address");
    let response_body = serde_json::to_string(&models_body).expect("serialize models body");

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let body = response_body.clone();
            tokio::spawn(async move {
                let mut buffer = [0u8; 2048];
                let _ = stream.read(&mut buffer).await;
                let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    format!("http://{}", address)
}

#[tokio::test]
async fn fetch_authenticated_providers_returns_only_authenticated_entries() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = amux_shared::providers::PROVIDER_ID_OPENAI.to_string();
    config.api_key = "test-key".to_string();
    config.base_url = "https://api.openai.example/v1".to_string();
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let tool_call = ToolCall::with_default_weles_review(
        "tool-fetch-authenticated-providers".to_string(),
        ToolFunction {
            name: "fetch_authenticated_providers".to_string(),
            arguments: serde_json::json!({}).to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-provider-discovery",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "provider discovery should succeed: {}",
        result.content
    );
    let providers: Vec<crate::agent::types::ProviderAuthState> =
        serde_json::from_str(&result.content).expect("parse authenticated provider list");
    assert!(providers.iter().all(|provider| provider.authenticated));
    assert!(providers
        .iter()
        .any(|provider| { provider.provider_id == amux_shared::providers::PROVIDER_ID_OPENAI }));
}

#[tokio::test]
async fn fetch_provider_models_uses_authenticated_provider_config() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = amux_shared::providers::PROVIDER_ID_OPENAI.to_string();
    config.api_key = "test-key".to_string();
    config.base_url = spawn_model_fetch_server(serde_json::json!({
        "data": [
            {"id": "gpt-4.1", "name": "GPT-4.1", "context_length": 1048576},
            {"id": "gpt-5.4", "name": "GPT-5.4", "context_length": 1048576}
        ]
    }))
    .await;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let tool_call = ToolCall::with_default_weles_review(
        "tool-fetch-provider-models".to_string(),
        ToolFunction {
            name: "fetch_provider_models".to_string(),
            arguments: serde_json::json!({
                "provider": amux_shared::providers::PROVIDER_ID_OPENAI,
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-provider-models",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "model discovery should succeed: {}",
        result.content
    );
    let models: Vec<crate::agent::llm_client::FetchedModel> =
        serde_json::from_str(&result.content).expect("parse fetched models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gpt-4.1");
    assert_eq!(models[1].id, "gpt-5.4");
}

#[tokio::test]
async fn list_providers_returns_auth_state_rows() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = amux_shared::providers::PROVIDER_ID_OPENAI.to_string();
    config.api_key = "test-key".to_string();
    config.base_url = "https://api.openai.example/v1".to_string();
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let tool_call = ToolCall::with_default_weles_review(
        "tool-list-providers".to_string(),
        ToolFunction {
            name: "list_providers".to_string(),
            arguments: serde_json::json!({}).to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-list-providers",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "list_providers should succeed: {}",
        result.content
    );
    let providers: Vec<crate::agent::types::ProviderAuthState> =
        serde_json::from_str(&result.content).expect("parse provider auth states");
    assert!(providers.iter().any(|provider| {
        provider.provider_id == amux_shared::providers::PROVIDER_ID_OPENAI && provider.authenticated
    }));
}

#[tokio::test]
async fn list_models_returns_remote_models_for_authenticated_provider() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = amux_shared::providers::PROVIDER_ID_OPENAI.to_string();
    config.api_key = "test-key".to_string();
    config.base_url = spawn_model_fetch_server(serde_json::json!({
        "data": [
            {"id": "gpt-4.1", "name": "GPT-4.1", "context_length": 1048576},
            {"id": "gpt-5.4", "name": "GPT-5.4", "context_length": 1048576}
        ]
    }))
    .await;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let tool_call = ToolCall::with_default_weles_review(
        "tool-list-models".to_string(),
        ToolFunction {
            name: "list_models".to_string(),
            arguments: serde_json::json!({
                "provider": amux_shared::providers::PROVIDER_ID_OPENAI,
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-list-models",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "list_models should succeed: {}",
        result.content
    );
    let models: Vec<crate::agent::llm_client::FetchedModel> =
        serde_json::from_str(&result.content).expect("parse fetched models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gpt-4.1");
    assert_eq!(models[1].id, "gpt-5.4");
}

#[tokio::test]
async fn list_agents_returns_effective_runtime_targets() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = amux_shared::providers::PROVIDER_ID_OPENAI.to_string();
    config.model = "gpt-5.4-mini".to_string();
    config.api_key = "test-key".to_string();
    config.concierge.provider = Some(amux_shared::providers::PROVIDER_ID_GROQ.to_string());
    config.concierge.model = Some("llama-3.3-70b-versatile".to_string());
    config.builtin_sub_agents.weles.provider =
        Some(amux_shared::providers::PROVIDER_ID_ANTHROPIC.to_string());
    config.builtin_sub_agents.weles.model = Some("claude-sonnet-4-20250514".to_string());
    config.sub_agents.push(crate::agent::SubAgentDefinition {
        id: "reviewer".to_string(),
        name: "Reviewer".to_string(),
        provider: amux_shared::providers::PROVIDER_ID_OPENAI.to_string(),
        model: "gpt-5.4".to_string(),
        role: Some("review".to_string()),
        system_prompt: None,
        tool_whitelist: None,
        tool_blacklist: None,
        context_budget_tokens: None,
        max_duration_secs: None,
        supervisor_config: None,
        enabled: true,
        builtin: false,
        immutable_identity: false,
        disable_allowed: true,
        delete_allowed: true,
        protected_reason: None,
        reasoning_effort: None,
        created_at: 0,
    });
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let tool_call = ToolCall::with_default_weles_review(
        "tool-list-agents".to_string(),
        ToolFunction {
            name: "list_agents".to_string(),
            arguments: serde_json::json!({}).to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        "thread-list-agents",
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "list_agents should succeed: {}",
        result.content
    );
    let agents: serde_json::Value =
        serde_json::from_str(&result.content).expect("parse list_agents payload");
    let rows = agents
        .as_array()
        .expect("list_agents should return an array");
    let svarog = rows
        .iter()
        .find(|row| row.get("agent").and_then(|value| value.as_str()) == Some("svarog"))
        .expect("svarog row should be present");
    assert_eq!(
        svarog.get("provider").and_then(|value| value.as_str()),
        Some(amux_shared::providers::PROVIDER_ID_OPENAI)
    );
    assert_eq!(
        svarog.get("model").and_then(|value| value.as_str()),
        Some("gpt-5.4-mini")
    );
    let rarog = rows
        .iter()
        .find(|row| row.get("agent").and_then(|value| value.as_str()) == Some("rarog"))
        .expect("rarog row should be present");
    assert_eq!(
        rarog.get("provider").and_then(|value| value.as_str()),
        Some(amux_shared::providers::PROVIDER_ID_GROQ)
    );
    let perun = rows
        .iter()
        .find(|row| row.get("agent").and_then(|value| value.as_str()) == Some("perun"))
        .expect("perun row should be present");
    assert_eq!(
        perun.get("name").and_then(|value| value.as_str()),
        Some("Perun")
    );
    assert_eq!(
        perun.get("kind").and_then(|value| value.as_str()),
        Some("builtin")
    );
    let mokosh = rows
        .iter()
        .find(|row| row.get("agent").and_then(|value| value.as_str()) == Some("mokosh"))
        .expect("mokosh row should be present");
    assert_eq!(
        mokosh.get("name").and_then(|value| value.as_str()),
        Some("Mokosh")
    );
    let dazhbog = rows
        .iter()
        .find(|row| row.get("agent").and_then(|value| value.as_str()) == Some("dazhbog"))
        .expect("dazhbog row should be present");
    assert_eq!(
        dazhbog.get("name").and_then(|value| value.as_str()),
        Some("Dazhbog")
    );
    let weles = rows
        .iter()
        .find(|row| row.get("agent").and_then(|value| value.as_str()) == Some("weles"))
        .expect("weles row should be present");
    assert_eq!(
        weles.get("provider").and_then(|value| value.as_str()),
        Some(amux_shared::providers::PROVIDER_ID_ANTHROPIC)
    );
    assert!(rows.iter().any(|row| {
        row.get("agent").and_then(|value| value.as_str()) == Some("reviewer")
            && row.get("model").and_then(|value| value.as_str()) == Some("gpt-5.4")
    }));
}

#[tokio::test]
async fn list_participants_returns_thread_participant_rows() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-list-participants";

    engine.threads.write().await.insert(
        thread_id.to_string(),
        crate::agent::types::AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Participant list".to_string(),
            messages: vec![crate::agent::types::AgentMessage::user(
                "Inspect participants.",
                1,
            )],
            pinned: false,
            upstream_thread_id: None,
            upstream_transport: None,
            upstream_provider: None,
            upstream_model: None,
            upstream_assistant_id: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            created_at: 1,
            updated_at: 1,
        },
    );
    engine
        .upsert_thread_participant(thread_id, "weles", "watch this thread")
        .await
        .expect("participant should register");

    let tool_call = ToolCall::with_default_weles_review(
        "tool-list-participants".to_string(),
        ToolFunction {
            name: "list_participants".to_string(),
            arguments: serde_json::json!({}).to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "list_participants should succeed: {}",
        result.content
    );
    let participants: serde_json::Value =
        serde_json::from_str(&result.content).expect("parse list_participants payload");
    let rows = participants
        .as_array()
        .expect("list_participants should return an array");
    assert!(rows.iter().any(|row| {
        row.get("agent").and_then(|value| value.as_str()) == Some("weles")
            && row.get("status").and_then(|value| value.as_str()) == Some("active")
    }));
}

#[tokio::test]
async fn switch_model_updates_targeted_agent_settings_from_svarog_scope() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = amux_shared::providers::PROVIDER_ID_OPENAI.to_string();
    config.model = "gpt-5.4-mini".to_string();
    config.api_key = "test-key".to_string();
    config.concierge.provider = Some(amux_shared::providers::PROVIDER_ID_OPENAI.to_string());
    config.concierge.model = Some("gpt-4.1-mini".to_string());
    config.providers.insert(
        amux_shared::providers::PROVIDER_ID_GROQ.to_string(),
        crate::agent::types::ProviderConfig {
            base_url: String::new(),
            model: String::new(),
            api_key: "groq-key".to_string(),
            assistant_id: String::new(),
            auth_source: crate::agent::types::AuthSource::ApiKey,
            api_transport: crate::agent::types::ApiTransport::default(),
            reasoning_effort: "high".to_string(),
            context_window_tokens: 128000,
            response_schema: None,
            stop_sequences: None,
            temperature: None,
            top_p: None,
            top_k: None,
            metadata: None,
            service_tier: None,
            container: None,
            inference_geo: None,
            cache_control: None,
            max_tokens: None,
            anthropic_tool_choice: None,
            output_effort: None,
        },
    );
    config.sub_agents.push(crate::agent::SubAgentDefinition {
        id: "reviewer".to_string(),
        name: "Reviewer".to_string(),
        provider: amux_shared::providers::PROVIDER_ID_OPENAI.to_string(),
        model: "gpt-4.1".to_string(),
        role: None,
        system_prompt: None,
        tool_whitelist: None,
        tool_blacklist: None,
        context_budget_tokens: None,
        max_duration_secs: None,
        supervisor_config: None,
        enabled: true,
        builtin: false,
        immutable_identity: false,
        disable_allowed: true,
        delete_allowed: true,
        protected_reason: None,
        reasoning_effort: None,
        created_at: 0,
    });
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    for (agent, provider, model) in [
        (
            "svarog",
            amux_shared::providers::PROVIDER_ID_OPENAI,
            "gpt-5.4",
        ),
        (
            "rarog",
            amux_shared::providers::PROVIDER_ID_GROQ,
            "llama-3.3-70b-versatile",
        ),
        (
            "weles",
            amux_shared::providers::PROVIDER_ID_OPENAI,
            "gpt-5.4-mini",
        ),
        (
            "reviewer",
            amux_shared::providers::PROVIDER_ID_OPENAI,
            "gpt-5.4-mini",
        ),
    ] {
        let tool_call = ToolCall::with_default_weles_review(
            format!("tool-switch-model-{agent}"),
            ToolFunction {
                name: "switch_model".to_string(),
                arguments: serde_json::json!({
                    "agent": agent,
                    "provider": provider,
                    "model": model,
                })
                .to_string(),
            },
        );

        let result = crate::agent::agent_identity::run_with_agent_scope(
            crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
            async {
                execute_tool(
                    &tool_call,
                    &engine,
                    "thread-switch-model",
                    None,
                    &manager,
                    None,
                    &event_tx,
                    root.path(),
                    &engine.http_client,
                    None,
                )
                .await
            },
        )
        .await;

        assert!(
            !result.is_error,
            "switch_model should succeed for {agent}: {}",
            result.content
        );
    }

    let stored = engine.get_config().await;
    assert_eq!(stored.provider, amux_shared::providers::PROVIDER_ID_OPENAI);
    assert_eq!(stored.model, "gpt-5.4");
    assert_eq!(
        stored.concierge.provider.as_deref(),
        Some(amux_shared::providers::PROVIDER_ID_GROQ)
    );
    assert_eq!(
        stored.concierge.model.as_deref(),
        Some("llama-3.3-70b-versatile")
    );
    assert_eq!(
        stored.builtin_sub_agents.weles.provider.as_deref(),
        Some(amux_shared::providers::PROVIDER_ID_OPENAI)
    );
    assert_eq!(
        stored.builtin_sub_agents.weles.model.as_deref(),
        Some("gpt-5.4-mini")
    );
    let reviewer = stored
        .sub_agents
        .iter()
        .find(|agent| agent.id == "reviewer")
        .expect("reviewer subagent should exist");
    assert_eq!(
        reviewer.provider,
        amux_shared::providers::PROVIDER_ID_OPENAI
    );
    assert_eq!(reviewer.model, "gpt-5.4-mini");
}

#[tokio::test]
async fn switch_model_is_rejected_outside_svarog_scope() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = amux_shared::providers::PROVIDER_ID_OPENAI.to_string();
    config.model = "gpt-5.4-mini".to_string();
    config.api_key = "test-key".to_string();
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let tool_call = ToolCall::with_default_weles_review(
        "tool-switch-model-rarog-scope".to_string(),
        ToolFunction {
            name: "switch_model".to_string(),
            arguments: serde_json::json!({
                "agent": "svarog",
                "provider": amux_shared::providers::PROVIDER_ID_OPENAI,
                "model": "gpt-5.4"
            })
            .to_string(),
        },
    );

    let result = crate::agent::agent_identity::run_with_agent_scope(
        crate::agent::agent_identity::CONCIERGE_AGENT_ID.to_string(),
        async {
            execute_tool(
                &tool_call,
                &engine,
                "thread-switch-model-rarog",
                None,
                &manager,
                None,
                &event_tx,
                root.path(),
                &engine.http_client,
                None,
            )
            .await
        },
    )
    .await;

    assert!(result.is_error, "non-svarog switch_model call should fail");
    assert!(
        result.content.contains("svarog"),
        "rejection should mention svarog-only access: {}",
        result.content
    );
}

#[tokio::test]
async fn spawn_subagent_rejects_model_without_provider_and_points_to_fetch_tools() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let error = super::execute_spawn_subagent(
        &serde_json::json!({
            "title": "Write foundational skill files",
            "description": "Create the foundational skill files in parallel.",
            "model": "gpt-5.4"
        }),
        &engine,
        "thread-parent",
        None,
        &manager,
        None,
        &event_tx,
    )
    .await
    .expect_err("model-only override should be rejected");

    let message = error.to_string();
    assert!(message.contains("provider"));
    assert!(message.contains("list_providers"));
    assert!(message.contains("list_models"));
}

#[tokio::test]
async fn spawn_subagent_rejects_unauthenticated_provider_and_points_to_fetch_tools() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let error = super::execute_spawn_subagent(
        &serde_json::json!({
            "title": "Write foundational skill files",
            "description": "Create the foundational skill files in parallel.",
            "provider": amux_shared::providers::PROVIDER_ID_OPENAI
        }),
        &engine,
        "thread-parent",
        None,
        &manager,
        None,
        &event_tx,
    )
    .await
    .expect_err("unauthenticated provider override should be rejected");

    let message = error.to_string();
    assert!(message.contains("list_providers"));
    assert!(message.contains(amux_shared::providers::PROVIDER_ID_OPENAI));
}

#[tokio::test]
async fn spawn_subagent_persists_explicit_provider_and_model_override() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = amux_shared::providers::PROVIDER_ID_OPENAI.to_string();
    config.api_key = "test-key".to_string();
    config.base_url = spawn_model_fetch_server(serde_json::json!({
        "data": [
            {"id": "gpt-4.1", "name": "GPT-4.1", "context_length": 1048576},
            {"id": "gpt-5.4", "name": "GPT-5.4", "context_length": 1048576}
        ]
    }))
    .await;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    let result = super::execute_spawn_subagent(
        &serde_json::json!({
            "title": "Write foundational skill files",
            "description": "Create the foundational skill files in parallel.",
            "provider": amux_shared::providers::PROVIDER_ID_OPENAI,
            "model": "gpt-5.4"
        }),
        &engine,
        "thread-parent",
        None,
        &manager,
        None,
        &event_tx,
    )
    .await
    .expect("explicit provider/model override should succeed");

    let tasks = engine.list_tasks().await;
    let task = tasks
        .into_iter()
        .find(|task| result.contains(&task.id))
        .expect("spawned subagent should exist");
    assert_eq!(
        task.override_provider.as_deref(),
        Some(amux_shared::providers::PROVIDER_ID_OPENAI)
    );
    assert_eq!(task.override_model.as_deref(), Some("gpt-5.4"));
}

#[tokio::test]
async fn spawn_subagent_bootstraps_todos_for_goal_run_tasks() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-goal-subagents";
    let task_id = "task-goal-subagents";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            crate::agent::types::AgentThread {
                id: thread_id.to_string(),
                agent_name: None,
                title: "Goal work".to_string(),
                messages: vec![crate::agent::types::AgentMessage::user(
                    "Start the release goal and delegate the code review.",
                    1,
                )],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                created_at: 1,
                updated_at: 1,
            },
        );
    }
    engine
        .tasks
        .lock()
        .await
        .push_back(crate::agent::types::AgentTask {
            id: task_id.to_string(),
            title: "Run release goal".to_string(),
            description: "Delegate review from the goal run.".to_string(),
            status: crate::agent::types::TaskStatus::Queued,
            priority: crate::agent::types::TaskPriority::High,
            progress: 0,
            created_at: 1,
            started_at: None,
            completed_at: None,
            error: None,
            result: None,
            thread_id: Some(thread_id.to_string()),
            source: "goal_run".to_string(),
            notify_on_complete: false,
            notify_channels: Vec::new(),
            dependencies: Vec::new(),
            command: None,
            session_id: None,
            goal_run_id: Some("goal-1".to_string()),
            goal_run_title: Some("Release the build".to_string()),
            goal_step_id: Some("step-1".to_string()),
            goal_step_title: Some("Delegate review".to_string()),
            parent_task_id: None,
            parent_thread_id: None,
            runtime: "daemon".to_string(),
            retry_count: 0,
            max_retries: 0,
            next_retry_at: None,
            scheduled_at: None,
            blocked_reason: None,
            awaiting_approval_id: None,
            policy_fingerprint: None,
            approval_expires_at: None,
            containment_scope: None,
            compensation_status: None,
            compensation_summary: None,
            lane_id: None,
            last_error: None,
            logs: Vec::new(),
            tool_whitelist: None,
            tool_blacklist: None,
            context_budget_tokens: None,
            context_overflow_action: None,
            termination_conditions: None,
            success_criteria: None,
            max_duration_secs: None,
            supervisor_config: None,
            override_provider: None,
            override_model: None,
            override_system_prompt: None,
            sub_agent_def_id: None,
        });

    let tool_call = ToolCall::with_default_weles_review(
            "tool-goal-spawn-subagent-bootstrap".to_string(),
            ToolFunction {
                name: "spawn_subagent".to_string(),
                arguments: serde_json::json!({
                    "title": "Review release notes",
                    "description": "Check the release notes and report issues before the goal continues."
                })
                .to_string(),
            },
        );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        Some(task_id),
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "goal-run subagent spawning should bootstrap plan state instead of failing: {}",
        result.content
    );
    assert!(result.content.contains("Spawned subagent"));

    let todos = engine.get_todos(thread_id).await;
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0].status, TodoStatus::InProgress);
    assert!(
        todos[0].content.contains("Review release notes"),
        "goal-run bootstrap todo should reflect the delegated work"
    );
}

#[tokio::test]
async fn spawn_subagent_rejects_recursive_spawn_beyond_default_flat_depth() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    engine
        .tasks
        .lock()
        .await
        .push_back(crate::agent::types::AgentTask {
            id: "task-parent-subagent".to_string(),
            title: "Parent subagent".to_string(),
            description: "Already delegated child work".to_string(),
            status: crate::agent::types::TaskStatus::Queued,
            priority: crate::agent::types::TaskPriority::Normal,
            progress: 0,
            created_at: 1,
            started_at: None,
            completed_at: None,
            error: None,
            result: None,
            thread_id: Some("thread-parent".to_string()),
            source: "subagent".to_string(),
            notify_on_complete: false,
            notify_channels: Vec::new(),
            dependencies: Vec::new(),
            command: None,
            session_id: None,
            goal_run_id: None,
            goal_run_title: None,
            goal_step_id: None,
            goal_step_title: None,
            parent_task_id: Some("task-root".to_string()),
            parent_thread_id: Some("thread-parent".to_string()),
            runtime: "daemon".to_string(),
            retry_count: 0,
            max_retries: 0,
            next_retry_at: None,
            scheduled_at: None,
            blocked_reason: None,
            awaiting_approval_id: None,
            policy_fingerprint: None,
            approval_expires_at: None,
            containment_scope: None,
            compensation_status: None,
            compensation_summary: None,
            lane_id: None,
            last_error: None,
            logs: Vec::new(),
            tool_whitelist: None,
            tool_blacklist: None,
            context_budget_tokens: None,
            context_overflow_action: None,
            termination_conditions: None,
            success_criteria: None,
            max_duration_secs: None,
            supervisor_config: None,
            override_provider: None,
            override_model: None,
            override_system_prompt: None,
            sub_agent_def_id: None,
        });

    let error = super::execute_spawn_subagent(
        &serde_json::json!({
            "title": "Grandchild helper",
            "description": "Try to recurse under a flat default subagent."
        }),
        &engine,
        "thread-parent",
        Some("task-parent-subagent"),
        &manager,
        None,
        &event_tx,
    )
    .await
    .expect_err("nested recursion should be rejected by default");

    assert!(error.to_string().contains("max_depth") || error.to_string().contains("depth"));
}

#[tokio::test]
async fn spawn_subagent_allows_recursive_spawn_when_parent_scope_permits_and_derives_limits() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);

    engine
        .tasks
        .lock()
        .await
        .push_back(crate::agent::types::AgentTask {
            id: "task-parent-subagent".to_string(),
            title: "Parent subagent".to_string(),
            description: "Allowed to recurse one level deeper".to_string(),
            status: crate::agent::types::TaskStatus::Queued,
            priority: crate::agent::types::TaskPriority::Normal,
            progress: 0,
            created_at: 1,
            started_at: None,
            completed_at: None,
            error: None,
            result: None,
            thread_id: Some("thread-parent".to_string()),
            source: "subagent".to_string(),
            notify_on_complete: false,
            notify_channels: Vec::new(),
            dependencies: Vec::new(),
            command: None,
            session_id: None,
            goal_run_id: None,
            goal_run_title: None,
            goal_step_id: None,
            goal_step_title: None,
            parent_task_id: Some("task-root".to_string()),
            parent_thread_id: Some("thread-parent".to_string()),
            runtime: "daemon".to_string(),
            retry_count: 0,
            max_retries: 0,
            next_retry_at: None,
            scheduled_at: None,
            blocked_reason: None,
            awaiting_approval_id: None,
            policy_fingerprint: None,
            approval_expires_at: None,
            containment_scope: Some("subagent-depth:1/2".to_string()),
            compensation_status: None,
            compensation_summary: None,
            lane_id: None,
            last_error: None,
            logs: Vec::new(),
            tool_whitelist: None,
            tool_blacklist: None,
            context_budget_tokens: Some(20_000),
            context_overflow_action: None,
            termination_conditions: Some("tool_call_count(10)".to_string()),
            success_criteria: None,
            max_duration_secs: Some(240),
            supervisor_config: None,
            override_provider: None,
            override_model: None,
            override_system_prompt: None,
            sub_agent_def_id: None,
        });

    let result = super::execute_spawn_subagent(
        &serde_json::json!({
            "title": "Grandchild helper",
            "description": "Recurse one level deeper with inherited depth allowance."
        }),
        &engine,
        "thread-parent",
        Some("task-parent-subagent"),
        &manager,
        None,
        &event_tx,
    )
    .await
    .expect("nested recursion should succeed when parent scope permits it");

    assert!(result.contains("Delegation depth: 2/2"));

    let task = engine
        .list_tasks()
        .await
        .into_iter()
        .find(|task| result.contains(&task.id))
        .expect("spawned recursive subagent should exist");
    assert_eq!(
        task.containment_scope.as_deref(),
        Some("subagent-depth:2/2")
    );
    assert_eq!(task.context_budget_tokens, Some(20_000));
    assert_eq!(task.max_duration_secs, Some(180));
    assert!(task
        .termination_conditions
        .as_deref()
        .is_some_and(|dsl| dsl.contains("tool_call_count(10)")));
    assert_eq!(
        task.context_overflow_action,
        Some(crate::agent::types::ContextOverflowAction::Error)
    );
}

#[tokio::test]
async fn list_subagents_includes_depth_and_budget_remaining() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;

    let parent = engine
        .enqueue_task(
            "Parent coordinator".to_string(),
            "Coordinate the child work".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "user",
            None,
            None,
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;

    let mut child = engine
        .enqueue_task(
            "Depth child".to_string(),
            "Inspect deployment risks".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    child.containment_scope = Some("subagent-depth:1/2".to_string());
    child.context_budget_tokens = Some(10_000);
    child.max_duration_secs = Some(100);
    child.termination_conditions = Some("tool_call_count(15)".to_string());
    child.started_at = Some(crate::agent::now_millis().saturating_sub(20_000));
    {
        let mut tasks = engine.tasks.lock().await;
        if let Some(existing) = tasks.iter_mut().find(|task| task.id == child.id) {
            *existing = child.clone();
        }
    }

    engine
        .history
        .upsert_subagent_metrics(
            &child.id,
            Some(&parent.id),
            Some("thread-parent"),
            7,
            5,
            2,
            2500,
            Some(10_000),
            0.2,
            Some(crate::agent::now_millis().saturating_sub(5_000)),
            0.1,
            "healthy",
            1,
            crate::agent::now_millis(),
        )
        .await
        .expect("subagent metrics should persist");

    let result = super::execute_list_subagents(
        &serde_json::json!({ "parent_task_id": parent.id }),
        &engine,
        "thread-parent",
        None,
    )
    .await
    .expect("list_subagents should succeed");

    let payload: serde_json::Value = serde_json::from_str(&result).expect("valid JSON payload");
    let items = payload
        .as_array()
        .expect("list_subagents should return an array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["depth"].as_u64(), Some(1));
    assert_eq!(items[0]["max_depth"].as_u64(), Some(2));
    assert_eq!(
        items[0]["budget_remaining"]["tool_calls_remaining"].as_u64(),
        Some(8)
    );
    assert!(items[0]["budget_remaining"]["tokens_pct"]
        .as_f64()
        .is_some_and(|value| value > 0.7));
}

#[tokio::test]
async fn list_subagents_reports_exhausted_budget_limits() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;

    let parent = engine
        .enqueue_task(
            "Parent coordinator".to_string(),
            "Coordinate the child work".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "user",
            None,
            None,
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;

    let mut child = engine
        .enqueue_task(
            "Depth child".to_string(),
            "Inspect deployment risks".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    child.containment_scope = Some("subagent-depth:1/2".to_string());
    child.context_budget_tokens = Some(10_000);
    child.max_duration_secs = Some(10);
    child.termination_conditions = Some("tool_call_count(3)".to_string());
    child.started_at = Some(crate::agent::now_millis().saturating_sub(30_000));
    {
        let mut tasks = engine.tasks.lock().await;
        if let Some(existing) = tasks.iter_mut().find(|task| task.id == child.id) {
            *existing = child.clone();
        }
    }

    engine
        .history
        .upsert_subagent_metrics(
            &child.id,
            Some(&parent.id),
            Some("thread-parent"),
            3,
            2,
            1,
            10_500,
            Some(10_000),
            0.0,
            Some(crate::agent::now_millis().saturating_sub(20_000)),
            0.9,
            "stalled",
            1,
            crate::agent::now_millis(),
        )
        .await
        .expect("subagent metrics should persist");

    let result = super::execute_list_subagents(
        &serde_json::json!({ "parent_task_id": parent.id }),
        &engine,
        "thread-parent",
        None,
    )
    .await
    .expect("list_subagents should succeed");

    let payload: serde_json::Value = serde_json::from_str(&result).expect("valid JSON payload");
    let items = payload
        .as_array()
        .expect("list_subagents should return an array");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["budget_remaining"]["tokens_pct"].as_f64(),
        Some(0.0)
    );
    assert_eq!(items[0]["budget_remaining"]["time_pct"].as_f64(), Some(0.0));
    assert_eq!(
        items[0]["budget_remaining"]["tool_calls_remaining"].as_u64(),
        Some(0)
    );
    assert_eq!(items[0]["budget_exhausted"].as_bool(), Some(true));
    assert_eq!(
        items[0]["exhausted_limits"]
            .as_array()
            .map(|items| items.len()),
        Some(3)
    );
    assert_eq!(items[0]["exhausted_limits"][0], "tokens");
    assert_eq!(items[0]["exhausted_limits"][1], "time");
    assert_eq!(items[0]["exhausted_limits"][2], "tool_calls");
}

#[tokio::test]
async fn list_subagents_includes_descendants_for_parent_task_tree() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;

    let parent = engine
        .enqueue_task(
            "Parent coordinator".to_string(),
            "Coordinate the child work".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "user",
            None,
            None,
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;

    let mut child = engine
        .enqueue_task(
            "Depth child".to_string(),
            "Inspect deployment risks".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    child.containment_scope = Some("subagent-depth:1/3".to_string());
    {
        let mut tasks = engine.tasks.lock().await;
        if let Some(existing) = tasks.iter_mut().find(|task| task.id == child.id) {
            *existing = child.clone();
        }
    }

    let mut grandchild = engine
        .enqueue_task(
            "Grandchild helper".to_string(),
            "Inspect one narrow edge case".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(child.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    grandchild.containment_scope = Some("subagent-depth:2/3".to_string());
    {
        let mut tasks = engine.tasks.lock().await;
        if let Some(existing) = tasks.iter_mut().find(|task| task.id == grandchild.id) {
            *existing = grandchild.clone();
        }
    }

    let result = super::execute_list_subagents(
        &serde_json::json!({ "parent_task_id": parent.id }),
        &engine,
        "thread-parent",
        None,
    )
    .await
    .expect("list_subagents should succeed");

    let payload: serde_json::Value = serde_json::from_str(&result).expect("valid JSON payload");
    let items = payload
        .as_array()
        .expect("list_subagents should return an array");
    assert_eq!(
        items.len(),
        2,
        "parent task tree should include descendants"
    );

    let returned_ids = items
        .iter()
        .filter_map(|item| item.get("id").and_then(|value| value.as_str()))
        .collect::<std::collections::HashSet<_>>();
    assert!(returned_ids.contains(child.id.as_str()));
    assert!(returned_ids.contains(grandchild.id.as_str()));

    let grandchild_item = items
        .iter()
        .find(|item| {
            item.get("id").and_then(|value| value.as_str()) == Some(grandchild.id.as_str())
        })
        .expect("grandchild should be present in parent tree listing");
    assert_eq!(grandchild_item["depth"].as_u64(), Some(2));
    assert_eq!(grandchild_item["max_depth"].as_u64(), Some(3));
}

#[tokio::test]
async fn list_subagents_parent_task_tree_excludes_unrelated_same_thread_subagents() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;

    let parent = engine
        .enqueue_task(
            "Parent coordinator".to_string(),
            "Coordinate the child work".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "user",
            None,
            None,
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;

    let mut child = engine
        .enqueue_task(
            "Depth child".to_string(),
            "Inspect deployment risks".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some(parent.id.clone()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    child.containment_scope = Some("subagent-depth:1/3".to_string());

    let mut unrelated = engine
        .enqueue_task(
            "Unrelated sibling".to_string(),
            "Different subtree in the same thread".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "subagent",
            None,
            Some("task-other-root".to_string()),
            Some("thread-parent".to_string()),
            Some("daemon".to_string()),
        )
        .await;
    unrelated.containment_scope = Some("subagent-depth:1/3".to_string());

    {
        let mut tasks = engine.tasks.lock().await;
        if let Some(existing) = tasks.iter_mut().find(|task| task.id == child.id) {
            *existing = child.clone();
        }
        if let Some(existing) = tasks.iter_mut().find(|task| task.id == unrelated.id) {
            *existing = unrelated.clone();
        }
    }

    let result = super::execute_list_subagents(
        &serde_json::json!({ "parent_task_id": parent.id }),
        &engine,
        "thread-parent",
        None,
    )
    .await
    .expect("list_subagents should succeed");

    let payload: serde_json::Value = serde_json::from_str(&result).expect("valid JSON payload");
    let items = payload
        .as_array()
        .expect("list_subagents should return an array");
    let returned_ids = items
        .iter()
        .filter_map(|item| item.get("id").and_then(|value| value.as_str()))
        .collect::<std::collections::HashSet<_>>();

    assert!(returned_ids.contains(child.id.as_str()));
    assert!(
        !returned_ids.contains(unrelated.id.as_str()),
        "parent task tree should not include unrelated same-thread subagents"
    );
}

#[tokio::test]
async fn handoff_thread_agent_push_updates_active_responder_and_writes_system_event() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-handoff-push";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            crate::agent::types::AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
                title: "Handoff candidate".to_string(),
                messages: vec![crate::agent::types::AgentMessage::user(
                    "Please ask Weles to review the risky migration.",
                    1,
                )],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                created_at: 1,
                updated_at: 1,
            },
        );
    }
    engine
        .set_thread_handoff_state(
            thread_id,
            crate::agent::ThreadHandoffState {
                origin_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                active_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                responder_stack: vec![crate::agent::ThreadResponderFrame {
                    agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                    agent_name: crate::agent::agent_identity::MAIN_AGENT_NAME.to_string(),
                    entered_at: 1,
                    entered_via_handoff_event_id: None,
                    linked_thread_id: None,
                }],
                events: Vec::new(),
                pending_approval_id: None,
            },
        )
        .await;

    let tool_call = ToolCall::with_default_weles_review(
            "tool-handoff-thread-push".to_string(),
            ToolFunction {
                name: "handoff_thread_agent".to_string(),
                arguments: serde_json::json!({
                    "action": "push_handoff",
                    "target_agent_id": "weles",
                    "reason": "Risky migration needs governance review",
                    "summary": "Review the migration plan, identify risk, and continue answering from Weles.",
                    "requested_by": "user"
                })
                .to_string(),
            },
        );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "handoff push should succeed: {}",
        result.content
    );
    assert!(result.pending_approval.is_none());
    let state = engine
        .thread_handoff_state(thread_id)
        .await
        .expect("handoff state should exist");
    assert_eq!(state.active_agent_id, "weles");
    assert_eq!(state.responder_stack.len(), 2);

    let thread = engine
        .threads
        .read()
        .await
        .get(thread_id)
        .cloned()
        .expect("thread should exist");
    assert_eq!(thread.agent_name.as_deref(), Some("Weles"));
    let system_event = thread
        .messages
        .iter()
        .find(|message| message.role == crate::agent::types::MessageRole::System)
        .expect("handoff should append a system event");
    assert!(
        system_event.content.contains("[[handoff_event]]"),
        "system event should use the structured handoff marker"
    );
    assert!(system_event.content.contains("\"to_agent_name\":\"Weles\""));
}

#[tokio::test]
async fn participant_managed_handoff_rejects_non_participant_target() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-handoff-non-participant-rejected";

    engine.threads.write().await.insert(
        thread_id.to_string(),
        crate::agent::types::AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Participant-managed handoff".to_string(),
            messages: vec![crate::agent::types::AgentMessage::user(
                "Hand off to a participant only.",
                1,
            )],
            pinned: false,
            upstream_thread_id: None,
            upstream_transport: None,
            upstream_provider: None,
            upstream_model: None,
            upstream_assistant_id: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            created_at: 1,
            updated_at: 1,
        },
    );
    engine
        .upsert_thread_participant(thread_id, "weles", "watch this thread")
        .await
        .expect("participant should register");
    engine
        .set_thread_handoff_state(
            thread_id,
            crate::agent::ThreadHandoffState {
                origin_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                active_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                responder_stack: vec![crate::agent::ThreadResponderFrame {
                    agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                    agent_name: crate::agent::agent_identity::MAIN_AGENT_NAME.to_string(),
                    entered_at: 1,
                    entered_via_handoff_event_id: None,
                    linked_thread_id: None,
                }],
                events: Vec::new(),
                pending_approval_id: None,
            },
        )
        .await;

    let tool_call = ToolCall::with_default_weles_review(
        "tool-handoff-thread-non-participant".to_string(),
        ToolFunction {
            name: "handoff_thread_agent".to_string(),
            arguments: serde_json::json!({
                "action": "push_handoff",
                "target_agent_id": "radogost",
                "reason": "Try handing to a non participant",
                "summary": "This should be rejected.",
                "requested_by": "user"
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(result.is_error, "non-participant handoff should fail");
    assert!(
        result.content.contains("participant"),
        "rejection should mention participant-only ownership: {}",
        result.content
    );
}

#[tokio::test]
async fn participant_managed_handoff_auto_registers_previous_responder() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-handoff-auto-registers-previous";

    engine.threads.write().await.insert(
        thread_id.to_string(),
        crate::agent::types::AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Participant-managed handoff".to_string(),
            messages: vec![crate::agent::types::AgentMessage::user(
                "Hand off to Weles.",
                1,
            )],
            pinned: false,
            upstream_thread_id: None,
            upstream_transport: None,
            upstream_provider: None,
            upstream_model: None,
            upstream_assistant_id: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            created_at: 1,
            updated_at: 1,
        },
    );
    engine
        .upsert_thread_participant(thread_id, "weles", "watch this thread")
        .await
        .expect("participant should register");
    engine
        .set_thread_handoff_state(
            thread_id,
            crate::agent::ThreadHandoffState {
                origin_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                active_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                responder_stack: vec![crate::agent::ThreadResponderFrame {
                    agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                    agent_name: crate::agent::agent_identity::MAIN_AGENT_NAME.to_string(),
                    entered_at: 1,
                    entered_via_handoff_event_id: None,
                    linked_thread_id: None,
                }],
                events: Vec::new(),
                pending_approval_id: None,
            },
        )
        .await;

    let tool_call = ToolCall::with_default_weles_review(
        "tool-handoff-thread-auto-register".to_string(),
        ToolFunction {
            name: "handoff_thread_agent".to_string(),
            arguments: serde_json::json!({
                "action": "push_handoff",
                "target_agent_id": "weles",
                "reason": "Participant takeover",
                "summary": "Let Weles own future replies.",
                "requested_by": "user"
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "participant handoff should succeed: {}",
        result.content
    );
    let participants = engine.list_thread_participants(thread_id).await;
    assert!(participants.iter().any(|participant| {
        participant
            .agent_id
            .eq_ignore_ascii_case(crate::agent::agent_identity::MAIN_AGENT_ID)
            && participant.status == crate::agent::ThreadParticipantStatus::Active
    }));
}

#[tokio::test]
async fn handoff_thread_agent_push_accepts_svarog_alias_for_main_agent() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-handoff-push-svarog-alias";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            crate::agent::types::AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::WELES_AGENT_NAME.to_string()),
                title: "Handoff alias candidate".to_string(),
                messages: vec![crate::agent::types::AgentMessage::user(
                    "Please hand this back to Svarog.",
                    1,
                )],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                created_at: 1,
                updated_at: 1,
            },
        );
    }
    engine
        .set_thread_handoff_state(
            thread_id,
            crate::agent::ThreadHandoffState {
                origin_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                active_agent_id: crate::agent::agent_identity::WELES_AGENT_ID.to_string(),
                responder_stack: vec![
                    crate::agent::ThreadResponderFrame {
                        agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                        agent_name: crate::agent::agent_identity::MAIN_AGENT_NAME.to_string(),
                        entered_at: 1,
                        entered_via_handoff_event_id: None,
                        linked_thread_id: None,
                    },
                    crate::agent::ThreadResponderFrame {
                        agent_id: crate::agent::agent_identity::WELES_AGENT_ID.to_string(),
                        agent_name: crate::agent::agent_identity::WELES_AGENT_NAME.to_string(),
                        entered_at: 2,
                        entered_via_handoff_event_id: Some("handoff-existing".to_string()),
                        linked_thread_id: Some("handoff:existing".to_string()),
                    },
                ],
                events: Vec::new(),
                pending_approval_id: None,
            },
        )
        .await;

    let tool_call = ToolCall::with_default_weles_review(
        "tool-handoff-thread-push-svarog-alias".to_string(),
        ToolFunction {
            name: "handoff_thread_agent".to_string(),
            arguments: serde_json::json!({
                "action": "push_handoff",
                "target_agent_id": "svarog",
                "reason": "Operator requested to switch to Svarog",
                "summary": "Operator wants to continue with Svarog, the main agent.",
                "requested_by": "user"
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "svarog alias handoff should succeed: {}",
        result.content
    );
    let state = engine
        .thread_handoff_state(thread_id)
        .await
        .expect("handoff state should exist");
    assert_eq!(
        state.active_agent_id,
        crate::agent::agent_identity::MAIN_AGENT_ID
    );
}

#[tokio::test]
async fn handoff_thread_agent_agent_push_requires_approval_outside_yolo() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.managed_execution.security_level = SecurityLevel::Moderate;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-handoff-approval";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            crate::agent::types::AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
                title: "Approval handoff".to_string(),
                messages: vec![crate::agent::types::AgentMessage::user(
                    "Maybe ask Weles to take over this risky thread.",
                    1,
                )],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                created_at: 1,
                updated_at: 1,
            },
        );
    }
    engine
        .set_thread_handoff_state(
            thread_id,
            crate::agent::ThreadHandoffState {
                origin_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                active_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                responder_stack: vec![crate::agent::ThreadResponderFrame {
                    agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                    agent_name: crate::agent::agent_identity::MAIN_AGENT_NAME.to_string(),
                    entered_at: 1,
                    entered_via_handoff_event_id: None,
                    linked_thread_id: None,
                }],
                events: Vec::new(),
                pending_approval_id: None,
            },
        )
        .await;

    let tool_call = ToolCall::with_default_weles_review(
        "tool-handoff-thread-approval".to_string(),
        ToolFunction {
            name: "handoff_thread_agent".to_string(),
            arguments: serde_json::json!({
                "action": "push_handoff",
                "target_agent_id": "weles",
                "reason": "Governance review required",
                "summary": "Let Weles take over and continue this safety-sensitive thread.",
                "requested_by": "agent"
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(!result.is_error, "approval handoff should not hard-fail");
    let pending = result
        .pending_approval
        .as_ref()
        .expect("agent-initiated handoff should request approval");
    assert!(pending.command.contains("handoff_thread_agent"));

    let state = engine
        .thread_handoff_state(thread_id)
        .await
        .expect("handoff state should still exist");
    assert_eq!(
        state.active_agent_id,
        crate::agent::agent_identity::MAIN_AGENT_ID,
        "active responder should not switch before approval"
    );
    assert_eq!(
        state.pending_approval_id.as_deref(),
        Some(pending.approval_id.as_str())
    );

    let tasks = engine.list_tasks().await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].status,
        crate::agent::types::TaskStatus::AwaitingApproval
    );
    assert_eq!(tasks[0].source, "thread_handoff");
    assert_eq!(
        tasks[0].awaiting_approval_id.as_deref(),
        Some(pending.approval_id.as_str())
    );
}

#[tokio::test]
async fn handoff_thread_agent_return_pops_stack_and_restores_previous_responder() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-handoff-return";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            crate::agent::types::AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::WELES_AGENT_NAME.to_string()),
                title: "Return handoff".to_string(),
                messages: vec![crate::agent::types::AgentMessage::user(
                    "Let Weles finish and then hand the thread back.",
                    1,
                )],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                created_at: 1,
                updated_at: 1,
            },
        );
    }
    engine
        .set_thread_handoff_state(
            thread_id,
            crate::agent::ThreadHandoffState {
                origin_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                active_agent_id: crate::agent::agent_identity::WELES_AGENT_ID.to_string(),
                responder_stack: vec![
                    crate::agent::ThreadResponderFrame {
                        agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                        agent_name: crate::agent::agent_identity::MAIN_AGENT_NAME.to_string(),
                        entered_at: 1,
                        entered_via_handoff_event_id: None,
                        linked_thread_id: None,
                    },
                    crate::agent::ThreadResponderFrame {
                        agent_id: crate::agent::agent_identity::WELES_AGENT_ID.to_string(),
                        agent_name: crate::agent::agent_identity::WELES_AGENT_NAME.to_string(),
                        entered_at: 2,
                        entered_via_handoff_event_id: Some("handoff-existing".to_string()),
                        linked_thread_id: Some("handoff:existing".to_string()),
                    },
                ],
                events: Vec::new(),
                pending_approval_id: None,
            },
        )
        .await;

    let tool_call = ToolCall::with_default_weles_review(
        "tool-handoff-thread-return".to_string(),
        ToolFunction {
            name: "handoff_thread_agent".to_string(),
            arguments: serde_json::json!({
                "action": "return_handoff",
                "reason": "Weles completed the governance pass",
                "summary": "Returning control to Swarog with the review result and next steps.",
                "requested_by": "agent"
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !result.is_error,
        "handoff return should succeed: {}",
        result.content
    );
    assert!(result.pending_approval.is_none());
    let state = engine
        .thread_handoff_state(thread_id)
        .await
        .expect("handoff state should exist");
    assert_eq!(
        state.active_agent_id,
        crate::agent::agent_identity::MAIN_AGENT_ID
    );
    assert_eq!(state.responder_stack.len(), 1);

    let thread = engine
        .threads
        .read()
        .await
        .get(thread_id)
        .cloned()
        .expect("thread should exist");
    assert_eq!(
        thread.agent_name.as_deref(),
        Some(crate::agent::agent_identity::MAIN_AGENT_NAME)
    );
    let system_event = thread
        .messages
        .iter()
        .find(|message| message.role == crate::agent::types::MessageRole::System)
        .expect("return handoff should append a system event");
    assert!(system_event.content.contains("\"kind\":\"return\""));
    assert!(
        system_event.content.contains(&format!(
            "\"to_agent_name\":\"{}\"",
            crate::agent::agent_identity::MAIN_AGENT_NAME
        )),
        "system event should announce the restored responder"
    );
}

#[tokio::test]
async fn approved_thread_handoff_activation_updates_stack_and_thread_identity() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.managed_execution.security_level = SecurityLevel::Moderate;
    let engine = AgentEngine::new_test(manager.clone(), config, root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-handoff-approved";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            crate::agent::types::AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
                title: "Approve handoff".to_string(),
                messages: vec![crate::agent::types::AgentMessage::user(
                    "If needed, let Weles take over after approval.",
                    1,
                )],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 0,
                total_output_tokens: 0,
                created_at: 1,
                updated_at: 1,
            },
        );
    }
    engine
        .set_thread_handoff_state(
            thread_id,
            crate::agent::ThreadHandoffState {
                origin_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                active_agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                responder_stack: vec![crate::agent::ThreadResponderFrame {
                    agent_id: crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
                    agent_name: crate::agent::agent_identity::MAIN_AGENT_NAME.to_string(),
                    entered_at: 1,
                    entered_via_handoff_event_id: None,
                    linked_thread_id: None,
                }],
                events: Vec::new(),
                pending_approval_id: None,
            },
        )
        .await;

    let tool_call = ToolCall::with_default_weles_review(
        "tool-handoff-thread-approved".to_string(),
        ToolFunction {
            name: "handoff_thread_agent".to_string(),
            arguments: serde_json::json!({
                "action": "push_handoff",
                "target_agent_id": "weles",
                "reason": "Governance review required",
                "summary": "Let Weles take over and continue this safety-sensitive thread.",
                "requested_by": "agent"
            })
            .to_string(),
        },
    );

    let result = execute_tool(
        &tool_call,
        &engine,
        thread_id,
        None,
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    let pending = result
        .pending_approval
        .as_ref()
        .expect("agent-initiated handoff should request approval");
    assert!(
        engine
            .handle_task_approval_resolution(
                &pending.approval_id,
                amux_protocol::ApprovalDecision::ApproveOnce
            )
            .await,
        "approval resolution should find the synthetic handoff task"
    );

    let state = engine
        .thread_handoff_state(thread_id)
        .await
        .expect("handoff state should exist");
    assert_eq!(
        state.active_agent_id,
        crate::agent::agent_identity::WELES_AGENT_ID
    );
    assert!(state.pending_approval_id.is_none());
    assert_eq!(state.responder_stack.len(), 2);

    let thread = engine
        .threads
        .read()
        .await
        .get(thread_id)
        .cloned()
        .expect("thread should exist");
    assert_eq!(thread.agent_name.as_deref(), Some("Weles"));
    assert!(
        thread.messages.iter().any(|message| {
            message.role == crate::agent::types::MessageRole::System
                && message.content.contains("\"approval_id\":\"")
        }),
        "approval activation should append a structured handoff system event"
    );
}

#[tokio::test]
async fn get_todos_returns_thread_scoped_items_with_optional_task_id() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager.clone(), AgentConfig::default(), root.path()).await;
    let (event_tx, _) = broadcast::channel(8);
    let thread_id = "thread-get-todos";
    let task = engine
        .enqueue_task(
            "Track thread todos".to_string(),
            "Keep a live thread todo list available to the runtime.".to_string(),
            "normal",
            None,
            None,
            Vec::new(),
            None,
            "agent",
            None,
            None,
            Some(thread_id.to_string()),
            Some("daemon".to_string()),
        )
        .await;

    let update_call = ToolCall::with_default_weles_review(
        "tool-update-todos-runtime".to_string(),
        ToolFunction {
            name: "update_todo".to_string(),
            arguments: serde_json::json!({
                "items": [
                    { "content": "Inspect current thread todos", "status": "in_progress" },
                    { "content": "Return todo snapshot", "status": "pending" }
                ]
            })
            .to_string(),
        },
    );

    let update_result = execute_tool(
        &update_call,
        &engine,
        thread_id,
        Some(task.id.as_str()),
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !update_result.is_error,
        "update_todo should succeed before get_todos is called: {}",
        update_result.content
    );

    let get_call = ToolCall::with_default_weles_review(
        "tool-get-todos-runtime".to_string(),
        ToolFunction {
            name: "get_todos".to_string(),
            arguments: serde_json::json!({
                "thread_id": thread_id,
                "task_id": task.id,
            })
            .to_string(),
        },
    );

    let get_result = execute_tool(
        &get_call,
        &engine,
        thread_id,
        Some(task.id.as_str()),
        &manager,
        None,
        &event_tx,
        root.path(),
        &engine.http_client,
        None,
    )
    .await;

    assert!(
        !get_result.is_error,
        "get_todos should return thread-scoped items: {}",
        get_result.content
    );

    let payload: serde_json::Value =
        serde_json::from_str(&get_result.content).expect("get_todos should return JSON");
    assert_eq!(payload["thread_id"], thread_id);
    let items = payload["items"]
        .as_array()
        .expect("items should be serialized as an array");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["content"], "Inspect current thread todos");
    assert_eq!(items[0]["status"], "in_progress");
    assert_eq!(items[1]["content"], "Return todo snapshot");
    assert_eq!(items[1]["status"], "pending");
}
