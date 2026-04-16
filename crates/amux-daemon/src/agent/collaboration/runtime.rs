use super::participants::{apply_vote_to_disagreement, normalize_position, normalize_topic};
use super::*;
use crate::agent::debate::protocol::create_debate_session;
use crate::agent::debate::types::{Argument, DebateSession, RoleKind};
use crate::agent::handoff::divergent::Framing;
use std::collections::{BTreeSet, HashMap};

fn bid_sort_key(availability: &BidAvailability) -> u8 {
    match availability {
        BidAvailability::Available => 0,
        BidAvailability::Busy => 1,
        BidAvailability::Unavailable => 2,
    }
}

fn bid_availability_label(availability: &BidAvailability) -> &'static str {
    match availability {
        BidAvailability::Available => "available",
        BidAvailability::Busy => "busy",
        BidAvailability::Unavailable => "unavailable",
    }
}

#[derive(Clone)]
struct PendingDebateSeed {
    disagreement_id: String,
    debate_session: DebateSession,
    arguments: Vec<Argument>,
}

#[derive(Clone)]
struct PendingDebateLaunch {
    seed: PendingDebateSeed,
    collaboration_snapshot: CollaborationSession,
}

fn build_pending_debate_seed(
    session: &CollaborationSession,
    debate_config: &DebateConfig,
) -> Option<PendingDebateSeed> {
    let thread_id = session.thread_id.as_ref()?.clone();

    for disagreement in &session.disagreements {
        if disagreement.resolution != "pending" || disagreement.debate_session_id.is_some() {
            continue;
        }

        let mut latest_by_task: HashMap<&str, (&Contribution, usize)> = HashMap::new();
        for (source_index, contribution) in session.contributions.iter().enumerate() {
            if contribution.topic != disagreement.topic {
                continue;
            }
            latest_by_task.insert(contribution.task_id.as_str(), (contribution, source_index));
        }

        let mut latest = latest_by_task.into_values().collect::<Vec<_>>();
        latest.sort_by(|(left, left_index), (right, right_index)| {
            left.created_at
                .cmp(&right.created_at)
                .then(left_index.cmp(right_index))
        });
        let latest = latest
            .into_iter()
            .map(|(contribution, _)| contribution)
            .collect::<Vec<_>>();

        let distinct_positions = latest
            .iter()
            .map(|contribution| normalize_position(&contribution.position))
            .collect::<BTreeSet<_>>();
        if latest.len() < 2 || distinct_positions.len() < 2 {
            continue;
        }

        let mut ordered_positions = Vec::new();
        for contribution in &latest {
            let position = normalize_position(&contribution.position);
            if !ordered_positions
                .iter()
                .any(|existing| existing == &position)
            {
                ordered_positions.push(position);
            }
        }
        let proponent_position = ordered_positions
            .first()
            .cloned()
            .unwrap_or_else(|| "recommend".to_string());
        let skeptic_position = ordered_positions
            .iter()
            .find(|position| **position != proponent_position)
            .cloned()
            .unwrap_or_else(|| "reject".to_string());

        let mut framings = Vec::new();
        for position in ordered_positions.iter().take(2) {
            framings.push(Framing {
                label: format!("{position}-lens"),
                system_prompt_override: format!(
                    "Defend the `{position}` position for {} using the imported collaboration evidence.",
                    disagreement.topic
                ),
                task_id: latest
                    .iter()
                    .find(|contribution| normalize_position(&contribution.position) == *position)
                    .map(|contribution| contribution.task_id.clone()),
                contribution_id: latest
                    .iter()
                    .find(|contribution| normalize_position(&contribution.position) == *position)
                    .map(|contribution| contribution.id.clone()),
            });
        }
        framings.push(Framing {
            label: "synthesis-lens".to_string(),
            system_prompt_override: format!(
                "Synthesize the strongest recommendation for {} without erasing the imported disagreement.",
                disagreement.topic
            ),
            task_id: None,
            contribution_id: None,
        });

        let debate_session = create_debate_session(
            disagreement.topic.clone(),
            framings,
            debate_config.default_max_rounds,
            debate_config.role_rotation,
            Some(thread_id.clone()),
            session.goal_run_id.clone(),
        )
        .ok()?;

        let arguments = latest
            .into_iter()
            .map(|contribution| {
                let position = normalize_position(&contribution.position);
                Argument {
                    id: format!("arg_{}", uuid::Uuid::new_v4()),
                    round: 1,
                    role: if position == proponent_position {
                        RoleKind::Proponent
                    } else if position == skeptic_position {
                        RoleKind::Skeptic
                    } else {
                        RoleKind::Skeptic
                    },
                    agent_id: contribution.task_id.clone(),
                    content: contribution.position.clone(),
                    evidence_refs: if contribution.evidence.is_empty() {
                        vec![format!("collaboration contribution {}", contribution.id)]
                    } else {
                        contribution.evidence.clone()
                    },
                    responds_to: None,
                    timestamp_ms: contribution.created_at,
                }
            })
            .collect::<Vec<_>>();

        return Some(PendingDebateSeed {
            disagreement_id: disagreement.id.clone(),
            debate_session,
            arguments,
        });
    }

    None
}

