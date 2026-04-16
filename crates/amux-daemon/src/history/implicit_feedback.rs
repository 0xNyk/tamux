use super::*;

impl HistoryStore {
    pub async fn insert_implicit_signal(&self, row: &ImplicitSignalRow) -> Result<()> {
        let row = row.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO implicit_signals (id, session_id, signal_type, weight, timestamp_ms, context_snapshot_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        row.id,
                        row.session_id,
                        row.signal_type,
                        row.weight,
                        row.timestamp_ms as i64,
                        row.context_snapshot_json,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn list_implicit_signals(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<ImplicitSignalRow>> {
        let session_id = session_id.to_string();
        let limit = limit.max(1) as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, signal_type, weight, timestamp_ms, context_snapshot_json
                     FROM implicit_signals
                     WHERE session_id = ?1
                     ORDER BY timestamp_ms DESC, id DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![session_id, limit], |row| {
                    Ok(ImplicitSignalRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        signal_type: row.get(2)?,
                        weight: row.get(3)?,
                        timestamp_ms: row.get::<_, i64>(4)?.max(0) as u64,
                        context_snapshot_json: row.get(5)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Into::into)
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn insert_satisfaction_score(&self, row: &SatisfactionScoreRow) -> Result<()> {
        let row = row.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO satisfaction_scores (id, session_id, score, computed_at_ms, label, signal_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        row.id,
                        row.session_id,
                        row.score,
                        row.computed_at_ms as i64,
                        row.label,
                        row.signal_count as i64,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn insert_intent_prediction(&self, row: &IntentPredictionRow) -> Result<()> {
        let row = row.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO intent_predictions (id, session_id, context_state_hash, predicted_action, confidence, actual_action, was_correct, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        row.id,
                        row.session_id,
                        row.context_state_hash,
                        row.predicted_action,
                        row.confidence,
                        row.actual_action,
                        row.was_correct.map(|value| if value { 1i64 } else { 0i64 }),
                        row.created_at_ms as i64,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn insert_system_outcome_prediction(
        &self,
        row: &SystemOutcomePredictionRow,
    ) -> Result<()> {
        let row = row.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO system_outcome_predictions (id, session_id, prediction_type, predicted_outcome, confidence, actual_outcome, was_correct, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        row.id,
                        row.session_id,
                        row.prediction_type,
                        row.predicted_outcome,
                        row.confidence,
                        row.actual_outcome,
                        row.was_correct.map(|value| if value { 1i64 } else { 0i64 }),
                        row.created_at_ms as i64,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn list_intent_predictions(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<IntentPredictionRow>> {
        let session_id = session_id.to_string();
        let limit = limit.max(1) as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, context_state_hash, predicted_action, confidence, actual_action, was_correct, created_at_ms
                     FROM intent_predictions
                     WHERE session_id = ?1
                     ORDER BY created_at_ms DESC, id DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![session_id, limit], |row| {
                    Ok(IntentPredictionRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        context_state_hash: row.get(2)?,
                        predicted_action: row.get(3)?,
                        confidence: row.get(4)?,
                        actual_action: row.get(5)?,
                        was_correct: row
                            .get::<_, Option<i64>>(6)?
                            .map(|value| value != 0),
                        created_at_ms: row.get::<_, i64>(7)?.max(0) as u64,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Into::into)
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn list_system_outcome_predictions(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SystemOutcomePredictionRow>> {
        let session_id = session_id.to_string();
        let limit = limit.max(1) as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, prediction_type, predicted_outcome, confidence, actual_outcome, was_correct, created_at_ms
                     FROM system_outcome_predictions
                     WHERE session_id = ?1
                     ORDER BY created_at_ms DESC, id DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![session_id, limit], |row| {
                    Ok(SystemOutcomePredictionRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        prediction_type: row.get(2)?,
                        predicted_outcome: row.get(3)?,
                        confidence: row.get(4)?,
                        actual_outcome: row.get(5)?,
                        was_correct: row
                            .get::<_, Option<i64>>(6)?
                            .map(|value| value != 0),
                        created_at_ms: row.get::<_, i64>(7)?.max(0) as u64,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Into::into)
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn resolve_latest_intent_prediction(
        &self,
        session_id: &str,
        actual_action: &str,
        matched_predicted_action: Option<&str>,
    ) -> Result<()> {
        let session_id = session_id.to_string();
        let actual_action = actual_action.to_string();
        let matched_predicted_action = matched_predicted_action.map(str::to_string);
        self.conn
            .call(move |conn| {
                let latest: Option<String> = conn
                    .query_row(
                        "SELECT id FROM intent_predictions
                         WHERE session_id = ?1 AND was_correct IS NULL
                         ORDER BY created_at_ms DESC, id DESC
                         LIMIT 1",
                        params![session_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(id) = latest else {
                    return Ok(());
                };
                let was_correct = matched_predicted_action.is_some();
                conn.execute(
                    "UPDATE intent_predictions
                     SET actual_action = ?2, was_correct = ?3
                     WHERE id = ?1",
                    params![id, actual_action, if was_correct { 1i64 } else { 0i64 },],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn resolve_latest_system_outcome_prediction(
        &self,
        session_id: &str,
        actual_outcome: &str,
        matched_predicted_outcome: Option<&str>,
    ) -> Result<()> {
        let session_id = session_id.to_string();
        let actual_outcome = actual_outcome.to_string();
        let matched_predicted_outcome = matched_predicted_outcome.map(str::to_string);
        self.conn
            .call(move |conn| {
                let latest: Option<String> = conn
                    .query_row(
                        "SELECT id FROM system_outcome_predictions
                         WHERE session_id = ?1 AND was_correct IS NULL
                         ORDER BY created_at_ms DESC, id DESC
                         LIMIT 1",
                        params![session_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(id) = latest else {
                    return Ok(());
                };
                let was_correct = matched_predicted_outcome.is_some();
                conn.execute(
                    "UPDATE system_outcome_predictions
                     SET actual_outcome = ?2, was_correct = ?3
                     WHERE id = ?1",
                    params![id, actual_outcome, if was_correct { 1i64 } else { 0i64 },],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn list_satisfaction_scores(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SatisfactionScoreRow>> {
        let session_id = session_id.to_string();
        let limit = limit.max(1) as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, score, computed_at_ms, label, signal_count
                     FROM satisfaction_scores
                     WHERE session_id = ?1
                     ORDER BY computed_at_ms DESC, id DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![session_id, limit], |row| {
                    Ok(SatisfactionScoreRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        score: row.get(2)?,
                        computed_at_ms: row.get::<_, i64>(3)?.max(0) as u64,
                        label: row.get(4)?,
                        signal_count: row.get::<_, i64>(5)?.max(0) as u64,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Into::into)
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn list_recent_implicit_signal_samples(
        &self,
        limit: usize,
    ) -> Result<Vec<(f64, u64)>> {
        let limit = limit.max(1) as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT weight, timestamp_ms
                     FROM implicit_signals
                     ORDER BY timestamp_ms DESC, id DESC
                     LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit], |row| {
                    Ok((row.get::<_, f64>(0)?, row.get::<_, i64>(1)?.max(0) as u64))
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Into::into)
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub async fn recent_intent_prediction_success_rate(
        &self,
        predicted_action: &str,
        limit: usize,
    ) -> Result<Option<f64>> {
        let predicted_action = predicted_action.to_string();
        let limit = limit.max(1) as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT was_correct
                     FROM intent_predictions
                     WHERE predicted_action = ?1 AND was_correct IS NOT NULL
                     ORDER BY created_at_ms DESC, id DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![predicted_action, limit], |row| {
                    Ok(row.get::<_, i64>(0)? != 0)
                })?;
                let values = rows.collect::<std::result::Result<Vec<_>, _>>()?;
                if values.is_empty() {
                    return Ok(None);
                }
                let success = values.iter().filter(|value| **value).count() as f64;
                Ok(Some(success / values.len() as f64))
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}
