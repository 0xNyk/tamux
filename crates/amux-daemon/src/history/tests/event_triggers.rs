use super::*;
use crate::history::schema_helpers::table_has_column;

#[tokio::test]
async fn init_schema_adds_event_trigger_registry_table() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .conn
        .call(|conn| {
            conn.execute_batch(
                "
                DROP TABLE IF EXISTS event_triggers;
                ",
            )?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    store.init_schema().await?;

    let status = store
        .conn
        .call(|conn| {
            let has_event_family = table_has_column(conn, "event_triggers", "event_family")?;
            let has_event_kind = table_has_column(conn, "event_triggers", "event_kind")?;
            let has_target_state = table_has_column(conn, "event_triggers", "target_state")?;
            let has_cooldown_secs = table_has_column(conn, "event_triggers", "cooldown_secs")?;
            let has_risk_label = table_has_column(conn, "event_triggers", "risk_label")?;
            let has_notification_kind =
                table_has_column(conn, "event_triggers", "notification_kind")?;
            let trigger_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_event_triggers_family_kind_enabled'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            Ok((
                has_event_family,
                has_event_kind,
                has_target_state,
                has_cooldown_secs,
                has_risk_label,
                has_notification_kind,
                trigger_index,
            ))
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    assert!(status.0);
    assert!(status.1);
    assert!(status.2);
    assert!(status.3);
    assert!(status.4);
    assert!(status.5);
    assert_eq!(
        status.6.as_deref(),
        Some("idx_event_triggers_family_kind_enabled")
    );

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn event_trigger_registry_round_trips_weles_health_trigger() -> Result<()> {
    let (store, root) = make_test_store().await?;

    let row = EventTriggerRow {
        id: "trigger-weles-degraded".to_string(),
        event_family: "health".to_string(),
        event_kind: "weles_health".to_string(),
        target_state: Some("degraded".to_string()),
        thread_id: None,
        enabled: true,
        cooldown_secs: 900,
        risk_label: "medium".to_string(),
        notification_kind: "weles_health_degraded".to_string(),
        title_template: "WELES review degraded".to_string(),
        body_template: "WELES health changed to {state}: {reason}".to_string(),
        created_at: 100,
        updated_at: 120,
        last_fired_at: Some(150),
    };

    store.upsert_event_trigger(&row).await?;

    let rows = store
        .list_event_triggers(Some("health"), Some("weles_health"))
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "trigger-weles-degraded");
    assert_eq!(rows[0].target_state.as_deref(), Some("degraded"));
    assert!(rows[0].enabled);
    assert_eq!(rows[0].cooldown_secs, 900);
    assert_eq!(rows[0].risk_label, "medium");
    assert_eq!(rows[0].notification_kind, "weles_health_degraded");
    assert_eq!(rows[0].last_fired_at, Some(150));

    store
        .record_event_trigger_fired("trigger-weles-degraded", 222)
        .await?;
    let rows = store
        .list_event_triggers(Some("health"), Some("weles_health"))
        .await?;
    assert_eq!(rows[0].last_fired_at, Some(222));

    std::fs::remove_dir_all(root)?;
    Ok(())
}
