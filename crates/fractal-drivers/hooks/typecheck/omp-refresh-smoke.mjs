// Real shipped extension + channel wire; the fixture dispatches effects only after its veto.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
const root = fs.mkdtempSync(path.join(os.tmpdir(), "omp-refresh-"));
const inbox = path.join(root, "in"), outbox = path.join(root, "out"), peer = path.join(root, "peer");
fs.writeFileSync(inbox, "");
fs.writeFileSync(outbox, "");
fs.writeFileSync(peer, `#!${process.execPath}
import fs from "node:fs";
process.stdout.write('{"type":"hello","protocol":2}\\n');
process.stdin.on("data", chunk => fs.appendFileSync(${JSON.stringify(inbox)}, chunk));
process.stdin.on("end", () => process.exit(0));
let offset = 0;
setInterval(() => { const data = fs.readFileSync(${JSON.stringify(outbox)}, "utf8");
process.stdout.write(data.slice(offset)); offset = data.length; }, 5);
`, { mode: 0o755 });
Object.assign(process.env, { ST_OMP_CHANNEL_BIN: peer, ST_OMP_CHANNEL_CATALOG: root,
  ST_OMP_CHANNEL_IDENTITY: "fixture.worker", ST_OMP_CHANNEL_SESSION: "incumbent", ST_OMP_CHANNEL_SEQ: "7" });
const handlers = new Map();
let idle = true, jobs = [], delivered = 0, executed = 0;
const ctx = { isIdle: () => idle, getAsyncJobSnapshot: () => ({ running: jobs }),
  ui: { notify() {} }, sessionManager: { getSessionId: () => "native-session" } };
const { default: extension } = await import(process.argv[2]);
extension({ on: (name, handler) => handlers.set(name, handler),
  sendUserMessage: () => { delivered += 1; }, sendMessage() {} });
const frames = () => fs.readFileSync(inbox, "utf8").split("\n").filter(Boolean).map(JSON.parse);
const until = async (predicate) => {
  const deadline = Date.now() + 5000;
  while (!predicate()) { assert.ok(Date.now() < deadline, "adapter response timed out");
    await new Promise(resolve => setTimeout(resolve, 5)); }
};
const send = (frame) => fs.appendFileSync(outbox, JSON.stringify(frame) + "\n");
let serial = 0;
const request = async (action, receipt = {}) => {
  const request_id = String(++serial);
  send({ type: "refresh", refresh: { action, ...receipt, request_id } });
  await until(() => frames().some(frame => frame.request_id === request_id));
  return frames().find(frame => frame.request_id === request_id);
};
const dispatch = async (id, context = ctx) => {
  const veto = await handlers.get("tool_call")({ toolName: "bash", toolCallId: id }, context);
  if (!veto?.block) executed += 1;
  return veto;
};
try {
  await handlers.get("session_start")({}, ctx);
  await until(() => frames().some(frame => frame.type === "ready"));
  let activity = (await request("activity")).receipt;
  assert.equal(activity.owner_sequence, 7);
  assert.equal((await request("close", { ...activity, incarnation: "predecessor" })).result, "refused");
  await dispatch("busy");
  activity = (await request("activity")).receipt;
  assert.equal(activity.open_non_person_tools, 1);
  assert.equal((await request("close", activity)).result, "refused");
  await handlers.get("tool_result")({ toolCallId: "busy" }, ctx);
  const subCtx = { ...ctx, agent: { kind: "sub", id: "child" } };
  await dispatch("child-tool", subCtx);
  activity = (await request("activity")).receipt;
  assert.equal((await request("close", activity)).result, "refused", "subagent tools are counted");
  await handlers.get("tool_result")({ toolCallId: "child-tool" }, subCtx);
  jobs = [{ id: "background" }]; // Tool returned, but its effects have not finished.
  assert.equal((await request("close", activity)).result, "refused");
  jobs = [];
  idle = false;
  await handlers.get("agent_start")({}, ctx);
  assert.equal((await request("close", activity)).result, "refused");
  idle = true;
  await handlers.get("agent_end")({}, ctx);
  const closed = await request("close", activity);
  assert.equal(closed.result, "closed");
  assert.equal(closed.receipt.gate, "closed");
  const before = executed;
  assert.match((await dispatch("late")).reason, /refresh admission closed/);
  assert.equal((await dispatch("late-child", subCtx)).block, true);
  assert.equal(executed, before, "closed tool never executes");
  send({ type: "message", content: "mailbox input", meta: { messageId: "mail" } });
  await until(() => frames().some(frame => frame.type === "failed" && frame.meta?.messageId === "mail"));
  assert.equal(delivered, 0, "closed mailbox never starts a delivered turn");
  // Accepted boundary: native text-only turns may start; their tools remain vetoed.
  await handlers.get("agent_start")({}, ctx);
  assert.equal((await dispatch("native-after-close")).block, true);
  await handlers.get("agent_end")({}, ctx);
  assert.equal((await request("reopen", { ...closed.receipt, owner_sequence: 8 })).result, "refused");
  assert.equal((await request("reopen", closed.receipt)).result, "opened");
  assert.equal(await dispatch("restored"), undefined);
  await handlers.get("tool_result")({ toolCallId: "restored" }, ctx);
  assert.equal((await request("close", activity)).result, "refused", "old epoch cannot close");
  const missingJobs = ctx.getAsyncJobSnapshot;
  ctx.getAsyncJobSnapshot = () => { throw new Error("snapshot unavailable"); };
  assert.equal((await request("activity")).result, "unsupported");
  ctx.getAsyncJobSnapshot = missingJobs;
  const badContext = { ...ctx, sessionManager: { getSessionId() { throw new Error("veto failure"); } } };
  const failed = await dispatch("error-path", badContext);
  assert.equal(failed.block, true);
  assert.match(failed.reason, /refresh admission failed.*veto failure/);
  assert.equal(executed, before + 1, "veto-handler failure never executes effects");
  assert.equal((await request("activity")).result, "unsupported", "uncertain accounting cannot close");
  console.log("OMP refresh adapter: idle, busy, jobs, stale owner/epoch, late tool, mailbox, reopen and error veto passed");
} finally {
  await handlers.get("session_shutdown")({}, ctx);
  fs.rmSync(root, { recursive: true, force: true });
}
