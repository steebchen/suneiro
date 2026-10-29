import readline from "node:readline";
import { readFileSync } from "node:fs";
const init = JSON.parse(readFileSync(new URL("./initialize.json", import.meta.url)));
const session = JSON.parse(readFileSync(new URL("./session.json", import.meta.url)));
if (process.argv.includes("no-images")) init.agentCapabilities.promptCapabilities.image = false;
if (process.argv.includes("no-models")) session.configOptions[0].options = [];
if (process.argv.includes("load-only")) delete init.agentCapabilities.sessionCapabilities.resume;
const send = (message) => {
          if (process.argv.includes("provider-error") && message.result?.stopReason) {
            message = { id: message.id, error: { code: -32603, message: "Authentication failed for provider" } };
          }
          process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
        };
const waiting = new Map();
let stop;
let next = 1000;
const ask = (method, params) => new Promise((resolve) => {
  const id = next++;
  waiting.set(id, resolve);
  send({ id, method, params });
});
readline.createInterface({ input: process.stdin }).on("line", async (line) => {
  const { id, method, params, result } = JSON.parse(line);
  if (!method) { waiting.get(id)?.(result); waiting.delete(id); return; }
  const update = (update) => send({ method: "session/update", params: { sessionId: params.sessionId, update } });
  const text = (text) => update({ sessionUpdate: "agent_message_chunk", content: { type: "text", text } });
  switch (method) {
    case "initialize": return send({ id, result: init });
    case "session/new": return send({ id, result: session });
    case "session/load":
      text("replayed history must not be duplicated");
      return send({ id, result: session });
    case "session/resume": return send({ id, result: session });
    case "session/set_config_option": {
      const option = session.configOptions.find((o) => o.id === params.configId);
      if (!option?.options.some((o) => o.value === params.value)) return send({ id, error: { code: -32602, message: `Invalid params: ${params.configId} not found: ${params.value}` } });
      option.currentValue = params.value;
      return send({ id, result: { configOptions: session.configOptions } });
    }
    case "session/cancel": stop?.("cancelled"); return;
    case "session/prompt": {
      const prompt = params.prompt.find((b) => b.type === "text")?.text;
      if (prompt === "wait") {
        const reason = await new Promise((resolve) => { stop = resolve; text("waiting"); });
        stop = undefined;
        return send({ id, result: { stopReason: reason } });
      }
      if (prompt === "exercise") {
        update({ sessionUpdate: "tool_call", toolCallId: "read-1", title: "Read fixture", kind: "read", status: "in_progress" });
        const permission = await ask("session/request_permission", { sessionId: params.sessionId, toolCall: { toolCallId: "read-1", title: "Read fixture" }, options: [{ optionId: "allow", kind: "allow_once", name: "Allow" }] });
        text(`permission:${permission.outcome.optionId}`);
        update({ sessionUpdate: "tool_call_update", toolCallId: "read-1", status: "completed", content: [{ type: "content", content: { type: "text", text: "fixture" } }] });
        const answer = await ask("elicitation/create", { sessionId: params.sessionId, message: "Which color?", mode: "form", requestedSchema: { type: "object", properties: { color: { type: "string", enum: ["blue", "red"] } }, required: ["color"] } });
        text(`answer:${JSON.stringify(answer)}`);
      }
      text(`blocks:${JSON.stringify(params.prompt.map((b) => b.type))}`);
      return send({ id, result: { stopReason: "end_turn" } });
    }
    default: if (id !== undefined) send({ id, error: { code: -32601, message: "Method not found" } });
  }
});
