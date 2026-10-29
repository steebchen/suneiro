// Run without frontend dependencies: node --test apps/desktop/src/dev/verification.test.mjs
import assert from "node:assert/strict";
import { test } from "node:test";
import { verificationMock } from "./verification.ts";

function fixture(scenario = "") {
  const events = [];
  const workspaces = [
    { id: "w1", repoId: "r1", path: "/mock/target", archivedAt: null },
    { id: "w2", repoId: "r1", path: "/mock/source", archivedAt: null },
    { id: "w3", repoId: "r2", path: "/mock/other", archivedAt: null },
  ];
  const sessions = {};
  const backend = verificationMock({ workspaces, sessions, scenario, emit: (...batch) => events.push(...batch) });
  return { ...backend, events, sessions, api: backend.handlers };
}

test("commands are explicit repo app settings; first tests fail, retry passes, content edit stales results", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { api, events } = fixture();
  assert.deepEqual(api.verification_presets({ repoId: "r1" }), []);
  assert.throws(() => api.run_check({ workspaceId: "w1", presetId: "tests" }), /Configure/);
  api.save_verification_presets({ repoId: "r1", presets: [{ id: "tests", command: "pnpm test" }] });
  assert.deepEqual(api.verification_presets({ repoId: "r2" }), []);
  assert.throws(() => api.save_verification_presets({ repoId: "r1", presets: [{ id: "agent-suggestion", command: "unsafe" }] }), /Invalid/);
  const args = { workspaceId: "w1", presetId: "tests" };
  const first = api.run_check(args);
  assert.equal(first.status, "running");
  assert.equal(first.cwd, "/mock/target");
  t.mock.timers.tick(1000);
  let results = api.verification_status({ workspaceId: "w1" });
  assert.equal(results[0].status, "failed");
  assert.equal(results[0].exitCode, 1);
  assert.match(results[0].log, /Retry-After/);
  api.run_check(args);
  t.mock.timers.tick(1000);
  results = api.verification_status({ workspaceId: "w1" });
  assert.equal(results[0].status, "passed");
  assert.equal(results[0].exitCode, 0);
  t.mock.timers.tick(6000);
  assert.ok(api.verification_status({ workspaceId: "w1" }).every((r) => r.stale));
  assert.ok(events.some((e) => e.type === "verificationChanged" && e.run.stale));
  assert.equal(first.status, "running", "returned snapshots cannot be mutated by later events");
});

test("cancelled runs stay cancelled; workspace-scoped cancellation and integration running guards", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { api } = fixture();
  api.save_verification_presets({ repoId: "r1", presets: [{ id: "lint", command: "pnpm lint" }] });
  const run = api.run_check({ workspaceId: "w1", presetId: "lint" });
  assert.equal(api.integration_preview({ workspaceId: "w1", sourceWorkspaceId: "w2" }).canApply, false);
  assert.throws(() => api.cancel_check({ workspaceId: "w2", runId: run.id }), /not found/);
  api.cancel_check({ workspaceId: "w1", runId: run.id });
  t.mock.timers.tick(2000);
  const result = api.verification_status({ workspaceId: "w1" })[0];
  assert.equal(result.status, "cancelled");
  assert.equal(result.exitCode, null);
  assert.equal(api.integration_preview({ workspaceId: "w1", sourceWorkspaceId: "w2" }).canApply, false, "source checks are required");
  api.run_check({ workspaceId: "w2", presetId: "lint" });
  t.mock.timers.tick(1000);
  assert.equal(api.integration_preview({ workspaceId: "w1", sourceWorkspaceId: "w2" }).canApply, true);
});

test("review rejects unsupported agents and reports exact protection, with curated tasks preserved", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { api, sessions, histories } = fixture();
  assert.throws(() => api.start_review({ workspaceId: "w1", agentId: "codex", files: [] }), /Select/);
  const review = api.start_review({ workspaceId: "w1", agentId: "codex", files: ["src/file.ts"] });
  assert.equal(review.protection, "agent-enforced read-only");
  assert.throws(() => api.start_review({ workspaceId: "w1", agentId: "opencode", files: ["src/file.ts"] }), /does not support/);
  const fallback = api.start_review({ workspaceId: "w1", agentId: "claude", files: ["src/file.ts"] });
  assert.equal(fallback.protection, "plan mode; permissions denied; not sandboxed");
  assert.equal(review.session.agentId, "codex");
  const history = histories.get(review.session.id);
  assert.match(history[0].text, /src\/file.ts/);
  assert.match(history[0].text, /mutation permission requests are denied/);
  assert.ok(history.some((e) => e.type === "sessionMode" && e.plan));
  const text = "Fix only the selected Retry-After finding.";
  const task = api.verification_task({ workspaceId: "w1", text });
  assert.equal(histories.get(task.id)[0].text, text);
  assert.equal(sessions.w1.length, 3);
  t.mock.timers.tick(1000);
  assert.ok(history.some((e) => e.type === "turnEnd"));
});

test("merge validates repo, revision pair and current passing checks; creates a merge commit", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { api } = fixture();
  const args = { workspaceId: "w1", sourceWorkspaceId: "w2" };
  assert.throws(() => api.integration_preview({ ...args, sourceWorkspaceId: "w3" }), /same repository/);
  assert.equal(api.integration_preview(args).canApply, false, "no configured checks");
  api.save_verification_presets({ repoId: "r1", presets: [{ id: "lint", command: "pnpm lint" }] });
  api.run_check({ workspaceId: "w2", presetId: "lint" });
  t.mock.timers.tick(1000);
  const preview = api.integration_preview(args);
  assert.equal(preview.canApply, true);
  assert.throws(() => api.integrate_workspace({ workspaceId: "w1", preview: { ...preview, source: { ...preview.source, fingerprint: "changed" } } }), /Revisions changed/);
  api.integrate_workspace({ workspaceId: "w1", preview });
  assert.match(api.integration_preview(args).target.head, /^merge-/);
  assert.notEqual(api.integration_preview(args).target.head, preview.source.head);
  assert.throws(() => api.integrate_workspace({ workspaceId: "w1", preview }), /Revisions changed/);
  api.save_verification_presets({ repoId: "r1", presets: [{ id: "lint", command: "pnpm lint --strict" }] });
  assert.equal(api.integration_preview(args).canApply, false, "changed commands invalidate checks");
  for (const scenario of ["dirty", "conflict", "running"]) {
    const blocked = fixture(scenario).api;
    const result = blocked.integration_preview(args);
    assert.equal(result.canApply, false, scenario);
    assert.throws(() => blocked.integrate_workspace({ workspaceId: "w1", preview: { ...result, canApply: true } }), /blocked/);
    if (scenario === "conflict") assert.ok(result.conflicts.length);
  }
});
