//! Idle-time memory consolidation -- trace review, fact aging, heuristic promotion.

use super::*;

#[path = "consolidation_pure.rs"]
mod pure;
#[path = "consolidation_refinement.rs"]
mod refinement;
pub(crate) use pure::{compute_decay_confidence, is_idle_for_consolidation};
#[cfg(test)]
pub(crate) use pure::{DEFAULT_HALF_LIFE_HOURS, DEFAULT_IDLE_THRESHOLD_MS};

const DISTILLATION_LAST_RUN_KEY_PREFIX: &str = "distillation_last_run_ms";
const FORGE_LAST_RUN_KEY_PREFIX: &str = "forge_last_run_ms";

// ---------------------------------------------------------------------------
// Consolidation entry point and sub-tasks
// ---------------------------------------------------------------------------

impl AgentEngine {
    /// Run a consolidation tick if idle conditions are met. Per D-03, this is called
    /// from within run_structured_heartbeat_adaptive() as a sub-phase.
    pub(super) async fn maybe_run_consolidation_if_idle(
        &self,
        budget: std::time::Duration,
    ) -> Option<ConsolidationResult> {
        let config = self.config.read().await.clone();
        if !config.consolidation.enabled {
            return None;
        }

        // Check idle conditions per D-01
        let active_tasks = self
            .tasks
            .lock()
            .await
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::InProgress))
            .count();
        let running_goals = self
            .goal_runs
            .lock()
            .await
            .iter()
            .filter(|g| matches!(g.status, GoalRunStatus::Running | GoalRunStatus::Planning))
            .count();
        let active_streams = self.stream_cancellations.lock().await.len();
        let last_presence = self.anticipatory.read().await.last_presence_at;
        let now = now_millis();
        let idle_threshold_ms = config.consolidation.idle_threshold_secs * 1000;

        if !is_idle_for_consolidation(
            active_tasks,
            running_goals,
            active_streams,
            last_presence,
            now,
            idle_threshold_ms,
        ) {
            return None;
        }

        tracing::info!("idle conditions met -- starting consolidation tick");

        let deadline = std::time::Instant::now() + budget;
        let mut result = ConsolidationResult::default();

        // Sub-task 1: Review execution traces -> promote heuristics (MEMO-01, MEMO-06)
        if std::time::Instant::now() < deadline {
            result.traces_reviewed = self.review_execution_traces(&config, &deadline).await;
        }

        // Sub-task 1.5: Distill durable memory from older threads.
        if std::time::Instant::now() < deadline {
            self.maybe_run_distillation_subphase(&deadline, &mut result)
                .await;
        }

        // Sub-task 1.6: Forge strategy hints from recent execution traces.
        if std::time::Instant::now() < deadline {
            self.maybe_run_forge_subphase(&deadline, &mut result).await;
        }

        // Sub-task 2: Decay stale memory facts and tombstone low-confidence ones (MEMO-02)
        if std::time::Instant::now() < deadline {
            result.facts_decayed = self.apply_fact_decay(&config, &deadline).await;
        }

        // Sub-task 3: Cleanup expired tombstones - 7-day TTL (MEMO-05)
        if std::time::Instant::now() < deadline {
            result.tombstones_purged = self.cleanup_expired_tombstones(&config).await;
        }

        // Sub-task 4: Proactive memory refinement (LLM call, most expensive -- runs LAST)
        if std::time::Instant::now() < deadline {
            result.facts_refined = self.refine_memory_facts(&deadline).await;
        }

        // Sub-task 5: Flag skill draft candidates (SKIL-01, SKIL-02, per D-02)
        if std::time::Instant::now() < deadline {
            result.skill_candidates_flagged =
                self.flag_skill_draft_candidates(&config, &deadline).await;
        }

        // Sub-task 6: Draft flagged candidates into SKILL.md (SKIL-01, per D-03)
        if std::time::Instant::now() < deadline {
            result.skills_drafted = self
                .draft_flagged_skill_candidates(&config, &deadline)
                .await;
        }

        // Sub-task 7: Run mental tests on Draft skills (SKIL-04, per D-05)
        if std::time::Instant::now() < deadline {
            result.skills_tested = self.run_skill_mental_tests(&config, &deadline).await;
        }

        // Sub-task 8: Check lifecycle promotions (SKIL-05, per D-06)
        if std::time::Instant::now() < deadline {
            result.skills_promoted = self.check_skill_promotions(&config, &deadline).await;
        }

        // Sub-task 9: Expire stale negative knowledge constraints (Phase 1: Memory Foundation - NKNO-04)
        if std::time::Instant::now() < deadline {
            match self.expire_negative_constraints().await {
                Ok(n) if n > 0 => {
                    tracing::info!(
                        expired = n,
                        "Expired negative knowledge constraints during consolidation"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Failed to expire negative knowledge constraints: {e}");
                }
            }
        }

        // Sub-task 10: Expire old episodes past TTL (Phase 1: Memory Foundation - EPIS-09)
        if std::time::Instant::now() < deadline {
            if let Err(e) = self.expire_old_episodes().await {
                tracing::warn!("Failed to expire old episodes: {e}");
            }
        }

        // Persist learning stores after consolidation updates (D-10)
        if result.traces_reviewed > 0 {
            self.persist_learning_stores().await;
        }

        // Log provenance for the consolidation tick (MEMO-04)
        self.record_provenance_event(
            "memory_consolidation",
            &format!(
                "Consolidation tick: {} traces reviewed, distillation ran={}, forge ran={}, {} facts decayed, {} tombstones purged, {} facts refined, {} skill candidates flagged, {} skills drafted, {} skills tested, {} skills promoted",
                result.traces_reviewed, result.distillation_ran, result.forge_ran, result.facts_decayed, result.tombstones_purged, result.facts_refined,
                result.skill_candidates_flagged, result.skills_drafted, result.skills_tested, result.skills_promoted
            ),
            serde_json::json!({
                "traces_reviewed": result.traces_reviewed,
                "distillation_ran": result.distillation_ran,
                "distillation_threads_analyzed": result.distillation_threads_analyzed,
                "distillation_candidates_generated": result.distillation_candidates_generated,
                "distillation_auto_applied": result.distillation_auto_applied,
                "distillation_queued_for_review": result.distillation_queued_for_review,
                "forge_ran": result.forge_ran,
                "forge_traces_analyzed": result.forge_traces_analyzed,
                "forge_patterns_detected": result.forge_patterns_detected,
                "forge_hints_generated": result.forge_hints_generated,
                "forge_hints_auto_applied": result.forge_hints_auto_applied,
                "forge_hints_logged_only": result.forge_hints_logged_only,
                "facts_decayed": result.facts_decayed,
                "tombstones_purged": result.tombstones_purged,
                "facts_refined": result.facts_refined,
                "skill_candidates_flagged": result.skill_candidates_flagged,
                "skills_drafted": result.skills_drafted,
                "skills_tested": result.skills_tested,
                "skills_promoted": result.skills_promoted,
            }),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        Some(result)
    }

    async fn maybe_run_distillation_subphase(
        &self,
        deadline: &std::time::Instant,
        result: &mut ConsolidationResult,
    ) {
        if std::time::Instant::now() >= *deadline {
            return;
        }

        let agent_id = crate::agent::agent_identity::current_agent_scope_id();
        let config = super::memory_distillation::DistillationConfig {
            agent_id: agent_id.clone(),
            ..Default::default()
        };
        if !self
            .should_run_learning_subphase(
                DISTILLATION_LAST_RUN_KEY_PREFIX,
                &agent_id,
                config.interval_hours,
            )
            .await
        {
            return;
        }

        match super::memory_distillation::run_distillation_pass(
            &self.history,
            &config,
            &self.data_dir,
        )
        .await
        {
            Ok(pass) => {
                result.distillation_ran = true;
                result.distillation_threads_analyzed = pass.threads_analyzed;
                result.distillation_candidates_generated = pass.candidates_generated;
                result.distillation_auto_applied = pass.auto_applied;
                result.distillation_queued_for_review = pass.queued_for_review;
                self.record_learning_subphase_run(DISTILLATION_LAST_RUN_KEY_PREFIX, &agent_id)
                    .await;
            }
            Err(error) => {
                tracing::warn!(agent_id = %agent_id, "distillation sub-phase failed: {error}");
            }
        }
    }

    async fn maybe_run_forge_subphase(
        &self,
        deadline: &std::time::Instant,
        result: &mut ConsolidationResult,
    ) {
        if std::time::Instant::now() >= *deadline {
            return;
        }

        let agent_id = crate::agent::agent_identity::current_agent_scope_id();
        let config = super::forge::ForgeConfig {
            agent_id: agent_id.clone(),
            ..Default::default()
        };
        if !self
            .should_run_learning_subphase(
                FORGE_LAST_RUN_KEY_PREFIX,
                &agent_id,
                config.interval_hours,
            )
            .await
        {
            return;
        }

        match super::forge::run_forge_pass(&self.history, &config, &self.data_dir).await {
            Ok(pass) => {
                result.forge_ran = true;
                result.forge_traces_analyzed = pass.traces_analyzed;
                result.forge_patterns_detected = pass.patterns_detected;
                result.forge_hints_generated = pass.hints_generated;
                result.forge_hints_auto_applied = pass.hints_auto_applied;
                result.forge_hints_logged_only = pass.hints_logged_only;
                self.record_learning_subphase_run(FORGE_LAST_RUN_KEY_PREFIX, &agent_id)
                    .await;
            }
            Err(error) => {
                tracing::warn!(agent_id = %agent_id, "forge sub-phase failed: {error}");
            }
        }
    }

    async fn should_run_learning_subphase(
        &self,
        key_prefix: &str,
        agent_id: &str,
        interval_hours: u64,
    ) -> bool {
        if interval_hours == 0 {
            return true;
        }

        let key = format!("{key_prefix}:{agent_id}");
        let now = now_millis();
        match self.history.get_consolidation_state(&key).await {
            Ok(Some(value)) => value
                .parse::<u64>()
                .map(|last_run| {
                    now.saturating_sub(last_run) >= interval_hours.saturating_mul(60 * 60 * 1000)
                })
                .unwrap_or(true),
            Ok(None) => true,
            Err(error) => {
                tracing::warn!(key = %key, "failed to read learning sub-phase cadence state: {error}");
                true
            }
        }
    }

    async fn record_learning_subphase_run(&self, key_prefix: &str, agent_id: &str) {
        let key = format!("{key_prefix}:{agent_id}");
        let now = now_millis();
        if let Err(error) = self
            .history
            .set_consolidation_state(&key, &now.to_string(), now)
            .await
        {
            tracing::warn!(key = %key, "failed to persist learning sub-phase cadence state: {error}");
        }
    }

    /// Review recent execution traces and promote successful tool sequences to
    /// heuristics when they cross the promotion threshold (MEMO-01, MEMO-06).
    ///
    /// Uses a watermark to avoid re-reviewing traces across restarts (Pitfall 5).
    async fn review_execution_traces(
        &self,
        config: &AgentConfig,
        deadline: &std::time::Instant,
    ) -> usize {
        // Get watermark from consolidation state
        let watermark: u64 = match self
            .history
            .get_consolidation_state("trace_review_watermark")
            .await
        {
            Ok(Some(val)) => val.parse().unwrap_or(0),
            _ => 0,
        };

        // Fetch a batch of successful traces since the watermark
        let traces = match self
            .history
            .list_recent_successful_traces(watermark, 50)
            .await
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "failed to list recent traces for consolidation");
                return 0;
            }
        };

        if traces.is_empty() {
            return 0;
        }

        let promotion_threshold = config.consolidation.heuristic_promotion_threshold;
        let now = now_millis();
        let mut reviewed = 0;
        let mut last_created_at: u64 = watermark;

        for trace in &traces {
            // Budget check at the start of each iteration
            if std::time::Instant::now() >= *deadline {
                break;
            }

            // Extract tool sequence from tool_sequence_json
            let tool_names: Vec<String> = trace
                .tool_sequence_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
                .unwrap_or_default();

            if tool_names.is_empty() {
                reviewed += 1;
                last_created_at = last_created_at.max(trace.created_at as u64);
                continue;
            }

            let task_type = trace.task_type.as_deref().unwrap_or("unknown");

            let duration_ms = trace.duration_ms.unwrap_or(0) as u64;

            // Record in PatternStore
            {
                let mut patterns = self.pattern_store.write().await;
                patterns.record_sequence(&tool_names, task_type, true, now);
            }

            // Check if the pattern crosses the promotion threshold
            let should_promote = {
                let patterns = self.pattern_store.read().await;
                let matching = patterns.find_patterns(
                    task_type,
                    super::learning::patterns::PatternType::SuccessSequence,
                );
                matching
                    .iter()
                    .any(|p| p.tool_sequence == tool_names && p.occurrences >= promotion_threshold)
            };

            if should_promote {
                // Promote each tool in the sequence to HeuristicStore
                {
                    let mut heuristics = self.heuristic_store.write().await;
                    let avg_duration = duration_ms / tool_names.len().max(1) as u64;
                    for tool_name in &tool_names {
                        heuristics.update_tool(tool_name, task_type, true, avg_duration);
                    }
                }

                // Record provenance for the promotion
                self.record_provenance_event(
                    "heuristic_promotion",
                    &format!(
                        "Promoted tool sequence {:?} for task type '{}' (>= {} occurrences)",
                        tool_names, task_type, promotion_threshold
                    ),
                    serde_json::json!({
                        "tool_sequence": tool_names,
                        "task_type": task_type,
                        "threshold": promotion_threshold,
                        "trace_id": trace.id,
                    }),
                    trace.goal_run_id.as_deref(),
                    trace.task_id.as_deref(),
                    None,
                    None,
                    None,
                )
                .await;
            }

            reviewed += 1;
            last_created_at = last_created_at.max(trace.created_at as u64);
        }

        // Update watermark for next run (Pitfall 5: watermark-based pagination)
        if last_created_at > watermark {
            if let Err(e) = self
                .history
                .set_consolidation_state(
                    "trace_review_watermark",
                    &last_created_at.to_string(),
                    now,
                )
                .await
            {
                tracing::warn!(error = %e, "failed to update trace review watermark");
            }
        }

        reviewed
    }

    /// Decay stale memory facts and tombstone those below the configurable threshold
    /// (MEMO-02). Facts with confidence below `fact_decay_supersede_threshold` are
    /// actually tombstoned via `supersede_memory_fact`, not just counted.
    async fn apply_fact_decay(&self, config: &AgentConfig, deadline: &std::time::Instant) -> usize {
        let half_life = config.consolidation.memory_decay_half_life_hours;
        let threshold = config.consolidation.fact_decay_supersede_threshold;
        let now = now_millis();

        // Read MEMORY.md content
        let memory_path =
            memory_paths_for_scope(&self.data_dir, &current_agent_scope_id()).memory_path;
        let content = match tokio::fs::read_to_string(&memory_path).await {
            Ok(c) => c,
            Err(_) => return 0,
        };

        // Extract fact candidates from MEMORY.md
        let facts = extract_memory_fact_candidates(&content);
        if facts.is_empty() {
            return 0;
        }

        // For each fact, look up its last provenance timestamp to approximate
        // last_confirmed_at. Fall back to using the current time minus 30 days
        // if no provenance record exists (treating it as very old).
        let mut decayed_count = 0;

        for fact in &facts {
            if std::time::Instant::now() >= *deadline {
                break;
            }

            // Skip facts that are already superseded
            if fact.display.contains("[SUPERSEDED]") {
                continue;
            }

            // Query provenance for this fact key to get last_confirmed_at
            let last_confirmed_at = match self
                .history
                .memory_provenance_report(Some(MemoryTarget::Memory.label()), 50)
                .await
            {
                Ok(report) => {
                    // Find the most recent provenance entry mentioning this fact key
                    report
                        .entries
                        .iter()
                        .find(|e| e.fact_keys.contains(&fact.key))
                        .map(|e| e.created_at as u64)
                        .unwrap_or(0)
                }
                Err(_) => 0,
            };

            // If no provenance record, skip (we cannot compute meaningful decay)
            if last_confirmed_at == 0 {
                continue;
            }

            let confidence = compute_decay_confidence(last_confirmed_at, now, half_life);

            if confidence < threshold {
                // Actually tombstone the fact via supersede_memory_fact (MEMO-02)
                if let Err(e) = self
                    .supersede_memory_fact(
                        MemoryTarget::Memory,
                        &fact.display,
                        &fact.key,
                        "", // empty replacement -- fact is simply removed from active memory
                        "fact_decay",
                    )
                    .await
                {
                    tracing::warn!(
                        fact_key = %fact.key,
                        error = %e,
                        "failed to tombstone decayed fact"
                    );
                    continue;
                }

                // Record provenance for the decay action (MEMO-04)
                self.record_provenance_event(
                    "fact_decay",
                    &format!(
                        "Decayed fact '{}' (confidence {:.2} < threshold {:.2})",
                        fact.key, confidence, threshold
                    ),
                    serde_json::json!({
                        "fact_key": fact.key,
                        "confidence": confidence,
                        "threshold": threshold,
                        "half_life_hours": half_life,
                    }),
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await;

                decayed_count += 1;
            }
        }

        decayed_count
    }

    /// Cleanup expired tombstones older than the configured TTL (MEMO-05).
    async fn cleanup_expired_tombstones(&self, config: &AgentConfig) -> usize {
        let max_age_ms = config.consolidation.tombstone_ttl_days * 24 * 60 * 60 * 1000;
        let now = now_millis();

        match self
            .history
            .delete_expired_tombstones(max_age_ms, now)
            .await
        {
            Ok(count) => {
                if count > 0 {
                    tracing::debug!(
                        purged = count,
                        ttl_days = config.consolidation.tombstone_ttl_days,
                        "purged expired memory tombstones"
                    );
                }
                count
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to delete expired tombstones");
                0
            }
        }
    }

    /// Proactive memory refinement: detect redundant/contradictory facts and merge via LLM.
    /// Per D-12: budget within the 30-second tick. Runs LAST (most expensive sub-task).
    /// Per Pitfall 3: check circuit breaker before LLM call. Skip if open.
    async fn refine_memory_facts(&self, deadline: &std::time::Instant) -> usize {
        // 1. Check remaining budget -- need at least 10 seconds for an LLM call
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.as_secs() < 10 {
            tracing::debug!("skipping memory refinement -- insufficient budget");
            return 0;
        }

        // 2. Check circuit breaker for configured provider
        let config = self.config.read().await.clone();
        if let Err(_e) = self.check_circuit_breaker(&config.provider).await {
            tracing::debug!(
                provider = %config.provider,
                "skipping memory refinement -- circuit breaker open"
            );
            return 0;
        }

        // 3. Read MEMORY.md content
        let memory_path =
            memory_paths_for_scope(&self.data_dir, &current_agent_scope_id()).memory_path;
        let content = match tokio::fs::read_to_string(&memory_path).await {
            Ok(c) => c,
            Err(_) => return 0,
        };

        // 4. Extract fact candidates and find contradictions/redundancies
        let candidates = extract_memory_fact_candidates(&content);
        if candidates.len() < 2 {
            return 0; // Need at least 2 facts to find contradictions
        }

        // Find facts with overlapping keys (potential contradictions/redundancies)
        let mut key_groups: std::collections::HashMap<String, Vec<&MemoryFactCandidate>> =
            std::collections::HashMap::new();
        for candidate in &candidates {
            let normalized_key = candidate.key.to_lowercase().trim().to_string();
            key_groups
                .entry(normalized_key)
                .or_default()
                .push(candidate);
        }

        let conflicting: Vec<_> = key_groups
            .values()
            .filter(|group| group.len() > 1)
            .collect();

        if conflicting.is_empty() {
            return 0; // No contradictions found
        }

        // 5. Build a short LLM prompt to merge the first conflict group
        // Only handle one conflict per tick to stay within budget
        let conflict = &conflicting[0];
        let facts_text: String = conflict
            .iter()
            .map(|f| format!("- {}: {}", f.key, f.display))
            .collect::<Vec<_>>()
            .join("\n");

        let refinement_prompt = format!(
            "You are a memory consolidation agent. These memory facts appear to be about the same topic but may conflict or be redundant.\n\n\
             Facts:\n{}\n\n\
             Merge these into a single, accurate fact. Return ONLY the merged fact as a single line. \
             If they truly conflict, keep the most recent/specific one. If redundant, combine into one concise fact.",
            facts_text
        );

        // 6. Make LLM call with timeout -- following the pattern from memory_flush.rs
        let llm_timeout = remaining.saturating_sub(std::time::Duration::from_secs(2));

        let merged_content = match tokio::time::timeout(
            llm_timeout,
            self.send_refinement_llm_call(&config, &refinement_prompt),
        )
        .await
        {
            Ok(Ok(response)) => response.trim().to_string(),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "memory refinement LLM call failed");
                return 0;
            }
            Err(_) => {
                tracing::debug!("memory refinement LLM call timed out within budget");
                return 0;
            }
        };

        if merged_content.is_empty() {
            return 0;
        }

        // 7. Supersede the original facts with the merged result
        // Per Pitfall 2: tombstone-before-update via supersede_memory_fact
        let original_content = conflict
            .iter()
            .map(|f| f.display.clone())
            .collect::<Vec<_>>()
            .join("\n");

        match self
            .supersede_memory_fact(
                MemoryTarget::Memory,
                &original_content,
                &conflict[0].key,
                &merged_content,
                "consolidation_refinement",
            )
            .await
        {
            Ok(()) => {
                tracing::info!(key = %conflict[0].key, "refined memory fact via LLM merge");
                1
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to supersede memory fact during refinement");
                0
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tests/consolidation.rs"]
mod tests;
