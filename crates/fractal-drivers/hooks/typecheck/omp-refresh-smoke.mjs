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
let expireRestore;
if (process.argv[3] === "--restoring-ask") {
  process.env.ST3_OMP_PENDING_ASK = "original-ask";
  const timeout = globalThis.setTimeout;
  globalThis.setTimeout = (callback, ms, ...args) => ms === 120_000
    ? (expireRestore = callback, { unref() {} }) : timeout(callback, ms, ...args);
}
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
const dispatch = async (id, context = ctx, toolName = "bash") => {
  const veto = await handlers.get("tool_call")({ toolName, toolCallId: id }, context);
  if (!veto?.block) executed += 1;
  return veto;
};
try {
  await handlers.get("session_start")({}, ctx);
  await until(() => frames().some(frame => frame.type === "ready"));
  if (process.argv[3] === "--restoring-ask") {
    let pending = await request("activity");
    assert.deepEqual(pending.blockers, ["unsupported-native-prompt-preservation"]);
    assert.notEqual((await request("close", pending.receipt)).result, "closed");
    assert.equal(typeof expireRestore, "function");
    expireRestore();
    pending = await request("activity");
    assert.deepEqual(pending.blockers, ["unsupported-native-prompt-preservation"]);
    assert.notEqual((await request("close", pending.receipt)).result, "closed", "timeout cannot erase an unresolved ask");
    console.log("OMP restored native ask: pending and timed-out continuation remain blocked");
  } else if (process.argv[3] === "--native-user") {
    for (const name of ["user_bash", "user_python"]) {
      assert.equal(handlers.get(name)({ command: "fixture", code: "fixture" }, ctx), undefined);
      await handlers.get("agent_end")({}, ctx);
      assert.equal((await request("activity")).result, "unsupported", "idle cannot prove native command settlement");
    }
    console.log("OMP native user execution: unknown completion remains unsupported");
  } else {
  let activity = (await request("activity")).receipt;
  assert.equal(activity.owner_sequence, 7);
  assert.equal((await request("close", { ...activity, incarnation: "predecessor" })).result, "refused");
  const capture = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
  const denied = capture.events.find(frame => frame.type === "tool_approval_resolved" && !frame.event.approved).event;
  idle = false;
  await handlers.get("agent_start")({}, ctx);
  await handlers.get("tool_call")(denied, ctx);
  await handlers.get("tool_approval_resolved")(denied, ctx);
  assert.equal((await request("activity")).receipt.open_non_person_tools, 0, "explicit native denial settles without execution");
  const terminal = capture.events.find(frame => frame.type === "message_end" && frame.event.message.role === "toolResult").event;
  await handlers.get("message_end")(terminal, ctx); // Producer-denied call has no extension tool_result.
  assert.equal((await request("activity")).receipt.open_non_person_tools, 0, "settled before agent_end");
  await handlers.get("agent_end")(capture.events.findLast(frame => frame.type === "agent_end").event, ctx);
  idle = true;
  const deniedSeal = await request("close", (await request("activity")).receipt);
  assert.equal(deniedSeal.result, "closed");
  assert.equal((await request("reopen", deniedSeal.receipt)).result, "opened");
  await handlers.get("tool_call")(denied, ctx); // Terminal fallback without an approval/result callback.
  await handlers.get("message_end")(terminal, ctx);
  assert.equal((await request("activity")).receipt.open_non_person_tools, 0);
  const ask = { toolName: "ask", toolCallId: "original-ask",
    input: { questions: [{ id: "choice", question: "Keep the original choice?", options: [{ label: "Keep" }] }] } };
  await handlers.get("tool_call")(ask, ctx);
  let prompt = await request("activity");
  assert.deepEqual(prompt.blockers, ["unsupported-native-prompt-preservation"]);
  assert.notEqual((await request("close", prompt.receipt)).result, "closed");
  await handlers.get("tool_result")({ ...ask, isError: false }, ctx);
  assert.deepEqual((await request("activity")).blockers, []);
  const permission = { toolCallId: "original-permission", toolName: "bash" };
  await handlers.get("tool_call")(permission, ctx);
  await handlers.get("tool_approval_requested")(permission, ctx);
  prompt = await request("activity");
  assert.deepEqual(prompt.blockers, ["unsupported-native-prompt-preservation"]);
  assert.notEqual((await request("close", prompt.receipt)).result, "closed");
  // An unrelated stored answer cannot resolve the pending native call.
  await handlers.get("tool_approval_resolved")({ ...permission, toolCallId: "different-call", approved: true }, ctx);
  assert.deepEqual((await request("activity")).blockers, ["unsupported-native-prompt-preservation"]);
  await handlers.get("tool_approval_resolved")({ ...permission, approved: false }, ctx);
  assert.deepEqual((await request("activity")).blockers, []);
  for (const name of ["xd", "device", "device"]) await dispatch("nested-call", ctx, name);
  assert.equal((await request("activity")).receipt.open_non_person_tools, 3);
  await handlers.get("message_end")({ message: { role: "toolResult", toolCallId: "nested-call", toolName: "xd" } }, ctx);
  assert.equal((await request("activity")).receipt.open_non_person_tools, 2, "root completion cannot release live nested effects");
  await handlers.get("message_end")({ message: { role: "toolResult", toolCallId: "nested-call", toolName: "device" } }, ctx);
  assert.equal((await request("activity")).receipt.open_non_person_tools, 2, "ambiguous same-name terminal cannot release overlapping effects");
  for (const [toolName, remaining] of [["device", 1], ["device", 0]]) {
    await handlers.get("tool_result")({ toolCallId: "nested-call", toolName }, ctx);
    assert.equal((await request("activity")).receipt.open_non_person_tools, remaining);
  }
  await handlers.get("message_end")({ message: { role: "toolResult", toolCallId: "nested-call", toolName: "xd" } }, ctx);
  assert.equal((await request("activity")).receipt.open_non_person_tools, 0, "terminal notification is idempotent");
  await dispatch("busy");
  activity = (await request("activity")).receipt;
  assert.equal(activity.open_non_person_tools, 1);
  assert.equal((await request("close", activity)).result, "refused");
  await handlers.get("tool_result")({ toolCallId: "busy", toolName: "bash" }, ctx);
  const subCtx = { ...ctx, agent: { kind: "sub", id: "child" } };
  await dispatch("child-tool", subCtx);
  activity = (await request("activity")).receipt;
  assert.equal((await request("close", activity)).result, "refused", "subagent tools are counted");
  await handlers.get("tool_result")({ toolCallId: "child-tool", toolName: "bash" }, subCtx);
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
  for (const name of ["user_bash", "user_python"])
    assert.match(handlers.get(name)({}, ctx).result.output, /refresh admission closed/, "native user execution is synchronously substituted");
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
  await handlers.get("tool_result")({ toolCallId: "restored", toolName: "bash" }, ctx);
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
  }
} finally {
  await handlers.get("session_shutdown")({}, ctx);
  fs.rmSync(root, { recursive: true, force: true });
}
