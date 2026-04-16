use super::*;
use amux_shared::providers::PROVIDER_ID_OPENAI;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn persisted_assistant_messages_reload_provider_final_result_metadata() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    let thread_id = "thread_provider_final_result_reload";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            AgentThread {
                id: thread_id.to_string(),
                agent_name: None,
                title: "Provider final result reload".to_string(),
                messages: vec![
                    AgentMessage::user("hello", 1),
                    AgentMessage {
                        id: "assistant-provider-final-result".to_string(),
                        role: MessageRole::Assistant,
                        content: "Hello from Responses.".to_string(),
                        tool_calls: None,
                        tool_call_id: None,
                        tool_name: None,
                        tool_arguments: None,
                        tool_status: None,
                        weles_review: None,
                        input_tokens: 7,
                        output_tokens: 3,
                        cost: None,
                        provider: Some(amux_shared::providers::PROVIDER_ID_OPENAI.to_string()),
                        model: Some("gpt-5.4-mini".to_string()),
                        api_transport: Some(ApiTransport::Responses),
                        response_id: Some("resp_provider_final_result".to_string()),
                        upstream_message: None,
                        provider_final_result: Some(
                            CompletionProviderFinalResult::OpenAiResponses(
                                CompletionOpenAiResponsesFinalResult {
                                    id: Some("resp_provider_final_result".to_string()),
                                    output_text: "Hello from Responses.".to_string(),
                                    reasoning: None,
                                    tool_calls: Vec::new(),
                                    response: Some(
                                        crate::agent::llm_client::OpenAiResponsesTerminalResponse {
                                            id: "resp_provider_final_result".to_string(),
                                            object: "response".to_string(),
                                            status: "completed".to_string(),
                                            output: Vec::new(),
                                            usage: crate::agent::llm_client::OpenAiResponsesResponseUsage {
                                                input_tokens: 7,
                                                output_tokens: 3,
                                                total_tokens: None,
                                            },
                                            error: None,
                                        },
                                    ),
                                    response_json: Some(r#"{"id":"resp_provider_final_result","object":"response","status":"completed","output":[],"usage":{"input_tokens":7,"output_tokens":3},"error":null,"metadata":{"source":"persisted-test"}}"#.to_string()),
                                    input_tokens: Some(7),
                                    output_tokens: Some(3),
                                },
                            ),
                        ),
                        author_agent_id: None,
                        author_agent_name: None,
                        reasoning: None,
                        message_kind: AgentMessageKind::Normal,
                        compaction_strategy: None,
                        compaction_payload: None,
                        offloaded_payload_id: None,
                        structural_refs: Vec::new(),
                        timestamp: 2,
                    },
                ],
                pinned: false,
                upstream_thread_id: None,
                upstream_transport: None,
                upstream_provider: None,
                upstream_model: None,
                upstream_assistant_id: None,
                total_input_tokens: 7,
                total_output_tokens: 3,
                created_at: 1,
                updated_at: 2,
            },
        );
    }

    engine.persist_thread_by_id(thread_id).await;
    drop(engine);

    let manager = SessionManager::new_test(root.path()).await;
    let reloaded_engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    reloaded_engine.hydrate().await.expect("hydrate");
    let threads = reloaded_engine.threads.read().await;
    let thread = threads.get(thread_id).expect("thread should reload");
    let assistant = thread
        .messages
        .iter()
        .find(|message| message.role == MessageRole::Assistant)
        .expect("assistant message should reload");

    match assistant
        .provider_final_result
        .as_ref()
        .expect("provider final result should reload")
    {
        CompletionProviderFinalResult::OpenAiResponses(response) => {
            assert_eq!(response.id.as_deref(), Some("resp_provider_final_result"));
            assert_eq!(response.output_text, "Hello from Responses.");
            assert_eq!(response.input_tokens, Some(7));
            assert_eq!(response.output_tokens, Some(3));
            let terminal_response = response
                .response
                .as_ref()
                .expect("canonical terminal response should reload");
            assert_eq!(terminal_response.id, "resp_provider_final_result");
            assert_eq!(terminal_response.object, "response");
            assert_eq!(terminal_response.status, "completed");
            assert_eq!(terminal_response.output, Vec::<serde_json::Value>::new());
            assert_eq!(terminal_response.usage.input_tokens, 7);
            assert_eq!(terminal_response.usage.output_tokens, 3);
            let response_json: serde_json::Value = serde_json::from_str(
                response
                    .response_json
                    .as_deref()
                    .expect("raw terminal response JSON should reload"),
            )
            .expect("response_json should decode");
            assert_eq!(response_json["metadata"]["source"], "persisted-test");
        }
        other => panic!("expected OpenAI Responses final result, got {other:?}"),
    }
}

