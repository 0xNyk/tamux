use super::*;
use std::fs;
use uuid::Uuid;

async fn make_test_store() -> Result<(HistoryStore, PathBuf)> {
    let root = std::env::temp_dir().join(format!("tamux-history-test-{}", Uuid::new_v4()));
    let store = HistoryStore::new_test_store(&root).await?;
    Ok((store, root))
}

mod elastic_context;
mod emergent_protocol;
mod event_triggers;
mod goal_runs;
mod governance;
mod metacognition;
mod misc;
mod notifications;
mod provider_auth;
mod skill_variants;
mod sqlite_audit;
mod statistics;
mod tombstones_gateway;
