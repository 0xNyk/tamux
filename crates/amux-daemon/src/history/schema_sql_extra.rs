pub(super) fn extended_schema_sql() -> &'static str {
    r#"
            CREATE TABLE IF NOT EXISTS causal_traces (
                id                    TEXT PRIMARY KEY,
                thread_id             TEXT,
                goal_run_id           TEXT,
                task_id               TEXT,
                decision_type         TEXT NOT NULL,
                selected_json         TEXT NOT NULL,
                rejected_options_json TEXT,
                context_hash          TEXT,
                causal_factors_json   TEXT,
                outcome_json          TEXT NOT NULL,
                model_used            TEXT,
                created_at            INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_causal_traces_decision_type ON causal_traces(decision_type, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_causal_traces_task_id ON causal_traces(task_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS memory_provenance (
                id            TEXT PRIMARY KEY,
                target        TEXT NOT NULL,
                mode          TEXT NOT NULL,
                source_kind   TEXT NOT NULL,
                content       TEXT NOT NULL,
                fact_keys_json TEXT NOT NULL DEFAULT '[]',
                thread_id     TEXT,
                task_id       TEXT,
                goal_run_id   TEXT,
                created_at    INTEGER NOT NULL,
                confirmed_at  INTEGER,
                retracted_at  INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_memory_provenance_target_ts ON memory_provenance(target, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memory_provenance_goal_run ON memory_provenance(goal_run_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS memory_provenance_relationships (
                id              TEXT PRIMARY KEY,
                source_entry_id TEXT NOT NULL,
                target_entry_id TEXT NOT NULL,
                relation_type   TEXT NOT NULL,
                fact_key        TEXT,
                created_at      INTEGER NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_provenance_rel_unique ON memory_provenance_relationships(source_entry_id, target_entry_id, relation_type, fact_key);
            CREATE INDEX IF NOT EXISTS idx_memory_provenance_rel_source ON memory_provenance_relationships(source_entry_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS skill_variants (
                variant_id         TEXT PRIMARY KEY,
                skill_name         TEXT NOT NULL,
                variant_name       TEXT NOT NULL,
                relative_path      TEXT NOT NULL UNIQUE,
                parent_variant_id  TEXT,
                version            TEXT NOT NULL,
                context_tags_json  TEXT NOT NULL DEFAULT '[]',
                use_count          INTEGER NOT NULL DEFAULT 0,
                success_count      INTEGER NOT NULL DEFAULT 0,
                failure_count      INTEGER NOT NULL DEFAULT 0,
                fitness_score      REAL NOT NULL DEFAULT 0,
                status             TEXT NOT NULL DEFAULT 'active',
                last_used_at       INTEGER,
                created_at         INTEGER NOT NULL,
                updated_at         INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_skill_variants_name ON skill_variants(skill_name, status, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_skill_variants_path ON skill_variants(relative_path);

            CREATE TABLE IF NOT EXISTS skill_variant_usage (
                usage_id           TEXT PRIMARY KEY,
                variant_id         TEXT NOT NULL,
                thread_id          TEXT,
                task_id            TEXT,
                goal_run_id        TEXT,
                context_tags_json  TEXT NOT NULL DEFAULT '[]',
                consulted_at       INTEGER NOT NULL,
                resolved_at        INTEGER,
                outcome            TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_skill_variant_usage_variant ON skill_variant_usage(variant_id, consulted_at DESC);
            CREATE INDEX IF NOT EXISTS idx_skill_variant_usage_resolution ON skill_variant_usage(task_id, goal_run_id, thread_id, resolved_at);

            CREATE TABLE IF NOT EXISTS skill_variant_history (
                id                 TEXT PRIMARY KEY,
                variant_id         TEXT NOT NULL,
                recorded_at        INTEGER NOT NULL,
                outcome            TEXT NOT NULL,
                fitness_score      REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_skill_variant_history_variant_ts ON skill_variant_history(variant_id, recorded_at DESC);

            CREATE TABLE IF NOT EXISTS gene_pool (
                parent_a        TEXT NOT NULL,
                parent_b        TEXT NOT NULL,
                offspring_id    TEXT NOT NULL,
                lifecycle_state TEXT NOT NULL,
                created_at      INTEGER NOT NULL,
                PRIMARY KEY (parent_a, parent_b)
            );
            CREATE INDEX IF NOT EXISTS idx_gene_pool_offspring ON gene_pool(offspring_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS collaboration_sessions (
                parent_task_id TEXT PRIMARY KEY,
                session_json   TEXT NOT NULL,
                updated_at     INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_collaboration_sessions_updated ON collaboration_sessions(updated_at DESC);

            CREATE TABLE IF NOT EXISTS debate_sessions (
                session_id   TEXT PRIMARY KEY,
                session_json TEXT NOT NULL,
                updated_at   INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_debate_sessions_updated ON debate_sessions(updated_at DESC);

            CREATE TABLE IF NOT EXISTS debate_arguments (
                session_id     TEXT NOT NULL,
                argument_json  TEXT NOT NULL,
                created_at     INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_debate_arguments_session_created ON debate_arguments(session_id, created_at ASC);

            CREATE TABLE IF NOT EXISTS debate_verdicts (
                session_id    TEXT PRIMARY KEY,
                verdict_json  TEXT NOT NULL,
                updated_at    INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_debate_verdicts_updated ON debate_verdicts(updated_at DESC);

            CREATE TABLE IF NOT EXISTS critique_sessions (
                session_id   TEXT PRIMARY KEY,
                session_json TEXT NOT NULL,
                updated_at   INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_critique_sessions_updated ON critique_sessions(updated_at DESC);

            CREATE TABLE IF NOT EXISTS critique_arguments (
                session_id     TEXT NOT NULL,
                role           TEXT NOT NULL,
                claim          TEXT NOT NULL,
                weight         REAL NOT NULL,
                evidence_json  TEXT NOT NULL,
                created_at     INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_critique_arguments_session_created ON critique_arguments(session_id, created_at ASC);

            CREATE TABLE IF NOT EXISTS critique_resolutions (
                session_id       TEXT PRIMARY KEY,
                decision         TEXT NOT NULL,
                resolution_json  TEXT NOT NULL,
                risk_score       REAL NOT NULL,
                confidence       REAL NOT NULL,
                resolved_at      INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_critique_resolutions_resolved ON critique_resolutions(resolved_at DESC);

            CREATE TABLE IF NOT EXISTS gateway_threads (
                channel_key TEXT PRIMARY KEY,
                thread_id   TEXT NOT NULL,
                updated_at  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_gateway_threads_updated ON gateway_threads(updated_at DESC);

            CREATE TABLE IF NOT EXISTS gateway_channel_modes (
                channel_key TEXT PRIMARY KEY,
                route_mode  TEXT NOT NULL,
                updated_at  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_gateway_channel_modes_updated ON gateway_channel_modes(updated_at DESC);

            CREATE TABLE IF NOT EXISTS gateway_replay_cursors (
                platform    TEXT NOT NULL,
                channel_id  TEXT NOT NULL,
                cursor_value TEXT NOT NULL,
                cursor_type TEXT NOT NULL,
                updated_at  INTEGER NOT NULL,
                PRIMARY KEY (platform, channel_id)
            );
            CREATE INDEX IF NOT EXISTS idx_gateway_replay_cursors_platform ON gateway_replay_cursors(platform, updated_at DESC);

            CREATE TABLE IF NOT EXISTS gateway_health_snapshots (
                platform    TEXT PRIMARY KEY,
                state_json  TEXT NOT NULL,
                updated_at  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_gateway_health_snapshots_updated ON gateway_health_snapshots(updated_at DESC);

            CREATE TABLE IF NOT EXISTS whatsapp_provider_state (
                provider_id    TEXT PRIMARY KEY,
                linked_phone   TEXT,
                auth_json      TEXT,
                metadata_json  TEXT,
                last_reset_at  INTEGER,
                last_linked_at INTEGER,
                updated_at     INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_whatsapp_provider_state_updated ON whatsapp_provider_state(updated_at DESC);

            CREATE TABLE IF NOT EXISTS operator_profile_sessions (
                session_id   TEXT PRIMARY KEY,
                kind         TEXT NOT NULL,
                session_json TEXT NOT NULL,
                updated_at   INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_operator_profile_sessions_updated ON operator_profile_sessions(updated_at DESC);

            CREATE TABLE IF NOT EXISTS agent_config_items (
                key_path   TEXT PRIMARY KEY,
                value_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_agent_config_items_updated ON agent_config_items(updated_at DESC);

            CREATE TABLE IF NOT EXISTS agent_config_updates (
                id         TEXT PRIMARY KEY,
                key_path   TEXT NOT NULL,
                value_json TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_agent_config_updates_key_ts ON agent_config_updates(key_path, updated_at DESC);

            CREATE TABLE IF NOT EXISTS provider_auth_state (
                provider_id TEXT NOT NULL,
                auth_mode   TEXT NOT NULL,
                state_json  TEXT NOT NULL,
                updated_at  INTEGER NOT NULL,
                PRIMARY KEY (provider_id, auth_mode)
            );
            CREATE INDEX IF NOT EXISTS idx_provider_auth_state_updated ON provider_auth_state(updated_at DESC);

            CREATE TABLE IF NOT EXISTS heartbeat_history (
                id              TEXT PRIMARY KEY,
                cycle_timestamp INTEGER NOT NULL,
                checks_json     TEXT NOT NULL,
                synthesis_json  TEXT,
                actionable      INTEGER NOT NULL DEFAULT 0,
                digest_text     TEXT,
                llm_tokens_used INTEGER NOT NULL DEFAULT 0,
                duration_ms     INTEGER NOT NULL DEFAULT 0,
                status          TEXT NOT NULL DEFAULT 'completed'
            );
            CREATE INDEX IF NOT EXISTS idx_heartbeat_history_ts ON heartbeat_history(cycle_timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_heartbeat_history_actionable ON heartbeat_history(actionable, cycle_timestamp DESC);

            CREATE TABLE IF NOT EXISTS meta_cognition_model (
                id                  INTEGER PRIMARY KEY CHECK (id = 1),
                agent_id            TEXT NOT NULL,
                calibration_offset  REAL NOT NULL DEFAULT 0.0,
                last_updated_at     INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS cognitive_biases (
                id                    INTEGER PRIMARY KEY AUTOINCREMENT,
                model_id              INTEGER NOT NULL,
                name                  TEXT NOT NULL,
                trigger_pattern_json  TEXT NOT NULL,
                mitigation_prompt     TEXT NOT NULL,
                severity              REAL NOT NULL,
                occurrence_count      INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (model_id) REFERENCES meta_cognition_model(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_cognitive_biases_model ON cognitive_biases(model_id, severity DESC);

            CREATE TABLE IF NOT EXISTS workflow_profiles (
                id                 INTEGER PRIMARY KEY AUTOINCREMENT,
                model_id           INTEGER NOT NULL,
                name               TEXT NOT NULL,
                avg_success_rate   REAL NOT NULL,
                avg_steps          INTEGER NOT NULL,
                typical_tools_json TEXT NOT NULL,
                FOREIGN KEY (model_id) REFERENCES meta_cognition_model(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_workflow_profiles_model ON workflow_profiles(model_id, avg_success_rate DESC);

            CREATE TABLE IF NOT EXISTS action_audit (
                id                TEXT PRIMARY KEY,
                timestamp         INTEGER NOT NULL,
                action_type       TEXT NOT NULL,
                summary           TEXT NOT NULL,
                explanation       TEXT,
                confidence        REAL,
                confidence_band   TEXT,
                causal_trace_id   TEXT,
                thread_id         TEXT,
                goal_run_id       TEXT,
                task_id           TEXT,
                raw_data_json     TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_action_audit_ts ON action_audit(timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_action_audit_type_ts ON action_audit(action_type, timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_action_audit_thread ON action_audit(thread_id, timestamp DESC);

            CREATE TABLE IF NOT EXISTS memory_tombstones (
                id TEXT PRIMARY KEY,
                target TEXT NOT NULL,
                original_content TEXT NOT NULL,
                fact_key TEXT,
                replaced_by TEXT,
                replaced_at INTEGER NOT NULL,
                source_kind TEXT NOT NULL,
                provenance_id TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tombstones_created ON memory_tombstones(created_at);
            CREATE INDEX IF NOT EXISTS idx_tombstones_target ON memory_tombstones(target, created_at DESC);

            CREATE TABLE IF NOT EXISTS consolidation_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS offloaded_payloads (
                payload_id    TEXT PRIMARY KEY,
                thread_id     TEXT NOT NULL,
                tool_name     TEXT NOT NULL,
                tool_call_id  TEXT,
                storage_path  TEXT NOT NULL,
                content_type  TEXT NOT NULL,
                byte_size     INTEGER NOT NULL,
                summary       TEXT NOT NULL,
                created_at    INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_offloaded_payloads_thread_created ON offloaded_payloads(thread_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS thread_structural_memory (
                thread_id   TEXT PRIMARY KEY,
                state_json  TEXT NOT NULL,
                updated_at  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_thread_structural_memory_updated ON thread_structural_memory(updated_at DESC);

            CREATE TABLE IF NOT EXISTS implicit_signals (
                id                    TEXT PRIMARY KEY,
                session_id            TEXT NOT NULL,
                signal_type           TEXT NOT NULL,
                weight                REAL NOT NULL,
                timestamp_ms          INTEGER NOT NULL,
                context_snapshot_json TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_implicit_signals_session_ts ON implicit_signals(session_id, timestamp_ms DESC);

            CREATE TABLE IF NOT EXISTS satisfaction_scores (
                id             TEXT PRIMARY KEY,
                session_id     TEXT NOT NULL,
                score          REAL NOT NULL,
                computed_at_ms INTEGER NOT NULL,
                label          TEXT NOT NULL,
                signal_count   INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_satisfaction_scores_session_ts ON satisfaction_scores(session_id, computed_at_ms DESC);

            CREATE TABLE IF NOT EXISTS intent_predictions (
                id                 TEXT PRIMARY KEY,
                session_id         TEXT NOT NULL,
                context_state_hash TEXT NOT NULL,
                predicted_action   TEXT NOT NULL,
                confidence         REAL NOT NULL,
                actual_action      TEXT,
                was_correct        INTEGER,
                created_at_ms      INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_intent_predictions_session_ts ON intent_predictions(session_id, created_at_ms DESC);

            CREATE TABLE IF NOT EXISTS system_outcome_predictions (
                id                TEXT PRIMARY KEY,
                session_id        TEXT NOT NULL,
                prediction_type   TEXT NOT NULL,
                predicted_outcome TEXT NOT NULL,
                confidence        REAL NOT NULL,
                actual_outcome    TEXT,
                was_correct       INTEGER,
                created_at_ms     INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_system_outcome_predictions_session_ts ON system_outcome_predictions(session_id, created_at_ms DESC);

            CREATE TABLE IF NOT EXISTS thread_protocol_candidates (
                thread_id   TEXT PRIMARY KEY,
                state_json  TEXT NOT NULL,
                updated_at  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_thread_protocol_candidates_updated ON thread_protocol_candidates(updated_at DESC);

            CREATE TABLE IF NOT EXISTS emergent_protocols (
                protocol_id              TEXT PRIMARY KEY,
                token                    TEXT NOT NULL UNIQUE,
                description              TEXT NOT NULL,
                agent_a                  TEXT NOT NULL,
                agent_b                  TEXT NOT NULL,
                thread_id                TEXT NOT NULL,
                normalized_pattern       TEXT NOT NULL,
                signal_kind              TEXT NOT NULL,
                context_signature_json   TEXT NOT NULL,
                created_at               INTEGER NOT NULL,
                activated_at             INTEGER NOT NULL,
                last_used_at             INTEGER,
                usage_count              INTEGER NOT NULL DEFAULT 0,
                success_rate             REAL NOT NULL DEFAULT 1.0,
                source_candidate_id      TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_emergent_protocols_thread_activated ON emergent_protocols(thread_id, activated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_emergent_protocols_pattern ON emergent_protocols(normalized_pattern, activated_at DESC);

            CREATE TABLE IF NOT EXISTS protocol_steps (
                protocol_id          TEXT NOT NULL,
                step_index           INTEGER NOT NULL,
                intent               TEXT NOT NULL,
                tool_name            TEXT,
                args_template_json   TEXT NOT NULL,
                PRIMARY KEY (protocol_id, step_index),
                FOREIGN KEY (protocol_id) REFERENCES emergent_protocols(protocol_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_protocol_steps_protocol ON protocol_steps(protocol_id, step_index ASC);

            CREATE TABLE IF NOT EXISTS protocol_usage_log (
                id                 TEXT PRIMARY KEY,
                protocol_id        TEXT NOT NULL,
                used_at            INTEGER NOT NULL,
                execution_time_ms  INTEGER,
                success            INTEGER NOT NULL,
                fallback_reason    TEXT,
                FOREIGN KEY (protocol_id) REFERENCES emergent_protocols(protocol_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_protocol_usage_log_protocol_used ON protocol_usage_log(protocol_id, used_at DESC);

            CREATE TABLE IF NOT EXISTS plugins (
                name            TEXT PRIMARY KEY,
                version         TEXT NOT NULL,
                description     TEXT,
                author          TEXT,
                manifest_json   TEXT NOT NULL,
                install_source  TEXT NOT NULL DEFAULT 'local',
                enabled         INTEGER NOT NULL DEFAULT 1,
                installed_at    TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS plugin_settings (
                plugin_name     TEXT NOT NULL,
                key             TEXT NOT NULL,
                value           TEXT NOT NULL,
                is_secret       INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (plugin_name, key),
                FOREIGN KEY (plugin_name) REFERENCES plugins(name) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS plugin_credentials (
                plugin_name      TEXT NOT NULL,
                credential_type  TEXT NOT NULL,
                encrypted_value  BLOB,
                expires_at       TEXT,
                created_at       TEXT NOT NULL,
                updated_at       TEXT NOT NULL,
                PRIMARY KEY (plugin_name, credential_type),
                FOREIGN KEY (plugin_name) REFERENCES plugins(name) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS operator_profile_fields (
                field_key        TEXT PRIMARY KEY,
                field_value_json TEXT NOT NULL,
                confidence       REAL NOT NULL DEFAULT 0.0,
                source           TEXT NOT NULL DEFAULT 'unknown',
                updated_at       INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_op_profile_fields_updated ON operator_profile_fields(updated_at DESC);

            CREATE TABLE IF NOT EXISTS operator_profile_consents (
                consent_key TEXT PRIMARY KEY,
                granted     INTEGER NOT NULL DEFAULT 0,
                updated_at  INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS operator_profile_events (
                id            TEXT PRIMARY KEY,
                event_type    TEXT NOT NULL,
                field_key     TEXT,
                value_json    TEXT,
                source        TEXT NOT NULL DEFAULT 'unknown',
                metadata_json TEXT,
                created_at    INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_op_profile_events_created ON operator_profile_events(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_op_profile_events_field ON operator_profile_events(field_key, created_at DESC);

            CREATE TABLE IF NOT EXISTS memory_distillation_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_thread_id TEXT NOT NULL,
                source_message_range TEXT,
                source_message_span_json TEXT,
                distilled_fact TEXT NOT NULL,
                target_file TEXT NOT NULL,
                category TEXT NOT NULL,
                confidence REAL NOT NULL,
                created_at_ms INTEGER NOT NULL,
                applied_to_memory INTEGER DEFAULT 0,
                agent_id TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_distillation_log_thread ON memory_distillation_log(source_thread_id, created_at_ms DESC);
            CREATE INDEX IF NOT EXISTS idx_distillation_log_applied ON memory_distillation_log(applied_to_memory, created_at_ms DESC);

            CREATE TABLE IF NOT EXISTS memory_distillation_progress (
                source_thread_id TEXT PRIMARY KEY,
                last_processed_created_at_ms INTEGER NOT NULL,
                last_processed_message_id TEXT NOT NULL,
                last_processed_span_json TEXT,
                last_run_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                agent_id TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_distillation_progress_updated ON memory_distillation_progress(updated_at_ms DESC);

            CREATE TABLE IF NOT EXISTS forge_pass_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                period_start_ms INTEGER NOT NULL,
                period_end_ms INTEGER NOT NULL,
                traces_analyzed INTEGER NOT NULL,
                patterns_found INTEGER NOT NULL,
                hints_applied INTEGER NOT NULL,
                hints_logged INTEGER NOT NULL,
                completed_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_forge_pass_log_agent ON forge_pass_log(agent_id, completed_at_ms DESC);

            CREATE TABLE IF NOT EXISTS event_triggers (
                id                 TEXT PRIMARY KEY,
                event_family       TEXT NOT NULL,
                event_kind         TEXT NOT NULL,
                target_state       TEXT,
                thread_id          TEXT,
                enabled            INTEGER NOT NULL DEFAULT 1,
                cooldown_secs      INTEGER NOT NULL DEFAULT 0,
                risk_label         TEXT NOT NULL DEFAULT 'low',
                notification_kind  TEXT NOT NULL,
                title_template     TEXT NOT NULL,
                body_template      TEXT NOT NULL,
                created_at         INTEGER NOT NULL,
                updated_at         INTEGER NOT NULL,
                last_fired_at      INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_event_triggers_family_kind_enabled ON event_triggers(event_family, event_kind, enabled, updated_at DESC);

            CREATE TABLE IF NOT EXISTS operator_profile_checkins (
                id            TEXT PRIMARY KEY,
                kind          TEXT NOT NULL,
                scheduled_at  INTEGER NOT NULL,
                shown_at      INTEGER,
                status        TEXT NOT NULL DEFAULT 'pending',
                response_json TEXT,
                created_at    INTEGER NOT NULL,
                updated_at    INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_op_profile_checkins_scheduled ON operator_profile_checkins(scheduled_at ASC);
            CREATE INDEX IF NOT EXISTS idx_op_profile_checkins_status ON operator_profile_checkins(status, scheduled_at ASC);
    "#
}
