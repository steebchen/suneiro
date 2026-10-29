# Reusable profiles and explicit context handoff

Open **Profiles / handoff** beside the new-chat button in a workspace. Implementer, Reviewer and Investigator are included. Edit names/instructions, add/delete profiles, and save model/effort preferences separately for Claude, Codex and OpenCode. Profiles live in core SQLite settings (`agent_profiles:v1`), not localStorage or provider configuration.

Enter a task, optionally add acceptance criteria, select individual source messages, and list individual workspace-relative file/diff paths (one per line). Nothing is selected automatically. Preview shows the entire frozen brief plus resolved provider/model/effort and any unsupported-preference warnings. **Start new conversation** sends that exact brief in a new chat. Source conversations remain untouched; native session identifiers and hidden agent state are never transferred.

## Safety boundary

- Maximum 32 selected items; 16 KiB per text item; 64 KiB for the complete UTF-8 brief, including profile instructions and headers. Profile instructions have a separate 8 KiB limit. Oversize input is rejected, never truncated.
- Only selected user/assistant text messages are available. Thoughts, tool output, images and blanket transcript copies are excluded. Messages larger than 16 KiB are not selectable. Source conversations must belong to the selected workspace.
- Files/diffs use literal, relative paths inside that workspace. No directory or wildcard expansion, traversal, absolute paths, outside symlinks or secret-path aliases. `.git`, `.env`, `.env.*`, `.envrc`, `.ssh`, `.aws`, credentials/secrets and common key files are blocked, even when explicitly entered. This intentionally also excludes `.env.example`.
- Each selected existing file must be regular UTF-8 text and at most 16 KiB, including files selected for diffs. Diffs are bounded reads with external diff/textconv disabled and a 10-second timeout. Untracked text files can be diffed; unchanged tracked files are rejected.
- Preview snapshots do not change if files, messages or profiles change afterward. One outstanding preview per workspace is kept in SQLite (`context_preview:<workspace>`); newer previews invalidate older ones. Launch atomically consumes it before creating the session, so double launches cannot replay it. Startup errors stay in the new conversation; users must explicitly preview again for a retry.
- `Agents::prompt_brief` sends one literal text block. Ordinary `prompt` expands `@path` mentions and must not be used for approved handoffs.
- This is a context-sharing boundary, **not a sandbox**. New agents retain their normal workspace/filesystem/tool permissions. Selected text itself may contain secrets; users must inspect the preview. Repository instructions and provider system prompts are separate from the shared brief.

## Core integration

`crates/core/src/profiles.rs` implements methods on `Core`:

- `profiles() -> Result<Vec<AgentProfile>>`
- `save_profiles(Vec<AgentProfile>) -> Result<()>`; emits `ProfilesChanged` after saving.
- `context_messages(workspace_id, session_id) -> Result<Vec<ContextMessage>>`; indices reference persisted source events.
- `prepare_context(workspace_id, profile_id, agent_id, ContextSelection) -> Result<ContextPackage>` (async).
- `launch_profile(workspace_id, package_id) -> Result<Session>`; starts a new **root** session through `Core::create_session`, then `Agents::prompt_brief`.

The profile instructions/task/criteria/excerpts are provider-neutral ACP text. Model and effort preferences are used only when that provider's cached ACP catalog advertises the exact values. Unsupported or undiscovered preferences produce preview warnings and use provider defaults; no model identifiers are translated or guessed. Capability changes after preview are subject to the existing agent startup/config error path, not silent remapping.

Thin commands are isolated in `apps/desktop/src-tauri/src/profiles.rs`; frontend contracts/API in `src/lib/profiles.ts`; UI in `src/components/ProfileDialog.tsx`; browser fixtures in `src/dev/profiles.ts`. Central wiring is limited to module/command registration, one event branch, and the launch button. Hierarchy integration should preserve root creation (handoff is not automatically a child/delegation).

## Verification

- `cargo test -p suneiro-core -- --test-threads=1`
- `cargo clippy -p suneiro-core --all-targets`
- `pnpm typecheck`
- `pnpm --filter desktop test:profiles` (Node 22.6+ type stripping)
- `pnpm --filter desktop build` (frontend-only Vite build; succeeds with existing CSS `::highlight`/chunk-size warnings)
- `pnpm --filter desktop dev --host 127.0.0.1 --port 5147`

Rust coverage includes profile persistence/validation, single-use preview storage, capability preference resolution, path/secret/symlink/byte bounds, explicit source selection, stale previews, files/diffs and frozen packages. A scripted ACP peer verifies that all three provider IDs receive exactly one approved text block (including literal `@.env`) in a new session and that original native session IDs are unchanged. These are **fixture checks**, not real-provider smoke tests.

Mock tests cover editing persistence within the mock process, events, selections/limits and exact new-conversation payloads. Browser verification exercised saved profile editing, model/effort preferences, selecting one message/file/diff, preview-to-chat byte-for-byte equality, light/dark token colors, modal Escape/focus restoration and rejection of secret/outside/oversized selections on the isolated port. Screenshot capture was unavailable in the embedded browser; DOM interaction, numeric layout and computed-color checks succeeded.

Whole-workspace Tauri gates are deferred to the parent integration lane because concurrent builds exhausted disk space. The initial parallel core suite exposed an existing fixed-300ms persistence-test race; the complete serial core suite passed. Core clippy has only the existing Rust 1.99 `recent.rs:106` `sort_by_key` warning, fixed independently by the parent.
