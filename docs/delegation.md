# Agent delegation

Delegation is a Suneiro task protocol, not an ACP extension. ACP starts a supported
**stdio MCP server** for each authenticated Suneiro session. The desktop executable
handles `--suneiro-delegation-mcp` before starting Tauri. The same shim is available
as the `suneiro-delegation-mcp` core binary for other trusted hosts.

The bridge is Unix-only (macOS/Linux): a random temporary directory (0700), Unix
socket (0600), and a random rotating capability bound to one Suneiro session. No
TCP listener, network endpoint, caller-selected session, workspace path or arbitrary
command is accepted. Capabilities are revoked when sessions close. This scopes the
API; it is not an OS sandbox against another process running as the same user.
Other core hosts explicitly opt in with `Core::set_delegation_executable`; they must
implement the MCP flag. Users can always open related conversations from the tab bar
and work through ordinary chat, including when MCP tools are unavailable. Tool
availability must be verified in the actual adapter: accepting an ACP session is not
proof a model used a tool. No provider-parity claim is made by fixture tests.

## Operations

- `suneiro_create_task`: stable `requestKey`, `agentId`, optional `model`/`effort`,
  explicit `brief`, `acceptanceCriteria`, `selectedContext`, `relation` child/sibling.
  `isolation` defaults to `isolated`; `same_workspace` is explicit shared-file opt-in.
  `base` defaults to `caller_head`: checkout is pinned to the caller's committed HEAD.
  `repo_default` explicitly starts at the repository default branch. Neither copies
  uncommitted parent files. The comparison/PR base remains the repository base branch.
  Normal repository worktree copy/setup configuration still applies.
- `suneiro_task_status`: only the creator or assigned session can inspect a task.
- `suneiro_send_message`: durable participant mailbox; never sends or steers a prompt.
- `suneiro_messages`: read the session's mailbox as untrusted data, not instructions.
- `suneiro_cancel_task`: creator-only, cancels owned descendants and closes their agents.
- `suneiro_finish`: assigned session only; immutable evidence/report. End-of-turn is
  **not** task completion. A failed agent also records a terminal failure report.
- `suneiro_reports`: inspect assigned/created tasks and durable reports. Mailbox tools
  return `{items,nextOffset}`; use `offset` until `nextOffset` is null.

Ask for related work through chat. Agent-created tasks appear as compact **Chat updates**
with progress, results, **Open chat** and **Stop**. The normal composer and model picker
remain the interface for each conversation. Browser previews use clearly identified
sample data and never pretend to launch real agents.

## Reliability and limits

SQLite reserves `(creator, requestKey)` before side effects; reuse with different
arguments is rejected. Duplicate calls return the existing task, including after
restart. Prompts are literal text (`Agents::prompt_brief`), so `@path` text cannot
implicitly attach files or pick up queued notes. Context is not a native provider
session transfer.

A task has one immutable report row, written atomically with its terminal state.
Repeated identical finish succeeds; conflicting completion/cancellation cannot
replace the first report. Reports survive closed chats and process restart. Live
`DelegationChanged {session_id,task}` events update both participants. Startup
`Core::reconcile_delegation_reports` re-emits terminal records so durable inbox
observers can repair a crash between report commit and inbox insertion; consumers
must deduplicate by task ID. Reconciliation never prompts an agent.

In-flight tasks become `interrupted` on restart. They retain their original IDs and
consume concurrency until explicitly finished/cancelled. Nothing is auto-retried or
auto-resumed, including uncertain launch windows. A crash between creating a
workspace/session and binding the task can leave an orphan visible workspace; it
must be inspected manually, not blindly relaunched.

Limits: four unfinished tasks per repository; delegation ownership-chain depth four
and configured tab depth; 64 launch keys per creator; 64 KiB each brief, acceptance
criteria, selected context and report; 128 messages per task, 8 KiB per message.
Bridge frames are bounded to 1 MiB, 16 concurrent connections, and timed socket
reads/writes. Task/report mailbox pages contain two records; message pages contain 64.

## Verification

```sh
cargo test -p suneiro-core delegation -- --test-threads=1
cargo test -p suneiro-core --test delegation -- --test-threads=1
# Opt-in, authenticated real agent and billable tokens; deliberately ignored by default:
SUNEIRO_DELEGATION_AGENT=codex SUNEIRO_DELEGATION_MODEL=gpt-6-luna SUNEIRO_DELEGATION_EFFORT=low \
  cargo test -p suneiro-core --test delegation real_adapter_can_use_local_mcp_tools -- --ignored --nocapture
SUNEIRO_DELEGATION_AGENT=opencode SUNEIRO_DELEGATION_MODEL=opencode/big-pickle \
  cargo test -p suneiro-core --test delegation real_adapter_can_use_local_mcp_tools -- --ignored --nocapture
```

Deterministic tests cover boundaries/defaults, duplicate launches, owner scope,
immutable completion, cancellation races, restart/no replay, depth/concurrency,
socket permissions, invalid/revoked/rotated tokens, caller spoofing, actual ACP
MCP configuration and subprocess shim, isolated HEAD inheritance, literal context,
shared-workspace siblings and manual/API cancellation. The ignored real-adapter test
is a separate capability check, not evidence of universal adapter support.

### Live read-only reports qualification

Bounded real runs in disposable empty git fixtures passed for these selected models:

- **Codex ACP 2.0.0, `gpt-6-luna`, effort `low`:** ACP `session/update`
  emitted `tool_call`, title `mcp.suneiro-delegation.suneiro_reports`, with
  `rawInput = {server:"suneiro-delegation",tool:"suneiro_reports",arguments:{}}`.
  A correlated permission request was granted **allow_once** for that exact session,
  toolCallId, server, tool and empty arguments only. Its `tool_call_update` completed
  with `rawOutput.result.content` containing `{"items":[],"nextOffset":null}`.
- **Native OpenCode, `opencode/big-pickle`:** ACP `session/update` emitted
  `tool_call`, title `suneiro-delegation_suneiro_reports`; the matching
  `tool_call_update` completed with both ACP text content and `rawOutput.output`
  equal to `{"items":[],"nextOffset":null}`. No permission request was emitted.

Both then emitted assistant text chunks concatenating to exactly
`BRIDGE_REPORTS_OK`. Each successful run contained one reports tool call and no
other tool calls. The real fixture stays in plan/read-only mode, permits at most
one exact correlated reports `allow_once`, declines all other permissions/forms,
bounds the turn to 150 seconds and shuts down on success, failure or panic.

An initial Codex harness run denied every permission, so its real reports call was
cancelled and returned `BRIDGE_REPORTS_UNAVAILABLE`. The stricter oracle correctly
failed it; one explicitly authorized corrective retry used the narrow allowlist
above. No quota retry or broad auto-accept elevation was used. Claude was **not**
called in this qualification because its known monthly quota was exhausted.

The oracle requires a real named ACP tool call, matching completed toolCallId,
actual empty reports result and exact assistant marker. A tool name echoed in a
prompt or assistant response cannot pass. Deterministic regressions cover text
false positives, failed/pending or mismatched tool IDs, another session/tool,
extra tool calls, and the exact permission allowlist.

These observations qualify the **reports bridge path only** for those selected
adapters/models. They do not qualify live create/cancel/finish, other models,
Windows, or universal provider parity; those lifecycle operations remain covered
by deterministic bridge fixtures and direct core API tests.