#[tokio::test]
async fn persisted_thread_metadata_reloads_thread_participants() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    let thread_id = "thread_participants_reload";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
                title: "Participants reload".to_string(),
                messages: vec![AgentMessage::user("hello", 1)],
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
    engine.thread_participants.write().await.insert(
        thread_id.to_string(),
        vec![
            crate::agent::ThreadParticipantState {
                agent_id: "weles".to_string(),
                agent_name: "Weles".to_string(),
                instruction: "verify claims".to_string(),
                status: crate::agent::ThreadParticipantStatus::Active,
                created_at: 10,
                updated_at: 11,
                deactivated_at: None,
                last_contribution_at: Some(12),
                last_observed_visible_message_at: Some(12),
                always_auto_response: false,
            },
            crate::agent::ThreadParticipantState {
                agent_id: "rarog".to_string(),
                agent_name: "Rarog".to_string(),
                instruction: "watch performance".to_string(),
                status: crate::agent::ThreadParticipantStatus::Inactive,
                created_at: 20,
                updated_at: 21,
                deactivated_at: Some(22),
                last_contribution_at: None,
                last_observed_visible_message_at: None,
                always_auto_response: false,
            },
        ],
    );

    engine.persist_thread_by_id(thread_id).await;
    drop(engine);

    let manager = SessionManager::new_test(root.path()).await;
    let reloaded_engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    reloaded_engine.hydrate().await.expect("hydrate");

    let participants = reloaded_engine
        .thread_participants
        .read()
        .await
        .get(thread_id)
        .cloned()
        .expect("participants should reload");

    assert_eq!(participants.len(), 2);
    assert_eq!(participants[0].agent_id, "weles");
    assert_eq!(participants[0].instruction, "verify claims");
    assert_eq!(
        participants[0].status,
        crate::agent::ThreadParticipantStatus::Active
    );
    assert_eq!(participants[0].last_contribution_at, Some(12));
    assert_eq!(participants[0].last_observed_visible_message_at, Some(12));
    assert_eq!(participants[1].agent_id, "rarog");
    assert_eq!(
        participants[1].status,
        crate::agent::ThreadParticipantStatus::Inactive
    );
    assert_eq!(participants[1].deactivated_at, Some(22));
    assert_eq!(participants[1].last_observed_visible_message_at, None);
}

