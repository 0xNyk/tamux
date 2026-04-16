use crate::agent::debate::protocol::{
    advance_round, assign_roles, build_debate_round_requests, create_debate_session,
    finalize_verdict, validate_argument,
};
use crate::agent::debate::types::{Argument, DebateStatus, RoleKind};
use crate::agent::handoff::divergent::Framing;
use crate::agent::{AgentConfig, AgentEngine};
use crate::session_manager::SessionManager;
use tempfile::tempdir;

fn sample_framings() -> Vec<Framing> {
    vec![
        Framing {
            label: "analytical-lens".to_string(),
            system_prompt_override: "Analyze formally".to_string(),
            task_id: None,
            contribution_id: None,
        },
        Framing {
            label: "pragmatic-lens".to_string(),
            system_prompt_override: "Be pragmatic".to_string(),
            task_id: None,
            contribution_id: None,
        },
        Framing {
            label: "synthesis-lens".to_string(),
            system_prompt_override: "Synthesize".to_string(),
            task_id: None,
            contribution_id: None,
        },
    ]
}

#[test]
fn create_debate_session_initializes_round_one() {
    let session = create_debate_session(
        "cache strategy".to_string(),
        sample_framings(),
        3,
        true,
        Some("thread-1".to_string()),
        None,
    )
    .expect("create session");
    assert_eq!(session.current_round, 1);
    assert_eq!(session.status, DebateStatus::InProgress);
    assert_eq!(session.roles.len(), 3);
    assert_eq!(session.roles[0].role, RoleKind::Proponent);
}

#[test]
fn assign_roles_rotates_proponent_and_skeptic_after_round_two() {
    let framings = sample_framings();
    let round_one = assign_roles(&framings, 1, true);
    let round_three = assign_roles(&framings, 3, true);
    assert_eq!(round_one[0].agent_id, "analytical-lens");
    assert_eq!(round_one[1].agent_id, "pragmatic-lens");
    assert_eq!(round_three[0].agent_id, "pragmatic-lens");
    assert_eq!(round_three[1].agent_id, "analytical-lens");
}

#[test]
fn validate_argument_requires_evidence_and_known_response_target() {
    let bad = Argument {
        id: "a1".to_string(),
        round: 2,
        role: RoleKind::Skeptic,
        agent_id: "skeptic".to_string(),
        content: "counterargument".to_string(),
        evidence_refs: vec![],
        responds_to: Some("missing".to_string()),
        timestamp_ms: 1,
    };
    assert!(validate_argument(&bad, 1, &[]).is_err());

    let good = Argument {
        evidence_refs: vec!["file:Cargo.toml".to_string()],
        responds_to: Some("a0".to_string()),
        ..bad
    };
    assert!(validate_argument(&good, 1, &["a0".to_string()]).is_ok());
}

#[test]
fn advance_round_and_finalize_verdict_progress_session() {
    let mut session = create_debate_session(
        "debate topic".to_string(),
        sample_framings(),
        3,
        true,
        None,
        None,
    )
    .expect("create session");
    advance_round(&mut session, true).expect("advance round");
    assert_eq!(session.current_round, 2);

    finalize_verdict(
        &mut session,
        vec!["agree on phased rollout".to_string()],
        vec!["observability budget".to_string()],
        "Run a small canary first".to_string(),
        0.8,
    )
    .expect("finalize verdict");
    assert_eq!(session.status, DebateStatus::Completed);
    assert!(session.verdict.is_some());
}

