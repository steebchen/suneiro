# Claude compatibility

Suneiro starts the pinned `@agentclientprotocol/claude-agent-acp@0.83.0` adapter through `npx`. The adapter requires **Node.js 22+** and npm/npx on the user's login-shell PATH, as well as the official Claude Code CLI. Claude keeps its own login; Suneiro does not copy, store, or proxy credentials. Install/login remain `claude auth login` and the official CLI installer.

## Read-only checks

```sh
cargo run -p suneiro-core --example claude_smoke -- --check
cargo run -p suneiro-core --example claude_smoke -- --catalog
```

`--check` (also the default) runs CLI version/auth and Node/npx probes, without a model prompt. JSON authentication is parsed from stdout only; warnings on stderr must not turn an authenticated account into a false logout. Failed, malformed, or logged-out probes never expose stale account details. A timed-out probe is killed. Unconfirmed authentication is not proof that credentials are absent: run `claude auth status --json` yourself to diagnose the CLI.

`--catalog` additionally starts the actual pinned ACP adapter, initializes a plan-mode session in a disposable git repository, and reads the advertised model/effort catalog. First use may download the pinned adapter via npm. It sends no model prompt. Node/npx preflight cannot prove adapter startup, network access, model entitlement, or a successful model turn.

## Explicitly opt-in, billed smoke suite

```sh
cargo run -p suneiro-core --example claude_smoke -- --run
# Optional advertised model and effort, applied model-first:
SUNEIRO_CLAUDE_MODEL=opus SUNEIRO_CLAUDE_EFFORT=high \
  cargo run -p suneiro-core --example claude_smoke -- --run
```

The suite uses a fresh temporary git repository, database and workspace. No user repository is opened, no remote is configured, no PR is created, and no login command is run. Real Claude sessions still appear in Claude's own history. Model prompts use the user's current CLI account and can incur charges. The suite automatically answers only its own fixture question and explicitly authorizes tools only within its disposable workspace; normal user questions remain interactive. Existing Claude user settings/hooks still apply, so use a trusted CLI configuration.

Assertions: initialization and selected model/effort, a plan-mode prompt, a structured question in auto mode, an auto-approved Write tool and tool updates, process restart/resume with remembered context and no replay duplication, persisted USD usage, and cancellation of a disposable `sleep` tool. Each wait is bounded; failure shuts down the agent. Model behavior is nondeterministic: a failure to use the requested tool/question is reported as a failed smoke assertion, not silently passed. Live image recognition and live steering are not asserted by this suite; deterministic wire contracts below cover image negotiation and cancellation fallback, but cannot prove real provider behavior.

## Recorded real-agent verification (2026-10-29)

Claude Code 2.1.312, Node 24.2.0, npx 11.19.0, pinned ACP adapter 0.83.0: authenticated CLI checks and real adapter startup/catalog passed. `--run` with advertised `haiku` reproduced the startup mode/model ordering issue described below; after the fix, initialization, requested Haiku, and plan config passed. The first real model prompt then failed with Claude's **monthly spend limit** error. The suite aborted immediately without retries. No real text reply, question, tool execution, resume, usage, or cancellation pass is claimed from that run. Remaining live checks require renewed provider quota; synthetic tests do not replace them.

## Offline regression coverage

```sh
cargo test -p suneiro-core --test claude_compatibility
cargo test -p suneiro-core --lib setup::tests
```

The scripted `tests/fixtures/claude_acp.mjs` agent is a **synthetic contract fixture**, not a captured or real Claude run. It covers:

- Model-dependent grouped effort options; reply-only config changes update session persistence, catalog, and the model attached to usage.
- Claude startup applies permission mode before model/effort. Real adapter probes showed that selecting Haiku and then entering plan mode can switch the model to Sonnet; entering plan mode before selecting Haiku preserves the explicit model choice. The fixture reproduces this reset for plan and auto startup.
- Auto permissions versus interactive plan permissions; complete tool updates; Claude-style elicitation schema and tool-call linkage.
- Cancellation resolving pending questions; unsupported steering cancels the running turn and sends a replacement prompt.
- Inline base64 image content only when advertised, otherwise a file resource link.
- Session/load replay suppression and cumulative resumed cost without double counting.
- Resume/load failure remains an error and preserves the existing conversation instead of silently starting a new one.

Transport ordering is a separate runtime invariant: updates preceding a prompt response must be consumed before usage is finalized. This fixture yields before the response; it does not prove that transport ordering invariant. Runtime reliability owns `acp.rs` and its ordering regression tests.