#[tokio::test]
async fn visible_thread_participant_send_records_message_author_and_updates_participant_state() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind participant visible-send server");
    let addr = listener
        .local_addr()
        .expect("participant visible-send addr");
    let request_counter = Arc::new(AtomicUsize::new(0));
    let request_counter_task = request_counter.clone();

    tokio::spawn(async move {
        for _ in 0..3 {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let body = read_http_request_body(&mut socket)
                .await
                .expect("read participant visible-send request");
            let response_body = if body.contains("Role: participant observer") {
                concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_visible_observer\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"NO_SUGGESTION\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_visible_observer\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":1},\"error\":null}}\n\n"
                )
            } else {
                match request_counter_task.fetch_add(1, Ordering::SeqCst) {
                0 => concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_visible\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"I checked that claim and it is inaccurate.\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_visible\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":11,\"output_tokens\":8},\"error\":null}}\n\n"
                ),
                1 => concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_visible_follow_up\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Swarozyc acknowledged the verification.\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_visible_follow_up\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":9,\"output_tokens\":5},\"error\":null}}\n\n"
                ),
                _ => concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_visible_tail\"}}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_visible_tail\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0},\"error\":null}}\n\n"
                ),
                }
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write participant visible-send response");
        }
    });

    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = format!("http://{addr}/v1");
    config.model = "gpt-5.4-mini".to_string();
    config.api_key = "test-key".to_string();
    config.auth_source = AuthSource::ApiKey;
    config.api_transport = ApiTransport::Responses;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 1;

    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let thread_id = "thread_participant_visible_send";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
                title: "Participant visible send".to_string(),
                messages: vec![AgentMessage::user("Can someone verify this?", 1)],
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
        .upsert_thread_participant(thread_id, "weles", "verify claims and jump in when needed")
        .await
        .expect("participant should register");

    engine
        .send_visible_thread_participant_message(
            thread_id,
            "weles",
            None,
            "The operator asked for a factual verification. Reply only if you found something important.",
        )
        .await
        .expect("participant visible send should succeed");

    let thread_messages = {
        let threads = engine.threads.read().await;
        threads
            .get(thread_id)
            .expect("thread should still exist after participant send")
            .messages
            .clone()
    };
    let participant_idx = thread_messages
        .iter()
        .position(|message| {
            message.role == MessageRole::Assistant
                && message.author_agent_id.as_deref() == Some("weles")
        })
        .expect("participant visible send should append a participant-authored assistant message");
    let assistant = &thread_messages[participant_idx];

    assert_eq!(
        assistant.content,
        "I checked that claim and it is inaccurate."
    );
    assert_eq!(assistant.author_agent_id.as_deref(), Some("weles"));
    assert_eq!(assistant.author_agent_name.as_deref(), Some("Weles"));
    assert_eq!(
        thread_messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .count(),
        1,
        "visible participant sends should not append an extra user turn"
    );
    let follow_up = thread_messages[participant_idx + 1..]
        .iter()
        .find(|message| {
            message.role == MessageRole::Assistant
                && message.author_agent_id.as_deref() != Some("weles")
        })
        .expect("visible participant send should trigger a separate active-agent follow-up");
    assert_eq!(follow_up.content, "Swarozyc acknowledged the verification.");

    let participants = engine.list_thread_participants(thread_id).await;
    let weles = participants
        .iter()
        .find(|participant| participant.agent_id == "weles")
        .expect("weles participant should remain registered");
    assert!(
        weles.last_contribution_at.is_some(),
        "participant send should stamp last contribution time"
    );
}

#[tokio::test]
async fn visible_thread_participant_send_does_not_trigger_self_reply_for_active_participant() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind participant self-reply server");
    let addr = listener.local_addr().expect("participant self-reply addr");
    let request_counter = Arc::new(AtomicUsize::new(0));
    let request_counter_task = request_counter.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let body = read_http_request_body(&mut socket)
                .await
                .expect("read participant self-reply request");
            let response_body = if body.contains("Role: participant observer") {
                concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_self_reply_observer\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"NO_SUGGESTION\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_self_reply_observer\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":1},\"error\":null}}\n\n"
                )
            } else {
                match request_counter_task.fetch_add(1, Ordering::SeqCst) {
                    0 => concat!(
                        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_self_reply\"}}\n\n",
                        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Weles sends the verification.\"}\n\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_self_reply\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":8,\"output_tokens\":4},\"error\":null}}\n\n"
                    ),
                    _ => concat!(
                        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_self_reply_extra\"}}\n\n",
                        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"unexpected extra request\"}\n\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_self_reply_extra\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1},\"error\":null}}\n\n"
                    ),
                }
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write participant self-reply response");
        }
    });

    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = format!("http://{addr}/v1");
    config.model = "gpt-5.4-mini".to_string();
    config.api_key = "test-key".to_string();
    config.auth_source = AuthSource::ApiKey;
    config.api_transport = ApiTransport::Responses;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 1;

    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let thread_id = "thread_participant_visible_send_self_reply";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::WELES_AGENT_NAME.to_string()),
                title: "Participant visible self reply".to_string(),
                messages: vec![AgentMessage::user("Weles, verify this.", 1)],
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
            ThreadHandoffState {
                origin_agent_id: MAIN_AGENT_ID.to_string(),
                active_agent_id: crate::agent::agent_identity::WELES_AGENT_ID.to_string(),
                responder_stack: vec![
                    ThreadResponderFrame {
                        agent_id: MAIN_AGENT_ID.to_string(),
                        agent_name: MAIN_AGENT_NAME.to_string(),
                        entered_at: 1,
                        entered_via_handoff_event_id: None,
                        linked_thread_id: None,
                    },
                    ThreadResponderFrame {
                        agent_id: crate::agent::agent_identity::WELES_AGENT_ID.to_string(),
                        agent_name: crate::agent::agent_identity::WELES_AGENT_NAME.to_string(),
                        entered_at: 2,
                        entered_via_handoff_event_id: Some("handoff-1".to_string()),
                        linked_thread_id: Some("dm:svarog:weles".to_string()),
                    },
                ],
                events: Vec::new(),
                pending_approval_id: None,
            },
        )
        .await;

    engine
        .upsert_thread_participant(thread_id, "weles", "verify claims and jump in when needed")
        .await
        .expect("participant should register");

    engine
        .send_visible_thread_participant_message(
            thread_id,
            "weles",
            None,
            "Reply only if you found something important.",
        )
        .await
        .expect("participant visible send should succeed");

    let thread_messages = {
        let threads = engine.threads.read().await;
        threads
            .get(thread_id)
            .expect("thread should still exist after participant send")
            .messages
            .clone()
    };
    assert_eq!(
        thread_messages
            .iter()
            .filter(|message| message.role == MessageRole::Assistant)
            .count(),
        1,
        "active participant should not trigger a self-reply continuation"
    );
    assert_eq!(
        request_counter.load(Ordering::SeqCst),
        1,
        "active participant visible send should only issue the participant reply request"
    );
}