fn seed_debate_from_bid_resolution(
    session: &mut CollaborationSession,
    ranked: &[ConsensusBid],
    debate_config: &DebateConfig,
) -> Option<PendingDebateSeed> {
    let thread_id = session.thread_id.as_ref()?.clone();
    let call_metadata = session.call_metadata.clone()?;
    let primary = ranked.first()?.clone();
    let reviewer = ranked
        .iter()
        .find(|bid| bid.task_id != primary.task_id)
        .cloned()?;

    let tied_confidence = (primary.confidence - reviewer.confidence).abs() <= f64::EPSILON;
    let tied_availability_rank =
        bid_sort_key(&primary.availability) == bid_sort_key(&reviewer.availability);
    if !tied_confidence || !tied_availability_rank {
        return None;
    }

    let topic = normalize_topic(&format!("bid resolution for {}", session.mission));
    if session
        .disagreements
        .iter()
        .any(|disagreement| disagreement.topic == topic && disagreement.debate_session_id.is_some())
    {
        return None;
    }

    let disagreement_id = if let Some(disagreement) = session
        .disagreements
        .iter_mut()
        .find(|disagreement| disagreement.topic == topic)
    {
        disagreement.agents = vec![primary.task_id.clone(), reviewer.task_id.clone()];
        disagreement.positions = vec![
            format!("assign {} as primary", primary.task_id),
            format!("assign {} as primary", reviewer.task_id),
        ];
        disagreement.confidence_gap = (primary.confidence - reviewer.confidence).abs();
        disagreement.resolution = "pending".to_string();
        disagreement.votes.clear();
        disagreement.id.clone()
    } else {
        let disagreement_id = format!("disagree_{}", uuid::Uuid::new_v4());
        session.disagreements.push(Disagreement {
            id: disagreement_id.clone(),
            topic: topic.clone(),
            agents: vec![primary.task_id.clone(), reviewer.task_id.clone()],
            positions: vec![
                format!("assign {} as primary", primary.task_id),
                format!("assign {} as primary", reviewer.task_id),
            ],
            confidence_gap: (primary.confidence - reviewer.confidence).abs(),
            resolution: "pending".to_string(),
            votes: Vec::new(),
            debate_session_id: None,
        });
        disagreement_id
    };

    let mut framings = Vec::new();
    for bid in [&primary, &reviewer] {
        framings.push(Framing {
            label: format!("{}-bid-lens", bid.task_id),
            system_prompt_override: format!(
                "Defend assigning {} as primary for {} using the contested bid evidence.",
                bid.task_id, session.mission
            ),
            task_id: Some(bid.task_id.clone()),
            contribution_id: None,
        });
    }
    framings.push(Framing {
        label: "synthesis-lens".to_string(),
        system_prompt_override: format!(
            "Synthesize the strongest assignment for {} without erasing the contested bid evidence.",
            session.mission
        ),
        task_id: None,
        contribution_id: None,
    });

    let debate_session = create_debate_session(
        topic.clone(),
        framings,
        debate_config.default_max_rounds,
        debate_config.role_rotation,
        Some(thread_id),
        session.goal_run_id.clone(),
    )
    .ok()?;

    if let Some(disagreement) = session
        .disagreements
        .iter_mut()
        .find(|disagreement| disagreement.id == disagreement_id)
    {
        disagreement.debate_session_id = Some(debate_session.id.clone());
    }

    let eligible_agents = if call_metadata.eligible_agents.is_empty() {
        vec![primary.task_id.clone(), reviewer.task_id.clone()]
    } else {
        call_metadata.eligible_agents.clone()
    };
    let shared_evidence = vec![
        format!(
            "bid call parent_task_id={} caller_task_id={} eligible_agents={}",
            session.parent_task_id,
            call_metadata.caller_task_id,
            eligible_agents.join(",")
        ),
        format!(
            "contested bid candidates={},{} confidences={:.2},{:.2} availability={:?},{:?}",
            primary.task_id,
            reviewer.task_id,
            primary.confidence,
            reviewer.confidence,
            primary.availability,
            reviewer.availability
        ),
    ];

    let arguments = vec![
        Argument {
            id: format!("arg_{}", uuid::Uuid::new_v4()),
            round: 1,
            role: RoleKind::Proponent,
            agent_id: primary.task_id.clone(),
            content: format!(
                "Assign {} as primary for {}.",
                primary.task_id, session.mission
            ),
            evidence_refs: shared_evidence.clone(),
            responds_to: None,
            timestamp_ms: primary.created_at,
        },
        Argument {
            id: format!("arg_{}", uuid::Uuid::new_v4()),
            round: 1,
            role: RoleKind::Skeptic,
            agent_id: reviewer.task_id.clone(),
            content: format!(
                "Assign {} as primary for {}.",
                reviewer.task_id, session.mission
            ),
            evidence_refs: shared_evidence,
            responds_to: None,
            timestamp_ms: reviewer.created_at,
        },
    ];

    Some(PendingDebateSeed {
        disagreement_id,
        debate_session,
        arguments,
    })
}

