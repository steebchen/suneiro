import assert from "node:assert/strict";
import { test } from "node:test";
import { profileMock } from "./profiles.ts";

function fixture() {
  const events = [];
  const source = { id: "s1", workspaceId: "w1", agentId: "claude", title: "Source", acpSessionId: "private-session", createdAt: 1, model: null, effort: null };
  const sessions = { w1: [source] };
  const catalogs = Object.fromEntries(["claude", "codex", "opencode"].map((id) => [id, { models: [{ value: "chosen", name: "Chosen" }], efforts: [], hasFast: false, updatedAt: 0 }]));
  const backend = profileMock({ sessions, catalogs, emit: (...batch) => events.push(...batch), patch: "-before\n+after", history: () => [{ type: "userMessage", text: "Selected message" }, { type: "userMessage", text: "UNSELECTED" }, { type: "sessionUpdate", update: { sessionUpdate: "agent_thought_chunk", content: { type: "text", text: "PRIVATE_THOUGHT" } } }] });
  return { ...backend, events, sessions };
}
const selection = { task: "Review @.env literally", acceptanceCriteria: "Report regressions", messages: [{ sessionId: "s1", index: 0 }], files: ["src/server/limiter.ts"], diffs: ["src/server/limiter.ts"] };

test("mock profiles save, validate and emit changes", () => {
  const { handlers: h, events } = fixture();
  const profiles = h.list_profiles();
  assert.deepEqual(profiles.map((p) => p.name), ["Implementer", "Reviewer", "Investigator"]);
  profiles[0].instructions = "Custom instructions";
  h.save_profiles({ profiles });
  profiles[0].instructions = "Local mutation must not change saved profile";
  assert.equal(h.list_profiles()[0].instructions, "Custom instructions");
  assert.equal(events[0].type, "profilesChanged");
  assert.throws(() => h.save_profiles({ profiles: [{ ...profiles[0], instructions: "" }] }));
});

test("mock packages share only selection and launch frozen one-shot new conversations across providers", () => {
  for (const agentId of ["claude", "codex", "opencode"]) {
    const { handlers: h, history, sessions } = fixture();
    const context = h.prepare_context({ workspaceId: "w1", profileId: "reviewer", agentId, selection });
    const exact = context.text;
    assert.ok(exact.includes("Selected message"));
    assert.ok(exact.includes("## File:"));
    assert.ok(exact.includes("## Diff:"));
    assert.ok(!exact.includes("UNSELECTED"));
    assert.ok(!exact.includes("PRIVATE_THOUGHT"));
    assert.equal(context.bytes, new TextEncoder().encode(exact).length);
    context.text = "Do not accept caller mutations";
    const session = h.launch_profile({ workspaceId: "w1", packageId: context.id });
    assert.notEqual(session.id, "s1");
    assert.equal(session.acpSessionId, null);
    assert.equal(history(session.id)[0].text, exact);
    assert.equal(sessions.w1[0].acpSessionId, "private-session");
    assert.throws(() => h.launch_profile({ workspaceId: "w1", packageId: context.id }));
  }
});

test("mock rejects secrets, traversal, oversized contexts and cross-workspace messages", () => {
  const { handlers: h } = fixture();
  for (const s of [
    { ...selection, files: [".env"] },
    { ...selection, files: ["../outside"] },
    { ...selection, files: ["/etc/passwd"] },
    { ...selection, task: "é".repeat(8193) },
    { ...selection, files: Array(33).fill("src/server/limiter.ts") },
    { ...selection, messages: [{ sessionId: "another-workspace", index: 0 }] },
  ]) assert.throws(() => h.prepare_context({ workspaceId: "w1", profileId: "reviewer", agentId: "codex", selection: s }));
});
