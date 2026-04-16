use super::*;

#[tokio::test]
async fn register_skill_document_infers_variant_metadata() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let skill_path = root.join("skills/generated/debug-rust-stack-overflow--async-runtime.md");
    fs::write(
        &skill_path,
        "# Rust async debugging\nUse tokio console for async stack inspection.\n",
    )?;

    let record = store.register_skill_document(&skill_path).await?;

    assert_eq!(record.skill_name, "debug-rust-stack-overflow");
    assert_eq!(record.variant_name, "async-runtime");
    assert!(record.context_tags.iter().any(|tag| tag == "rust"));
    assert!(record.context_tags.iter().any(|tag| tag == "async"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn register_skill_document_preserves_persisted_fitness_score_on_reregistration() -> Result<()>
{
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let skill_path = root.join("skills/generated/build-pipeline.md");
    fs::write(&skill_path, "# Build pipeline\nRun cargo build.\n")?;

    let record = store.register_skill_document(&skill_path).await?;
    let variant_id = record.variant_id.clone();
    store
        .conn
        .call(move |conn| {
            conn.execute(
                "UPDATE skill_variants SET success_count = 0, failure_count = 0, fitness_score = 7.5 WHERE variant_id = ?1",
                params![variant_id],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let reregistered = store.register_skill_document(&skill_path).await?;

    assert_eq!(reregistered.variant_id, record.variant_id);
    assert_eq!(reregistered.fitness_score, 7.5);

    let fetched = store
        .get_skill_variant(&record.variant_id)
        .await?
        .expect("variant should still exist after re-registration");
    assert_eq!(fetched.fitness_score, 7.5);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn register_skill_document_prefers_explicit_frontmatter_tags() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let skill_path = root.join("skills/generated/react-debugger.md");
    fs::write(
        &skill_path,
        "---\nname: react_debugger\ndescription: Fixes UI event issues.\ncontext_tags:\n  - frontend\n  - typescript\n---\n# React Debugger\nRun cargo test from a terminal when validating the workspace.\n",
    )?;

    let record = store.register_skill_document(&skill_path).await?;

    assert_eq!(
        record.context_tags,
        vec!["frontend".to_string(), "typescript".to_string()]
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn register_skill_document_reads_nested_tamux_context_tags() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let skill_path = root.join("skills/community/slack-bridge/SKILL.md");
    fs::create_dir_all(skill_path.parent().expect("skill directory"))?;
    fs::write(
        &skill_path,
        "---\nname: slack-bridge\ndescription: Route operator alerts.\ntamux:\n  context_tags:\n    - messaging\n    - desktop\n---\n# Slack Bridge\nThis doc mentions React only as an example integration target.\n",
    )?;

    let record = store.register_skill_document(&skill_path).await?;

    assert_eq!(
        record.context_tags,
        vec!["desktop".to_string(), "messaging".to_string()]
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn register_skill_document_prefers_explicit_frontmatter_name() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let skill_path = root.join("skills/development/superpowers/alias-dir/SKILL.md");
    fs::create_dir_all(skill_path.parent().expect("skill directory"))?;
    fs::write(
        &skill_path,
        "---\nname: subagent-driven-development\ndescription: Execute implementation work through subagents.\n---\n# Alias Dir\n",
    )?;

    let record = store.register_skill_document(&skill_path).await?;

    assert_eq!(record.skill_name, "subagent-driven-development");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn resolve_skill_variant_prefers_existing_document_over_stale_legacy_row() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let skill_path = root
        .join("skills")
        .join("development")
        .join("superpowers")
        .join("subagent-driven-development")
        .join("SKILL.md");
    fs::create_dir_all(skill_path.parent().expect("skill directory"))?;
    fs::write(
        &skill_path,
        "---\nname: subagent-driven-development\ndescription: Execute implementation work through subagents.\n---\n# Subagent-Driven Development\n",
    )?;

    let record = store.register_skill_document(&skill_path).await?;
    let stale_skill_name = record.skill_name.clone();
    let stale_variant_name = record.variant_name.clone();
    let stale_context_tags_json = serde_json::to_string(&record.context_tags)?;
    let stale_variant_id = "stale-builtin-row".to_string();

    store
        .conn
        .call(move |conn| {
            conn.execute(
                "INSERT INTO skill_variants \
                 (variant_id, skill_name, variant_name, relative_path, parent_variant_id, version, context_tags_json, use_count, success_count, failure_count, fitness_score, status, last_used_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, 99, 99, 0, 99.0, 'active', NULL, 10, 20)",
                params![
                    stale_variant_id,
                    stale_skill_name,
                    stale_variant_name,
                    "builtin/superpowers/subagent-driven-development/SKILL.md",
                    "v99.0",
                    stale_context_tags_json,
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let resolved = store
        .resolve_skill_variant("subagent-driven-development", &[])
        .await?
        .expect("variant should resolve");

    assert_eq!(
        resolved.relative_path, "development/superpowers/subagent-driven-development/SKILL.md",
        "existing on-disk skill should outrank stale legacy rows"
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn resolve_skill_variant_prefers_context_overlap_and_tracks_usage() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let canonical = root.join("skills/generated/build-pipeline.md");
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    fs::write(&canonical, "# Build pipeline\nRun cargo build.\n")?;
    fs::write(
        &frontend,
        "# Frontend build pipeline\nUse react build checks.\n",
    )?;

    let canonical_record = store.register_skill_document(&canonical).await?;
    let frontend_record = store.register_skill_document(&frontend).await?;
    let resolved = store
        .resolve_skill_variant("build-pipeline", &["frontend".to_string()])
        .await?
        .expect("variant should resolve");
    assert_eq!(resolved.variant_id, frontend_record.variant_id);

    store
        .record_skill_variant_use(&frontend_record.variant_id, Some(true))
        .await?;
    let refreshed = store
        .resolve_skill_variant("build-pipeline", &["frontend".to_string()])
        .await?
        .expect("variant should still resolve");
    assert_eq!(refreshed.use_count, 1);
    assert_eq!(refreshed.success_count, 1);
    assert_eq!(canonical_record.use_count, 0);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn resolve_skill_variant_prefers_improving_fitness_trend_when_snapshot_is_equal() -> Result<()>
{
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let declining = root.join("skills/generated/build-pipeline--declining.md");
    let improving = root.join("skills/generated/build-pipeline--improving.md");
    fs::write(&declining, "# Declining build pipeline\nRun cargo build.\n")?;
    fs::write(&improving, "# Improving build pipeline\nRun cargo build.\n")?;

    let declining_record = store.register_skill_document(&declining).await?;
    let improving_record = store.register_skill_document(&improving).await?;
    let tags = vec!["frontend".to_string()];

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-declining-1",
            variant_id: &declining_record.variant_id,
            thread_id: Some("thread-declining-1"),
            task_id: Some("task-declining-1"),
            goal_run_id: Some("goal-declining-1"),
            context_tags: &tags,
            consulted_at: 100,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-declining-1"),
            Some("task-declining-1"),
            Some("goal-declining-1"),
            "success",
        )
        .await?;
    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-declining-2",
            variant_id: &declining_record.variant_id,
            thread_id: Some("thread-declining-2"),
            task_id: Some("task-declining-2"),
            goal_run_id: Some("goal-declining-2"),
            context_tags: &tags,
            consulted_at: 101,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-declining-2"),
            Some("task-declining-2"),
            Some("goal-declining-2"),
            "failure",
        )
        .await?;

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-improving-1",
            variant_id: &improving_record.variant_id,
            thread_id: Some("thread-improving-1"),
            task_id: Some("task-improving-1"),
            goal_run_id: Some("goal-improving-1"),
            context_tags: &tags,
            consulted_at: 102,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-improving-1"),
            Some("task-improving-1"),
            Some("goal-improving-1"),
            "failure",
        )
        .await?;
    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-improving-2",
            variant_id: &improving_record.variant_id,
            thread_id: Some("thread-improving-2"),
            task_id: Some("task-improving-2"),
            goal_run_id: Some("goal-improving-2"),
            context_tags: &tags,
            consulted_at: 103,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-improving-2"),
            Some("task-improving-2"),
            Some("goal-improving-2"),
            "success",
        )
        .await?;

    let declining_id = declining_record.variant_id.clone();
    let improving_id = improving_record.variant_id.clone();
    store
        .conn
        .call(move |conn| {
            conn.execute(
                "UPDATE skill_variants SET updated_at = 500 WHERE variant_id = ?1",
                params![declining_id],
            )?;
            conn.execute(
                "UPDATE skill_variants SET updated_at = 500 WHERE variant_id = ?1",
                params![improving_id],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let resolved = store
        .resolve_skill_variant("build-pipeline", &[])
        .await?
        .expect("one variant should resolve");

    assert_eq!(declining_record.fitness_score, 0.0);
    assert_eq!(improving_record.fitness_score, 0.0);
    assert_eq!(resolved.variant_id, improving_record.variant_id);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn sequential_settlements_append_distinct_skill_fitness_history_records() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let skill = root.join("skills/generated/build-pipeline.md");
    fs::write(&skill, "# Build pipeline\nRun cargo build.\n")?;

    let record = store.register_skill_document(&skill).await?;
    let tags = vec!["frontend".to_string()];

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-history-1",
            variant_id: &record.variant_id,
            thread_id: Some("thread-history-1"),
            task_id: Some("task-history-1"),
            goal_run_id: Some("goal-history-1"),
            context_tags: &tags,
            consulted_at: 100,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-history-1"),
            Some("task-history-1"),
            Some("goal-history-1"),
            "success",
        )
        .await?;

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-history-2",
            variant_id: &record.variant_id,
            thread_id: Some("thread-history-2"),
            task_id: Some("task-history-2"),
            goal_run_id: Some("goal-history-2"),
            context_tags: &tags,
            consulted_at: 101,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-history-2"),
            Some("task-history-2"),
            Some("goal-history-2"),
            "failure",
        )
        .await?;

    let history = store
        .list_skill_variant_fitness_history(&record.variant_id, 10)
        .await?;

    assert_eq!(history.len(), 2);
    assert_eq!(history[0].outcome, "success");
    assert_eq!(history[0].fitness_score, 1.0);
    assert_eq!(history[1].outcome, "failure");
    assert_eq!(history[1].fitness_score, 0.0);
    assert_ne!(history[0].id, history[1].id);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn skill_variant_consultation_settlement_updates_outcomes_once() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    fs::write(
        &frontend,
        "# Frontend build pipeline\nUse react build checks.\n",
    )?;

    let frontend_record = store.register_skill_document(&frontend).await?;
    let tags = vec!["frontend".to_string()];
    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-1",
            variant_id: &frontend_record.variant_id,
            thread_id: Some("thread-1"),
            task_id: Some("task-1"),
            goal_run_id: Some("goal-1"),
            context_tags: &tags,
            consulted_at: 100,
        })
        .await?;

    let pending = store
        .settle_skill_variant_usage(Some("thread-1"), Some("task-1"), Some("goal-1"), "success")
        .await?;
    assert_eq!(pending.0, 1);
    assert_eq!(pending.1, vec!["build-pipeline".to_string()]);
    assert_eq!(
        store
            .settle_skill_variant_usage(
                Some("thread-1"),
                Some("task-1"),
                Some("goal-1"),
                "success",
            )
            .await?
            .0,
        0
    );

    let refreshed = store
        .resolve_skill_variant("build-pipeline", &["frontend".to_string()])
        .await?
        .expect("variant should resolve");
    assert_eq!(refreshed.use_count, 1);
    assert_eq!(refreshed.success_count, 1);
    assert_eq!(refreshed.failure_count, 0);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn successful_skill_variant_settlement_persists_fitness_score_for_next_inspection(
) -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    fs::write(
        &frontend,
        "# Frontend build pipeline\nUse react build checks.\n",
    )?;

    let frontend_record = store.register_skill_document(&frontend).await?;
    let tags = vec!["frontend".to_string()];
    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-fitness-persist-1",
            variant_id: &frontend_record.variant_id,
            thread_id: Some("thread-1"),
            task_id: Some("task-1"),
            goal_run_id: Some("goal-1"),
            context_tags: &tags,
            consulted_at: 100,
        })
        .await?;

    let settled = store
        .settle_skill_variant_usage(Some("thread-1"), Some("task-1"), Some("goal-1"), "success")
        .await?;
    assert_eq!(settled.0, 1);

    let inspection = store
        .inspect_skill_variants("build-pipeline", &["frontend".to_string()])
        .await?;
    let item = inspection
        .into_iter()
        .find(|item| item.record.variant_id == frontend_record.variant_id)
        .expect("settled variant should be inspectable");

    assert_eq!(item.record.fitness_score, 1.0);
    assert_eq!(item.fitness_score, 1.0);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn cancelled_skill_variant_consultation_does_not_increment_failure_count() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let skill = root.join("skills/generated/build-pipeline.md");
    fs::write(&skill, "# Build pipeline\nRun cargo build.\n")?;

    let record = store.register_skill_document(&skill).await?;
    let tags = vec!["frontend".to_string()];
    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-cancelled-1",
            variant_id: &record.variant_id,
            thread_id: Some("thread-1"),
            task_id: Some("task-1"),
            goal_run_id: Some("goal-1"),
            context_tags: &tags,
            consulted_at: 100,
        })
        .await?;

    let settled = store
        .settle_skill_variant_usage(
            Some("thread-1"),
            Some("task-1"),
            Some("goal-1"),
            "cancelled",
        )
        .await?;
    assert_eq!(settled.0, 1);
    assert_eq!(settled.1, vec!["build-pipeline".to_string()]);

    let refreshed = store
        .resolve_skill_variant("build-pipeline", &["frontend".to_string()])
        .await?
        .expect("variant should resolve");
    assert_eq!(refreshed.use_count, 0);
    assert_eq!(refreshed.success_count, 0);
    assert_eq!(
        refreshed.failure_count, 0,
        "cancelled consultations should resolve usage without counting as failures"
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn rebalance_skill_variants_archives_weak_variant() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let canonical = root.join("skills/generated/build-pipeline.md");
    let weak = root.join("skills/generated/build-pipeline--legacy.md");
    fs::write(&canonical, "# Build pipeline\nRun cargo build.\n")?;
    fs::write(&weak, "# Legacy build pipeline\nOld slow workflow.\n")?;

    let canonical_record = store.register_skill_document(&canonical).await?;
    let weak_record = store.register_skill_document(&weak).await?;
    let wv = weak_record.variant_id.clone();
    let cv = canonical_record.variant_id.clone();
    store.conn.call(move |conn| {
        conn.execute(
            "UPDATE skill_variants SET use_count = 4, success_count = 0, failure_count = 4, last_used_at = ?2 WHERE variant_id = ?1",
            params![wv, now_ts() as i64],
        )?;
        conn.execute(
            "UPDATE skill_variants SET use_count = 4, success_count = 3, failure_count = 1, last_used_at = ?2 WHERE variant_id = ?1",
            params![cv, now_ts() as i64],
        )?;
        Ok(())
    }).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let variants = store.rebalance_skill_variants("build-pipeline").await?;
    let weak_variant = variants
        .iter()
        .find(|variant| variant.variant_id == weak_record.variant_id)
        .expect("weak variant should exist");
    let resolved = store
        .resolve_skill_variant("build-pipeline", &["legacy".to_string()])
        .await?
        .expect("canonical should still resolve");

    assert_eq!(weak_variant.status, "archived");
    assert_eq!(resolved.variant_id, canonical_record.variant_id);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn rebalance_skill_variants_promotes_strong_variant() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let canonical = root.join("skills/generated/build-pipeline.md");
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    fs::write(&canonical, "# Build pipeline\nRun cargo build.\n")?;
    fs::write(
        &frontend,
        "# Frontend build pipeline\nUse react build checks.\n",
    )?;

    let canonical_record = store.register_skill_document(&canonical).await?;
    let frontend_record = store.register_skill_document(&frontend).await?;
    let cv = canonical_record.variant_id.clone();
    let fv = frontend_record.variant_id.clone();
    store.conn.call(move |conn| {
        conn.execute(
            "UPDATE skill_variants SET use_count = 5, success_count = 2, failure_count = 3, last_used_at = ?2 WHERE variant_id = ?1",
            params![cv, now_ts() as i64],
        )?;
        conn.execute(
            "UPDATE skill_variants SET use_count = 5, success_count = 5, failure_count = 0, last_used_at = ?2 WHERE variant_id = ?1",
            params![fv, now_ts() as i64],
        )?;
        Ok(())
    }).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let variants = store.rebalance_skill_variants("build-pipeline").await?;
    let promoted = variants
        .iter()
        .find(|variant| variant.variant_id == frontend_record.variant_id)
        .expect("frontend variant should exist");
    let canonical_variant = variants
        .iter()
        .find(|variant| variant.variant_id == canonical_record.variant_id)
        .expect("canonical variant should exist");
    let resolved = store
        .resolve_skill_variant("build-pipeline", &[])
        .await?
        .expect("promoted variant should resolve");

    assert_eq!(promoted.status, "promoted-to-canonical");
    assert_eq!(canonical_variant.status, "deprecated");
    assert_eq!(resolved.variant_id, frontend_record.variant_id);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn successful_context_mismatch_branches_new_variant() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let canonical = root.join("skills/generated/build-pipeline.md");
    fs::write(&canonical, "# Build pipeline\nRun cargo build.\n")?;

    let canonical_record = store.register_skill_document(&canonical).await?;
    let branch_tags = vec![
        "rust".to_string(),
        "frontend".to_string(),
        "database".to_string(),
    ];
    for index in 0..3 {
        let usage_id = format!("usage-{index}");
        let task_id = format!("task-{index}");
        store
            .record_skill_variant_consultation(&SkillVariantConsultationRecord {
                usage_id: &usage_id,
                variant_id: &canonical_record.variant_id,
                thread_id: Some("thread-1"),
                task_id: Some(&task_id),
                goal_run_id: Some("goal-1"),
                context_tags: &branch_tags,
                consulted_at: 100 + index,
            })
            .await?;
        store
            .settle_skill_variant_usage(Some("thread-1"), Some(&task_id), Some("goal-1"), "success")
            .await?;
    }

    let variants = store
        .list_skill_variants(Some("build-pipeline"), 10)
        .await?;
    let branched = variants
        .iter()
        .find(|variant| {
            variant.variant_name.contains("database") && variant.variant_name.contains("frontend")
        })
        .expect("branched variant should exist");
    let branched_path = root.join("skills").join(&branched.relative_path);

    assert!(branched_path.exists());
    assert!(branched.context_tags.iter().any(|tag| tag == "frontend"));
    assert!(branched.context_tags.iter().any(|tag| tag == "database"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn cross_breed_skill_variants_creates_candidate_offspring_variant() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    let rust = root.join("skills/generated/build-pipeline--rust.md");
    fs::write(
        &frontend,
        "# Build pipeline (frontend)\n\n## When To Use\nUse this variant for frontend build pipelines.\n\n## How\nRun react build checks first.\n",
    )?;
    fs::write(
        &rust,
        "# Build pipeline (rust)\n\n## When To Use\nUse this variant for rust build pipelines.\n\n## How\nRun cargo build --workspace.\n",
    )?;

    let frontend_record = store.register_skill_document(&frontend).await?;
    let rust_record = store.register_skill_document(&rust).await?;
    let frontend_tags = vec!["frontend".to_string()];
    let rust_tags = vec!["rust".to_string()];

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-cross-breed-frontend",
            variant_id: &frontend_record.variant_id,
            thread_id: Some("thread-cross-breed-frontend"),
            task_id: Some("task-cross-breed-frontend"),
            goal_run_id: Some("goal-cross-breed-frontend"),
            context_tags: &frontend_tags,
            consulted_at: 100,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-cross-breed-frontend"),
            Some("task-cross-breed-frontend"),
            Some("goal-cross-breed-frontend"),
            "success",
        )
        .await?;

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-cross-breed-rust",
            variant_id: &rust_record.variant_id,
            thread_id: Some("thread-cross-breed-rust"),
            task_id: Some("task-cross-breed-rust"),
            goal_run_id: Some("goal-cross-breed-rust"),
            context_tags: &rust_tags,
            consulted_at: 101,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-cross-breed-rust"),
            Some("task-cross-breed-rust"),
            Some("goal-cross-breed-rust"),
            "success",
        )
        .await?;

    let offspring = store
        .cross_breed_skill_variants(&frontend_record, &rust_record)
        .await?
        .expect("cross-breeding should create a candidate offspring variant");

    assert_eq!(offspring.skill_name, "build-pipeline");
    assert_eq!(offspring.status, "draft");
    assert_ne!(offspring.variant_id, frontend_record.variant_id);
    assert_ne!(offspring.variant_id, rust_record.variant_id);
    assert!(offspring.context_tags.iter().any(|tag| tag == "frontend"));
    assert!(offspring.context_tags.iter().any(|tag| tag == "rust"));
    assert!(root.join("skills").join(&offspring.relative_path).exists());

    let fetched = store
        .get_skill_variant(&offspring.variant_id)
        .await?
        .expect("offspring should be inserted into the variant table");
    assert_eq!(fetched.status, "draft");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn cross_breed_skill_variants_reuses_existing_offspring_for_same_parent_pair() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    let rust = root.join("skills/generated/build-pipeline--rust.md");
    fs::write(
        &frontend,
        "# Build pipeline (frontend)\n\n## When To Use\nUse this variant for frontend build pipelines.\n\n## How\nRun react build checks first.\n",
    )?;
    fs::write(
        &rust,
        "# Build pipeline (rust)\n\n## When To Use\nUse this variant for rust build pipelines.\n\n## How\nRun cargo build --workspace.\n",
    )?;

    let frontend_record = store.register_skill_document(&frontend).await?;
    let rust_record = store.register_skill_document(&rust).await?;

    let first = store
        .cross_breed_skill_variants(&frontend_record, &rust_record)
        .await?
        .expect("first cross-breed should create a candidate offspring");
    let second = store
        .cross_breed_skill_variants(&rust_record, &frontend_record)
        .await?
        .expect("second cross-breed should reuse the same candidate offspring");

    assert_eq!(first.variant_id, second.variant_id);
    assert_eq!(first.relative_path, second.relative_path);

    let variants = store
        .list_skill_variants(Some("build-pipeline"), 20)
        .await?;
    let offspring = variants
        .into_iter()
        .filter(|variant| {
            variant.status == "draft"
                && variant.variant_id != frontend_record.variant_id
                && variant.variant_id != rust_record.variant_id
        })
        .collect::<Vec<_>>();
    assert_eq!(offspring.len(), 1);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn gene_pool_registry_returns_offspring_record_for_parent_pair() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    let rust = root.join("skills/generated/build-pipeline--rust.md");
    fs::write(
        &frontend,
        "# Build pipeline (frontend)\n\n## When To Use\nUse this variant for frontend build pipelines.\n\n## How\nRun react build checks first.\n",
    )?;
    fs::write(
        &rust,
        "# Build pipeline (rust)\n\n## When To Use\nUse this variant for rust build pipelines.\n\n## How\nRun cargo build --workspace.\n",
    )?;

    let frontend_record = store.register_skill_document(&frontend).await?;
    let rust_record = store.register_skill_document(&rust).await?;
    let offspring = store
        .cross_breed_skill_variants(&frontend_record, &rust_record)
        .await?
        .expect("cross-breeding should create a candidate offspring variant");

    let registry = store
        .get_gene_pool_entry(&frontend_record.variant_id, &rust_record.variant_id)
        .await?
        .expect("gene pool should return a registry row for the parent pair");

    let expected_parent_a = frontend_record
        .variant_id
        .clone()
        .min(rust_record.variant_id.clone());
    let expected_parent_b = frontend_record
        .variant_id
        .clone()
        .max(rust_record.variant_id.clone());
    assert_eq!(registry.offspring_id, offspring.variant_id);
    assert_eq!(registry.lifecycle_state, "draft");
    assert_eq!(registry.parent_a, expected_parent_a);
    assert_eq!(registry.parent_b, expected_parent_b);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn promote_skill_variant_marks_offspring_active_in_variant_and_gene_pool() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    let rust = root.join("skills/generated/build-pipeline--rust.md");
    fs::write(
        &frontend,
        "# Build pipeline (frontend)\n\n## When To Use\nUse this variant for frontend build pipelines.\n\n## How\nRun react build checks first.\n",
    )?;
    fs::write(
        &rust,
        "# Build pipeline (rust)\n\n## When To Use\nUse this variant for rust build pipelines.\n\n## How\nRun cargo build --workspace.\n",
    )?;

    let frontend_record = store.register_skill_document(&frontend).await?;
    let rust_record = store.register_skill_document(&rust).await?;
    let offspring = store
        .cross_breed_skill_variants(&frontend_record, &rust_record)
        .await?
        .expect("cross-breeding should create a candidate offspring variant");

    store.promote_skill_variant(&offspring.variant_id).await?;

    let promoted = store
        .get_skill_variant(&offspring.variant_id)
        .await?
        .expect("promoted offspring should still exist");
    assert_eq!(promoted.status, "active");

    let registry = store
        .get_gene_pool_entry(&frontend_record.variant_id, &rust_record.variant_id)
        .await?
        .expect("gene pool row should exist for the offspring");
    assert_eq!(registry.offspring_id, offspring.variant_id);
    assert_eq!(registry.lifecycle_state, "active");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn retire_skill_variant_marks_offspring_archived_in_variant_and_gene_pool() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    let rust = root.join("skills/generated/build-pipeline--rust.md");
    fs::write(
        &frontend,
        "# Build pipeline (frontend)\n\n## When To Use\nUse this variant for frontend build pipelines.\n\n## How\nRun react build checks first.\n",
    )?;
    fs::write(
        &rust,
        "# Build pipeline (rust)\n\n## When To Use\nUse this variant for rust build pipelines.\n\n## How\nRun cargo build --workspace.\n",
    )?;

    let frontend_record = store.register_skill_document(&frontend).await?;
    let rust_record = store.register_skill_document(&rust).await?;
    let offspring = store
        .cross_breed_skill_variants(&frontend_record, &rust_record)
        .await?
        .expect("cross-breeding should create a candidate offspring variant");

    store.retire_skill_variant(&offspring.variant_id).await?;

    let retired = store
        .get_skill_variant(&offspring.variant_id)
        .await?
        .expect("retired offspring should still exist");
    assert_eq!(retired.status, "archived");

    let registry = store
        .get_gene_pool_entry(&frontend_record.variant_id, &rust_record.variant_id)
        .await?
        .expect("gene pool row should exist for the offspring");
    assert_eq!(registry.offspring_id, offspring.variant_id);
    assert_eq!(registry.lifecycle_state, "archived");

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn stable_variant_merges_back_into_canonical() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let canonical = root.join("skills/generated/build-pipeline.md");
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    fs::write(
        &canonical,
        "# Build pipeline\n\n## When To Use\nUse this for standard builds.\n\n## How\nRun cargo build.\n",
    )?;
    fs::write(
        &frontend,
        "# Build pipeline (frontend)\n\n> Auto-branched from `generated/build-pipeline.md` (variant `canonical`) after 4 successful consultations in contexts: frontend, rust.\n\n## When To Use\nUse this variant when the workspace context includes: frontend, rust.\n\n## How\nRun cargo build.\n",
    )?;

    let canonical_record = store.register_skill_document(&canonical).await?;
    let frontend_record = store.register_skill_document(&frontend).await?;
    let fv = frontend_record.variant_id.clone();
    let cv = canonical_record.variant_id.clone();
    store.conn.call(move |conn| {
        conn.execute(
            "UPDATE skill_variants SET use_count = 5, success_count = 5, failure_count = 0, parent_variant_id = ?2, last_used_at = ?3 WHERE variant_id = ?1",
            params![fv, cv, now_ts() as i64],
        )?;
        Ok(())
    }).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let variants = store.maybe_merge_skill_variants("build-pipeline").await?;
    let merged = variants
        .iter()
        .find(|variant| variant.variant_id == frontend_record.variant_id)
        .expect("frontend variant should still exist after merge");
    let canonical_content = fs::read_to_string(&canonical)?;
    let resolved = store
        .resolve_skill_variant("build-pipeline", &["frontend".to_string()])
        .await?
        .expect("canonical should resolve once branch is merged");

    assert_eq!(merged.status, "merged");
    assert!(canonical_content.contains("## Learned Variant Contexts"));
    assert!(canonical_content.contains("frontend"));
    assert_eq!(resolved.variant_id, canonical_record.variant_id);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn paged_skill_variant_listing_advances_with_cursor() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let first = root.join("skills/generated/build-pipeline.md");
    let second = root.join("skills/generated/debug-rust-build.md");
    fs::write(&first, "# Build pipeline\nRun cargo build.\n")?;
    fs::write(&second, "# Debug rust build\nRun cargo test.\n")?;

    let first_record = store.register_skill_document(&first).await?;
    let second_record = store.register_skill_document(&second).await?;

    let page_one = store.list_skill_variants_page(None, None, 1).await?;
    assert_eq!(page_one.variants.len(), 1);
    assert!(page_one.next_cursor.is_some());

    let page_two = store
        .list_skill_variants_page(None, page_one.next_cursor.as_deref(), 1)
        .await?;
    assert_eq!(page_two.variants.len(), 1);
    assert_ne!(
        page_one.variants[0].variant_id,
        page_two.variants[0].variant_id
    );
    assert!([first_record.variant_id, second_record.variant_id]
        .contains(&page_two.variants[0].variant_id));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn inspect_skill_variants_returns_ordered_fitness_history_for_variant() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let skill = root.join("skills/generated/build-pipeline.md");
    fs::write(&skill, "# Build pipeline\nRun cargo build.\n")?;

    let record = store.register_skill_document(&skill).await?;
    let empty_inspection = store.inspect_skill_variants("build-pipeline", &[]).await?;
    let empty_item = empty_inspection
        .iter()
        .find(|item| item.record.variant_id == record.variant_id)
        .expect("variant should be inspectable before settlements");
    assert!(empty_item.fitness_history.is_empty());

    let tags = vec!["frontend".to_string()];
    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-inspect-history-1",
            variant_id: &record.variant_id,
            thread_id: Some("thread-inspect-history-1"),
            task_id: Some("task-inspect-history-1"),
            goal_run_id: Some("goal-inspect-history-1"),
            context_tags: &tags,
            consulted_at: 100,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-inspect-history-1"),
            Some("task-inspect-history-1"),
            Some("goal-inspect-history-1"),
            "success",
        )
        .await?;

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-inspect-history-2",
            variant_id: &record.variant_id,
            thread_id: Some("thread-inspect-history-2"),
            task_id: Some("task-inspect-history-2"),
            goal_run_id: Some("goal-inspect-history-2"),
            context_tags: &tags,
            consulted_at: 101,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-inspect-history-2"),
            Some("task-inspect-history-2"),
            Some("goal-inspect-history-2"),
            "failure",
        )
        .await?;

    let inspection = store.inspect_skill_variants("build-pipeline", &[]).await?;
    let item = inspection
        .into_iter()
        .find(|item| item.record.variant_id == record.variant_id)
        .expect("variant should remain inspectable after settlements");

    assert_eq!(item.fitness_history.len(), 2);
    assert_eq!(item.fitness_history[0].outcome, "success");
    assert_eq!(item.fitness_history[0].fitness_score, 1.0);
    assert_eq!(item.fitness_history[1].outcome, "failure");
    assert_eq!(item.fitness_history[1].fitness_score, 0.0);
    assert!(item.fitness_history[0].recorded_at <= item.fitness_history[1].recorded_at);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn inspect_skill_variants_explains_archived_lifecycle() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let canonical = root.join("skills/generated/build-pipeline.md");
    let weak = root.join("skills/generated/build-pipeline--legacy.md");
    fs::write(&canonical, "# Build pipeline\nRun cargo build.\n")?;
    fs::write(&weak, "# Legacy build pipeline\nOld slow workflow.\n")?;

    let canonical_record = store.register_skill_document(&canonical).await?;
    let weak_record = store.register_skill_document(&weak).await?;
    let wv = weak_record.variant_id.clone();
    let cv = canonical_record.variant_id.clone();
    store.conn.call(move |conn| {
        conn.execute(
            "UPDATE skill_variants SET use_count = 4, success_count = 0, failure_count = 4, last_used_at = ?2 WHERE variant_id = ?1",
            params![wv, now_ts() as i64],
        )?;
        conn.execute(
            "UPDATE skill_variants SET use_count = 4, success_count = 3, failure_count = 1, last_used_at = ?2 WHERE variant_id = ?1",
            params![cv, now_ts() as i64],
        )?;
        Ok(())
    }).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    store.rebalance_skill_variants("build-pipeline").await?;

    let inspection = store
        .inspect_skill_variants("build-pipeline", &["legacy".to_string()])
        .await?;
    let weak_variant = inspection
        .iter()
        .find(|item| item.record.variant_id == weak_record.variant_id)
        .expect("weak variant should be inspectable");

    assert_eq!(weak_variant.record.status, "archived");
    assert!(
        weak_variant.lifecycle_summary.contains("underperformed"),
        "expected archived lifecycle summary to explain why it was retired: {}",
        weak_variant.lifecycle_summary
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn inspect_skill_variants_marks_context_best_match_as_selected() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let canonical = root.join("skills/generated/build-pipeline.md");
    let frontend = root.join("skills/generated/build-pipeline--frontend.md");
    fs::write(&canonical, "# Build pipeline\nRun cargo build.\n")?;
    fs::write(
        &frontend,
        "# Frontend build pipeline\nUse react build checks.\n",
    )?;

    store.register_skill_document(&canonical).await?;
    let frontend_record = store.register_skill_document(&frontend).await?;

    let inspection = store
        .inspect_skill_variants("build-pipeline", &["frontend".to_string()])
        .await?;
    let selected = inspection
        .iter()
        .find(|item| item.selected_for_context)
        .expect("one variant should be selected");

    assert_eq!(selected.record.variant_id, frontend_record.variant_id);
    assert!(
        selected
            .selection_summary
            .contains("matched context tags: frontend"),
        "expected selection summary to explain the context match: {}",
        selected.selection_summary
    );

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn inspect_skill_variants_reports_named_fitness_score() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let success = root.join("skills/generated/build-pipeline--success.md");
    let failure = root.join("skills/generated/build-pipeline--failure.md");
    let cancelled = root.join("skills/generated/build-pipeline--cancelled.md");
    fs::write(&success, "# Success build pipeline\nRun cargo build.\n")?;
    fs::write(&failure, "# Failure build pipeline\nRun cargo build.\n")?;
    fs::write(&cancelled, "# Cancelled build pipeline\nRun cargo build.\n")?;

    let success_record = store.register_skill_document(&success).await?;
    let failure_record = store.register_skill_document(&failure).await?;
    let cancelled_record = store.register_skill_document(&cancelled).await?;
    let tags = vec!["frontend".to_string()];

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-success-fitness",
            variant_id: &success_record.variant_id,
            thread_id: Some("thread-success"),
            task_id: Some("task-success"),
            goal_run_id: Some("goal-success"),
            context_tags: &tags,
            consulted_at: 100,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-success"),
            Some("task-success"),
            Some("goal-success"),
            "success",
        )
        .await?;

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-failure-fitness",
            variant_id: &failure_record.variant_id,
            thread_id: Some("thread-failure"),
            task_id: Some("task-failure"),
            goal_run_id: Some("goal-failure"),
            context_tags: &tags,
            consulted_at: 101,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-failure"),
            Some("task-failure"),
            Some("goal-failure"),
            "failure",
        )
        .await?;

    store
        .record_skill_variant_consultation(&SkillVariantConsultationRecord {
            usage_id: "usage-cancelled-fitness",
            variant_id: &cancelled_record.variant_id,
            thread_id: Some("thread-cancelled"),
            task_id: Some("task-cancelled"),
            goal_run_id: Some("goal-cancelled"),
            context_tags: &tags,
            consulted_at: 102,
        })
        .await?;
    store
        .settle_skill_variant_usage(
            Some("thread-cancelled"),
            Some("task-cancelled"),
            Some("goal-cancelled"),
            "cancelled",
        )
        .await?;

    let inspection = store
        .inspect_skill_variants("build-pipeline", &["frontend".to_string()])
        .await?;

    let success_item = inspection
        .iter()
        .find(|item| item.record.variant_id == success_record.variant_id)
        .expect("success variant should be inspectable");
    let failure_item = inspection
        .iter()
        .find(|item| item.record.variant_id == failure_record.variant_id)
        .expect("failure variant should be inspectable");
    let cancelled_item = inspection
        .iter()
        .find(|item| item.record.variant_id == cancelled_record.variant_id)
        .expect("cancelled variant should be inspectable");

    assert!(
        success_item.fitness_score > cancelled_item.fitness_score,
        "successful settlement should raise fitness above neutral"
    );
    assert!(
        cancelled_item.fitness_score > failure_item.fitness_score,
        "failed settlement should reduce fitness below neutral while cancelled stays neutral"
    );
    assert_eq!(cancelled_item.fitness_score, 0.0);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn inspect_skill_variants_reads_persisted_fitness_score_from_record() -> Result<()> {
    let (store, root) = make_test_store().await?;
    store.init_schema().await?;
    let canonical = root.join("skills/generated/build-pipeline.md");
    fs::write(&canonical, "# Build pipeline\nRun cargo build.\n")?;

    let record = store.register_skill_document(&canonical).await?;
    let variant_id = record.variant_id.clone();
    store.conn.call(move |conn| {
        conn.execute(
            "UPDATE skill_variants SET use_count = 4, success_count = 3, failure_count = 1, fitness_score = 7.5, updated_at = 4242 WHERE variant_id = ?1",
            params![variant_id],
        )?;
        Ok(())
    }).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let inspection = store.inspect_skill_variants("build-pipeline", &[]).await?;
    let item = inspection
        .into_iter()
        .find(|item| item.record.variant_id == record.variant_id)
        .expect("variant should be inspectable");

    assert_eq!(item.record.fitness_score, 7.5);
    assert_eq!(item.fitness_score, 7.5);
    assert_eq!(item.fitness_snapshot.fitness_score, 7.5);
    assert_eq!(item.fitness_snapshot.recorded_at, 4242);
    assert_eq!(item.fitness_snapshot.use_count, 4);
    assert!((item.fitness_snapshot.success_rate - 0.75).abs() < f64::EPSILON);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn skill_tag_excerpt_respects_utf8_boundaries() {
    let content = format!("{}\n{}", "a".repeat(3998), "│architecture");
    let excerpt = excerpt_on_char_boundary(&content, 4000);

    assert!(excerpt.is_char_boundary(excerpt.len()));
    assert!(excerpt.len() <= 4000);
    assert!(!excerpt.ends_with('\u{FFFD}'));

    let mut tags = BTreeSet::new();
    infer_skill_tags("skills/terminal-architecture.md", &content, &mut tags);
    assert!(tags.contains("terminal"));
}

#[test]
fn infer_skill_tags_ignores_incidental_body_mentions() {
    let content = "---\nname: debug-service\ndescription: Generic debugging workflow.\n---\n# Debug Service\nThis note references React frontend issues, a terminal transcript, cargo commands, and Slack alerts only as examples.\n";

    let mut tags = BTreeSet::new();
    infer_skill_tags("skills/generated/debug-service.md", &content, &mut tags);

    assert!(tags.is_empty());
}