#[tokio::test]
async fn visible_thread_participant_send_keeps_no_suggestion_visible_but_skips_main_follow_up() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind participant no-suggestion server");
    let addr = listener
        .local_addr()
        .expect("participant no-suggestion addr");
    let request_counter = Arc::new(AtomicUsize::new(0));
    let request_counter_task = request_counter.clone();

    tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let body = read_http_request_body(&mut socket)
                .await
                .expect("read participant no-suggestion request");
            let response_body = if body.contains("Role: participant observer") {
                concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_visible_observer\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"NO_SUGGESTION\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_visible_observer\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":1},\"error\":null}}\n\n"
                )
            } else {
                match request_counter_task.fetch_add(1, Ordering::SeqCst) {
                    0 => concat!(
                        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_visible_no_suggestion\"}}\n\n",
                        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"NO_SUGGESTION\"}\n\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_visible_no_suggestion\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":1},\"error\":null}}\n\n"
                    ),
                    _ => concat!(
                        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_visible_tail\"}}\n\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_visible_tail\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0},\"error\":null}}\n\n"
                    ),
                }
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write participant no-suggestion response");
        }
    });

    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = format!("http://{addr}/v1");
    config.model = "gpt-5.4-mini".to_string();
    config.api_key = "test-key".to_string();
    config.auth_source = AuthSource::ApiKey;
    config.api_transport = ApiTransport::Responses;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 1;

    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let thread_id = "thread_participant_visible_send_no_suggestion";

    {
        let mut threads = engine.threads.write().await;
        threads.insert(
            thread_id.to_string(),
            AgentThread {
                id: thread_id.to_string(),
                agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
                title: "Participant visible send no suggestion".to_string(),
                messages: vec![AgentMessage::user("Can someone verify this?", 1)],
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
        .upsert_thread_participant(thread_id, "weles", "verify claims and jump in when needed")
        .await
        .expect("participant should register");

    engine
        .send_visible_thread_participant_message(
            thread_id,
            "weles",
            None,
            "Reply only if you found something important.",
        )
        .await
        .expect("participant visible send should succeed");

    let thread_messages = {
        let threads = engine.threads.read().await;
        threads
            .get(thread_id)
            .expect("thread should still exist after participant send")
            .messages
            .clone()
    };
    let participant_messages = thread_messages
        .iter()
        .filter(|message| message.author_agent_id.as_deref() == Some("weles"))
        .collect::<Vec<_>>();
    assert_eq!(
        participant_messages.len(),
        1,
        "visible participant send should still append the participant-authored no-suggestion turn"
    );
    assert_eq!(
        participant_messages[0].content, "NO_SUGGESTION",
        "literal NO_SUGGESTION may remain visible in the transcript"
    );
    assert!(
        thread_messages.iter().all(|message| {
            !(message.role == MessageRole::Assistant
                && message.author_agent_id.as_deref() != Some("weles")
                && message.content != "NO_SUGGESTION")
        }),
        "no-suggestion participant sends must not force a main-agent follow-up"
    );
    let participants = engine.list_thread_participants(thread_id).await;
    let weles = participants
        .iter()
        .find(|participant| participant.agent_id == "weles")
        .expect("weles participant should remain registered");
    assert!(
        weles.last_contribution_at.is_some(),
        "visible no-suggestion turns still count as participant-authored visible output"
    );
}

#[tokio::test]
async fn visible_thread_participant_send_stays_hidden_until_final_message_is_ready() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind participant hidden visible-send server");
    let addr = listener
        .local_addr()
        .expect("participant hidden visible-send addr");
    let first_request_started = Arc::new(tokio::sync::Notify::new());
    let release_first_response = Arc::new(tokio::sync::Notify::new());
    let first_request_started_task = first_request_started.clone();
    let release_first_response_task = release_first_response.clone();

    tokio::spawn(async move {
        for attempt in 0..2 {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let body = read_http_request_body(&mut socket)
                .await
                .expect("read participant hidden visible-send request");
            let response_body = match attempt {
                0 => {
                    assert!(
                        body.contains("Role: visible thread participant"),
                        "first request should be the hidden participant drafting prompt"
                    );
                    first_request_started_task.notify_waiters();
                    release_first_response_task.notified().await;
                    concat!(
                        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_hidden_visible\"}}\n\n",
                        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hidden participant final reply.\"}\n\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_hidden_visible\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":9,\"output_tokens\":5},\"error\":null}}\n\n"
                    )
                }
                _ => concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_hidden_follow_up\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Swarozyc saw the participant reply.\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_hidden_follow_up\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":8,\"output_tokens\":4},\"error\":null}}\n\n"
                ),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write participant hidden visible-send response");
        }
    });

    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = format!("http://{addr}/v1");
    config.model = "gpt-5.4-mini".to_string();
    config.api_key = "test-key".to_string();
    config.auth_source = AuthSource::ApiKey;
    config.api_transport = ApiTransport::Responses;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 1;

    let engine = Arc::new(AgentEngine::new_test(manager, config, root.path()).await);
    let thread_id = "thread_participant_visible_send_hidden_until_ready";

    engine.threads.write().await.insert(
        thread_id.to_string(),
        AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Participant hidden visible send".to_string(),
            messages: vec![AgentMessage::user("Can someone verify this?", 1)],
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
        .upsert_thread_participant(thread_id, "weles", "verify claims and jump in when needed")
        .await
        .expect("participant should register");

    let send_task = tokio::spawn({
        let engine = engine.clone();
        async move {
            engine
                .send_visible_thread_participant_message(
                    thread_id,
                    "weles",
                    None,
                    "Reply only if you found something important.",
                )
                .await
        }
    });

    timeout(Duration::from_secs(1), first_request_started.notified())
        .await
        .expect("hidden participant drafting request should start");

    let in_flight_messages = {
        let threads = engine.threads.read().await;
        threads
            .get(thread_id)
            .expect("thread should exist while participant drafts")
            .messages
            .clone()
    };
    assert_eq!(
        in_flight_messages
            .iter()
            .filter(|message| message.role == MessageRole::User)
            .count(),
        1,
        "participant drafting should not inject a temporary visible user turn"
    );
    {
        let streams = engine.stream_cancellations.lock().await;
        assert!(
            !streams.contains_key(thread_id),
            "hidden participant drafting should not register a visible thread stream"
        );
    }

    release_first_response.notify_waiters();
    timeout(Duration::from_secs(2), send_task)
        .await
        .expect("participant hidden visible send should finish")
        .expect("join participant hidden visible send task")
        .expect("participant hidden visible send should succeed");

    let final_messages = engine
        .get_thread(thread_id)
        .await
        .expect("thread should exist after participant hidden visible send")
        .messages;
    assert!(
        final_messages.iter().any(|message| {
            message.role == MessageRole::Assistant
                && message.author_agent_id.as_deref() == Some("weles")
                && message.content == "Hidden participant final reply."
        }),
        "final visible thread should contain only the completed participant-authored message"
    );
    let playground_thread_id =
        crate::agent::agent_identity::participant_playground_thread_id(thread_id, "weles");
    let playground = engine
        .get_thread_filtered(&playground_thread_id, true, None, 0)
        .await
        .expect("participant playground thread should be created")
        .thread;
    assert!(
        playground
            .messages
            .iter()
            .any(|message| message.role == MessageRole::Assistant),
        "hidden participant drafting should persist in the dedicated playground thread"
    );
}

#[tokio::test]
async fn force_send_interrupts_stream() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind participant force server");
    let addr = listener.local_addr().expect("participant force addr");
    let request_counter = Arc::new(AtomicUsize::new(0));
    let request_counter_task = request_counter.clone();

    tokio::spawn(async move {
        for _ in 0..3 {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let _ = read_http_request_body(&mut socket)
                .await
                .expect("read participant force request");
            let response_body = match request_counter_task.fetch_add(1, Ordering::SeqCst) {
                0 => concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_force_follow_up\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Swarozyc is handling the urgent fix.\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_force_follow_up\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":8,\"output_tokens\":5},\"error\":null}}\n\n"
                ),
                _ => concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_force_tail\"}}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_force_tail\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0},\"error\":null}}\n\n"
                ),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write participant force response");
        }
    });
    let server_url = format!("http://{addr}/v1");

    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = server_url;
    config.model = "gpt-5.4-mini".to_string();
    config.api_key = "test-key".to_string();
    config.auth_source = AuthSource::ApiKey;
    config.api_transport = ApiTransport::Responses;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 1;

    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let thread_id = "thread_participant_force_interrupt";
    engine.threads.write().await.insert(
        thread_id.to_string(),
        AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Participant force interrupt".to_string(),
            messages: vec![AgentMessage::user("hello", 1)],
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
        .upsert_thread_participant(thread_id, "weles", "verify claims")
        .await
        .expect("participant should register");

    let (_generation, token, _retry_now) = engine.begin_stream_cancellation(thread_id).await;

    let suggestion = engine
        .queue_thread_participant_suggestion(thread_id, "weles", "Urgent fix", true)
        .await
        .expect("force-send enqueue should succeed");

    assert_eq!(suggestion.target_agent_id, "weles");
    assert!(
        token.is_cancelled(),
        "force-send should cancel the active stream"
    );

    let thread_messages = {
        let threads = engine.threads.read().await;
        threads
            .get(thread_id)
            .expect("thread should still exist after force-send")
            .messages
            .clone()
    };
    assert!(thread_messages.iter().any(|message| {
        message.role == MessageRole::Assistant
            && message.author_agent_id.as_deref() == Some("weles")
    }));
    let follow_up = thread_messages
        .iter()
        .rev()
        .find(|message| {
            message.role == MessageRole::Assistant
                && message.author_agent_id.as_deref() != Some("weles")
        })
        .expect("force-send should trigger a separate active-agent follow-up");
    assert_eq!(follow_up.content, "Swarozyc is handling the urgent fix.");
}

#[tokio::test]
async fn force_send_auto_posts_on_enqueue() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind participant auto-post server");
    let addr = listener.local_addr().expect("participant auto-post addr");
    let request_counter = Arc::new(AtomicUsize::new(0));
    let request_counter_task = request_counter.clone();

    tokio::spawn(async move {
        for _ in 0..3 {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let _ = read_http_request_body(&mut socket)
                .await
                .expect("read participant auto-post request");
            let response_body = match request_counter_task.fetch_add(1, Ordering::SeqCst) {
                0 => concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_force_auto_follow_up\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Swarozyc picked up the auto-posted fix.\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_force_auto_follow_up\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":8,\"output_tokens\":6},\"error\":null}}\n\n"
                ),
                _ => concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_participant_force_auto_tail\"}}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_participant_force_auto_tail\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0},\"error\":null}}\n\n"
                ),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write participant auto-post response");
        }
    });
    let server_url = format!("http://{addr}/v1");

    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = server_url;
    config.model = "gpt-5.4-mini".to_string();
    config.api_key = "test-key".to_string();
    config.auth_source = AuthSource::ApiKey;
    config.api_transport = ApiTransport::Responses;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 1;

    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let thread_id = "thread_participant_force_auto";
    engine.threads.write().await.insert(
        thread_id.to_string(),
        AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Participant force auto".to_string(),
            messages: vec![AgentMessage::user("hello", 1)],
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
        .upsert_thread_participant(thread_id, "weles", "verify claims")
        .await
        .expect("participant should register");

    let _ = engine
        .queue_thread_participant_suggestion(thread_id, "weles", "Auto-posted fix", true)
        .await
        .expect("force-send enqueue should succeed");

    let thread_messages = {
        let threads = engine.threads.read().await;
        threads
            .get(thread_id)
            .expect("thread should still exist after auto-post")
            .messages
            .clone()
    };
    assert!(thread_messages.iter().any(|message| {
        message.role == MessageRole::Assistant
            && message.author_agent_id.as_deref() == Some("weles")
    }));
    let follow_up = thread_messages
        .iter()
        .rev()
        .find(|message| {
            message.role == MessageRole::Assistant
                && message.author_agent_id.as_deref() != Some("weles")
        })
        .expect("force-send enqueue should trigger a separate active-agent follow-up");
    assert_eq!(follow_up.content, "Swarozyc picked up the auto-posted fix.");
}