#[test]
fn build_debate_round_requests_from_existing_session() {
    let mut session = create_debate_session(
        "debate topic".to_string(),
        vec![
            Framing {
                label: "analytical-lens".to_string(),
                system_prompt_override: "Analyze formally".to_string(),
                task_id: Some("task-analytical".to_string()),
                contribution_id: Some("contrib-analytical".to_string()),
            },
            Framing {
                label: "pragmatic-lens".to_string(),
                system_prompt_override: "Be pragmatic".to_string(),
                task_id: Some("task-pragmatic".to_string()),
                contribution_id: Some("contrib-pragmatic".to_string()),
            },
            Framing {
                label: "synthesis-lens".to_string(),
                system_prompt_override: "Synthesize".to_string(),
                task_id: None,
                contribution_id: None,
            },
        ],
        3,
        true,
        Some("thread-1".to_string()),
        Some("goal-1".to_string()),
    )
    .expect("create session");
    advance_round(&mut session, true).expect("advance round");
    session.arguments = vec![
        Argument {
            id: "arg-1".to_string(),
            round: 1,
            role: RoleKind::Proponent,
            agent_id: "analytical-lens".to_string(),
            content: "Prefer canary rollout.".to_string(),
            evidence_refs: vec!["evidence:a".to_string()],
            responds_to: None,
            timestamp_ms: 1,
        },
        Argument {
            id: "arg-2".to_string(),
            round: 1,
            role: RoleKind::Skeptic,
            agent_id: "pragmatic-lens".to_string(),
            content: "Question rollout overhead.".to_string(),
            evidence_refs: vec!["evidence:b".to_string()],
            responds_to: Some("arg-1".to_string()),
            timestamp_ms: 2,
        },
    ];

    let requests = build_debate_round_requests(&session);

    assert_eq!(requests.len(), 3);
    assert!(requests
        .iter()
        .all(|request| request.session_id == session.id));
    assert!(requests.iter().all(|request| request.round == 2));
    assert!(requests
        .iter()
        .all(|request| request.topic == "debate topic"));
    assert!(requests
        .iter()
        .all(|request| request.prior_argument_ids == vec!["arg-1", "arg-2"]));

    let proponent = requests
        .iter()
        .find(|request| request.role == RoleKind::Proponent)
        .expect("proponent request");
    assert_eq!(proponent.agent_id, "analytical-lens");
    assert_eq!(
        proponent.framing_task_id.as_deref(),
        Some("task-analytical")
    );
    assert_eq!(
        proponent.framing_contribution_id.as_deref(),
        Some("contrib-analytical")
    );
    assert!(proponent.prompt.contains("Debate topic: debate topic"));
    assert!(proponent.prompt.contains("Round: 2"));
    assert!(proponent.prompt.contains("Role: proponent"));

    let skeptic = requests
        .iter()
        .find(|request| request.role == RoleKind::Skeptic)
        .expect("skeptic request");
    assert_eq!(skeptic.agent_id, "pragmatic-lens");
    assert_eq!(skeptic.framing_task_id.as_deref(), Some("task-pragmatic"));
    assert_eq!(
        skeptic.framing_contribution_id.as_deref(),
        Some("contrib-pragmatic")
    );
    assert!(skeptic.prompt.contains("Role: skeptic"));

    let synthesizer = requests
        .iter()
        .find(|request| request.role == RoleKind::Synthesizer)
        .expect("synthesizer request");
    assert_eq!(synthesizer.agent_id, "synthesis-lens");
    assert!(synthesizer.framing_task_id.is_none());
    assert!(synthesizer.framing_contribution_id.is_none());
    assert!(synthesizer.prompt.contains("Role: synthesizer"));
}

#[tokio::test]
async fn start_debate_session_auto_runs_seeded_opening_round() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.debate.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let session_id = engine
        .start_debate_session(
            "cache strategy",
            Some(sample_framings()),
            "thread-1",
            Some("goal-1"),
        )
        .await
        .expect("start debate session");

    let payload = engine
        .get_debate_session_payload(&session_id)
        .await
        .expect("debate payload should load");

    assert_eq!(payload["status"], "in_progress");
    assert_eq!(payload["current_round"].as_u64(), Some(2));

    let arguments = payload["arguments"]
        .as_array()
        .expect("arguments should be present after opening round seeding");
    assert_eq!(arguments.len(), 3);
    assert!(arguments
        .iter()
        .any(|argument| argument["role"] == "proponent"));
    assert!(arguments
        .iter()
        .any(|argument| argument["role"] == "skeptic"));
    assert!(arguments
        .iter()
        .any(|argument| argument["role"] == "synthesizer"));
    assert!(arguments.iter().all(|argument| {
        argument["round"].as_u64() == Some(1)
            && argument["evidence_refs"]
                .as_array()
                .is_some_and(|refs| refs.len() >= 1)
    }));
}

#[tokio::test]
async fn dispatch_debate_round_request_persists_structured_argument() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.debate.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let session_id = engine
        .start_debate_session(
            "cache strategy",
            Some(sample_framings()),
            "thread-1",
            Some("goal-1"),
        )
        .await
        .expect("start debate session");

    let session = engine
        .get_persisted_debate_session(&session_id)
        .await
        .expect("load debate session")
        .expect("debate session exists");
    let request = build_debate_round_requests(&session)
        .into_iter()
        .find(|request| request.role == RoleKind::Proponent)
        .expect("proponent round request");

    let payload = engine
        .dispatch_debate_round_request(
            &request,
            "Defend canary rollout with concrete evidence.",
            vec!["file:Cargo.toml".to_string()],
            None,
        )
        .await
        .expect("dispatch request should append a debate argument");

    assert_eq!(payload["status"], "appended");
    assert_eq!(payload["session_id"], session_id);
    assert_eq!(payload["round"], 2);
    assert_eq!(payload["role"], "proponent");
    assert_eq!(payload["prompt"], request.prompt);

    let persisted = engine
        .get_debate_session_payload(&session_id)
        .await
        .expect("debate payload should load");
    let arguments = persisted["arguments"]
        .as_array()
        .expect("arguments should persist");
    assert_eq!(arguments.len(), 4);
    let appended = arguments
        .iter()
        .find(|argument| argument["content"] == "Defend canary rollout with concrete evidence.")
        .expect("manually dispatched debate argument should be present");
    assert_eq!(appended["round"].as_u64(), Some(request.round as u64));
    assert_eq!(appended["role"], "proponent");
    assert_eq!(appended["agent_id"], request.agent_id);
    assert_eq!(
        appended["content"],
        "Defend canary rollout with concrete evidence."
    );
}

