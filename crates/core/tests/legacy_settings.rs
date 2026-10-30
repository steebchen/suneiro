use std::sync::Arc;

use suneiro_core::Core;

#[tokio::test]
async fn unknown_saved_settings_survive_core_reopen() {
    let data = tempfile::tempdir().unwrap();
    let saved = [
        ("agent_profiles:v1", r#"[{"id":"saved","name":"Saved","instructions":"Keep existing data","preferences":{}}]"#),
        ("context_preview:legacy-workspace", r#"{"id":"saved-preview","text":"Existing saved context"}"#),
        ("unrecognized-setting", "existing value"),
    ];
    {
        let core = Core::new(data.path(), Arc::new(|_| {})).unwrap();
        for (key, value) in saved {
            core.store.set_setting(key, value).unwrap();
        }
    }
    let reopened = Core::new(data.path(), Arc::new(|_| {})).unwrap();
    for (key, value) in saved {
        assert_eq!(reopened.store.setting(key).unwrap().as_deref(), Some(value));
    }
}
