// Exercise the shipped extension and recorder pipe, not a copy of its normalizer.
import assert from "node:assert/strict";
import fs from "node:fs";

export const assertResponseEvidence = async (handlers, ctx, readFrames) => {
  const fixtures = JSON.parse(fs.readFileSync(new URL(
    "../../tests/fixtures/harness-timeline/omp-response-evidence.json", import.meta.url,
  ), "utf8"));
  const secret = "response-evidence-must-not-leak";
  const cases = [
    ...fixtures,
    { role: "assistant", responseId: "unknown-evidence" },
    { role: "assistant", responseId: "zero-evidence", duration: 0, ttft: 0,
      usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0,
        reasoningTokens: 0, cttl: { ephemeral1h: 0 }, cost: 0, premiumRequests: 0 } },
    { role: "assistant", responseId: "partial-evidence",
      usage: { inputTokens: 12, outputTokens: 3, cost: { input: 0.01 } } },
    { role: "assistant", responseId: "invalid-evidence", duration: Infinity, ttft: NaN,
      usage: { input: "12", output: Infinity, cost: { total: NaN }, premiumRequests: "0" } },
    { role: "assistant", responseId: "r".repeat(300), model: "m".repeat(200),
      provider: "p".repeat(100), api: "a".repeat(200), stopReason: "s".repeat(100) },
  ];
  const before = readFrames().filter(frame => frame.type === "timeline" && frame.event === "message_end").length;
  for (const message of cases) {
    await handlers.get("message_end")({ message: { ...message,
      request: { authorization: secret }, rawResponse: secret,
      ...(message.usage ? { usage: { ...message.usage, private: secret,
        ...(typeof message.usage.cost === "object" ? { cost: { ...message.usage.cost, private: secret } } : {}),
        ...(message.usage.cttl ? { cttl: { ...message.usage.cttl, private: secret } } : {}),
      } } : {}),
    } }, ctx);
  }
  const terminal = () => readFrames().filter(frame => frame.type === "timeline" && frame.event === "message_end").slice(before);
  const deadline = Date.now() + 5000;
  while (terminal().length < cases.length) {
    assert.ok(Date.now() < deadline, "all terminal response frames reach the channel recorder");
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  const frames = terminal();
  assert.equal(frames.length, cases.length);
  assert.ok(!JSON.stringify(frames).includes(secret), "raw/private response fields never leave the allowlist");
  const messages = frames.map(frame => frame.payload.message);
  for (const [index, fixture] of fixtures.entries()) {
    const emitted = messages[index];
    for (const [key, value] of Object.entries(fixture)) {
      if (key !== "usage") assert.deepEqual(emitted[key], value, `response metadata ${key} survives`);
    }
    for (const [key, value] of Object.entries(fixture.usage)) {
      assert.deepEqual(emitted.usage[key], value, `response usage ${key} survives`);
    }
    assert.equal(emitted.id, undefined, "response identity is not synthesized as a message id");
  }
  const [unknown, zero, partial, invalid, bounded] = messages.slice(fixtures.length);
  assert.equal(unknown.usage, undefined);
  for (const key of ["model", "provider", "api", "stopReason", "duration", "ttft", "timestamp", "completedAt"]) {
    assert.ok(unknown[key] == null, `${key} is unknown, not borrowed from session context`);
  }
  assert.equal(zero.duration, 0);
  assert.equal(zero.ttft, 0);
  assert.deepEqual(zero.usage, cases[fixtures.length + 1].usage, "observed zero is retained in every numeric bucket");
  assert.equal(partial.usage.input, 12);
  assert.equal(partial.usage.output, 3);
  assert.equal(partial.usage.cost.input, 0.01);
  for (const key of ["cacheRead", "cacheWrite", "totalTokens", "reasoningTokens", "premiumRequests"]) {
    assert.ok(partial.usage[key] == null, `${key} is not invented for partial usage`);
  }
  assert.ok(partial.usage.cost.total == null, "component-only cost does not invent a total");
  assert.ok(invalid.duration == null && invalid.ttft == null);
  assert.ok(invalid.usage.input == null && invalid.usage.output == null);
  assert.ok(invalid.usage.cost.total == null && invalid.usage.premiumRequests == null);
  for (const [key, length] of [["responseId", 256], ["model", 128], ["provider", 64], ["api", 128], ["stopReason", 64]]) {
    assert.equal(bounded[key].length, length, `${key} remains bounded`);
  }
};