#[tokio::test]
async fn participant_suggestions_fifo_order() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    let thread_id = "thread_participant_fifo";
    engine.threads.write().await.insert(
        thread_id.to_string(),
        AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Participant FIFO".to_string(),
            messages: vec![AgentMessage::user("hello", 1)],
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
        .upsert_thread_participant(thread_id, "weles", "verify claims")
        .await
        .expect("participant should register");
    {
        let mut streams = engine.stream_cancellations.lock().await;
        streams.insert(
            thread_id.to_string(),
            StreamCancellationEntry {
                generation: 1,
                token: CancellationToken::new(),
                retry_now: Arc::new(Notify::new()),
                started_at: 1,
                last_progress_at: 1,
                last_progress_kind: StreamProgressKind::Started,
                last_progress_excerpt: String::new(),
            },
        );
    }

    let first = engine
        .queue_thread_participant_suggestion(thread_id, "weles", "A", false)
        .await
        .expect("first suggestion");
    let second = engine
        .queue_thread_participant_suggestion(thread_id, "weles", "B", false)
        .await
        .expect("second suggestion");
    let suggestions = engine.list_thread_participant_suggestions(thread_id).await;
    assert_eq!(suggestions[0].id, first.id);
    assert_eq!(suggestions[1].id, second.id);
}

