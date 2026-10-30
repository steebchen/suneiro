# Chat hierarchy

Chats keep their workspace ownership and optionally reference a parent chat. A child may live in another workspace of the same repository; creating a child does not create or move a worktree.

- Existing chats migrate to roots, with their IDs, ACP conversations and history unchanged.
- New root chat (`+`, ⌘T), Child chat and Sibling chat are separate actions. The related-chat menu also offers existing workspaces in the same repository.
- The tree shows children and grandchildren, a breadcrumb to the selected chat, and input/running/error/unread indicators rolled up from descendants. Collapse state stays on the device.
- Arrow keys navigate/expand/collapse; Home/End move to the first/last visible row; Enter/Space opens a chat. Navigation retains keyboard focus rather than handing it to the composer.
- Detach makes a chat a root, retaining its descendants. Close deletes only that chat and detaches its direct children to roots. Grandchildren keep their surviving parents. The last chat in a workspace cannot be closed, including through core APIs.
- Switching agent/model creates a sibling with the same parent. An empty chat may be replaced only when it has no children and the new chat was created successfully.
- Settings → Chat hierarchy bounds new chats to 1–8 parent links (default 4, root depth 0). Lowering the limit does not delete or rewrite existing links.
- Workspace archiving does not delete relationships. Navigation displays available workspace chats; restore archived workspaces from Home to include their chats again.

## Core and event contract

`Session.parent_session_id: Option<String>` serializes as nullable `parentSessionId`. Migration adds nullable `sessions.parent_session_id` plus the parent index; flat sessions remain roots.

```rust
Core::create_child_session(parent_session_id, workspace_id, agent_id, model, effort) -> Result<Session>
Core::create_sibling_session(sibling_session_id, workspace_id, agent_id, model, effort) -> Result<Session>
Core::detach_session(session_id) -> Result<Session>
Core::delete_session(session_id) -> Result<()>
Store::add_session_with_depth(&Session, max_depth: u8) -> Result<()>
Store::subtree_session_ids(root) -> Result<Vec<String>>
```

IDs are `&str`, model/effort are `Option<String>`. Parent validation and insertion share one SQLite transaction: missing parents/workspaces, cross-repository ancestry, cycles and depth overflow fail without inserting a session. `subtree_session_ids` includes the root and descendants across workspace boundaries.

Tauri commands are `create_child_session`, `create_sibling_session`, `detach_session`; wrappers are `api.createChildSession`, `createSiblingSession`, `detachSession` with the same argument order. Existing root creation stays unchanged.

Events:

- `SessionAdded { session }` / `sessionAdded`: emitted after insertion, before agent warm-up.
- `SessionHierarchyChanged { session_id, parent_session_id }` / `sessionHierarchyChanged`: detach, including each direct child of a closed parent.
- `SessionRemoved { session_id, workspace_id }` / `sessionRemoved`: the closed chat only.

The UI deduplicates creation responses/events and changes only affected workspace arrays. Tree rows subscribe to scalar aggregate statuses, not transcripts or whole session views. Streamed transcript chunks do not rerender the tree.

## Verification

```sh
cargo test -p suneiro-core --test tab_hierarchy
pnpm typecheck
pnpm --filter desktop dev --host 127.0.0.1 --port 5145
```

Open `http://127.0.0.1:5145/?hierarchy` for a nested/cross-workspace mock fixture. Mock mutations use the same limits and non-cascading close semantics; they reset on page reload, unlike the real SQLite backend.

Optional reproducible browser smoke (Playwright runs outside the app's production dependencies):

```sh
pnpm dlx playwright install chromium
PLAYWRIGHT_BIN="$(pnpm dlx --package playwright -- which playwright)"
PLAYWRIGHT_MODULE="$(dirname "$PLAYWRIGHT_BIN")/../playwright/index.mjs" \
  SUNEIRO_UI_URL=http://127.0.0.1:5145 node scripts/tab-hierarchy-smoke.mjs
```

The smoke exercises creation, grandchildren, siblings, cross-workspace navigation, model-switch preservation, non-cascading close, detach, depth and last-chat protection, keyboard focus, collapse/expand, descendant input/unread rollup, light/dark screenshots and 1024/1280px tree bounds. Screenshots default to `/tmp/suneiro-tab-hierarchy/` (`SUNEIRO_SCREENSHOTS` overrides this).
