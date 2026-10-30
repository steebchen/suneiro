# Codex compatibility

Suneiro runs the pinned `@agentclientprotocol/codex-acp@2.0.0` adapter through `npx`. The official `codex` CLI and adapter use the user's existing Codex login. Suneiro does not read, copy, store, or proxy credentials. Install Node/npm and the official CLI, then run `codex login`; `codex login status` diagnoses authentication separately from adapter startup.

## Provider-specific behavior

- Authentication detection requires a successful CLI status and a recognized ChatGPT/API-key status line. Account labels are allowlisted (`ChatGPT` / `API key`), never the raw CLI line: API-key status can contain part of a key. Only ChatGPT login was checked live; API-key parsing is tested with synthetic text.
- Current adapter config separates `model` and `reasoning_effort` (`thought_level`). Legacy saved model IDs such as `model[low]` are split, with explicitly selected effort taking precedence. Apply model first, then validate effort against the model's newly advertised choices. Unsupported selections fail visibly, including retry, instead of silently using another model. Unsupported selections can be corrected in the picker or a new chat.
- Plan mode sets **both** `mode=read-only` and `collaboration_mode=plan`. Collaboration mode alone is not a sandbox. Auto-accept uses `mode=agent-full-access` and, when advertised, `collaboration_mode=default`, consistent with Suneiro's auto-accept policy. If required plan options are missing or rejected, report the incompatibility and disconnect; never send a prompt on the old full-access connection.
- Permission requests go to the user in plan mode; ordinary auto-accept can select an advertised allow option. Structured form questions always require the user, including in auto-accept mode. Codex's question auto-resolution metadata is forwarded, not interpreted as permission to invent an answer. Unsupported URL-mode elicitation is declined.
- Steering uses `_session/steering` **only when advertised**. Otherwise stop the turn and submit a tracked prompt. Cancellation resolves pending form questions and permissions through the shared session runtime.
- Process restart uses advertised ACP resume/load support and retains the conversation ID. Successful continuation is covered here; rejected resume and connection-lifecycle behavior belong to shared runtime tests. A failed continuation must not silently become an empty conversation.
- Prompt images follow `promptCapabilities.image`; unsupported images are passed as file resource links. This is a transport capability, not a guarantee that every model supports vision.
- The pinned adapter reports **per-turn** uncached input, cached reads, output, and reasoning separately in prompt results. Do not subtract cached tokens from `inputTokens` or difference consecutive prompt usage reports. Reasoning is included in Suneiro's stored output total. `usage_update.used` is context occupancy, not billable turn usage.
- A single `_meta.quota.model_usage` row identifies the model actually billed and takes precedence over session config. Multiple model rows are not falsely attributed to the first row; Suneiro retains the configured model and cannot currently split one turn into multiple model-priced rows. Codex has no reported USD cost in the checked adapter. Estimates use the user's per-million model prices at read time; without a matching price, cost remains unknown (not zero).

## Deterministic checks

```sh
cargo test -p suneiro-core --test codex_compatibility
cargo test -p suneiro-core codex::tests
```

`tests/fixtures/codex/session.json` and `agent.mjs` reproduce the config shape observed from adapter 2.0.0, using synthetic model names and scripted responses. They are **not** real-agent evidence. Coverage: legacy model/effort ordering and validation; read-only plan vs auto-accept; permission denial; structured answer/cancellation; unsupported URL elicitation; supported/unsupported steering; image block vs resource-link fallback; restart continuation without duplicate usage; separate cached/reasoning tokens; estimated cost; reported-model precedence; unsupported plan config; and API-key-safe login parsing.

The fixture's cached-token values come from an observed second prompt (193 uncached input, 23936 cached reads, 5 output). Its 7 thought tokens are synthetic to exercise reasoning accounting.

## Opt-in real smoke

This spends the user's model quota and writes a resumable session into Codex's normal session history. No credential files are opened or copied. All repository/data changes stay in temporary directories; the test never calls `gh`, creates external PRs, or touches a user's project. It requests no tools, keeps plan/read-only mode, and never approves a permission request.

```sh
SUNEIRO_CODEX_SMOKE=1 SUNEIRO_CODEX_MODEL=gpt-6-luna SUNEIRO_CODEX_EFFORT=low \
  cargo test -p suneiro-core --test codex_smoke -- --ignored --nocapture
```

Model and effort env vars are optional; only choose values advertised by the current adapter. The smoke verifies installed/authenticated CLI, model/effort application, read-only plan config, two real turns with tokens, then conversation recall across process restart. Failure is bounded by timeouts; core shutdown runs on failure too.

Checked locally: **codex-cli 0.165.0**, **codex-acp 2.0.0**, existing **ChatGPT login**, model **gpt-6-luna**, effort **low**. Live two-turn core smoke passed, including exact marker recall after process restart. A separate direct ACP probe confirmed cached reads are separate from uncached input and `usage_update.used` is context occupancy. Real structured elicitation, permission denial, interruption, mid-turn steering, API-key authentication, image understanding, rejected continuation, and multi-model billing were **not** verified live; their listed coverage is deterministic or a documented limitation, not parity claims.
