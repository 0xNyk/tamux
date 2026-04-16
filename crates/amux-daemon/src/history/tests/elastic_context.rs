use super::*;
use crate::history::schema_helpers::table_has_column;

#[tokio::test]
async fn init_schema_adds_elastic_context_tables_to_legacy_db() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .conn
        .call(|conn| {
            conn.execute_batch(
                "
                DROP TABLE IF EXISTS offloaded_payloads;
                DROP TABLE IF EXISTS thread_structural_memory;
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
            let has_offloaded_thread = table_has_column(conn, "offloaded_payloads", "thread_id")?;
            let has_offloaded_summary = table_has_column(conn, "offloaded_payloads", "summary")?;
            let summary_notnull: i64 = conn.query_row(
                "SELECT \"notnull\" FROM pragma_table_info('offloaded_payloads') WHERE name = 'summary'",
                [],
                |row| row.get(0),
            )?;
            let offloaded_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_offloaded_payloads_thread_created'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let has_structural_state =
                table_has_column(conn, "thread_structural_memory", "state_json")?;
            let has_structural_updated =
                table_has_column(conn, "thread_structural_memory", "updated_at")?;
            let has_memory_nodes_label = table_has_column(conn, "memory_nodes", "label")?;
            let has_memory_nodes_type = table_has_column(conn, "memory_nodes", "node_type")?;
            let has_memory_edges_relation =
                table_has_column(conn, "memory_edges", "relation_type")?;
            let has_memory_edges_weight = table_has_column(conn, "memory_edges", "weight")?;
            let structural_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_thread_structural_memory_updated'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let memory_node_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_memory_nodes_type_accessed'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let memory_edge_unique_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_memory_edges_unique'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            Ok((
                has_offloaded_thread,
                has_offloaded_summary,
                summary_notnull,
                offloaded_index,
                has_structural_state,
                has_structural_updated,
                structural_index,
                has_memory_nodes_label,
                has_memory_nodes_type,
                has_memory_edges_relation,
                has_memory_edges_weight,
                memory_node_index,
                memory_edge_unique_index,
            ))
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    assert!(status.0);
    assert!(status.1);
    assert_eq!(status.2, 1, "offloaded payload summary should be required");
    assert_eq!(
        status.3.as_deref(),
        Some("idx_offloaded_payloads_thread_created")
    );
    assert!(status.4);
    assert!(status.5);
    assert_eq!(
        status.6.as_deref(),
        Some("idx_thread_structural_memory_updated")
    );
    assert!(status.7);
    assert!(status.8);
    assert!(status.9);
    assert!(status.10);
    assert_eq!(status.11.as_deref(), Some("idx_memory_nodes_type_accessed"));
    assert_eq!(status.12.as_deref(), Some("idx_memory_edges_unique"));

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn init_schema_repairs_legacy_offloaded_payloads_upgrade_path() -> Result<()> {
    let (store, root) = make_test_store().await?;
    let expected_primary_path = root
        .join("offloaded-payloads")
        .join("thread-legacy")
        .join("payload-legacy.txt")
        .to_string_lossy()
        .into_owned();
    let expected_null_summary_path = root
        .join("offloaded-payloads")
        .join("thread-null-summary")
        .join("payload-null-summary.txt")
        .to_string_lossy()
        .into_owned();

    store
        .conn
        .call(|conn| {
            conn.execute_batch(
                "
                DROP TABLE IF EXISTS offloaded_payloads;
                CREATE TABLE offloaded_payloads (
                    payload_id TEXT PRIMARY KEY,
                    thread_id TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    tool_call_id TEXT,
                    storage_path TEXT NOT NULL,
                    content_type TEXT NOT NULL,
                    byte_size INTEGER NOT NULL,
                    summary TEXT,
                    created_at INTEGER NOT NULL
                );
                INSERT INTO offloaded_payloads (
                    payload_id,
                    thread_id,
                    tool_name,
                    tool_call_id,
                    storage_path,
                    content_type,
                    byte_size,
                    summary,
                    created_at
                ) VALUES (
                    'payload-legacy',
                    'thread-legacy',
                    'read_file',
                    'call-legacy',
                    '/tmp/legacy-payload.txt',
                    'text/plain',
                    512,
                    'Legacy summary survives migration',
                    1717170123
                );
                INSERT INTO offloaded_payloads (
                    payload_id,
                    thread_id,
                    tool_name,
                    tool_call_id,
                    storage_path,
                    content_type,
                    byte_size,
                    summary,
                    created_at
                ) VALUES (
                    'payload-null-summary',
                    'thread-null-summary',
                    'read_file',
                    NULL,
                    '/tmp/legacy-null-summary.txt',
                    'text/plain',
                    128,
                    NULL,
                    1717170456
                );
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
            let summary_notnull: i64 = conn.query_row(
                "SELECT \"notnull\" FROM pragma_table_info('offloaded_payloads') WHERE name = 'summary'",
                [],
                |row| row.get(0),
            )?;
            let offloaded_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_offloaded_payloads_thread_created'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let mut stmt = conn.prepare(
                "SELECT payload_id, thread_id, tool_name, tool_call_id, storage_path, content_type, byte_size, summary, created_at \
                 FROM offloaded_payloads ORDER BY payload_id ASC",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok((summary_notnull, offloaded_index, rows))
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    assert_eq!(
        status.0, 1,
        "offloaded payload summary should be required after migration"
    );
    assert_eq!(
        status.1.as_deref(),
        Some("idx_offloaded_payloads_thread_created")
    );
    assert_eq!(
        status.2.len(),
        2,
        "both legacy rows should survive migration"
    );

    let primary = &status.2[0];
    assert_eq!(primary.0, "payload-legacy");
    assert_eq!(primary.1, "thread-legacy");
    assert_eq!(primary.2, "read_file");
    assert_eq!(primary.3.as_deref(), Some("call-legacy"));
    assert_eq!(primary.4, expected_primary_path);
    assert_eq!(primary.5, "text/plain");
    assert_eq!(primary.6, 512);
    assert_eq!(primary.7, "Legacy summary survives migration");
    assert_eq!(primary.8, 1717170123);

    let null_summary = &status.2[1];
    assert_eq!(null_summary.0, "payload-null-summary");
    assert_eq!(null_summary.1, "thread-null-summary");
    assert_eq!(null_summary.2, "read_file");
    assert_eq!(null_summary.3, None);
    assert_eq!(null_summary.4, expected_null_summary_path);
    assert_eq!(null_summary.5, "text/plain");
    assert_eq!(null_summary.6, 128);
    assert_eq!(null_summary.7, "");
    assert_eq!(null_summary.8, 1717170456);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn offloaded_payload_metadata_round_trips() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .upsert_offloaded_payload_metadata(
            "payload-123",
            "thread-abc",
            "read_file",
            Some("call-456"),
            "text/plain",
            2_048,
            "Compacted tool output summary",
            1_717_170_000,
        )
        .await?;

    let record = store
        .get_offloaded_payload_metadata("payload-123")
        .await?
        .expect("payload metadata should be stored");

    assert_eq!(record.payload_id, "payload-123");
    assert_eq!(record.thread_id, "thread-abc");
    assert_eq!(record.tool_name, "read_file");
    assert_eq!(record.tool_call_id.as_deref(), Some("call-456"));
    assert_eq!(
        record.storage_path,
        root.join("offloaded-payloads")
            .join("thread-abc")
            .join("payload-123.txt")
            .to_string_lossy()
    );
    assert_eq!(record.content_type, "text/plain");
    assert_eq!(record.byte_size, 2_048);
    assert_eq!(record.summary, "Compacted tool output summary");
    assert_eq!(record.created_at, 1_717_170_000);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn thread_structural_memory_round_trips() -> Result<()> {
    let (store, root) = make_test_store().await?;

    let state_json = serde_json::json!({
        "summary": "Operator prefers focused validation before wider scans.",
        "artifacts": ["artifact://summary/1", "skill://brainstorming"],
        "confidence": 0.84
    });

    store
        .upsert_thread_structural_memory("thread-structural", &state_json, 1_717_170_777)
        .await?;

    let record = store
        .get_thread_structural_memory("thread-structural")
        .await?
        .expect("thread structural memory should be stored");

    assert_eq!(record.thread_id, "thread-structural");
    assert_eq!(record.state_json, state_json);
    assert_eq!(record.updated_at, 1_717_170_777);

    fs::remove_dir_all(root)?;
    Ok(())
}

mod structural_memory {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn observed_file_enrichment_records_manifest_and_structural_ref() -> Result<()> {
        let (store, root) = make_test_store().await?;

        fs::create_dir_all(root.join("src"))?;
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )?;
        fs::write(root.join("src/lib.rs"), "mod parser;\n")?;
        fs::write(root.join("src/parser.rs"), "pub fn parse() {}\n")?;

        let mut memory = crate::agent::context::structural_memory::discover_workspace_seeds(&root)
            .expect("workspace seed discovery should succeed");
        let manifest_refs =
            crate::agent::context::structural_memory::observe_successful_file_tool_result(
                &mut memory,
                &root,
                "read_file",
                &serde_json::json!({
                    "filePath": root.join("Cargo.toml").to_string_lossy().to_string(),
                })
                .to_string(),
                None,
            )
            .expect("manifest observation should succeed");
        let source_refs =
            crate::agent::context::structural_memory::observe_successful_file_tool_result(
                &mut memory,
                &root,
                "read_file",
                &serde_json::json!({
                    "filePath": root.join("src/lib.rs").to_string_lossy().to_string(),
                })
                .to_string(),
                Some("mod parser;\n"),
            )
            .expect("source observation should succeed");

        store
            .upsert_thread_structural_memory_state("thread-structural", &memory, 1_717_170_888)
            .await?;

        let restored: crate::agent::context::structural_memory::ThreadStructuralMemory = store
            .get_thread_structural_memory_state("thread-structural")
            .await?
            .expect("typed thread structural memory should be stored");

        assert_eq!(manifest_refs, vec!["node:file:Cargo.toml".to_string()]);
        assert_eq!(source_refs, vec!["node:file:src/lib.rs".to_string()]);
        assert!(restored
            .workspace_seeds
            .iter()
            .any(|seed| seed.node_id == "node:file:Cargo.toml"));
        assert!(restored
            .observed_files
            .iter()
            .any(|node| node.node_id == "node:file:src/lib.rs"));
        assert!(restored.edges.iter().any(|edge| {
            edge.from == "node:file:src/lib.rs"
                && edge.to == "node:file:src/parser.rs"
                && edge.kind == "imported_file"
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn persisted_thread_structural_memory_supports_graph_lookup() -> Result<()> {
        let (store, root) = make_test_store().await?;

        let memory = crate::agent::context::structural_memory::ThreadStructuralMemory {
            workspace_seed_scan_complete: true,
            language_hints: vec!["rust".to_string()],
            workspace_seeds: vec![crate::agent::context::structural_memory::WorkspaceSeed {
                node_id: "node:file:Cargo.toml".to_string(),
                relative_path: "Cargo.toml".to_string(),
                kind: "cargo_manifest".to_string(),
            }],
            observed_files: vec![crate::agent::context::structural_memory::ObservedFileNode {
                node_id: "node:file:src/lib.rs".to_string(),
                relative_path: "src/lib.rs".to_string(),
            }],
            edges: vec![
                crate::agent::context::structural_memory::StructuralEdge {
                    from: "node:file:src/lib.rs".to_string(),
                    to: "node:file:src/parser.rs".to_string(),
                    kind: "imported_file".to_string(),
                },
                crate::agent::context::structural_memory::StructuralEdge {
                    from: "node:file:src/lib.rs".to_string(),
                    to: "node:package:cargo:demo".to_string(),
                    kind: "file_in_package".to_string(),
                },
            ],
        };

        store
            .upsert_thread_structural_memory_state("thread-graph-lookup", &memory, 1_717_170_892)
            .await?;

        let restored: crate::agent::context::structural_memory::ThreadStructuralMemory = store
            .get_thread_structural_memory_state("thread-graph-lookup")
            .await?
            .expect("typed thread structural memory should be stored");

        let neighbors = restored.graph_lookup(&["node:file:src/lib.rs".to_string()], 4);
        assert_eq!(neighbors.len(), 2);
        assert!(neighbors.iter().any(|neighbor| {
            neighbor.node_id == "node:file:src/parser.rs"
                && neighbor.relation_kind == "imported_file"
                && neighbor.direction == "outgoing"
        }));
        assert!(neighbors.iter().any(|neighbor| {
            neighbor.node_id == "node:package:cargo:demo"
                && neighbor.relation_kind == "file_in_package"
                && neighbor.direction == "outgoing"
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn observed_file_enrichment_uses_filename_and_cwd_relative_resolution() -> Result<()> {
        let (_store, root) = make_test_store().await?;

        let cwd = root.join("frontend").join("src");
        fs::create_dir_all(&cwd)?;
        fs::write(cwd.join("generated.ts"), "export const generated = true;\n")?;

        let mut memory =
            crate::agent::context::structural_memory::ThreadStructuralMemory::default();
        let structural_refs =
            crate::agent::context::structural_memory::observe_successful_file_tool_result(
                &mut memory,
                &root,
                "create_file",
                &serde_json::json!({
                    "path": "",
                    "filename": "generated.ts",
                    "cwd": cwd.to_string_lossy().to_string(),
                    "content": "export const generated = true;\n",
                })
                .to_string(),
                None,
            )
            .expect("create_file observation should resolve filename against cwd");

        assert_eq!(
            structural_refs,
            vec!["node:file:frontend/src/generated.ts".to_string()]
        );
        assert!(memory.observed_files.iter().any(|node| {
            node.node_id == "node:file:frontend/src/generated.ts"
                && node.relative_path == "frontend/src/generated.ts"
        }));
        assert!(!memory
            .observed_files
            .iter()
            .any(|node| node.node_id == "node:file:generated.ts"));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn commented_js_imports_do_not_persist_imported_file_edges() -> Result<()> {
        let (store, root) = make_test_store().await?;

        fs::create_dir_all(root.join("src"))?;
        fs::write(
            root.join("src/app.ts"),
            "// import \"./commented\";\n/*\nexport * from \"./commented-block\";\n*/\nimport \"./live\";\nconst note = \"require('./literal')\";\n",
        )?;
        fs::write(root.join("src/live.ts"), "export const live = true;\n")?;
        fs::write(
            root.join("src/commented.ts"),
            "export const hidden = true;\n",
        )?;
        fs::write(
            root.join("src/commented-block.ts"),
            "export const hiddenBlock = true;\n",
        )?;
        fs::write(
            root.join("src/literal.ts"),
            "export const literal = true;\n",
        )?;

        let mut memory =
            crate::agent::context::structural_memory::ThreadStructuralMemory::default();
        crate::agent::context::structural_memory::observe_successful_file_tool_result(
            &mut memory,
            &root,
            "read_file",
            &serde_json::json!({
                "filePath": root.join("src/app.ts").to_string_lossy().to_string(),
            })
            .to_string(),
            Some(&fs::read_to_string(root.join("src/app.ts"))?),
        )
        .expect("source observation should succeed");

        store
            .upsert_thread_structural_memory_state(
                "thread-commented-imports",
                &memory,
                1_717_170_889,
            )
            .await?;

        let restored: crate::agent::context::structural_memory::ThreadStructuralMemory = store
            .get_thread_structural_memory_state("thread-commented-imports")
            .await?
            .expect("typed thread structural memory should be stored");

        assert!(restored.edges.iter().any(|edge| {
            edge.from == "node:file:src/app.ts"
                && edge.to == "node:file:src/live.ts"
                && edge.kind == "imported_file"
        }));
        assert!(!restored.edges.iter().any(|edge| {
            edge.from == "node:file:src/app.ts"
                && edge.to == "node:file:src/commented.ts"
                && edge.kind == "imported_file"
        }));
        assert!(!restored.edges.iter().any(|edge| {
            edge.from == "node:file:src/app.ts"
                && edge.to == "node:file:src/commented-block.ts"
                && edge.kind == "imported_file"
        }));
        assert!(!restored.edges.iter().any(|edge| {
            edge.from == "node:file:src/app.ts"
                && edge.to == "node:file:src/literal.ts"
                && edge.kind == "imported_file"
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn named_js_reexports_persist_imported_file_edges() -> Result<()> {
        let (store, root) = make_test_store().await?;

        fs::create_dir_all(root.join("src"))?;
        fs::write(
            root.join("src/index.ts"),
            "export { parser } from \"./parser\";\n",
        )?;
        fs::write(
            root.join("src/parser.ts"),
            "export const parser = () => true;\n",
        )?;

        let mut memory =
            crate::agent::context::structural_memory::ThreadStructuralMemory::default();
        crate::agent::context::structural_memory::observe_successful_file_tool_result(
            &mut memory,
            &root,
            "read_file",
            &serde_json::json!({
                "filePath": root.join("src/index.ts").to_string_lossy().to_string(),
            })
            .to_string(),
            Some(&fs::read_to_string(root.join("src/index.ts"))?),
        )
        .expect("named re-export observation should succeed");

        store
            .upsert_thread_structural_memory_state("thread-named-reexports", &memory, 1_717_170_891)
            .await?;

        let restored: crate::agent::context::structural_memory::ThreadStructuralMemory = store
            .get_thread_structural_memory_state("thread-named-reexports")
            .await?
            .expect("typed thread structural memory should be stored");

        assert!(restored.edges.iter().any(|edge| {
            edge.from == "node:file:src/index.ts"
                && edge.to == "node:file:src/parser.ts"
                && edge.kind == "imported_file"
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn parent_traversal_paths_do_not_persist_observed_file_nodes() -> Result<()> {
        let (_store, root) = make_test_store().await?;

        let mut memory =
            crate::agent::context::structural_memory::ThreadStructuralMemory::default();
        let structural_refs =
            crate::agent::context::structural_memory::observe_successful_file_tool_result(
                &mut memory,
                &root,
                "read_file",
                &serde_json::json!({
                    "filePath": "../outside/escape.ts",
                })
                .to_string(),
                Some("import \"./nested\";\n"),
            )
            .expect("escaped source observation should be skipped");

        assert!(structural_refs.is_empty());
        assert!(!memory
            .observed_files
            .iter()
            .any(|node| node.node_id == "node:file:outside/escape.ts"));
        assert!(!memory
            .workspace_seeds
            .iter()
            .any(|seed| seed.node_id == "node:file:outside/escape.ts"));
        assert!(!memory.edges.iter().any(|edge| {
            edge.from == "node:file:outside/escape.ts" || edge.to == "node:file:outside/escape.ts"
        }));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn escaped_apply_patch_paths_leave_structural_memory_unchanged() -> Result<()> {
        let (store, root) = make_test_store().await?;

        let mut memory = crate::agent::context::structural_memory::ThreadStructuralMemory {
            workspace_seed_scan_complete: false,
            language_hints: vec!["rust".to_string()],
            workspace_seeds: vec![crate::agent::context::structural_memory::WorkspaceSeed {
                node_id: "node:manifest:Cargo.toml".to_string(),
                relative_path: "Cargo.toml".to_string(),
                kind: "manifest".to_string(),
            }],
            observed_files: vec![crate::agent::context::structural_memory::ObservedFileNode {
                node_id: "node:file:src/lib.rs".to_string(),
                relative_path: "src/lib.rs".to_string(),
            }],
            edges: vec![crate::agent::context::structural_memory::StructuralEdge {
                from: "node:manifest:Cargo.toml".to_string(),
                to: "node:file:src/lib.rs".to_string(),
                kind: "contains".to_string(),
            }],
        };
        let expected = memory.clone();

        let structural_refs = crate::agent::context::structural_memory::observe_successful_file_tool_result(
            &mut memory,
            &root,
            "apply_patch",
            &serde_json::json!({
                "input": "*** Begin Patch\n*** Update File: ../outside/file.ts\n@@\n-old\n+new\n*** End Patch",
            })
            .to_string(),
            None,
        )
        .expect("escaped apply_patch observation should be skipped");

        store
            .upsert_thread_structural_memory_state(
                "thread-escaped-apply-patch",
                &memory,
                1_717_170_890,
            )
            .await?;

        let restored: crate::agent::context::structural_memory::ThreadStructuralMemory = store
            .get_thread_structural_memory_state("thread-escaped-apply-patch")
            .await?
            .expect("typed thread structural memory should be stored");

        assert!(structural_refs.is_empty());
        assert_eq!(memory, expected);
        assert_eq!(restored, expected);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn escaped_apply_patch_alias_paths_leave_structural_memory_unchanged() -> Result<()> {
        let (store, root) = make_test_store().await?;

        let mut memory = crate::agent::context::structural_memory::ThreadStructuralMemory {
            workspace_seed_scan_complete: false,
            language_hints: vec!["rust".to_string()],
            workspace_seeds: vec![crate::agent::context::structural_memory::WorkspaceSeed {
                node_id: "node:manifest:Cargo.toml".to_string(),
                relative_path: "Cargo.toml".to_string(),
                kind: "manifest".to_string(),
            }],
            observed_files: vec![crate::agent::context::structural_memory::ObservedFileNode {
                node_id: "node:file:src/lib.rs".to_string(),
                relative_path: "src/lib.rs".to_string(),
            }],
            edges: vec![crate::agent::context::structural_memory::StructuralEdge {
                from: "node:manifest:Cargo.toml".to_string(),
                to: "node:file:src/lib.rs".to_string(),
                kind: "contains".to_string(),
            }],
        };
        let expected = memory.clone();

        let structural_refs = crate::agent::context::structural_memory::observe_successful_file_tool_result(
            &mut memory,
            &root,
            "apply_patch",
            &serde_json::json!({
                "patch": "*** Begin Patch\n*** Update File: ../outside/file.ts\n@@\n-old\n+new\n*** End Patch",
            })
            .to_string(),
            None,
        )
        .expect("escaped apply_patch alias observation should be skipped");

        store
            .upsert_thread_structural_memory_state(
                "thread-escaped-apply-patch-alias",
                &memory,
                1_717_170_891,
            )
            .await?;

        let restored: crate::agent::context::structural_memory::ThreadStructuralMemory = store
            .get_thread_structural_memory_state("thread-escaped-apply-patch-alias")
            .await?
            .expect("typed thread structural memory should be stored");

        assert!(structural_refs.is_empty());
        assert_eq!(memory, expected);
        assert_eq!(restored, expected);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn observed_file_enrichment_reuses_cached_workspace_seed_discovery() -> Result<()> {
        let (_store, root) = make_test_store().await?;

        fs::create_dir_all(root.join("src"))?;
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )?;
        fs::write(root.join("src/lib.rs"), "mod parser;\n")?;
        fs::write(root.join("src/parser.rs"), "pub fn parse() {}\n")?;

        let mut memory =
            crate::agent::context::structural_memory::ThreadStructuralMemory::default();
        crate::agent::context::structural_memory::observe_successful_file_tool_result(
            &mut memory,
            &root,
            "read_file",
            &serde_json::json!({
                "filePath": root.join("src/lib.rs").to_string_lossy().to_string(),
            })
            .to_string(),
            Some("mod parser;\n"),
        )
        .expect("initial source observation should succeed");

        let initial_state_json = serde_json::to_value(&memory)?;
        assert_eq!(
            initial_state_json.get("workspace_seed_scan_complete"),
            Some(&serde_json::Value::Bool(true)),
            "thread structural memory should record that workspace seed discovery already ran"
        );

        fs::create_dir_all(root.join("frontend"))?;
        fs::write(
            root.join("frontend/package.json"),
            "{\"name\":\"late-seed\"}\n",
        )?;

        crate::agent::context::structural_memory::observe_successful_file_tool_result(
            &mut memory,
            &root,
            "read_file",
            &serde_json::json!({
                "filePath": root.join("src/parser.rs").to_string_lossy().to_string(),
            })
            .to_string(),
            Some("pub fn parse() {}\n"),
        )
        .expect("follow-up source observation should succeed");

        assert!(memory
            .workspace_seeds
            .iter()
            .any(|seed| seed.node_id == "node:file:Cargo.toml"));
        assert!(!memory
            .workspace_seeds
            .iter()
            .any(|seed| seed.node_id == "node:file:frontend/package.json"));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[tokio::test]
    async fn observed_write_file_manifest_after_cached_seed_scan_adds_structural_ref_and_javascript_hint(
    ) -> Result<()> {
        let (_store, root) = make_test_store().await?;

        fs::create_dir_all(root.join("src"))?;
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )?;
        fs::write(root.join("src/lib.rs"), "mod parser;\n")?;
        fs::write(root.join("src/parser.rs"), "pub fn parse() {}\n")?;

        let mut memory =
            crate::agent::context::structural_memory::ThreadStructuralMemory::default();
        crate::agent::context::structural_memory::observe_successful_file_tool_result(
            &mut memory,
            &root,
            "read_file",
            &serde_json::json!({
                "filePath": root.join("src/lib.rs").to_string_lossy().to_string(),
            })
            .to_string(),
            Some("mod parser;\n"),
        )
        .expect("initial source observation should succeed");

        assert!(memory.workspace_seed_scan_complete);

        fs::create_dir_all(root.join("frontend"))?;
        fs::write(
            root.join("frontend/package.json"),
            "{\"name\":\"late-seed\"}\n",
        )?;

        let structural_refs =
            crate::agent::context::structural_memory::observe_successful_file_tool_result(
                &mut memory,
                &root,
                "write_file",
                &serde_json::json!({
                    "path": root.join("frontend/package.json").to_string_lossy().to_string(),
                    "content": "{\"name\":\"late-seed\"}\n",
                })
                .to_string(),
                None,
            )
            .expect("late manifest write observation should succeed");

        assert_eq!(
            structural_refs,
            vec!["node:file:frontend/package.json".to_string()]
        );
        assert!(memory
            .observed_files
            .iter()
            .any(|node| node.node_id == "node:file:frontend/package.json"));
        assert!(memory
            .workspace_seeds
            .iter()
            .any(|seed| seed.node_id == "node:file:frontend/package.json"));
        assert!(memory.edges.iter().any(|edge| {
            edge.from == "node:file:frontend/package.json"
                && edge.to == "node:file:frontend"
                && edge.kind == "package_root"
        }));
        assert!(memory
            .language_hints
            .iter()
            .any(|hint| hint == "javascript"));

        fs::remove_dir_all(root)?;
        Ok(())
    }
}

#[tokio::test]
async fn memory_graph_round_trips_nodes_and_edges() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .upsert_memory_node(
            "node:file:src/lib.rs",
            "src/lib.rs",
            "file",
            Some("observed file"),
            1_717_180_001,
        )
        .await?;
    store
        .upsert_memory_node(
            "node:task:task-123",
            "Investigate parser",
            "task",
            Some("task status: queued"),
            1_717_180_002,
        )
        .await?;
    store
        .upsert_memory_edge(
            "node:task:task-123",
            "node:file:src/lib.rs",
            "task_touches_file",
            1.0,
            1_717_180_003,
        )
        .await?;
    store
        .upsert_memory_edge(
            "node:task:task-123",
            "node:file:src/lib.rs",
            "task_touches_file",
            1.0,
            1_717_180_004,
        )
        .await?;

    let node = store
        .get_memory_node("node:file:src/lib.rs")
        .await?
        .expect("memory node should exist");
    assert_eq!(node.label, "src/lib.rs");
    assert_eq!(node.node_type, "file");

    let edges = store
        .list_memory_edges_for_node("node:task:task-123")
        .await?;
    let edge = edges
        .iter()
        .find(|edge| edge.relation_type == "task_touches_file")
        .expect("task_touches_file edge should exist");
    assert_eq!(edge.source_node_id, "node:task:task-123");
    assert_eq!(edge.target_node_id, "node:file:src/lib.rs");
    assert_eq!(edge.weight, 2.0);

    fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn memory_graph_neighbor_lookup_returns_ranked_adjacent_nodes() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .upsert_memory_node(
            "node:file:src/lib.rs",
            "src/lib.rs",
            "file",
            Some("core file"),
            1_717_180_101,
        )
        .await?;
    store
        .upsert_memory_node(
            "node:package:cargo:demo",
            "demo",
            "package",
            Some("cargo package"),
            1_717_180_102,
        )
        .await?;
    store
        .upsert_memory_node(
            "node:error:read_file:missing",
            "missing file error",
            "error",
            Some("tool failure"),
            1_717_180_103,
        )
        .await?;
    store
        .upsert_memory_edge(
            "node:file:src/lib.rs",
            "node:package:cargo:demo",
            "file_in_package",
            3.0,
            1_717_180_104,
        )
        .await?;
    store
        .upsert_memory_edge(
            "node:file:src/lib.rs",
            "node:error:read_file:missing",
            "file_hit_error",
            1.0,
            1_717_180_105,
        )
        .await?;

    let neighbors = store
        .list_memory_graph_neighbors("node:file:src/lib.rs", 4)
        .await?;
    assert_eq!(neighbors.len(), 2);
    assert_eq!(neighbors[0].node.id, "node:package:cargo:demo");
    assert_eq!(neighbors[0].via_edge.relation_type, "file_in_package");
    assert_eq!(neighbors[1].node.id, "node:error:read_file:missing");
    assert_eq!(neighbors[1].via_edge.relation_type, "file_hit_error");

    fs::remove_dir_all(root)?;
    Ok(())
}
