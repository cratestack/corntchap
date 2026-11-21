// Drive a running server with the official TypeScript MCP SDK (handshake-era revisions).
// Usage: CORNTCHAP_URL=http://127.0.0.1:PORT CORNTCHAP_TOKEN=<agent token> PROJECT=DEMO \
//        node scripts/mcp_check_ts.mjs   (needs `npm i @modelcontextprotocol/sdk` next to it or in NODE_PATH-resolvable dir)
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const url = process.env.CORNTCHAP_URL;
const token = process.env.CORNTCHAP_TOKEN;
const project = process.env.PROJECT || "DEMO";
if (!url || !token) throw new Error("set CORNTCHAP_URL and CORNTCHAP_TOKEN");

const transport = new StreamableHTTPClientTransport(new URL(`${url}/mcp`), {
  requestInit: { headers: { Authorization: `Bearer ${token}` } },
});
const client = new Client({ name: "mcp-check-ts", version: "0.0.0" });
await client.connect(transport);
console.log("negotiated protocol:", transport.protocolVersion, "server:", client.getServerVersion());

const payload = (r) => JSON.parse(r.content[0].text);
const check = (ok, msg) => {
  if (!ok) throw new Error("FAILED: " + msg);
  console.log("ok  ", msg);
};

const { tools } = await client.listTools();
const names = tools.map((t) => t.name);
check(["claim_next", "check_item", "complete", "get_task_markdown"].every((n) => names.includes(n)), `tools/list (${names.length} tools)`);

const claim = await client.callTool({ name: "claim_next", arguments: { project } });
const c = payload(claim);
if (c.none) {
  console.log("nothing to claim in", project, "(seed a task first)");
} else {
  const id = c.task.id;
  const epoch = c.lease.epoch;
  check(!claim.isError, `claim_next -> ${id} epoch ${epoch}`);
  const ticked = await client.callTool({ name: "check_item", arguments: { id, epoch, position: 1, done: true } });
  check(!ticked.isError, "check_item");
  const md = await client.callTool({ name: "get_task_markdown", arguments: { id } });
  check(payload(md).markdown.includes(id), "get_task_markdown");
  const stale = await client.callTool({ name: "complete", arguments: { id, epoch: epoch + 1000, result: "stale" } });
  check(stale.isError === true && stale.content[0].text.includes("lease_lost"), "stale-epoch complete -> isError lease_lost");
  const done = await client.callTool({ name: "complete", arguments: { id, epoch, result: "done via TS SDK" } });
  check(!done.isError, "complete");
}
await client.close();
console.log("TS SDK check OK");
