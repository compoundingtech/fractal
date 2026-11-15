import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { ThreadEdits, threadRefusalMessage, type ThreadEdit, type ThreadEditResult } from "./threadEdits.ts";
import type { Snapshot } from "../../clients/typescript/fractal-client";

test("thread access refusals retain the server reason as a second sentence", () => {
  assert.equal(
    threadRefusalMessage("Fixture thread grant was revoked."),
    "You no longer have access to this thread. Fixture thread grant was revoked.",
  );
  assert.equal(threadRefusalMessage("Grant expired"), "You no longer have access to this thread. Grant expired.");
  assert.equal(threadRefusalMessage(""), "You no longer have access to this thread.");
});

const cases: readonly {
  edit: ThreadEdit;
  before: "open" | "closed";
  after: "open" | "closed";
  title: string;
  flash: string;
}[] = [
  {
    edit: { kind: "close" },
    before: "open",
    after: "closed",
    title: "Review",
    flash: 'Closed "Review" · Reopen thread to undo.',
  },
  { edit: { kind: "reopen" }, before: "closed", after: "open", title: "Review", flash: 'Reopened "Review"' },
  {
    edit: { kind: "rename", title: "New title" },
    before: "open",
    after: "open",
    title: "New title",
    flash: 'Renamed to "New title"',
  },
];
const errorText = (error: unknown): string => (error instanceof Error ? error.message : String(error));

for (const scenario of cases) {
  test(`${scenario.edit.kind} changes immediately, flashes only on success, and expires without undoing the edit`, async (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    let changes = 0;
    const edits = new ThreadEdits(() => {
      changes++;
    }, errorText);
    const thread = Object.freeze({ id: "thread/one", state: scenario.before, title: "Review" });
    const server = Promise.withResolvers<ThreadEditResult>();
    const result = edits.run(thread, scenario.edit, () => server.promise);
    assert.equal(changes, 1, "presentation publishes synchronously, before server acknowledgement");
    assert.equal(edits.project(thread).state, scenario.after);
    assert.equal(edits.project(thread).title, scenario.title);
    assert.equal(edits.flash(thread.id), undefined);
    assert.equal(thread.state, scenario.before, "the authoritative summary and fences are never mutated");
    assert.equal(thread.title, "Review");
    server.resolve({ how: "sent" });
    assert.deepEqual(await result, { how: "sent" });
    assert.equal(edits.flash(thread.id), scenario.flash);
    assert.equal(
      edits.project(thread).title,
      scenario.title,
      "a stale refresh must not flicker back after acknowledgement",
    );
    context.mock.timers.tick(4000);
    assert.equal(edits.flash(thread.id), undefined);
    assert.equal(edits.project(thread).state, scenario.after);
    assert.equal(edits.project(thread).title, scenario.title);
    const authoritative = { ...thread, state: scenario.after, title: scenario.title };
    edits.observe(authoritative);
    assert.equal(edits.project(authoritative), authoritative, "matching server state retires the overlay");
    const later = { ...authoritative, state: scenario.before, title: "Changed elsewhere" };
    assert.equal(edits.project(later), later, "subsequent server edits remain authoritative");
  });

  test(`${scenario.edit.kind} reverts and reports a refused server change without flashing`, async () => {
    const edits = new ThreadEdits(() => {}, errorText);
    const thread = { id: "thread/one", state: scenario.before, title: "Review" };
    const server = Promise.withResolvers<ThreadEditResult>();
    const result = edits.run(thread, scenario.edit, () => server.promise);
    assert.equal(edits.project(thread).state, scenario.after);
    assert.equal(edits.project(thread).title, scenario.title);
    server.resolve({ how: "failed", error: "Permission was revoked." });
    assert.deepEqual(await result, { how: "failed", error: "Permission was revoked." });
    assert.equal(edits.project(thread), thread);
    assert.equal(edits.flash(thread.id), undefined);
  });
}

for (const outcome of ["sent", "failed"] as const) {
  test(`an unconfirmed original request stays optimistic, and its ${outcome} retry settles presentation`, async (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    const edits = new ThreadEdits(() => {}, errorText);
    const thread = { id: "thread/one", state: "open" as const, title: "Review" };
    const retried = Promise.withResolvers<ThreadEditResult>();
    const result = await edits.run(thread, { kind: "close" }, async () => ({
      how: "unconfirmed",
      retry: () => retried.promise,
    }));
    assert.equal(edits.project(thread).state, "closed");
    assert.equal(edits.flash(thread.id), undefined);
    assert.equal(result.how, "unconfirmed");
    if (result.how !== "unconfirmed") assert.fail("The original request should be retryable.");
    const retry = result.retry();
    assert.equal(edits.flash(thread.id), undefined);
    retried.resolve(outcome === "sent" ? { how: "sent" } : { how: "failed", error: "No longer allowed." });
    await retry;
    assert.equal(edits.project(thread).state, outcome === "sent" ? "closed" : "open");
    assert.equal(edits.flash(thread.id), outcome === "sent" ? 'Closed "Review" · Reopen thread to undo.' : undefined);
  });
}

test("thrown server failures revert and expose the human error", async () => {
  const edits = new ThreadEdits(() => {}, errorText);
  const thread = { id: "thread/one", state: "open" as const, title: "Review" };
  const result = await edits.run(thread, { kind: "rename", title: "New title" }, async () => {
    throw new Error("The original connection is unavailable.");
  });
  assert.deepEqual(result, { how: "failed", error: "The original connection is unavailable." });
  assert.equal(edits.project(thread), thread);
  assert.equal(edits.flash(thread.id), undefined);
});

