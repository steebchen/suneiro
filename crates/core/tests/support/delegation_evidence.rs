use serde_json::Value;
use std::sync::Arc;
use suneiro_core::{Core, Event};

pub struct SmokeShutdown(pub Arc<Core>);
impl Drop for SmokeShutdown {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

pub fn reports_permission(events: &[Event], session: &str, request: &Value) -> bool {
    let Some(id) = request["toolCallId"].as_str().filter(|id| !id.is_empty()) else { return false };
    events.iter().any(|event| match event {
        Event::SessionUpdate { session_id, update }
            if session_id == session && update["sessionUpdate"] == "tool_call" && update["toolCallId"] == id =>
        {
            let input = &update["rawInput"];
            let precise = input["server"] == "suneiro-delegation" && input["tool"] == "suneiro_reports";

            let empty_args = input.get("arguments").is_some_and(|args| args.as_object().is_some_and(|object| object.is_empty()));
            precise && empty_args
        }
        _ => false,
    })
}
fn empty_reports(value: &Value, depth: u8) -> bool {
    if depth > 8 {
        return false;
    }
    match value {
        Value::Object(object) => {
            if object.get("isError") == Some(&Value::Bool(true)) || object.get("error").is_some_and(|error| !error.is_null()) {
                return false;
            }
            if object.get("items").and_then(Value::as_array).is_some_and(|items| items.is_empty())
                && object.get("nextOffset") == Some(&Value::Null)
            {
                return true;
            }
            object.values().any(|value| empty_reports(value, depth + 1))
        }
        Value::Array(values) => values.iter().any(|value| empty_reports(value, depth + 1)),
        Value::String(text) => serde_json::from_str::<Value>(text).is_ok_and(|value| empty_reports(&value, depth + 1)),
        _ => false,
    }
}
pub fn reports_evidence(events: &[Event], session: &str) -> (bool, bool) {
    let updates: Vec<&Value> = events
        .iter()
        .filter_map(|event| match event {
            Event::SessionUpdate { session_id, update } if session_id == session => Some(update),
            _ => None,
        })
        .collect();
    let calls: Vec<&&Value> = updates.iter().filter(|u| u["sessionUpdate"] == "tool_call").collect();
    let tool = calls.len() == 1
        && calls.iter().any(|call| {
            let id = call["toolCallId"].as_str().filter(|id| !id.is_empty());

            let input = &call["rawInput"];
            let named = if input.get("server").is_some() {
                input["server"] == "suneiro-delegation" && input["tool"] == "suneiro_reports"
            } else {
                ["suneiro-delegation_suneiro_reports", "mcp.suneiro-delegation.suneiro_reports", "mcp__suneiro-delegation__suneiro_reports"]
                    .iter()
                    .any(|name| call["title"] == *name)
            };
            let completed = updates.iter().any(|update| {
                matches!(update["sessionUpdate"].as_str(), Some("tool_call" | "tool_call_update"))
                    && update["toolCallId"].as_str() == id
                    && update["status"] == "completed"
                    && (empty_reports(&update["rawOutput"], 0) || empty_reports(&update["content"], 0))
            });
            id.is_some() && named && completed
        });
    let assistant = updates
        .iter()
        .filter(|u| u["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|u| u.pointer("/content/text").and_then(Value::as_str))
        .collect::<String>();
    (tool, assistant.trim() == "BRIDGE_REPORTS_OK")
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn update(value: Value) -> Event {
        Event::SessionUpdate { session_id: "session".into(), update: value }
    }
    #[test]
    fn tool_name_in_plain_text_or_user_prompt_cannot_pass() {
        let events = vec![
            Event::UserMessage { session_id: "session".into(), text: "suneiro_reports BRIDGE_REPORTS_OK".into(), ts: 0, images: vec![] },
            update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"suneiro_reports BRIDGE_REPORTS_OK"}})),
        ];
        assert_eq!(reports_evidence(&events, "session"), (false, false));
    }
    #[test]
    fn actual_named_completed_call_and_exact_assistant_marker_are_required() {
        let mut events = vec![
            update(
                json!({"sessionUpdate":"tool_call","toolCallId":"report-1","title":"suneiro-delegation_suneiro_reports","rawInput":{"name":"suneiro_reports","arguments":{}},"status":"in_progress"}),
            ),
            update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"BRIDGE_REPORTS_OK"}})),
        ];
        assert_eq!(reports_evidence(&events, "session"), (false, true));
        events.push(update(json!({"sessionUpdate":"tool_call_update","toolCallId":"unrelated","status":"completed"})));
        assert_eq!(reports_evidence(&events, "session"), (false, true));
        events.push(update(json!({"sessionUpdate":"tool_call_update","toolCallId":"report-1","status":"completed"})));
        assert_eq!(reports_evidence(&events, "session"), (false, true));
        events.push(update(json!({"sessionUpdate":"tool_call_update","toolCallId":"report-1","status":"failed"})));
        assert_eq!(reports_evidence(&events, "session"), (false, true));
        events.push(update(json!({"sessionUpdate":"tool_call_update","toolCallId":"report-1","status":"completed","rawOutput":{"content":[{"type":"text","text":"{\"items\":[],\"nextOffset\":null}"}],"isError":false}})));
        assert_eq!(reports_evidence(&events, "session"), (true, true));
        assert_eq!(reports_evidence(&events, "other-session"), (false, false));
        events.push(update(json!({"sessionUpdate":"tool_call","toolCallId":"shell-2","title":"shell","status":"completed"})));
        assert_eq!(reports_evidence(&events, "session"), (false, true));
    }
    #[test]
    fn permission_requires_correlated_exact_server_tool_and_empty_arguments() {
        let call = json!({"sessionUpdate":"tool_call","toolCallId":"report-1","rawInput":{"server":"suneiro-delegation","tool":"suneiro_reports","arguments":{}},"title":"mcp.suneiro-delegation.suneiro_reports"});
        let request = json!({"toolCallId":"report-1"});
        assert!(reports_permission(&[update(call.clone())], "session", &request));
        assert!(!reports_permission(&[update(call.clone())], "other", &request));
        assert!(!reports_permission(&[update(call.clone())], "session", &json!({"toolCallId":"other"})));
        let mut other = call.clone();
        other["rawInput"]["server"] = "other".into();
        assert!(!reports_permission(&[update(other)], "session", &request));
        let mut other = call.clone();
        other["rawInput"]["tool"] = "suneiro_create_task".into();
        assert!(!reports_permission(&[update(other)], "session", &request));
        let mut other = call.clone();
        other["rawInput"]["arguments"] = json!({"offset":1});
        assert!(!reports_permission(&[update(other)], "session", &request));
        let mut missing = call;
        missing["rawInput"].as_object_mut().unwrap().remove("arguments");
        assert!(!reports_permission(&[update(missing)], "session", &request));
    }
}