#[tokio::test]
async fn deactivating_missing_thread_participant_returns_ok() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    let thread_id = "thread-missing-participant";

    engine.threads.write().await.insert(
        thread_id.to_string(),
        AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Missing participant".to_string(),
            messages: vec![AgentMessage::user("hello", 1)],
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
        .apply_thread_participant_command(thread_id, "weles", "deactivate", None)
        .await
        .expect("missing participant deactivation should be non-fatal");

    assert!(
        engine.list_thread_participants(thread_id).await.is_empty(),
        "missing participant deactivation should not register a participant"
    );
}

#[tokio::test]
async fn auto_send_dismisses_stale_non_active_participant_suggestions() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    let thread_id = "thread-stale-non-active-participant-suggestion";

    engine.threads.write().await.insert(
        thread_id.to_string(),
        AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Stale non-active participant suggestion".to_string(),
            messages: vec![AgentMessage::user("hello", 1)],
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

    engine.thread_participant_suggestions.write().await.insert(
        thread_id.to_string(),
        vec![ThreadParticipantSuggestion {
            id: "stale-radogost".to_string(),
            target_agent_id: "radogost".to_string(),
            target_agent_name: "Radogost".to_string(),
            instruction: "Old stale suggestion".to_string(),
            suggestion_kind: ThreadParticipantSuggestionKind::PreparedMessage,
            force_send: false,
            status: ThreadParticipantSuggestionStatus::Queued,
            created_at: 1,
            updated_at: 1,
            auto_send_at: None,
            source_message_timestamp: None,
            error: None,
        }],
    );

    let sent = engine
        .maybe_auto_send_next_thread_participant_suggestion(thread_id)
        .await
        .expect("auto-send should succeed");
    assert!(
        !sent,
        "stale non-active participant suggestions should be pruned, not sent"
    );
    assert!(
        engine
            .list_thread_participant_suggestions(thread_id)
            .await
            .is_empty(),
        "stale non-active participant suggestions should be dismissed"
    );
}

