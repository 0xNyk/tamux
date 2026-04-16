pub(crate) mod protocol;
pub(crate) mod types;

use anyhow::Result;
use serde_json::json;

use crate::agent::engine::AgentEngine;
use crate::agent::handoff::divergent::Framing;

use self::protocol::{
    advance_round, build_debate_round_requests, create_debate_session, finalize_verdict,
    now_millis, validate_argument,
};
use self::types::{Argument, DebateRoundRequest, DebateSession, DebateStatus, RoleKind};

impl AgentEngine {
    pub(crate) async fn start_debate_session(
        &self,
        topic: &str,
        custom_framings: Option<Vec<Framing>>,
        thread_id: &str,
        goal_run_id: Option<&str>,
    ) -> Result<String> {
        let cfg = self.config.read().await.debate.clone();
        if !cfg.enabled {
            anyhow::bail!("debate capability is disabled in agent config");
        }
        let framings = custom_framings.unwrap_or_else(|| {
            vec![
                Framing {
                    label: "analytical-lens".to_string(),
                    system_prompt_override: format!("Analyze this topic formally: {topic}"),
                    task_id: None,
                    contribution_id: None,
                },
                Framing {
                    label: "pragmatic-lens".to_string(),
                    system_prompt_override: format!("Approach this topic pragmatically: {topic}"),
                    task_id: None,
                    contribution_id: None,
                },
            ]
        });
        let mut session = create_debate_session(
            topic.to_string(),
            framings,
            cfg.default_max_rounds,
            cfg.role_rotation,
            Some(thread_id.to_string()),
            goal_run_id.map(str::to_string),
        )?;
        self.persist_debate_session(&session).await?;

        let opening_round = session.current_round;
        let opening_roles = session.roles.clone();
        for role in opening_roles {
            let content = match role.role {
                RoleKind::Proponent => format!(
                    "Round {} opening case for '{}': defend the strongest actionable path with evidence.",
                    opening_round, session.topic
                ),
                RoleKind::Skeptic => format!(
                    "Round {} opening challenge for '{}': surface the sharpest evidence-backed risks and counterarguments.",
                    opening_round, session.topic
                ),
                RoleKind::Synthesizer => format!(
                    "Round {} opening synthesis for '{}': capture the main positions, early convergence, and key open tensions.",
                    opening_round, session.topic
                ),
            };
            let argument = Argument {
                id: format!("arg_{}", uuid::Uuid::new_v4()),
                round: opening_round,
                role: role.role,
                agent_id: role.agent_id,
                content,
                evidence_refs: vec![format!(
                    "debate:{}:round:{}:{}",
                    session.id,
                    opening_round,
                    role.role.as_str()
                )],
                responds_to: None,
                timestamp_ms: now_millis(),
            };
            self.append_debate_argument(&session.id, argument).await?;
        }

        session = self
            .get_persisted_debate_session(&session.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown debate session after opening round seed"))?;
        if session.arguments.len() >= session.roles.len()
            && session.current_round < session.max_rounds
        {
            advance_round(&mut session, cfg.role_rotation)?;
            self.persist_debate_session(&session).await?;
        }

        Ok(session.id.clone())
    }

    pub(crate) async fn persist_debate_session(&self, session: &DebateSession) -> Result<()> {
        let session_json = serde_json::to_string(session)?;
        self.history
            .upsert_debate_session(&session.id, &session_json, now_millis())
            .await
    }

    pub(crate) async fn persist_seeded_debate_session(
        &self,
        mut session: DebateSession,
        seed_arguments: Vec<Argument>,
    ) -> Result<DebateSession> {
        let cfg = self.config.read().await.debate.clone();
        if !cfg.enabled {
            anyhow::bail!("debate capability is disabled in agent config");
        }

        self.persist_debate_session(&session).await?;

        let mut known_argument_ids = Vec::new();
        for argument in seed_arguments {
            validate_argument(
                &argument,
                cfg.min_evidence_refs as usize,
                &known_argument_ids,
            )?;
            known_argument_ids.push(argument.id.clone());
            self.history
                .insert_debate_argument(
                    &session.id,
                    &serde_json::to_string(&argument)?,
                    argument.timestamp_ms,
                )
                .await?;
            session.arguments.push(argument);
        }

        if session.arguments.len() >= 2 && session.current_round < session.max_rounds {
            advance_round(&mut session, cfg.role_rotation)?;
        }

        self.persist_debate_session(&session).await?;
        Ok(session)
    }

    pub(crate) async fn append_debate_argument(
        &self,
        session_id: &str,
        argument: Argument,
    ) -> Result<()> {
        let cfg = self.config.read().await.debate.clone();
        let mut session = self
            .get_persisted_debate_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown debate session: {session_id}"))?;
        let known_argument_ids = session
            .arguments
            .iter()
            .map(|arg| arg.id.clone())
            .collect::<Vec<_>>();
        validate_argument(
            &argument,
            cfg.min_evidence_refs as usize,
            &known_argument_ids,
        )?;
        session.arguments.push(argument.clone());
        self.history
            .insert_debate_argument(
                session_id,
                &serde_json::to_string(&argument)?,
                argument.timestamp_ms,
            )
            .await?;
        self.persist_debate_session(&session).await?;
        Ok(())
    }

    pub(crate) async fn dispatch_debate_round_request(
        &self,
        request: &DebateRoundRequest,
        content: &str,
        evidence_refs: Vec<String>,
        responds_to: Option<String>,
    ) -> Result<serde_json::Value> {
        let argument = Argument {
            id: format!("arg_{}", uuid::Uuid::new_v4()),
            round: request.round,
            role: request.role,
            agent_id: request.agent_id.clone(),
            content: content.trim().to_string(),
            evidence_refs,
            responds_to,
            timestamp_ms: now_millis(),
        };
        self.append_debate_argument(&request.session_id, argument)
            .await?;
        Ok(json!({
            "status": "appended",
            "session_id": request.session_id,
            "round": request.round,
            "role": request.role.as_str(),
            "prompt": request.prompt,
        }))
    }

    pub(crate) async fn run_debate_round_cycle(
        &self,
        session_id: &str,
    ) -> Result<serde_json::Value> {
        let session = self
            .get_persisted_debate_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown debate session: {session_id}"))?;

        if session.status != DebateStatus::InProgress {
            anyhow::bail!("cannot run debate round cycle when status is not in_progress");
        }

        let requests = build_debate_round_requests(&session);
        if requests.is_empty() {
            anyhow::bail!("no debate round requests available for session {session_id}");
        }

        for request in requests {
            let already_recorded = session
                .arguments
                .iter()
                .any(|argument| argument.round == request.round && argument.role == request.role);
            if already_recorded {
                continue;
            }

            let content = match request.role {
                RoleKind::Proponent => format!(
                    "Round {} proponent case for '{}': back the strongest actionable path with evidence.",
                    request.round, request.topic
                ),
                RoleKind::Skeptic => format!(
                    "Round {} skeptic case for '{}': challenge the leading path with the sharpest evidence-backed counterargument.",
                    request.round, request.topic
                ),
                RoleKind::Synthesizer => format!(
                    "Round {} synthesis for '{}': capture convergence, tensions, and the safest next action.",
                    request.round, request.topic
                ),
            };
            let evidence_refs = vec![format!(
                "debate:{}:round:{}:{}",
                request.session_id,
                request.round,
                request.role.as_str()
            )];

            self.dispatch_debate_round_request(&request, &content, evidence_refs, None)
                .await?;
        }

        if session.current_round < session.max_rounds {
            self.advance_debate_round(session_id).await?;
        } else {
            self.complete_debate_session(session_id).await?;
        }

        self.get_debate_session_payload(session_id).await
    }

    pub(crate) async fn run_debate_to_completion(
        &self,
        session_id: &str,
    ) -> Result<serde_json::Value> {
        loop {
            let session = self
                .get_persisted_debate_session(session_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("unknown debate session: {session_id}"))?;

            if session.status == DebateStatus::Completed {
                return self.get_debate_session_payload(session_id).await;
            }

            self.run_debate_round_cycle(session_id).await?;
        }
    }

    pub(crate) async fn advance_debate_round(&self, session_id: &str) -> Result<DebateSession> {
        let role_rotation = self.config.read().await.debate.role_rotation;
        let mut session = self
            .get_persisted_debate_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown debate session: {session_id}"))?;
        advance_round(&mut session, role_rotation)?;
        self.persist_debate_session(&session).await?;
        Ok(session)
    }

    pub(crate) async fn complete_debate_session(
        &self,
        session_id: &str,
    ) -> Result<serde_json::Value> {
        let required_sections = self
            .config
            .read()
            .await
            .debate
            .verdict_required_sections
            .clone();
        let mut session = self
            .get_persisted_debate_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown debate session: {session_id}"))?;

        let consensus_points = vec![format!(
            "{} round(s) of structured debate completed",
            session.current_round
        )];
        let unresolved_tensions = session
            .arguments
            .iter()
            .filter(|arg| arg.role == RoleKind::Skeptic)
            .map(|arg| arg.content.clone())
            .take(3)
            .collect::<Vec<_>>();
        let recommended_action = session
            .arguments
            .iter()
            .rev()
            .find(|arg| arg.role == RoleKind::Synthesizer)
            .map(|arg| arg.content.clone())
            .or_else(|| session.arguments.last().map(|arg| arg.content.clone()))
            .unwrap_or_else(|| {
                "Review debate transcript and choose the safest next step".to_string()
            });

        finalize_verdict(
            &mut session,
            consensus_points,
            unresolved_tensions,
            recommended_action,
            0.7,
        )?;
        let verdict_json = serde_json::to_string(session.verdict.as_ref().expect("verdict set"))?;
        self.history
            .upsert_debate_verdict(session_id, &verdict_json, now_millis())
            .await?;
        self.persist_debate_session(&session).await?;

        Ok(json!({
            "session_id": session.id,
            "status": session.status,
            "required_sections": required_sections,
            "verdict": session.verdict,
            "arguments": session.arguments.len(),
            "current_round": session.current_round,
            "max_rounds": session.max_rounds,
        }))
    }

    pub(crate) async fn get_persisted_debate_session(
        &self,
        session_id: &str,
    ) -> Result<Option<DebateSession>> {
        let row = self.history.get_debate_session(session_id).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_str(&row.session_json)?))
    }

    pub(crate) async fn get_debate_session_payload(
        &self,
        session_id: &str,
    ) -> Result<serde_json::Value> {
        let session = self
            .get_persisted_debate_session(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown debate session: {session_id}"))?;
        Ok(json!({
            "session_id": session.id,
            "status": session.status,
            "topic": session.topic,
            "current_round": session.current_round,
            "max_rounds": session.max_rounds,
            "roles": session.roles,
            "arguments": session.arguments,
            "verdict": session.verdict,
            "created_at_ms": session.created_at_ms,
            "completed_at_ms": session.completed_at_ms,
        }))
    }
}

#[cfg(test)]
#[path = "tests/basic.rs"]
mod tests;
