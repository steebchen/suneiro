# Import from Conductor

`crates/core/src/conductor.rs` (scan, transcript conversion), `Core::conductor_scan` / `Core::conductor_import` in `lib.rs`, and `components/ConductorImport.tsx` (dialog and first-run hint).

## What is read

| Source | Used for |
| --- | --- |
| `~/conductor/workspaces/<repo>/<name>` | Candidate worktrees. Renamed workspaces are symlinks to the city-named folder; both resolve to one entry. |
| `<worktree>/.git` → `<git dir>/HEAD`, `commondir` | Branch and main checkout (the repository Suneiro adds). Read as files; git is never run inside the worktree. |
| `~/Library/Application Support/com.conductor.app/conductor.db` | Titles (`workspace_name`, `pr_title`, chat titles), base branch, archived state, last activity, repo scripts, chats. |
| `.conductor/settings.toml`, `settings.local.toml`, `conductor.json` | Scripts, read by the normal repo config loader for every workspace of the repo. |

The database is opened with `SQLITE_OPEN_READ_ONLY`, `immutable=1` and `query_only`, so SQLite takes no locks and never creates or writes `-wal`/`-shm` files. Missing columns are read as `NULL`, so other Conductor schema versions degrade to less metadata rather than failing. Without a database, worktrees are still listed (title = branch).

## What an import creates

- The repository (main checkout from `commondir`), if Suneiro doesn't have it yet.
- A workspace with `path` = the existing worktree, `branch` = its checked-out branch, status `ready`, a non-empty title (so the first prompt never renames the branch), and a `workspace_imports(source='conductor', key=<canonical worktree path>)` row, all in one transaction.
- Up to 20 most recent chats with a user message (hidden chats skipped), each capped to its latest 4000 transcript events. User prompts, assistant text and tool calls (title, kind, trimmed input, failed status) become normal transcript events; thinking and tool output are dropped. A note at the top says the agent doesn't see the history: there is no `acpSessionId`, so continuing starts a new agent session. Agents map `claude`→Claude, `codex`→Codex, OpenCode ACP sessions→OpenCode.
- Repo scripts from Conductor's database go into Settings → Repositories when the repo has neither app scripts nor a committed config file.

Importing is idempotent: a worktree already imported (by key) or already used by any Suneiro workspace (by path, archived ones included) is skipped.

## Imported workspaces afterwards

Archiving only hides the workspace: no archive script, no stash, no `git worktree remove`. Restoring shows it again if the worktree still exists. Everything else (chat, diff, PRs, terminals, checkpoints under `refs/runner/`) works as for any workspace; edits made by agents are edits to that worktree, which Conductor would also see.

## Not importable (listed with a reason)

- Worktrees whose git dir is gone, usually because the main repository moved. `git worktree repair <path>` in the repository fixes the `.git` file; Suneiro never does this itself because it writes into Conductor's folder.
- Detached HEADs, plain clones (a `.git` directory), and worktrees of bare repositories.

## Known limits

- Changes still only in Conductor's WAL (not yet checkpointed into `conductor.db`) aren't visible until SQLite checkpoints them (normally when Conductor closes cleanly).
- Attachments, `.context/` notes, diff comments, terminals and Conductor Cloud workspaces aren't imported. Cloud workspaces need Conductor's "Transfer to local" first.
- Model choices aren't carried over; imported chats use each agent's default.

## Checks

```sh
cargo test -p suneiro-core --lib conductor                 # parsing, worktree inspection, transcript conversion
cargo test -p suneiro-core --test conductor_import         # fixture install: read-only proof, idempotency, archive/restore
cargo run -p suneiro-core --example conductor               # read-only listing of this machine's Conductor workspaces
cargo run -p suneiro-core --example conductor -- --import /tmp/suneiro-trial   # import into a throwaway data dir
```

The fixture test snapshots every file, symlink and mtime under the fake `~/conductor` and app-data folders before and after scan, import, re-import, archive and restore, and requires them to be identical.
