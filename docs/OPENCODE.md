# Native OpenCode compatibility

Suneiro starts the installed `opencode acp`; it does not install an adapter or read,
copy, or store provider credentials. Configure providers using OpenCode's own
`opencode auth login` or documented environment variables. `opencode auth list`
and `opencode models` diagnose the local setup. A saved login is not required for
free/local models. An advertised model is not proof that its provider credentials,
quota, endpoint, or image support work; the provider validates those when used.

## Compatibility boundary

- Discover provider-qualified model IDs from ACP `configOptions`; keep the full ID
  (including extra `/` segments). Never substitute a static list or another model.
- A removed model, empty provider catalog, or unsupported preset produces an
  actionable session error instead of silently running OpenCode's default model.
  Picking a valid model recovers a chat with a stale initial preset.
- Only controls the session advertises can be changed. OpenCode 1.19.4 advertised
  model and build/plan mode, not reasoning effort or fast mode. UI capability
  explanations are integrated by the shared runtime/UI workstream.
- Images follow `promptCapabilities.image`; otherwise attachments become file
  links. This is agent transport support, not proof that every provider model has
  vision. Structured questions, when emitted as ACP elicitation, always await the
  user, even with automatic permission acceptance.
- Steering is negotiated separately. Without it, Suneiro cancels the active turn
  and submits a new prompt. Reconnect prefers advertised resume, otherwise load;
  replayed updates are suppressed so stored transcript history is not duplicated.
- Native OpenCode protocol versions without `configOptions` are not supported by
  this boundary. Upgrade OpenCode; no legacy model-switching methods are guessed.

## Deterministic tests (no provider requests)

```sh
cargo test -p suneiro-core --lib opencode
cargo test -p suneiro-core --test opencode_compatibility
```

The initialize fixture records the capability shape observed from OpenCode
1.19.4. Session fixture preserves that response structure with a small synthetic
model list; the scripted agent is a mock, not evidence of provider compatibility.
Tests cover environment-only provider setup, free/local readiness, absent models,
stale presets/recovery, unsupported controls, full model IDs, tool events,
questions, image/link negotiation, cancellation/continuation and resume/load replay.

## Opt-in real CLI checks

Tests create their own disposable git repositories and terminate the CLI on
success, error or timeout. No GitHub operations or credential copies occur.

Discovery and model switching send **no inference prompt**:

```sh
cargo test -p suneiro-core --test opencode_smoke native_opencode_discovery_and_controls -- --ignored --nocapture
```

Provider smoke uses your existing login/environment and **may incur charges**.
Choose an ID printed by `opencode models`; the model is never selected implicitly:

```sh
SUNEIRO_OPENCODE_MODEL=provider/model cargo test -p suneiro-core --test opencode_smoke native_opencode_prompt_tools_and_continuation -- --ignored --nocapture
SUNEIRO_OPENCODE_MODEL=provider/vision-model cargo test -p suneiro-core --test opencode_smoke native_opencode_images -- --ignored --nocapture
SUNEIRO_OPENCODE_MODEL=provider/model cargo test -p suneiro-core --test opencode_smoke native_opencode_questions -- --ignored --nocapture
```

Independent tests request a file read with cancellation/continuation, and image
transport when negotiated. Permission requests for shell/edit/network tools are rejected.
Structured questions are cancelled if emitted; their observed count is reported,
not assumed to establish real OpenCode question support. Image completion is not
a semantic vision accuracy test. Provider unavailability is a failed real smoke,
not a passing mock. A provider finishing before cancellation makes cancellation
explicitly unverified. Normal `cargo test --workspace` skips all real tests.

## Observations on this development machine

- Installed OpenCode: **1.19.4**.
- Real initialize/new, discovery and valid/invalid model switching passed.
- Negotiated image, embedded context, resume and load. No steering metadata;
  controls were model and mode. This does not establish provider inference support.
- Real `opencode/big-pickle` provider prompt exercised a read tool successfully;
  cancellation was acknowledged and a subsequent continuation completed.
- A valid 32×32 PNG image prompt completed on that model. Semantic vision accuracy
  is not asserted. An earlier malformed PNG was rejected; input corruption must
  not be mistaken for absent provider vision support.
- Explicit real question-tool prompt emitted no ACP elicitation. The standalone
  question smoke fails as unavailable/unverified; do not claim native structured
  question support for this CLI/model combination. Plain text is not a form.
- Paid/private provider authentication and missing-credential responses were not
  exercised against real providers. Authentication-error guidance has deterministic
  regressions; no credentials were copied or stored.
- Deterministic tests cover tools, forms, image fallback, cancellation and replay
  independently from the opt-in real-provider result.
