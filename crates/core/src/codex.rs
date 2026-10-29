//! Compatibility with the pinned Codex ACP adapter, not assumptions about other agents.

use anyhow::{bail, Result};
use serde_json::Value;

/// `codex login status` can include part of an API key. Never expose its raw text.
pub(crate) fn login_status(success: bool, output: &str) -> Option<String> {
    if !success {
        return None;
    }
    output.lines().find_map(|line| {
        let line = line.trim();
        if line == "Logged in using ChatGPT" {
            Some("ChatGPT".into())
        } else if line == "Logged in using an API key" || line.starts_with("Logged in using an API key - ") {
            Some("API key".into())
        } else {
            None
        }
    })
}

/// Older ACP model IDs combine model and effort; current config options separate them.
pub(crate) fn preset(model: Option<String>, effort: Option<String>) -> (Option<String>, Option<String>) {
    let Some(model) = model else { return (None, effort) };
    if let Some((base, suffix)) = model.strip_suffix(']').and_then(|m| m.rsplit_once('[')) {
        if !base.is_empty() && !suffix.is_empty() {
            return (Some(base.to_string()), effort.or_else(|| Some(suffix.to_string())));
        }
    }
    (Some(model), effort)
}

pub(crate) fn mode_changes(config: &Value, plan: bool) -> Result<Vec<(String, String)>> {
    let mut changes = Vec::new();
    for (id, value) in [
        ("mode", if plan { "read-only" } else { "agent-full-access" }),
        ("collaboration_mode", if plan { "plan" } else { "default" }),
    ] {
        let option = config.as_array().and_then(|opts| opts.iter().find(|o| o["id"] == id));
        let Some(option) = option else {
            if id == "collaboration_mode" && !plan {
                continue;
            }
            bail!("Codex does not advertise {id}; update the Codex ACP adapter before changing permission mode");
        };
        if !crate::catalog::choices(option).iter().any(|choice| choice.value == value) {
            bail!("Codex does not support {id}={value}; cannot apply permission mode safely");
        }
        if option["currentValue"] != value {
            changes.push((id.to_string(), value.to_string()));
        }
    }
    Ok(changes)
}

/// The adapter can report the model actually used, independently of session config.
/// Multiple model rows cannot be attributed to one model; retain the session model.
pub(crate) fn reported_model(result: &Value) -> Option<&str> {
    let models = result.pointer("/_meta/quota/model_usage")?.as_array()?;
    if models.len() != 1 {
        return None;
    }
    models[0]["model"].as_str().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn login_does_not_disclose_api_keys_or_accept_failed_status() {
        assert_eq!(
            login_status(true, "Logged in using an API key - sk-secret-part"),
            Some("API key".into())
        );
        assert_eq!(login_status(true, "Logged in using ChatGPT\n"), Some("ChatGPT".into()));
        assert_eq!(login_status(false, "Logged in using ChatGPT"), None);
        assert_eq!(login_status(true, "Not logged in"), None);
        assert_eq!(login_status(true, "warning: Logged in somewhere"), None);
    }

    #[test]
    fn legacy_presets_split_and_explicit_effort_wins() {
        assert_eq!(
            preset(Some("gpt-test[low]".into()), None),
            (Some("gpt-test".into()), Some("low".into()))
        );
        assert_eq!(
            preset(Some("gpt-test[low]".into()), Some("high".into())),
            (Some("gpt-test".into()), Some("high".into()))
        );
        assert_eq!(preset(Some("gpt-test[]".into()), None), (Some("gpt-test[]".into()), None));
        assert_eq!(preset(None, Some("low".into())), (None, Some("low".into())));
    }

    #[test]
    fn unsupported_plan_is_not_silently_treated_as_full_access() {
        let config = json!([{"id":"mode", "currentValue":"agent-full-access", "options":[{"value":"agent-full-access"}]}]);
        assert!(mode_changes(&config, true)
            .unwrap_err()
            .to_string()
            .contains("cannot apply permission mode safely"));
    }

    #[test]
    fn does_not_guess_a_model_for_multiple_usage_rows() {
        assert_eq!(
            reported_model(&json!({"_meta":{"quota":{"model_usage":[{"model":"a"},{"model":"b"}]}}})),
            None
        );
    }
}