test("edits are scoped to their connection and do not cross an identical thread ID", async () => {
  const first = new ThreadEdits(() => {}, errorText);
  const second = new ThreadEdits(() => {}, errorText);
  const thread = { id: "thread/one", state: "open" as const, title: "Review" };
  const server = Promise.withResolvers<ThreadEditResult>();
  const result = first.run(thread, { kind: "close" }, () => server.promise);
  assert.equal(first.project(thread).state, "closed");
  assert.equal(second.project(thread), thread);
  server.resolve({ how: "failed", error: "Disconnected." });
  await result;
});

test("thread actions have no native confirmation or rename dialog; rename uses the inline title field", () => {
  const source = readFileSync(new URL("./screens/Threads.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(source, /Alert\.(?:alert|prompt)|\bconfirm\s*\(/);
  assert.match(source, /accessibilityLabel="Thread title"/);
  assert.match(source, /onSubmitEditing=\{rename\}/);
  assert.match(source, /accessibilityLiveRegion="polite"/);
  assert.match(source, /AccessibilityInfo\.announceForAccessibility\(message\)/);
});

test("a subsequent action preserves an acknowledged rename while stale summaries catch up", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const edits = new ThreadEdits(() => {}, errorText);
  const thread = { id: "thread/one", state: "open" as const, title: "Review" };
  await edits.run(thread, { kind: "rename", title: "New title" }, async () => ({ how: "sent" }));
  const server = Promise.withResolvers<ThreadEditResult>();
  const close = edits.run(thread, { kind: "close" }, () => server.promise);
  assert.equal(edits.project(thread).title, "New title");
  assert.equal(edits.project(thread).state, "closed");
  server.resolve({ how: "failed", error: "Not allowed." });
  await close;
  assert.equal(edits.project(thread).title, "New title", "only the failed action rolls back");
  assert.equal(edits.project(thread).state, "open");
  await edits.run(thread, { kind: "close" }, async () => ({ how: "sent" }));
  assert.equal(edits.flash(thread.id), 'Closed "New title" · Reopen thread to undo.');
  edits.observe({ ...thread, title: "New title", state: "closed" });
  assert.equal(edits.project(thread), thread, "both authoritative fields retire together");
});

for (const scenario of [
  { edit: { kind: "rename", title: "B" } as const, foreign: { title: "C", state: "open" as const } },
  { edit: { kind: "close" } as const, foreign: { title: "A", state: "open" as const } },
]) {
  test(`${scenario.edit.kind} retires at an authoritative receipt-or-later snapshot when the intermediate value is skipped`, async (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    let changes = 0;
    const edits = new ThreadEdits(() => {
      changes++;
    }, errorText);
    const thread = { id: "thread/one", title: "A", state: "open" as const };
    const receipt: Snapshot = {
      id: "snapshot/accepted",
      host_id: "host/one",
      store_index: 10,
      created_at: "2026-11-14T00:00:00Z",
      projection_version: "client-projection.v0",
    };
    await edits.run(thread, scenario.edit, async () => ({ how: "sent", snapshot: receipt }));
    const expected = edits.project(thread);
    edits.observe(thread, { ...receipt, id: "snapshot/stale", store_index: 9 });
    assert.deepEqual(edits.project(thread), expected, "a pre-receipt snapshot cannot retire the overlay");
    edits.observe(expected, { ...receipt, id: "snapshot/stale-match", store_index: 9 });
    assert.deepEqual(
      edits.project(thread),
      expected,
      "a matching value below the receipt index cannot retire the overlay",
    );
    edits.observe(expected, { ...receipt, id: "snapshot/foreign-match", host_id: "host/two", store_index: 100 });
    assert.deepEqual(edits.project(thread), expected, "a matching value on another host cannot retire the overlay");
    edits.observe(expected);
    assert.deepEqual(
      edits.project(thread),
      expected,
      "a matching value without snapshot provenance cannot retire a receipted overlay",
    );
    const foreign = { ...thread, ...scenario.foreign };
    edits.observe(foreign, { ...receipt, id: "snapshot/other-host", host_id: "host/two", store_index: 100 });
    assert.deepEqual(edits.project(thread), expected, "store indices are never comparable across hosts");
    edits.observe(foreign, { ...receipt, id: "snapshot/later", store_index: 11 });
    assert.equal(
      edits.project(foreign),
      foreign,
      "the later server value wins without ever observing our intermediate value",
    );
    const afterRetirement = changes;
    edits.observe(foreign, { ...receipt, store_index: 12 });
    assert.equal(changes, afterRetirement, "a retired overlay does not publish repeatedly");
  });
}

test("close at 10 then reopen at 12 ignores cached open at 9 and remains open when closed at 10 arrives", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const edits = new ThreadEdits(() => {}, errorText);
  const thread = { id: "thread/one", title: "Review", state: "open" as const };
  const closed = { ...thread, state: "closed" as const };
  const snapshot: Snapshot = {
    id: "snapshot/close",
    host_id: "host/one",
    store_index: 10,
    created_at: "2026-11-14T00:00:00Z",
    projection_version: "client-projection.v0",
  };
  await edits.run(thread, { kind: "close" }, async () => ({ how: "sent", snapshot }));
  edits.observe(closed, snapshot);
  await edits.run(closed, { kind: "reopen" }, async () => ({
    how: "sent",
    snapshot: { ...snapshot, id: "snapshot/reopen", store_index: 12 },
  }));
  edits.observe(thread, { ...snapshot, id: "snapshot/cached-open", store_index: 9 });
  edits.observe(closed, snapshot);
  assert.equal(edits.project(closed).state, "open");
  edits.observe(thread, { ...snapshot, id: "snapshot/foreign-open", host_id: "host/two", store_index: 100 });
  edits.observe(closed, snapshot);
  assert.equal(edits.project(closed).state, "open");
});