impl AgentEngine {
    async fn ensure_task_collaboration_session(
        &self,
        task: &AgentTask,
    ) -> Result<CollaborationSession> {
        let mut collaboration = self.collaboration.write().await;
        let session =
            collaboration
                .entry(task.id.clone())
                .or_insert_with(|| CollaborationSession {
                    id: format!("collab_{}", uuid::Uuid::new_v4()),
                    parent_task_id: task.id.clone(),
                    thread_id: task.thread_id.clone().or(task.parent_thread_id.clone()),
                    goal_run_id: task.goal_run_id.clone(),
                    mission: task.description.clone(),
                    agents: Vec::new(),
                    call_metadata: None,
                    bids: Vec::new(),
                    role_assignment: None,
                    contributions: Vec::new(),
                    disagreements: Vec::new(),
                    consensus: None,
                    updated_at: now_millis(),
                });

        if !session.agents.iter().any(|agent| agent.task_id == task.id) {
            session.agents.push(CollaborativeAgent {
                task_id: task.id.clone(),
                title: task.title.clone(),
                role: infer_collaboration_role(task),
                confidence: 0.5,
                status: format!("{:?}", task.status).to_lowercase(),
            });
        }
        session.thread_id = session
            .thread_id
            .clone()
            .or(task.thread_id.clone())
            .or(task.parent_thread_id.clone());
        session.goal_run_id = session.goal_run_id.clone().or(task.goal_run_id.clone());
        if session.mission.trim().is_empty() {
            session.mission = task.description.clone();
        }
        session.updated_at = now_millis();

        let snapshot = session.clone();
        drop(collaboration);
        self.persist_collaboration_session(&snapshot).await?;
        Ok(snapshot)
    }

