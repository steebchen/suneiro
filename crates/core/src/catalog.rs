//! What each agent offers: models, effort levels and whether it has a fast
//! mode. Learned from the agent's own ACP `configOptions`, either from a short
//! discovery session or from any live session, and cached in the settings table.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::acp::{AcpConnection, PROTOCOL_VERSION};
use crate::agent::AgentDef;
use crate::store::{now, Store};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Catalog {
    pub models: Vec<Choice>,
    pub efforts: Vec<Choice>,
    pub has_fast: bool,
    pub updated_at: i64,
}

/// The option that picks the model (`id: "model"` or category `model`).
pub fn model_option(config: &Value) -> Option<&Value> {
    let opts = config.as_array()?;
    opts.iter()
        .find(|o| o["id"] == "model")
        .or_else(|| opts.iter().find(|o| o["category"] == "model"))
}

/// The option that sets reasoning effort (category `thought_level`).
pub fn effort_option(config: &Value) -> Option<&Value> {
    config.as_array()?.iter().find(|o| o["category"] == "thought_level")
}

pub fn choices(option: &Value) -> Vec<Choice> {
    let mut out = Vec::new();
    for o in option["options"].as_array().into_iter().flatten() {
        // Options may be grouped: { group, name, options: [...] }.
        let items = match o["options"].as_array() {
            Some(group) => group.iter().collect::<Vec<_>>(),
            None => vec![o],
        };
        for item in items {
            if let Some(value) = item["value"].as_str() {
                out.push(Choice {
                    value: value.into(),
                    name: item["name"].as_str().unwrap_or(value).into(),
                    description: item["description"].as_str().map(str::to_string),
                });
            }
        }
    }
    out
}

pub fn from_config(config: &Value) -> Option<Catalog> {
    let models: Vec<Choice> = choices(model_option(config)?)
        .into_iter()
        // "Default (recommended)" is an alias, not a model.
        .filter(|c| c.value != "default")
        .collect();
    if models.is_empty() {
        return None;
    }
    let efforts = effort_option(config).map(choices).unwrap_or_default();
    let has_fast = config
        .as_array()
        .is_some_and(|opts| opts.iter().any(|o| o["id"].as_str().is_some_and(|id| id.contains("fast"))));
    Some(Catalog { models, efforts, has_fast, updated_at: now() })
}

fn key(agent_id: &str) -> String {
    format!("catalog:{agent_id}")
}

pub fn load(store: &Store, agent_id: &str) -> Option<Catalog> {
    store.setting(&key(agent_id)).ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}

/// Save if it differs from what's stored (live sessions report config often).
pub fn save(store: &Store, agent_id: &str, catalog: &Catalog) {
    if let Some(old) = load(store, agent_id) {
        if old.models == catalog.models && old.efforts == catalog.efforts && old.has_fast == catalog.has_fast {
            return;
        }
    }
    if let Ok(s) = serde_json::to_string(catalog) {
        let _ = store.set_setting(&key(agent_id), &s);
    }
}

/// Start the agent just long enough to read its config options.
pub async fn discover(def: &AgentDef) -> Result<Catalog> {
    let dir = std::env::temp_dir().join("suneiro-catalog");
    std::fs::create_dir_all(&dir)?;
    let dir: PathBuf = dir.canonicalize().unwrap_or(dir);
    let (conn, _rx) = AcpConnection::spawn(&def.command, &def.args, &dir)?;
    let result = tokio::time::timeout(Duration::from_secs(90), async {
        conn.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "suneiro", "version": env!("CARGO_PKG_VERSION")}
            }),
        )
        .await?;
        let r = conn
            .request("session/new", json!({"cwd": dir.to_string_lossy(), "mcpServers": []}))
            .await?;
        from_config(&r["configOptions"]).ok_or_else(|| {
            if def.id == "opencode" { anyhow!(crate::opencode::NO_MODELS) }
            else { anyhow!("{} reported no models", def.name) }
        })
    })
    .await
    .map_err(|_| anyhow!("{} took too long to start", def.name))?;
    conn.kill();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_models_efforts_and_fast() {
        let config = json!([
            {"id": "mode", "category": "mode", "options": [{"value": "plan", "name": "Plan"}]},
            {"id": "model", "category": "model", "currentValue": "opus", "options": [
                {"value": "default", "name": "Default (recommended)"},
                {"value": "opus", "name": "Opus 5.5"},
                {"value": "sonnet", "name": "Sonnet 5"}
            ]},
            {"id": "effort", "category": "thought_level", "options": [{"value": "high", "name": "High"}]},
            {"id": "fast", "category": "model_config", "options": [{"value": "on", "name": "On"}]}
        ]);
        let c = from_config(&config).unwrap();
        assert_eq!(c.models.iter().map(|m| m.value.as_str()).collect::<Vec<_>>(), ["opus", "sonnet"]);
        assert_eq!(c.efforts[0].name, "High");
        assert!(c.has_fast);
    }

    #[test]
    fn flattens_grouped_options() {
        let config = json!([{"id": "model", "options": [
            {"group": "a", "name": "A", "options": [{"value": "a/x", "name": "X"}]},
            {"group": "b", "name": "B", "options": [{"value": "b/y", "name": "Y"}]}
        ]}]);
        assert_eq!(from_config(&config).unwrap().models.len(), 2);
    }
}
