//! Compatibility at the native OpenCode boundary. Provider/model IDs come from
//! the user's CLI, never from Suneiro's own credential store or a static catalog.

use anyhow::{bail, Context, Result};
use serde_json::Value;

pub fn prompt_result(result: Result<Value>, images: bool) -> Result<Value> {
    result.context(if images {
        "OpenCode could not complete the image prompt. ACP image transport support does not guarantee model vision support. Choose a vision-capable provider/model and check provider credentials or quota with the OpenCode CLI"
    } else {
        "OpenCode could not complete the prompt. Check the selected provider/model with `opencode models`, provider credentials with `opencode auth login`, and provider quota or connectivity"
    })
}

pub const NO_MODELS: &str = "OpenCode reported no available models. Run `opencode models` and configure a provider with `opencode auth login` (or its documented environment variables), then refresh models. Free/local providers do not require a saved login.";

/// `auth list` contains saved credentials AND environment-configured providers.
/// Keep provider names only; environment variable names are not account labels.
pub fn providers(text: &str) -> Vec<String> {
    let mut providers = Vec::new();
    for line in text.lines() {
        let Some(line) = line.trim_start().strip_prefix('●') else {
            continue;
        };
        let name = line
            .split_whitespace()
            .take_while(|word| {
                *word != "api"
                    && *word != "oauth"
                    && !(word.contains('_')
                        && word
                            .chars()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            })
            .collect::<Vec<_>>()
            .join(" ");
        if !name.is_empty() && !providers.contains(&name) {
            providers.push(name);
        }
    }
    providers
}

/// Free/local models can be usable without `auth list` containing credentials.
pub fn has_models(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        !line.chars().any(char::is_whitespace)
            && line
                .split_once('/')
                .is_some_and(|(provider, model)| !provider.is_empty() && !model.is_empty())
    })
}

/// Reject stale presets and unsupported controls instead of silently running the
/// default model. OpenCode only offers models for providers it knows about.
pub fn validate_control(config: &Value, id: &str, value: &Value) -> Result<()> {
    let option = config
        .as_array()
        .and_then(|options| options.iter().find(|o| o["id"] == id));
    let Some(option) = option else {
        bail!("OpenCode does not advertise the `{id}` control in this session. Refresh models or choose a supported control.");
    };
    if option["type"] == "boolean" && value.is_boolean() {
        return Ok(());
    }
    if !crate::catalog::choices(option)
        .iter()
        .any(|choice| value.as_str() == Some(&choice.value))
    {
        if id == "model" {
            bail!("OpenCode model {value} is unavailable for the configured providers. Run `opencode models`, check provider credentials with `opencode auth login`, then refresh models.");
        }
        bail!("OpenCode does not offer {value} for `{id}` in this session.");
    }
    Ok(())
}

pub fn validate_models(config: &Value) -> Result<()> {
    if crate::catalog::model_option(config)
        .map(crate::catalog::choices)
        .is_none_or(|models| models.is_empty())
    {
        bail!("{NO_MODELS}");
    }
    Ok(())
}

pub fn validate_initial(config: &Value, model: Option<&str>, effort: Option<&str>) -> Result<()> {
    validate_models(config)?;
    if let Some(model) = model.or_else(|| crate::catalog::model_option(config).and_then(|o| o["currentValue"].as_str()))
    {
        validate_control(config, "model", &Value::String(model.into()))?;
    }
    // Effort availability can depend on the requested model, so validate it
    // after model selection in apply_initial, not against the old model here.
    if effort.is_some()
        && crate::catalog::effort_option(config).is_none()
        && model.is_none_or(|model| {
            crate::catalog::model_option(config).is_some_and(|option| option["currentValue"] == model)
        })
    {
        bail!("OpenCode does not advertise reasoning effort for this session. Remove the effort preset or choose a model with an effort control.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn environment_providers_are_ready_without_saved_credentials() {
        let text = "┌ Credentials\n└ 0 credentials\n┌ Environment\n● Anthropic ANTHROPIC_API_KEY\n● Amazon Bedrock AWS_ACCESS_KEY_ID\n└ 2 environment variables\n";
        assert_eq!(providers(text), ["Anthropic", "Amazon Bedrock"]);
    }

    #[test]
    fn combines_saved_and_environment_providers_without_duplicates() {
        assert_eq!(
            providers("● OpenAI oauth\n● DevPass (LLM Gateway) api\n● OpenAI OPENAI_API_KEY\n"),
            ["OpenAI", "DevPass (LLM Gateway)"]
        );
        assert!(providers("└ 0 credentials").is_empty());
    }

    #[test]
    fn free_or_local_models_need_no_saved_login() {
        assert!(has_models("opencode/big-pickle\nollama/local-model\n"));
        assert!(!has_models("Error: provider/model not found\n\n"));
        assert!(!has_models("/\nprovider/\n/model"));
    }

    #[test]
    fn missing_models_and_controls_have_actionable_errors() {
        assert!(validate_models(&json!([]))
            .unwrap_err()
            .to_string()
            .contains("opencode auth login"));
        let config = json!([{"id":"model","options":[{"value":"local/model","name":"Local"}]}]);
        validate_models(&config).unwrap();
        validate_control(&config, "model", &json!("local/model")).unwrap();
        let boolean = json!([{"id":"boolean-control","type":"boolean"}]);
        validate_control(&boolean, "boolean-control", &json!(true)).unwrap();
        assert!(validate_control(&boolean, "boolean-control", &json!("true")).is_err());
        assert!(validate_control(&config, "model", &json!("missing/model"))
            .unwrap_err()
            .to_string()
            .contains("unavailable"));
        assert!(validate_control(&config, "effort", &json!("high"))
            .unwrap_err()
            .to_string()
            .contains("does not advertise"));
    }
}