    async fn bootstrap_bid_dispatch_collaboration(
        &self,
        parent_task_id: &str,
        bid_task_ids: &[String],
    ) -> Result<()> {
        let (parent, eligible_subagents) = {
            let tasks = self.tasks.lock().await;
            let parent = tasks
                .iter()
                .find(|task| task.id == parent_task_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown parent task {parent_task_id}"))?;
            let eligible_subagents = bid_task_ids
                .iter()
                .filter_map(|task_id| {
                    tasks.iter().find(|task| {
                        task.id == *task_id
                            && task.source == "subagent"
                            && task.parent_task_id.as_deref() == Some(parent_task_id)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            (parent, eligible_subagents)
        };

        let mut collaboration = self.collaboration.write().await;
        let session = collaboration
            .entry(parent_task_id.to_string())
            .or_insert_with(|| CollaborationSession {
                id: format!("collab_{}", uuid::Uuid::new_v4()),
                parent_task_id: parent_task_id.to_string(),
                thread_id: parent.thread_id.clone().or(parent.parent_thread_id.clone()),
                goal_run_id: parent.goal_run_id.clone(),
                mission: parent.description.clone(),
                agents: Vec::new(),
                call_metadata: None,
                bids: Vec::new(),
                role_assignment: None,
                contributions: Vec::new(),
                disagreements: Vec::new(),
                consensus: None,
                updated_at: now_millis(),
            });

        session.thread_id = session
            .thread_id
            .clone()
            .or(parent.thread_id.clone())
            .or(parent.parent_thread_id.clone());
        session.goal_run_id = session.goal_run_id.clone().or(parent.goal_run_id.clone());
        if session.mission.trim().is_empty() {
            session.mission = parent.description.clone();
        }

        for subagent in eligible_subagents {
            if session
                .agents
                .iter()
                .any(|agent| agent.task_id == subagent.id)
            {
                continue;
            }
            session.agents.push(CollaborativeAgent {
                task_id: subagent.id.clone(),
                title: subagent.title.clone(),
                role: infer_collaboration_role(&subagent),
                confidence: 0.5,
                status: format!("{:?}", subagent.status).to_lowercase(),
            });
        }
        session.updated_at = now_millis();
        let snapshot = session.clone();
        drop(collaboration);

        self.persist_collaboration_session(&snapshot).await?;
        Ok(())
    }

    async fn persisted_collaboration_session(
        &self,
        parent_task_id: &str,
    ) -> Result<Option<CollaborationSession>> {
        let row = self
            .history
            .list_collaboration_sessions()
            .await?
            .into_iter()
            .find(|row| row.parent_task_id == parent_task_id);
        let Some(row) = row else {
            return Ok(None);
        };
        serde_json::from_str(&row.session_json)
            .map(Some)
            .map_err(|error| anyhow::anyhow!(error))
    }

    async fn merged_persisted_collaboration_sessions(&self) -> Result<Vec<CollaborationSession>> {
        let persisted = self.history.list_collaboration_sessions().await?;
        let mut sessions = std::collections::BTreeMap::new();
        for row in persisted {
            let session: CollaborationSession =
                serde_json::from_str(&row.session_json).map_err(|error| anyhow::anyhow!(error))?;
            sessions.insert(session.parent_task_id.clone(), session);
        }

        let collaboration = self.collaboration.read().await;
        for session in collaboration.values() {
            sessions.insert(session.parent_task_id.clone(), session.clone());
        }

        Ok(sessions.into_values().collect())
    }

    async fn persist_collaboration_session(&self, session: &CollaborationSession) -> Result<()> {
        let session_json = serde_json::to_string(session)?;
        self.history
            .upsert_collaboration_session(
                &session.parent_task_id,
                &session_json,
                session.updated_at,
            )
            .await
    }

    pub(in crate::agent) async fn register_subagent_collaboration(
        &self,
        parent_task_id: &str,
        subagent: &AgentTask,
    ) {
        if !self.config.read().await.collaboration.enabled {
            return;
        }
        let parent = {
            let tasks = self.tasks.lock().await;
            tasks.iter().find(|task| task.id == parent_task_id).cloned()
        };
        let Some(parent) = parent else {
            return;
        };
        let mut collaboration = self.collaboration.write().await;
        let session = collaboration
            .entry(parent_task_id.to_string())
            .or_insert_with(|| CollaborationSession {
                id: format!("collab_{}", uuid::Uuid::new_v4()),
                parent_task_id: parent_task_id.to_string(),
                thread_id: parent.thread_id.clone().or(parent.parent_thread_id.clone()),
                goal_run_id: parent.goal_run_id.clone(),
                mission: parent.description.clone(),
                agents: Vec::new(),
                call_metadata: None,
                bids: Vec::new(),
                role_assignment: None,
                contributions: Vec::new(),
                disagreements: Vec::new(),
                consensus: None,
                updated_at: now_millis(),
            });

        if session
            .agents
            .iter()
            .any(|agent| agent.task_id == subagent.id)
        {
            return;
        }
        session.agents.push(CollaborativeAgent {
            task_id: subagent.id.clone(),
            title: subagent.title.clone(),
            role: infer_collaboration_role(subagent),
            confidence: 0.5,
            status: format!("{:?}", subagent.status).to_lowercase(),
        });
        session.updated_at = now_millis();
        let snapshot = session.clone();
        drop(collaboration);
        if let Err(error) = self.persist_collaboration_session(&snapshot).await {
            tracing::warn!(
                parent_task_id = %parent_task_id,
                "failed to persist collaboration session: {error}"
            );
        }
    }

    pub(in crate::agent) async fn call_for_bids(
        &self,
        parent_task_id: &str,
        eligible_agents: &[String],
    ) -> Result<serde_json::Value> {
        if !self.config.read().await.collaboration.enabled {
            anyhow::bail!("collaboration capability is disabled in agent config");
        }
        let mut collaboration = self.collaboration.write().await;
        let Some(session) = collaboration.get_mut(parent_task_id) else {
            anyhow::bail!("no collaboration session found for parent task {parent_task_id}");
        };
        session.call_metadata = Some(BidCallMetadata {
            caller_task_id: parent_task_id.to_string(),
            eligible_agents: eligible_agents.to_vec(),
            called_at: now_millis(),
        });
        session.bids.retain(|bid| {
            eligible_agents
                .iter()
                .any(|task_id| task_id == &bid.task_id)
        });
        session.role_assignment = None;
        session.updated_at = now_millis();
        let snapshot = session.clone();
        let report = serde_json::json!({
            "session_id": session.id,
            "eligible_agents": eligible_agents,
            "bid_count": session.bids.len(),
        });
        drop(collaboration);
        self.persist_collaboration_session(&snapshot).await?;
        Ok(report)
    }

    pub(in crate::agent) async fn submit_bid(
        &self,
        parent_task_id: &str,
        task_id: &str,
        confidence: f64,
        availability: BidAvailability,
    ) -> Result<serde_json::Value> {
        if !self.config.read().await.collaboration.enabled {
            anyhow::bail!("collaboration capability is disabled in agent config");
        }
        let mut collaboration = self.collaboration.write().await;
        let Some(session) = collaboration.get_mut(parent_task_id) else {
            anyhow::bail!("no collaboration session found for parent task {parent_task_id}");
        };
        if !session.agents.iter().any(|agent| agent.task_id == task_id) {
            anyhow::bail!("unknown collaborative agent {task_id}");
        }
        let bid = ConsensusBid {
            task_id: task_id.to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            availability,
            created_at: now_millis(),
        };
        if let Some(existing) = session.bids.iter_mut().find(|item| item.task_id == task_id) {
            *existing = bid.clone();
        } else {
            session.bids.push(bid.clone());
        }
        session.updated_at = now_millis();
        let snapshot = session.clone();
        let report = serde_json::json!({
            "session_id": session.id,
            "bid": bid,
        });
        drop(collaboration);
        self.persist_collaboration_session(&snapshot).await?;
        Ok(report)
    }

    pub(in crate::agent) async fn resolve_bids(
        &self,
        parent_task_id: &str,
    ) -> Result<serde_json::Value> {
        let config = self.config.read().await.clone();
        if !config.collaboration.enabled {
            anyhow::bail!("collaboration capability is disabled in agent config");
        }
        let debate_config = config.debate;
        let mut collaboration = self.collaboration.write().await;
        let Some(session) = collaboration.get_mut(parent_task_id) else {
            anyhow::bail!("no collaboration session found for parent task {parent_task_id}");
        };
        if session.bids.len() < 2 {
            anyhow::bail!("at least two bids are required to assign primary and reviewer roles");
        }
        let mut ranked = session.bids.clone();
        ranked.sort_by(|left, right| {
            bid_sort_key(&left.availability)
                .cmp(&bid_sort_key(&right.availability))
                .then_with(|| {
                    right
                        .confidence
                        .partial_cmp(&left.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| left.created_at.cmp(&right.created_at))
        });
        let primary = ranked
            .iter()
            .find(|bid| !matches!(bid.availability, BidAvailability::Unavailable))
            .cloned()
            .unwrap_or_else(|| ranked[0].clone());
        let reviewer = ranked
            .iter()
            .find(|bid| bid.task_id != primary.task_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("a distinct reviewer bid is required"))?;

        let assignment = ConsensusRoleAssignment {
            primary_task_id: primary.task_id.clone(),
            primary_role: "primary".to_string(),
            reviewer_task_id: reviewer.task_id.clone(),
            reviewer_role: "reviewer".to_string(),
            assigned_at: now_millis(),
        };
        session.role_assignment = Some(assignment.clone());
        for agent in session.agents.iter_mut() {
            if agent.task_id == assignment.primary_task_id {
                agent.role = "primary".to_string();
                agent.confidence = primary.confidence;
            } else if agent.task_id == assignment.reviewer_task_id {
                agent.role = "reviewer".to_string();
                agent.confidence = reviewer.confidence;
            }
        }
        let debate_launch = if debate_config.enabled {
            seed_debate_from_bid_resolution(session, &ranked, &debate_config).map(|seed| {
                PendingDebateLaunch {
                    collaboration_snapshot: session.clone(),
                    seed,
                }
            })
        } else {
            None
        };
        session.updated_at = now_millis();
        let snapshot = session.clone();
        let report = serde_json::json!({
            "session_id": session.id,
            "primary_task_id": assignment.primary_task_id,
            "reviewer_task_id": assignment.reviewer_task_id,
            "bids": ranked,
        });
        drop(collaboration);
        self.persist_collaboration_session(&snapshot).await?;

        if let Some(launch) = debate_launch {
            if let Err(error) = self
                .persist_seeded_debate_session(
                    launch.seed.debate_session.clone(),
                    launch.seed.arguments.clone(),
                )
                .await
            {
                let rollback_snapshot = {
                    let mut collaboration = self.collaboration.write().await;
                    let Some(session) = collaboration.get_mut(parent_task_id) else {
                        return Err(error);
                    };
                    if let Some(disagreement) = session
                        .disagreements
                        .iter_mut()
                        .find(|item| item.id == launch.seed.disagreement_id)
                    {
                        if disagreement.debate_session_id.as_deref()
                            == Some(launch.seed.debate_session.id.as_str())
                        {
                            disagreement.debate_session_id = None;
                        }
                    }
                    session.updated_at = now_millis();
                    session.clone()
                };
                self.persist_collaboration_session(&rollback_snapshot)
                    .await?;
                return Err(error);
            }
        }

        Ok(report)
    }

    pub(in crate::agent) async fn dispatch_via_bid_protocol(
        &self,
        parent_task_id: &str,
        bids: &[DispatchBidRequest],
    ) -> Result<serde_json::Value> {
        if bids.is_empty() {
            anyhow::bail!("dispatch_via_bid_protocol requires at least one bid request");
        }
        let eligible_agents = bids
            .iter()
            .map(|bid| bid.task_id.clone())
            .collect::<Vec<_>>();
        self.bootstrap_bid_dispatch_collaboration(parent_task_id, &eligible_agents)
            .await?;
        self.call_for_bids(parent_task_id, &eligible_agents).await?;
        for bid in bids {
            self.submit_bid(
                parent_task_id,
                &bid.task_id,
                bid.confidence,
                bid.availability.clone(),
            )
            .await?;
        }
        let mut report = self.resolve_bids(parent_task_id).await?;
        if let Ok(debate_completion) = self.resolve_seeded_bid_debate(parent_task_id).await {
            if let Some(report_map) = report.as_object_mut() {
                report_map.insert("debate".to_string(), debate_completion);
            }
        }
        Ok(report)
    }

    pub(in crate::agent) async fn record_collaboration_contribution(
        &self,
        parent_task_id: &str,
        task_id: &str,
        topic: &str,
        position: &str,
        evidence: Vec<String>,
        confidence: f64,
    ) -> Result<serde_json::Value> {
        if !self.config.read().await.collaboration.enabled {
            anyhow::bail!("collaboration capability is disabled in agent config");
        }
        let mut collaboration = self.collaboration.write().await;
        let Some(session) = collaboration.get_mut(parent_task_id) else {
            anyhow::bail!("no collaboration session found for parent task {parent_task_id}");
        };
        let contribution = Contribution {
            id: format!("contrib_{}", uuid::Uuid::new_v4()),
            task_id: task_id.to_string(),
            topic: normalize_topic(topic),
            position: position.trim().to_string(),
            evidence,
            confidence: confidence.clamp(0.0, 1.0),
            created_at: now_millis(),
        };
        if let Some(agent) = session
            .agents
            .iter_mut()
            .find(|agent| agent.task_id == task_id)
        {
            agent.confidence = contribution.confidence;
        }
        session.contributions.push(contribution.clone());
        detect_disagreements(session);
        session.updated_at = now_millis();
        let snapshot = session.clone();

        let escalation = session
            .disagreements
            .iter()
            .find(|item| item.resolution == "pending" && item.confidence_gap < 0.15)
            .cloned();
        let thread_id = session.thread_id.clone();
        let report = serde_json::json!({
            "session_id": session.id,
            "contribution": contribution,
            "disagreements": session.disagreements,
            "consensus": session.consensus,
        });
        drop(collaboration);
        self.persist_collaboration_session(&snapshot).await?;

        if let Some(disagreement) = escalation {
            if let Some(thread_id) = thread_id {
                let _ = self.event_tx.send(AgentEvent::WorkflowNotice {
                    thread_id,
                    kind: "collaboration".to_string(),
                    message: format!("Unresolved subagent disagreement on {}", disagreement.topic),
                    details: Some(disagreement.positions.join(" vs ")),
                });
            }
        }

        self.maybe_auto_escalate_collaboration_debate(parent_task_id)
            .await?;
        Ok(report)
    }

    pub(in crate::agent) async fn collaboration_peer_memory_json(
        &self,
        parent_task_id: &str,
        task_id: &str,
    ) -> Result<serde_json::Value> {
        if !self.config.read().await.collaboration.enabled {
            anyhow::bail!("collaboration capability is disabled in agent config");
        }
        let collaboration = self.collaboration.read().await;
        let Some(session) = collaboration.get(parent_task_id) else {
            anyhow::bail!("no collaboration session found for parent task {parent_task_id}");
        };
        Ok(serde_json::json!({
            "session_id": session.id,
            "mission": session.mission,
            "peers": session.agents.iter().filter(|agent| agent.task_id != task_id).collect::<Vec<_>>(),
            "shared_context": session.contributions.iter().filter(|entry| entry.task_id != task_id).collect::<Vec<_>>(),
            "disagreements": session.disagreements,
            "consensus": session.consensus,
        }))
    }

    pub(in crate::agent) async fn resolve_seeded_bid_debate(
        &self,
        parent_task_id: &str,
    ) -> Result<serde_json::Value> {
        let (
            debate_session_id,
            disagreement_id,
            mission,
            winner_task_id,
            reviewer_task_id,
            winner_bid,
            reviewer_bid,
        ) = {
            if !self.config.read().await.collaboration.enabled {
                anyhow::bail!("collaboration capability is disabled in agent config");
            }
            let collaboration = self.collaboration.read().await;
            let Some(session) = collaboration.get(parent_task_id) else {
                anyhow::bail!("no collaboration session found for parent task {parent_task_id}");
            };
            let assignment = session.role_assignment.clone().ok_or_else(|| {
                anyhow::anyhow!("no bid role assignment found for parent task {parent_task_id}")
            })?;
            let disagreement = session
                .disagreements
                .iter()
                .find(|item| {
                    item.debate_session_id.is_some()
                        && item.topic.starts_with("bid resolution for ")
                        && item.resolution == "pending"
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no pending seeded bid debate found for parent task {parent_task_id}"
                    )
                })?;
            let winner_bid = session
                .bids
                .iter()
                .find(|bid| bid.task_id == assignment.primary_task_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing primary bid for seeded bid debate"))?;
            let reviewer_bid = session
                .bids
                .iter()
                .find(|bid| bid.task_id == assignment.reviewer_task_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing reviewer bid for seeded bid debate"))?;
            (
                disagreement
                    .debate_session_id
                    .clone()
                    .expect("seeded bid debate session id should exist"),
                disagreement.id.clone(),
                session.mission.clone(),
                assignment.primary_task_id,
                assignment.reviewer_task_id,
                winner_bid,
                reviewer_bid,
            )
        };

        let debate_session = self
            .get_persisted_debate_session(&debate_session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown debate session: {debate_session_id}"))?;

        let shared_evidence = vec![
            format!(
                "seeded bid debate parent_task_id={} mission={}",
                parent_task_id, mission
            ),
            format!(
                "candidate={} confidence={:.2} availability={}",
                winner_task_id,
                winner_bid.confidence,
                bid_availability_label(&winner_bid.availability)
            ),
            format!(
                "candidate={} confidence={:.2} availability={}",
                reviewer_task_id,
                reviewer_bid.confidence,
                bid_availability_label(&reviewer_bid.availability)
            ),
        ];

        self.append_debate_argument(
            &debate_session_id,
            Argument {
                id: format!("arg_{}", uuid::Uuid::new_v4()),
                round: debate_session.current_round,
                role: RoleKind::Proponent,
                agent_id: winner_task_id.clone(),
                content: format!(
                    "Bid finalist argument: choose primary={} for {} because confidence={:.2} and availability={} remain strongest under the tie-break.",
                    winner_task_id,
                    mission,
                    winner_bid.confidence,
                    bid_availability_label(&winner_bid.availability)
                ),
                evidence_refs: shared_evidence.clone(),
                responds_to: None,
                timestamp_ms: now_millis(),
            },
        )
        .await?;

        self.append_debate_argument(
            &debate_session_id,
            Argument {
                id: format!("arg_{}", uuid::Uuid::new_v4()),
                round: debate_session.current_round,
                role: RoleKind::Skeptic,
                agent_id: reviewer_task_id.clone(),
                content: format!(
                    "Bid finalist counterargument: choose primary={} for {} because confidence={:.2} and availability={} keep the contest open.",
                    reviewer_task_id,
                    mission,
                    reviewer_bid.confidence,
                    bid_availability_label(&reviewer_bid.availability)
                ),
                evidence_refs: shared_evidence.clone(),
                responds_to: None,
                timestamp_ms: now_millis(),
            },
        )
        .await?;

        self.append_debate_argument(
            &debate_session_id,
            Argument {
                id: format!("arg_{}", uuid::Uuid::new_v4()),
                round: debate_session.current_round,
                role: RoleKind::Synthesizer,
                agent_id: "synthesis-lens".to_string(),
                content: format!(
                    "Winning assignment: primary={} reviewer={}. Preserve the deterministic ranking because both bids stayed tied at confidence={:.2} with availability {} vs {}.",
                    winner_task_id,
                    reviewer_task_id,
                    winner_bid.confidence,
                    bid_availability_label(&winner_bid.availability),
                    bid_availability_label(&reviewer_bid.availability)
                ),
                evidence_refs: shared_evidence,
                responds_to: None,
                timestamp_ms: now_millis(),
            },
        )
        .await?;

        let mut completion_ready_session = self
            .get_persisted_debate_session(&debate_session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown debate session: {debate_session_id}"))?;
        completion_ready_session.max_rounds = completion_ready_session.current_round;
        self.persist_debate_session(&completion_ready_session)
            .await?;

        let completion = self.run_debate_to_completion(&debate_session_id).await?;
        let rationale = completion["verdict"]["recommended_action"]
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "Winning assignment: primary={} reviewer={}",
                    winner_task_id, reviewer_task_id
                )
            });

        let snapshot = {
            let mut collaboration = self.collaboration.write().await;
            let Some(session) = collaboration.get_mut(parent_task_id) else {
                anyhow::bail!("no collaboration session found for parent task {parent_task_id}");
            };

            session.role_assignment = Some(ConsensusRoleAssignment {
                primary_task_id: winner_task_id.clone(),
                primary_role: "primary".to_string(),
                reviewer_task_id: reviewer_task_id.clone(),
                reviewer_role: "reviewer".to_string(),
                assigned_at: now_millis(),
            });
            session.consensus = Some(Consensus {
                topic: format!("bid resolution for {}", normalize_topic(&mission)),
                winner: winner_task_id.clone(),
                rationale: rationale.clone(),
                votes: Vec::new(),
            });
            for agent in session.agents.iter_mut() {
                if agent.task_id == winner_task_id {
                    agent.role = "primary".to_string();
                    agent.confidence = winner_bid.confidence;
                } else if agent.task_id == reviewer_task_id {
                    agent.role = "reviewer".to_string();
                    agent.confidence = reviewer_bid.confidence;
                }
            }
            if let Some(disagreement) = session
                .disagreements
                .iter_mut()
                .find(|item| item.id == disagreement_id)
            {
                disagreement.resolution = "resolved".to_string();
            }
            session.updated_at = now_millis();
            session.clone()
        };
        self.persist_collaboration_session(&snapshot).await?;

        Ok(serde_json::json!({
            "debate_session_id": debate_session_id,
            "winner_task_id": winner_task_id,
            "reviewer_task_id": reviewer_task_id,
            "verdict": completion["verdict"],
            "current_round": completion["current_round"],
            "status": completion["status"],
        }))
    }

    pub(crate) async fn vote_on_collaboration_disagreement(
        &self,
        parent_task_id: &str,
        disagreement_id: &str,
        task_id: &str,
        position: &str,
        confidence: Option<f64>,
    ) -> Result<serde_json::Value> {
        if !self.config.read().await.collaboration.enabled {
            anyhow::bail!("collaboration capability is disabled in agent config");
        }
        let mut collaboration = self.collaboration.write().await;
        let Some(session) = collaboration.get_mut(parent_task_id) else {
            anyhow::bail!("no collaboration session found for parent task {parent_task_id}");
        };
        let Some(disagreement) = session
            .disagreements
            .iter_mut()
            .find(|item| item.id == disagreement_id)
        else {
            anyhow::bail!("unknown disagreement {disagreement_id}");
        };
        session.consensus = apply_vote_to_disagreement(
            disagreement,
            &session.agents,
            task_id,
            position,
            confidence,
        );
        let resolution = disagreement.resolution.clone();
        let consensus = session.consensus.clone();
        let session_id = session.id.clone();
        let escalation = (disagreement.resolution == "escalated").then(|| disagreement.clone());
        let thread_id = session.thread_id.clone();
        session.updated_at = now_millis();
        let snapshot = session.clone();
        let report = serde_json::json!({
            "session_id": session_id,
            "resolution": resolution,
            "consensus": consensus,
        });
        drop(collaboration);
        self.persist_collaboration_session(&snapshot).await?;

        if let (Some(disagreement), Some(thread_id)) = (escalation, thread_id) {
            let _ = self.event_tx.send(AgentEvent::WorkflowNotice {
                thread_id,
                kind: "collaboration".to_string(),
                message: format!("Unresolved subagent disagreement on {}", disagreement.topic),
                details: Some(disagreement.positions.join(" vs ")),
            });
        }

        Ok(report)
    }

    pub async fn collaboration_sessions_json(
        &self,
        parent_task_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        if !self.config.read().await.collaboration.enabled {
            anyhow::bail!("collaboration capability is disabled in agent config");
        }
        if let Some(parent_task_id) = parent_task_id {
            let collaboration = self.collaboration.read().await;
            if let Some(session) = collaboration.get(parent_task_id) {
                return Ok(serde_json::to_value(session).unwrap_or_else(|_| serde_json::json!({})));
            }
            drop(collaboration);
            if let Some(session) = self.persisted_collaboration_session(parent_task_id).await? {
                return Ok(serde_json::to_value(session).unwrap_or_else(|_| serde_json::json!({})));
            }
            return Ok(serde_json::json!([]));
        }
        Ok(
            serde_json::to_value(self.merged_persisted_collaboration_sessions().await?)
                .unwrap_or_else(|_| serde_json::json!([])),
        )
    }

    pub(in crate::agent) async fn record_collaboration_outcome(
        &self,
        task: &AgentTask,
        outcome: &str,
    ) {
        if !self.config.read().await.collaboration.enabled {
            return;
        }
        let Some(parent_task_id) = task.parent_task_id.as_deref() else {
            return;
        };
        if task.source != "subagent" {
            return;
        }
        let summary = match outcome {
            "success" => task
                .result
                .as_deref()
                .or(task.logs.last().map(|entry| entry.message.as_str()))
                .unwrap_or("subagent completed successfully"),
            "failure" => task
                .last_error
                .as_deref()
                .or(task.error.as_deref())
                .unwrap_or("subagent failed"),
            "cancelled" => "subagent cancelled before conclusion",
            _ => "subagent updated",
        };
        let position = match outcome {
            "success" => "recommended",
            "failure" => "rejected",
            "cancelled" => "cancelled",
            _ => "reported",
        };
        if let Err(error) = self.ensure_task_collaboration_session(task).await {
            tracing::warn!(
                task_id = %task.id,
                "failed to initialize solo collaboration session: {error}"
            );
            return;
        }
        if let Err(error) = self
            .record_collaboration_contribution(
                &task.id,
                &task.id,
                &task.title,
                position,
                vec![crate::agent::summarize_text(summary, 220)],
                if outcome == "success" { 0.8 } else { 0.6 },
            )
            .await
        {
            tracing::warn!(
                task_id = %task.id,
                parent_task_id = %task.id,
                "failed to record collaboration outcome: {error}"
            );
        }

        let parent_session_exists = {
            let collaboration = self.collaboration.read().await;
            collaboration.contains_key(parent_task_id)
        };
        if parent_session_exists {
            let _ = self
                .record_collaboration_contribution(
                    parent_task_id,
                    &task.id,
                    &task.title,
                    position,
                    vec![crate::agent::summarize_text(summary, 220)],
                    if outcome == "success" { 0.8 } else { 0.6 },
                )
                .await;
        }
    }

    async fn maybe_auto_escalate_collaboration_debate(
        &self,
        parent_task_id: &str,
    ) -> Result<Option<String>> {
        let debate_config = {
            let config = self.config.read().await;
            if !config.collaboration.enabled || !config.debate.enabled {
                return Ok(None);
            }
            config.debate.clone()
        };

        let launch = {
            let mut collaboration = self.collaboration.write().await;
            let Some(session) = collaboration.get_mut(parent_task_id) else {
                return Ok(None);
            };
            let Some(seed) = build_pending_debate_seed(session, &debate_config) else {
                return Ok(None);
            };
            let Some(disagreement) = session
                .disagreements
                .iter_mut()
                .find(|item| item.id == seed.disagreement_id)
            else {
                return Ok(None);
            };
            disagreement.debate_session_id = Some(seed.debate_session.id.clone());
            session.updated_at = now_millis();

            PendingDebateLaunch {
                seed,
                collaboration_snapshot: session.clone(),
            }
        };

        self.persist_collaboration_session(&launch.collaboration_snapshot)
            .await?;

        match self
            .persist_seeded_debate_session(
                launch.seed.debate_session.clone(),
                launch.seed.arguments.clone(),
            )
            .await
        {
            Ok(session) => Ok(Some(session.id)),
            Err(error) => {
                let rollback_snapshot = {
                    let mut collaboration = self.collaboration.write().await;
                    let Some(session) = collaboration.get_mut(parent_task_id) else {
                        return Err(error);
                    };
                    if let Some(disagreement) = session
                        .disagreements
                        .iter_mut()
                        .find(|item| item.id == launch.seed.disagreement_id)
                    {
                        if disagreement.debate_session_id.as_deref()
                            == Some(launch.seed.debate_session.id.as_str())
                        {
                            disagreement.debate_session_id = None;
                        }
                    }
                    session.updated_at = now_millis();
                    session.clone()
                };
                self.persist_collaboration_session(&rollback_snapshot)
                    .await?;
                Err(error)
            }
        }
    }
}
