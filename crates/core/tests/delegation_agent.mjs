import readline from "node:readline";
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
let server;
const reply = (id, result) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id, result }) + "\n");
async function callTool(name, args) {
  if (!server) throw new Error("MCP server not supplied over ACP");
  const child = spawn(server.command, server.args, { env: { ...process.env, ...Object.fromEntries(server.env.map(({ name, value }) => [name, value])) } });
  const lines = readline.createInterface({ input: child.stdout });
  let resolve;
  const result = new Promise((r, reject) => { resolve = r; child.on("error", reject); child.on("exit", (code) => { if (code) reject(new Error(`MCP exited ${code}`)); }); });
  lines.on("line", (line) => { const value = JSON.parse(line); if (value.id === 3) resolve(value); });
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "fixture", version: "1" } } }) + "\n");
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list" }) + "\n");
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name, arguments: args } }) + "\n");
  const value = await result;
  child.stdin.end();
  if (value.result?.isError || value.error) throw new Error(JSON.stringify(value));
  return value;
}
readline.createInterface({ input: process.stdin }).on("line", async (line) => {
  const { id, method, params } = JSON.parse(line);
  try {
    if (method === "initialize") return reply(id, { protocolVersion: 1, agentCapabilities: { sessionCapabilities: { resume: {} } } });
    if (method === "session/new" || method === "session/resume") { server = params.mcpServers.find((s) => s.name === "suneiro-delegation"); return reply(id, { sessionId: "fixture", configOptions: [] }); }
    if (method === "session/prompt") {
      writeFileSync("prompt-observed.json", JSON.stringify(params.prompt));
      if (process.argv.includes("--hold")) return;
      const taskId = params.prompt[0].text.match(/^Delegated task ([\w-]+)/)?.[1];
      if (taskId) {
        await callTool("suneiro_finish", { taskId, report: "Fixture complete; literal context verified; tests passed" });
        await callTool("suneiro_finish", { taskId, report: "Fixture complete; literal context verified; tests passed" });
      }
      return reply(id, { stopReason: "end_turn" });
    }
    if (id !== undefined) reply(id, {});
  } catch (error) { process.stderr.write(String(error)); process.exit(1); }
});
