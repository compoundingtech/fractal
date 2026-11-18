import assert from "node:assert/strict";
import { test } from "node:test";
import { threadClock, threadExactTime, threadRelativeTime } from "./threadClock.ts";

const acceptedAt = Date.parse("2026-11-09T12:58:00Z");

test("thread ages use the shared web ago units at each threshold", () => {
  assert.equal(threadRelativeTime(acceptedAt, acceptedAt), "0s ago");
  assert.equal(threadRelativeTime(acceptedAt, acceptedAt + 999), "0s ago");
  assert.equal(threadRelativeTime(acceptedAt, acceptedAt + 59_999), "59s ago");
  assert.equal(threadRelativeTime(acceptedAt, acceptedAt + 60_000), "1m ago");
  assert.equal(threadRelativeTime(acceptedAt, acceptedAt + 300_000), "5m ago");
  assert.equal(threadRelativeTime(acceptedAt, acceptedAt + 3_599_999), "59m ago");
  assert.equal(threadRelativeTime(acceptedAt, acceptedAt + 3_600_000), "1h ago");
  assert.equal(threadRelativeTime(acceptedAt, acceptedAt + 86_399_999), "23h ago");
  assert.equal(threadRelativeTime(acceptedAt, acceptedAt + 86_400_000), "1d ago");
  assert.equal(threadRelativeTime(acceptedAt + 60_000, acceptedAt), "0s ago");
});

test("all thread timelines share a second tick and release it with the last reader", (context) => {
  context.mock.timers.enable({ apis: ["Date", "setInterval"], now: acceptedAt });
  const ages: string[][] = [[], []];
  const first = threadClock.subscribe(() => ages[0]!.push(threadRelativeTime(acceptedAt, threadClock.getSnapshot())));
  const second = threadClock.subscribe(() => ages[1]!.push(threadRelativeTime(acceptedAt, threadClock.getSnapshot())));
  context.after(first);
  context.after(second);
  assert.equal(threadClock.getSnapshot(), acceptedAt);
  context.mock.timers.tick(1_000);
  assert.deepEqual(ages, [["0s ago", "1s ago"], ["1s ago"]], "each reader receives exactly one update per shared tick");
  first();
  context.mock.timers.tick(1_000);
  assert.deepEqual(ages, [
    ["0s ago", "1s ago"],
    ["1s ago", "2s ago"],
  ]);
  second();
  context.mock.timers.tick(1_000);
  assert.equal(threadClock.getSnapshot(), acceptedAt + 2_000, "no clock work remains without readers");
  const resumed = threadClock.subscribe(() => {});
  assert.equal(threadClock.getSnapshot(), acceptedAt + 3_000, "the first returning reader refreshes the current time");
  resumed();
});

test("exact thread times retain a localized date and time including seconds", () => {
  assert.equal(
    threadExactTime(acceptedAt),
    new Date(acceptedAt).toLocaleString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }),
  );
});
