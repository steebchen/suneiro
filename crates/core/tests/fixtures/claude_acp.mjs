// Deterministic contract fixture, NOT evidence of real Claude compatibility.
import readline from "node:readline";
import fs from "node:fs";
const args = new Set(process.argv.slice(2));
const send = (msg) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...msg }) + "\n");
const replies = new Map();
let nextId = 1000;
let stop;
let model = "sonnet";
let effort = "medium";
let mode = "default";
const config = () => [
  { id: "model", category: "model", type: "select", currentValue: model, options: [{ value: "sonnet", name: "Sonnet" }, { value: "opus", name: "Opus" }] },
  { id: "effort", category: "thought_level", type: "select", currentValue: effort, options: [{ group: "reasoning", name: "Reasoning", options: (model === "opus" ? ["medium", "high", "max"] : ["medium", "high"]).map(value => ({ value, name: value })) }] },
  { id: "mode", category: "mode", type: "select", currentValue: mode, options: ["default", "plan", "bypassPermissions"].map(value => ({ value, name: value })) },
];
const call = (method, params) => new Promise(resolve => {
  const id = nextId++;
  replies.set(id, resolve);
  send({ id, method, params });
});
readline.createInterface({ input: process.stdin }).on("line", async line => {
  const msg = JSON.parse(line);
  if (!msg.method) {
    replies.get(msg.id)?.(msg.result);
    replies.delete(msg.id);
    return;
  }
  const { id, method, params } = msg;
  const update = update => send({ method: "session/update", params: { sessionId: "claude-fixture", update } });
  const text = text => update({ sessionUpdate: "agent_message_chunk", content: { type: "text", text } });
  switch (method) {
    case "initialize":
      return send({ id, result: { protocolVersion: 1, agentCapabilities: { loadSession: true, promptCapabilities: { image: !args.has("--no-images"), embeddedContext: true } } } });
    case "session/new":
      return send({ id, result: { sessionId: "claude-fixture", configOptions: config() } });
    case "session/load":
      if (args.has("--load-fails")) return send({ id, error: { code: -32000, message: "Claude session unavailable" } });
      text("REPLAY_MUST_NOT_APPEAR");
      return send({ id, result: { configOptions: config() } });
    case "session/set_config_option":
      if (params.configId === "model") model = params.value;
      if (params.configId === "effort") effort = params.value;
      if (params.configId === "mode") {
        mode = params.value;
        if (args.has("--mode-resets-model")) {
          model = "sonnet";
          effort = "medium";
        }
      }
      // Reply-only changes: ACP does not require a redundant notification.
      return send({ id, result: { configOptions: config() } });
    case "session/cancel":
      stop?.("cancelled");
      return;
    case "session/prompt": {
      const prompt = params.prompt.find(b => b.type === "text")?.text ?? "";
      if (prompt === "slow") {
        const reason = await new Promise(resolve => { stop = resolve; text("working"); });
        stop = undefined;
        return send({ id, result: { stopReason: reason } });
      }
      if (prompt === "permission") {
        update({ sessionUpdate: "tool_call", toolCallId: "write-1", title: "Write file", kind: "edit", status: "pending", rawInput: { file_path: "example.txt" } });
        const result = await call("session/request_permission", { sessionId: params.sessionId, toolCall: { toolCallId: "write-1", title: "Write file" }, options: [{ optionId: "allow", kind: "allow_once", name: "Allow" }, { optionId: "deny", kind: "reject_once", name: "Deny" }] });
        text(JSON.stringify(result));
        update({ sessionUpdate: "tool_call_update", toolCallId: "write-1", status: "completed", content: [{ type: "content", content: { type: "text", text: "done" } }] });
      } else if (prompt === "question") {
        const result = await call("elicitation/create", { sessionId: params.sessionId, mode: "form", toolCallId: "ask-1", message: "Pick a color", requestedSchema: { type: "object", properties: { color: { type: "string", enum: ["red", "blue"] } }, required: ["color"] } });
        text(JSON.stringify(result));
      } else if (prompt === "image") {
        const image = params.prompt.find(b => b.type === "image" || b.type === "resource_link");
        text(JSON.stringify(image));
      } else {
        text(`model:${model};effort:${effort}`);
      }
      const file = ".fixture-cost";
      const previous = fs.existsSync(file) ? Number(fs.readFileSync(file, "utf8")) : 0;
      const total = previous + 0.25;
      fs.writeFileSync(file, String(total));
      update({ sessionUpdate: "usage_update", cost: { amount: total, currency: "USD" } });
      // Give consumer a scheduling opportunity; transport ordering is tested separately.
      setImmediate(() => send({ id, result: { stopReason: "end_turn", usage: { inputTokens: 10, cachedReadTokens: 2, cachedWriteTokens: 3, outputTokens: 4, thoughtTokens: 1 } } }));
      return;
    }
  }
});