#[tokio::test]
async fn internal_delegate_does_not_register_participant() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let recorded_bodies = Arc::new(StdMutex::new(VecDeque::new()));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind internal delegate server");
    let addr = listener.local_addr().expect("internal delegate addr");
    let recorded_bodies_task = recorded_bodies.clone();

    tokio::spawn(async move {
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let body = read_http_request_body(&mut socket)
                .await
                .expect("read internal delegate request");
            recorded_bodies_task
                .lock()
                .expect("lock recorded bodies")
                .push_back(body);
            let response_body = if attempt == 0 {
                concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_internal_delegate_dm\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"I will continue on the visible thread, not here.\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_internal_delegate_dm\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":5,\"output_tokens\":9},\"error\":null}}\n\n"
                )
            } else {
                concat!(
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_internal_delegate_visible\"}}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Weles continued on the visible thread.\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_internal_delegate_visible\",\"object\":\"response\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":6,\"output_tokens\":7},\"error\":null}}\n\n"
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write internal delegate response");
        }
    });

    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = format!("http://{addr}/v1");
    config.model = "gpt-5.4-mini".to_string();
    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let thread_id = "thread-internal-delegate";

    engine.threads.write().await.insert(
        thread_id.to_string(),
        AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Internal delegate".to_string(),
            messages: vec![AgentMessage::user("hello", 1)],
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

    let before = engine.list_thread_participants(thread_id).await;
    engine
        .send_internal_delegate_message(
            Some(thread_id),
            "weles",
            None,
            "check the thread and continue the work there",
        )
        .await
        .expect("internal delegate should succeed");
    let after = engine.list_thread_participants(thread_id).await;

    assert_eq!(before, after);

    let recorded = recorded_bodies
        .lock()
        .expect("lock recorded bodies")
        .clone();
    assert_eq!(
        recorded.len(),
        2,
        "delegate should use DM plus visible-thread continuation"
    );
    assert!(
        recorded[0].contains("Continuation requested on visible thread: yes"),
        "delegate DM should explicitly mention visible-thread continuation: {}",
        recorded[0]
    );
    assert!(
        recorded[0].contains("Do not continue work in this internal DM thread."),
        "delegate DM should explicitly prohibit doing work inside the DM thread: {}",
        recorded[0]
    );
    let first_body: serde_json::Value =
        serde_json::from_str(&recorded[0]).expect("internal delegate request body should be json");
    assert!(
        first_body
            .get("tools")
            .and_then(|value| value.as_array())
            .map(|tools| tools.is_empty())
            .unwrap_or(true),
        "internal DM delegate request should not expose tools: {}",
        recorded[0]
    );

    let dm_thread_id = crate::agent::agent_identity::internal_dm_thread_id(
        crate::agent::agent_identity::MAIN_AGENT_ID,
        crate::agent::agent_identity::WELES_AGENT_ID,
    );
    let threads = engine.threads.read().await;
    let dm_thread = threads
        .get(&dm_thread_id)
        .expect("delegate should create the internal DM thread");
    assert!(
        dm_thread.messages.iter().any(|message| {
            message.role == MessageRole::Assistant
                && message.content == "I will continue on the visible thread, not here."
        }),
        "internal DM thread should contain the discussion-only reply"
    );
    let visible_thread = threads
        .get(thread_id)
        .expect("visible thread should remain present");
    let visible_follow_up = visible_thread
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.role == MessageRole::Assistant
                && message.author_agent_id.as_deref() == Some("weles")
        })
        .expect("delegate should continue the visible thread as the requested agent");
    assert_eq!(
        visible_follow_up.content,
        "Weles continued on the visible thread."
    );
}

#[tokio::test]
async fn upserting_builtin_alias_stores_canonical_agent_name() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    let thread_id = "thread-alias-canonical";

    engine.threads.write().await.insert(
        thread_id.to_string(),
        AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Canonical alias".to_string(),
            messages: vec![AgentMessage::user("hello", 1)],
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

    let participant = engine
        .upsert_thread_participant(thread_id, "swarog", "watch for regressions")
        .await
        .expect("builtin alias should resolve");

    assert_eq!(participant.agent_id, "swarog");
    assert_eq!(participant.agent_name, "Svarog");
}

#[tokio::test]
async fn upserting_veles_alias_stores_weles_canonical_name() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    let thread_id = "thread-veles-alias-canonical";

    engine.threads.write().await.insert(
        thread_id.to_string(),
        AgentThread {
            id: thread_id.to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Canonical veles alias".to_string(),
            messages: vec![AgentMessage::user("hello", 1)],
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

    let participant = engine
        .upsert_thread_participant(thread_id, "veles", "watch for regressions")
        .await
        .expect("veles alias should resolve");

    assert_eq!(participant.agent_id, "weles");
    assert_eq!(participant.agent_name, "Weles");
}
