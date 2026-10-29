import readline from "node:readline";
const send = (message) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
const options = [{ id: "model", category: "model", name: "Model", type: "select", currentValue: "chosen", options: [{ value: "chosen", name: "Chosen" }] }, { id: "mode", category: "mode", name: "Permissions", type: "select", currentValue: "agent-full-access", options: [{ value: "agent-full-access", name: "Full access" }, { value: "read-only", name: "Read only" }] }, { id: "collaboration_mode", name: "Collaboration", type: "select", currentValue: "default", options: [{ value: "default", name: "Default" }, { value: "plan", name: "Plan" }] }];
readline.createInterface({ input: process.stdin }).on("line", (line) => {
  const { id, method, params } = JSON.parse(line);
  if (method === "initialize") return send({ id, result: { protocolVersion: 1, agentCapabilities: {} } });
  if (method === "session/new") return send({ id, result: { sessionId: "fresh-session", configOptions: options } });
  if (method === "session/set_config_option") return send({ id, result: { configOptions: options } });
  if (method === "session/prompt") {
    send({ method: "session/update", params: { sessionId: params.sessionId, update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: JSON.stringify(params.prompt) } } } });
    return send({ id, result: { stopReason: "end_turn" } });
  }
  if (id !== undefined) send({ id, error: { code: -32601, message: `Unexpected native session method: ${method}` } });
});
