# Verification and review

Open a workspace's **Checks** panel. Configure repository commands for tests,
lint, typecheck and build, then **Save commands**. Empty commands disable that
preset. Commands are stored in app settings (SQLite), not discovered from agent
output or automatically trusted from a checked-out repository. Running one is an
explicit user action and executes the configured shell command with the user's
login-shell environment, in the displayed workspace directory.

Only one check runs per workspace. Agents, checkpoint restoration, commits,
reverts, merges and archive operations cannot start over an active check. Cancel
kills the shell's process group, including background children on Unix (Windows
uses taskkill /T). Checks time out after 30 minutes. Each log retains at most
64 KiB, and each workspace retains the most recent 20 runs. Interrupted runs are
reported cancelled after restart.

Each run records its command, HEAD and a SHA-256 content fingerprint of tracked
and non-ignored untracked files, including deletions, executable permissions,
symlink targets and the staged diff. Modification times are not used. Results
become stale when that content, HEAD or preset changes. Staleness observed during
a run remains stale even if files are changed back. The visible panel refreshes
freshness every two seconds; integration always checks freshness again. Ignored
files such as dependencies and local environment files are not fingerprinted;
checks are evidence about repository inputs, not a hermetic build guarantee.
Submodules and non-regular inputs are rejected rather than falsely verified.

In **Changes**, select files and launch a reviewer as a child of the selected
conversation (subject to the configured hierarchy depth). The core requires an
advertised and acknowledged restricted agent mode. A read-only mode is described
as **agent-enforced read-only**; a plan mode is explicitly described as **not
sandboxed**, not as read-only. All ACP permission requests from reviewers are
denied and configuration cannot be elevated. Agents without a supported mode
fail closed. These restrictions depend on the agent honoring its capabilities;
they are not an operating-system sandbox. The selected diff is bounded to
256 KiB and passed literally, with no implicit @-file expansion. Real-provider
smoke coverage belongs to each provider's compatibility workstream; this feature
also has a scripted ACP restriction/permission-denial test.

Select failed checks and/or paste only the reviewer findings you want fixed,
then **Send findings as new task**. This creates and starts an ordinary task chat
using the default loadout, with only the selected context. Agent output is never
executed as a check command. These task chats are not a separate durable
structured delegation ledger; the delegation workstream can wrap this entrypoint.

**Integrate branch** previews a source workspace from the same repository.
Both source and target must be clean and idle, with no merge conflicts, and every
configured source preset must have a current passing result (at least one
configured preset is required). Applying recomputes the preview under ordered
workspace locks and rejects changes to either HEAD or content. Git merges only
the reviewed source commit, with an explicit merge commit and no overwrite or
force/reset behavior. Ignored local files that would be replaced are listed as
conflicts, and `--no-overwrite-ignore` provides Git's final overwrite guard.
Failed checks, stale results or conflicts block apply;
resolve them and preview again. External editors are not locked by Suneiro; git
still refuses local-file overwrites. Checks should be rerun on the merged result.