#[tokio::test]
async fn run_debate_round_cycle_completes_final_round_and_persists_verdict() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.debate.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let session = create_debate_session(
        "cache strategy".to_string(),
        sample_framings(),
        2,
        true,
        Some("thread-1".to_string()),
        Some("goal-1".to_string()),
    )
    .expect("create debate session");

    let seeded = engine
        .persist_seeded_debate_session(
            session,
            vec![
                Argument {
                    id: "arg-1".to_string(),
                    round: 1,
                    role: RoleKind::Proponent,
                    agent_id: "analytical-lens".to_string(),
                    content: "Prefer a canary rollout first.".to_string(),
                    evidence_refs: vec!["evidence:canary".to_string()],
                    responds_to: None,
                    timestamp_ms: 1,
                },
                Argument {
                    id: "arg-2".to_string(),
                    round: 1,
                    role: RoleKind::Skeptic,
                    agent_id: "pragmatic-lens".to_string(),
                    content: "Question the operational overhead of a canary rollout.".to_string(),
                    evidence_refs: vec!["evidence:overhead".to_string()],
                    responds_to: Some("arg-1".to_string()),
                    timestamp_ms: 2,
                },
            ],
        )
        .await
        .expect("persist seeded debate session");

    assert_eq!(seeded.current_round, 2);
    assert_eq!(build_debate_round_requests(&seeded).len(), 3);

    let payload = engine
        .run_debate_round_cycle(&seeded.id)
        .await
        .expect("round cycle should complete the final round");

    assert_eq!(payload["session_id"], seeded.id);
    assert_eq!(payload["status"], "completed");
    assert_eq!(payload["current_round"].as_u64(), Some(2));
    assert_eq!(payload["max_rounds"].as_u64(), Some(2));
    assert_eq!(
        payload["arguments"].as_array().map(|items| items.len()),
        Some(5)
    );
    assert!(payload["verdict"].is_object());
    assert!(payload["verdict"]["recommended_action"]
        .as_str()
        .is_some_and(|text| text.contains("Round 2 synthesis for 'cache strategy'")));

    let persisted = engine
        .get_persisted_debate_session(&seeded.id)
        .await
        .expect("load persisted debate session")
        .expect("persisted debate session should exist");
    assert_eq!(persisted.status, DebateStatus::Completed);
    assert!(persisted.verdict.is_some());

    let verdict_row = engine
        .history
        .get_debate_verdict(&seeded.id)
        .await
        .expect("persisted verdict query should succeed");
    assert!(verdict_row.is_some());
}

#[tokio::test]
async fn run_debate_to_completion_cycles_remaining_rounds_and_persists_verdict() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.debate.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let session = create_debate_session(
        "cache strategy".to_string(),
        sample_framings(),
        3,
        true,
        Some("thread-1".to_string()),
        Some("goal-1".to_string()),
    )
    .expect("create debate session");

    let seeded = engine
        .persist_seeded_debate_session(
            session,
            vec![
                Argument {
                    id: "arg-1".to_string(),
                    round: 1,
                    role: RoleKind::Proponent,
                    agent_id: "analytical-lens".to_string(),
                    content: "Prefer a canary rollout first.".to_string(),
                    evidence_refs: vec!["evidence:canary".to_string()],
                    responds_to: None,
                    timestamp_ms: 1,
                },
                Argument {
                    id: "arg-2".to_string(),
                    round: 1,
                    role: RoleKind::Skeptic,
                    agent_id: "pragmatic-lens".to_string(),
                    content: "Question the operational overhead of a canary rollout.".to_string(),
                    evidence_refs: vec!["evidence:overhead".to_string()],
                    responds_to: Some("arg-1".to_string()),
                    timestamp_ms: 2,
                },
            ],
        )
        .await
        .expect("persist seeded debate session");

    assert_eq!(seeded.current_round, 2);
    assert_eq!(seeded.max_rounds, 3);

    let payload = engine
        .run_debate_to_completion(&seeded.id)
        .await
        .expect("multi-round helper should complete the remaining debate rounds");

    assert_eq!(payload["session_id"], seeded.id);
    assert_eq!(payload["status"], "completed");
    assert_eq!(payload["current_round"].as_u64(), Some(3));
    assert_eq!(payload["max_rounds"].as_u64(), Some(3));
    assert_eq!(
        payload["arguments"].as_array().map(|items| items.len()),
        Some(8)
    );
    assert!(payload["verdict"].is_object());
    assert!(payload["verdict"]["recommended_action"]
        .as_str()
        .is_some_and(|text| text.contains("Round 3 synthesis for 'cache strategy'")));

    let persisted = engine
        .get_persisted_debate_session(&seeded.id)
        .await
        .expect("load persisted debate session")
        .expect("persisted debate session should exist");
    assert_eq!(persisted.status, DebateStatus::Completed);
    assert!(persisted.verdict.is_some());

    let verdict_row = engine
        .history
        .get_debate_verdict(&seeded.id)
        .await
        .expect("persisted verdict query should succeed");
    assert!(verdict_row.is_some());
}
