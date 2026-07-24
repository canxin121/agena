use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::server::persistence::db;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub projects: Vec<Project>,

    // Optional configuration knobs used by GitHub device flow.
    #[serde(default)]
    pub github_client_id: Option<String>,
    #[serde(default)]
    pub github_scopes: Option<String>,

    // Preserve unknown fields so we can round-trip the settings file even when
    // only a subset is explicitly modeled.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub added_at: i64,
    #[serde(default)]
    pub last_opened_at: i64,
}

pub async fn init_settings(db: &crate::server::persistence::db::ServerStateDb) -> Settings {
    if let Ok(Some(settings)) = db
        .get_json::<Settings>(crate::server::persistence::db::KV_KEY_SETTINGS)
        .await
    {
        return settings;
    }

    let settings = Settings::default();
    let _ = db
        .set_json(crate::server::persistence::db::KV_KEY_SETTINGS, &settings)
        .await;
    settings
}

pub async fn persist_settings(
    db: &crate::server::persistence::db::ServerStateDb,
    settings: &Settings,
) -> Result<(), String> {
    db.set_json(crate::server::persistence::db::KV_KEY_SETTINGS, settings)
        .await
}
