//! Negotiated ACP features. Provider names never imply runtime capabilities.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCapabilities {
    pub steering: bool,
    pub images: bool,
    pub resume: bool,
    pub load: bool,
}

impl SessionCapabilities {
    pub fn from_initialize(response: &Value) -> Self {
        let capabilities = &response["agentCapabilities"];
        Self {
            steering: response.pointer("/_meta/steering/supported").and_then(Value::as_bool) == Some(true),
            images: capabilities.pointer("/promptCapabilities/image").and_then(Value::as_bool) == Some(true),
            resume: capabilities.pointer("/sessionCapabilities/resume").is_some_and(Value::is_object),
            load: capabilities.get("loadSession").and_then(Value::as_bool) == Some(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn absent_or_invalid_features_are_not_advertised() {
        for response in [Value::Null, json!({}), json!({
            "_meta": {"steering": {"supported": "true"}},
            "agentCapabilities": {"promptCapabilities": {"image": 1}, "sessionCapabilities": {"resume": false}, "loadSession": "true"}
        })] {
            assert_eq!(SessionCapabilities::from_initialize(&response), SessionCapabilities::default());
        }
    }

    #[test]
    fn features_follow_negotiation_not_provider_identity() {
        let response = json!({
            "agentInfo": {"name": "any-provider"},
            "_meta": {"steering": {"supported": true}},
            "agentCapabilities": {"promptCapabilities": {"image": true}, "sessionCapabilities": {"resume": {}}, "loadSession": true}
        });
        assert_eq!(SessionCapabilities::from_initialize(&response), SessionCapabilities { steering: true, images: true, resume: true, load: true });
    }
}
