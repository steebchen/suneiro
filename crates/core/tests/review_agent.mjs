import readline from "node:readline";
import { writeFileSync } from "node:fs";
const safeMode = process.argv[2] ?? "read-only";

const send = (msg) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...msg }) + "\n");
const configOptions = [{ id: "mode", name: "Mode", type: "select", currentValue: "agent-full-access", options: [{ value: safeMode, name: "Restricted" }, { value: "agent-full-access", name: "Full access" }] }];
let turn;
readline.createInterface({ input: process.stdin }).on("line", (line) => {
  const msg = JSON.parse(line);
  if (!msg.method) {
    if (msg.id === 1000) {
      if (msg.result?.outcome?.outcome !== "cancelled") writeFileSync("file.txt", "UNSAFE");
      send({ id: turn, result: { stopReason: "end_turn" } });
    }
    return;
  }
  switch (msg.method) {
    case "initialize": return send({ id: msg.id, result: { protocolVersion: 1, agentCapabilities: {} } });
    case "session/new": return send({ id: msg.id, result: { sessionId: "review-1", configOptions } });
    case "session/set_config_option":
      configOptions[0].currentValue = msg.params.value;
      return send({ id: msg.id, result: { configOptions } });
    case "session/prompt":
      if (configOptions[0].currentValue !== safeMode || msg.params.prompt.length !== 1) writeFileSync("file.txt", "UNSAFE");
      turn = msg.id;
      return send({ id: 1000, method: "session/request_permission", params: { sessionId: "review-1", toolCall: { title: "Edit" }, options: [{ optionId: "allow", kind: "allow_once", name: "Allow" }] } });
    default: if (msg.id !== undefined) send({ id: msg.id, error: { code: -32601, message: "unsupported" } });
  }
});
