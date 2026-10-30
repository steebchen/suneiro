import readline from "node:readline";
import { readFileSync, writeFileSync, existsSync } from "node:fs";

const profile = JSON.parse(readFileSync(new URL("./session.json", import.meta.url)));
const flags = new Set(process.argv.slice(2));
const config = profile.configOptions;
const waiting = new Map();
let nextId = 1000;
let stop;
const send = (message) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
const value = (id) => config.find((o) => o.id === id)?.currentValue;
const trace = (sessionId, text) => send({ method: "session/update", params: { sessionId, update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text } } } });
async function handle(message) {
  const { id, method, params = {} } = message;
  if (!method) { waiting.get(id)?.(message.result); waiting.delete(id); return; }
  const result = (result) => send({ id, result });
  const ask = (method, params) => new Promise((resolve) => { const id = nextId++; waiting.set(id, resolve); send({ id, method, params }); });
  switch (method) {
    case "initialize":
      if (flags.has("--no-plan")) config.splice(1, 1);
      return result({ protocolVersion: 1, agentCapabilities: { loadSession: true, promptCapabilities: { image: !flags.has("--no-images") }, sessionCapabilities: { resume: {} } }, _meta: flags.has("--no-steering") ? {} : { steering: { supported: true } } });
    case "session/new":
      if (existsSync("new-session")) profile.sessionId = "codex-fixture-restarted";
      writeFileSync("new-session", "1");
      return result(profile);
    case "session/resume":
      if (flags.has("--reject-resume")) return send({ id, error: { code: -32000, message: "Session not found" } });
      if (existsSync("session-state.json")) config.splice(0, config.length, ...JSON.parse(readFileSync("session-state.json", "utf8")));
      return result(profile);
    case "session/set_config_option": {
      const option = config.find((o) => o.id === params.configId);
      if (!option?.options.some((o) => o.value === params.value)) return send({ id, error: { code: -32602, message: "unsupported config value" } });
      option.currentValue = params.value;
      writeFileSync("session-state.json", JSON.stringify(config));
      if (option.id === "model") config.find((o) => o.id === "reasoning_effort").options = [{ value: "low" }, { value: "medium" }];
      return result({ configOptions: config });
    }
    case "session/cancel":
      stop?.("cancelled");
      return;
    case "_session/steering":
      if (flags.has("--no-steering")) throw new Error("unsupported steering called");
      trace(params.sessionId, "steered");
      return result({ outcome: "injected" });
    case "session/prompt": {
      const sessionId = params.sessionId;
      const text = params.prompt[0].text;
      trace(sessionId, JSON.stringify({ model: value("model"), effort: value("reasoning_effort"), mode: value("mode"), collaboration: value("collaboration_mode"), blocks: params.prompt }));
      if (text === "slow") {
        const reason = await new Promise((resolve) => { stop = resolve; });
        stop = undefined;
        return result({ stopReason: reason });
      }
      if (text === "permission") {
        const answer = await ask("session/request_permission", { sessionId, toolCall: { toolCallId: "edit", title: "Edit" }, options: [{ optionId: "yes", kind: "allow_once", name: "Allow" }, { optionId: "no", kind: "reject_once", name: "Deny" }] });
        trace(sessionId, JSON.stringify(answer));
      }
      if (text === "question" || text === "url-question") {
        const answer = await ask("elicitation/create", { sessionId, mode: text === "url-question" ? "url" : "form", message: "Which approach?", requestedSchema: { type: "object", properties: { approach: { type: "string", enum: ["small", "large"] } }, required: ["approach"] }, _meta: { codex: { autoResolutionMs: 60000 } } });
        trace(sessionId, JSON.stringify(answer));
      }
      return result({ stopReason: "end_turn", usage: { inputTokens: 193, cachedReadTokens: 23936, outputTokens: 5, thoughtTokens: 7 }, _meta: { quota: { model_usage: [{ model: "gpt-small" }] } } });
    }
    default:
      return send({ id, error: { code: -32601, message: "unsupported method" } });
  }
}
readline.createInterface({ input: process.stdin }).on("line", (line) => handle(JSON.parse(line)).catch((error) => { console.error(error.message); process.exit(1); }));
