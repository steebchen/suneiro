# Admission and notification internals

Questions and permissions are answered directly in chat. Tab activity and normal
notifications identify conversations that need attention; related results appear in
chat updates. This document describes internal admission, notification and persisted
policy compatibility, not a separate desktop management interface. Existing saved
pause/spend policies are preserved. When they block replies, a contextual recovery
notice requires an explicit user action before clearing them.

## Repository admission

`RepoControls` are durable SQLite settings under `supervision:<repo id>`:

- `maxConcurrent`: 1–64 concurrent **agent turns**, default 4. Warm connections are not turns.
- `paused`: prevent new turns, without interrupting active ones.
- `warningUsd`: optional warning on observed reported + estimated spend.
- `stopUsd`: optional refusal of new turns once that observed amount is reached.
- `spendSince`: inclusive lower timestamp bound for repository spend, default all history.

Every `Agents::prompt_with` call, including calls outside the desktop UI, acquires the
same admission mutex before setting running state or emitting a user message. Duplicate
session starts, repository capacity, pause and observed spend are checked under that
lock. Rejected starts are not queued in core. UI rejection restores composer contents;
there is no background retry. The permit stays held through usage recording and is
released on task drop/error. Existing human follow-up queues still require actual core
admission when they are sent.

These are **not an exact dollar cap**. Active requests can continue spending, usage may
arrive late, model prices are estimates and some turns have unknown cost. Pausing does
not kill processes. Stop thresholds do not reserve dollars or cancel already running
turns. Concurrency is process-local to the single Core instance; controls and usage are
durable, active permits intentionally are not resumed after restart.

## Usage

`Core::repo_resources(repo_id)` summarizes the repository's selected spend window.
`Core::subtree_resources(session_id)` uses `Store::subtree_session_ids` (including the
root, cross-workspace descendants exactly once) and all historical original usage rows.
Reported USD, estimated USD, unknown-cost turn counts and tokens are separate fields.
No aggregate is persisted or reapplied on restart, and child totals are never added to
already-inclusive parent totals. Each row remains one finished turn's usage.

## Durable attention and notifications

`supervision_inbox` stores open/resolved entries. Question and permission keys include
the admitted turn generation, so ACP request identifiers can safely repeat after reconnect.
Resolution, cancellation, failed/disconnected sessions and session removal clear stale
requests. At app restart requests tied to dead processes are retired; durable failures,
spend warnings and reports remain. Delegation calls
`Core::record_delegation_completion(task_id, session_id, report)` after a durable terminal
report, and reconciles terminal reports on startup; `delegation:<task_id>` is unique even
after dismissal. Normal agent idle/turn-end is **not** delegation completion.

Notification permission follows the existing Tauri plugin flow; permission requests are
coalesced, denial respected. Delivery claims are SQLite `INSERT OR IGNORE` keys, providing
at-most-once local notifications across event duplication and app restarts. Delivery is
best effort: a crash after a claim can lose an OS notification, but the inbox entry remains.
Reloading history or loading the inbox does not produce a notification storm. No push
service, credentials or external notification channel is involved.

## Interfaces and tests

Core events: `InboxUpdated { item }`, `InboxResolved { id }`,
`RepoControlsChanged { repo_id, controls }`. Tauri commands map directly to core:
`supervision_inbox`, `dismiss_inbox`, `claim_notification`, `repo_controls`,
`save_repo_controls`, `repo_resources`, `subtree_resources`.

Run `cargo test -p suneiro-core --lib --test supervision` for racing admission, persistent
policies, thresholds, request/notification deduplication, exact-once subtree rollups after
reopen and fake-agent end-to-end request/admission/report actions. The browser mock has
matching commands, core-like admission checks and live question inbox events.
