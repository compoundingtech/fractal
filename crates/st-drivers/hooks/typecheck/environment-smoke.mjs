// arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
// Exercise the actual standalone assets' environment helpers and ownership stashes without
// launching a provider/channel. The normal hook build transpiles all four inputs first.
import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

for (const file of ["pi", "omp", "st-pi", "st-omp"]) {
  const source = fs.readFileSync(new URL(`./smoke-out/${file}-channel.mjs`, import.meta.url), "utf8");
  const helpers = source.match(/(?:const|let|var) preferredName = [\s\S]*?(?=(?:const|let|var) BIN =)/)?.[0];
  const stashSource = source.match(/(?:const|let|var) stash = \(\) => \{[\s\S]*?\n\};/)?.[0];
  assert.ok(helpers, `${file} embeds standalone env helpers`);
  assert.ok(stashSource, `${file} has an ownership stash`);
  const harness = file.endsWith("omp") ? "omp" : "pi";
  const prefix = `ST_${harness.toUpperCase()}_CHANNEL_`;
  const preferred = `FRACTAL_${harness.toUpperCase()}_CHANNEL_`;
  const old = `ST2_${harness.toUpperCase()}_CHANNEL_`;
  const fields = { BIN: "bin", IDENTITY: "identity", RUNTIME_ID: "runtimeId", SESSION: "session", SEQ: "seq" };
  if (!file.startsWith("st-")) fields.CATALOG = "catalog";
  if (harness === "omp") Object.assign(fields, { EXPECTED_NATIVE_SESSION: "expectedNativeSession", RESUME_GENERATION: "resumeGeneration" });
  const constants = Object.keys(fields).map(name => `const ${name} = ${JSON.stringify(prefix + name)};`).join("\n");
  const bridge = file === "st-omp" ? 'const PENDING_ASK = "ST3_OMP_PENDING_ASK";' : "";
  const identityFields = new Set(["IDENTITY", "RUNTIME_ID", "SESSION", "SEQ", "EXPECTED_NATIVE_SESSION", "RESUME_GENERATION"].filter(suffix => suffix in fields));
  const modes = ["unset", "preferred", "current", "legacy", "equal", "conflict", "legacy-conflict", "legacy-empty", "empty", "current-empty", "reload"];
  if (harness === "omp") modes.push("expected-session-conflict");
  if (!file.startsWith("st-")) modes.push("reload-legacy");
  for (const mode of modes) {
    const env = {};
    const reports = [];
    const context = vm.createContext({ process: { env }, console: { error: message => reports.push(message) } });
    vm.runInContext(`${helpers}\n${constants}\n${bridge}\n${stashSource}\nglobalThis.contract = { readEnv, removeEnv, writeEnv, stash };`, context);
    for (const suffix of Object.keys(fields)) {
      if (["preferred", "equal", "conflict", "legacy-conflict", "legacy-empty", "empty"].includes(mode)) env[preferred + suffix] = mode === "empty" ? "" : "preferred-value";
      if (["current", "equal", "conflict", "legacy-empty", "empty", "current-empty"].includes(mode)) env[prefix + suffix] = mode === "equal" ? "preferred-value" : ["legacy-empty", "current-empty"].includes(mode) ? "" : "current-value";
      if (["legacy", "equal", "legacy-conflict", "current-empty", "empty"].includes(mode)) env[old + suffix] = "legacy-value";
      if (mode === "expected-session-conflict" && suffix === "EXPECTED_NATIVE_SESSION") {
        env[preferred + suffix] = "preferred-value";
        env[prefix + suffix] = "current-value";
      }
    }
    const held = Object.fromEntries(Object.values(fields).map(field => [field, "retained"]));
    const stashKey = harness === "omp" ? "__stOmpChannel" : "__stPiChannel";
    if (mode.startsWith("reload")) {
      context[mode === "reload-legacy" ? stashKey.replace("__st", "__st2") : stashKey] = held;
      // Re-instantiation must still unexport injected ownership values.
      for (const suffix of Object.keys(fields)) env[preferred + suffix] = "reinjected";
    }
    const state = context.contract.stash();
    for (const [suffix, field] of Object.entries(fields)) {
      let expected;
      if (mode.startsWith("reload")) expected = "retained";
      else if (["preferred", "equal", "conflict", "legacy-conflict", "legacy-empty"].includes(mode)) expected = ["conflict", "legacy-conflict", "legacy-empty"].includes(mode) && identityFields.has(suffix) ? undefined : "preferred-value";
      else if (mode === "current") expected = "current-value";
      else if (mode === "legacy") expected = "legacy-value";
      else if (["empty", "current-empty"].includes(mode)) expected = "";
      assert.equal(state[field], expected, `${file} ${mode} ${field}`);
      for (const name of [preferred + suffix, prefix + suffix, old + suffix]) assert.equal(env[name], undefined, `${file} ${mode} clears ${name}`);
    }
    const conflictCount = ["conflict", "legacy-conflict", "legacy-empty"].includes(mode) ? identityFields.size : mode === "expected-session-conflict" ? 1 : 0;
    assert.equal(reports.length, conflictCount, `${file} ${mode} conflict count`);
    for (const report of reports) {
      const legacyPrefix = mode === "legacy-conflict" ? old : prefix;
      assert.ok(report.includes(preferred) && report.includes(legacyPrefix), "diagnostic names preferred and selected legacy variables");
      assert.ok(!report.includes("preferred-value") && !report.includes("current-value") && !report.includes("legacy-value"), "diagnostic excludes values");
    }
    if (mode.startsWith("reload")) assert.equal(state, held, "reload preserves ownership");
    for (const value of [undefined, "", "owned"]) {
      const child = {};
      context.contract.writeEnv(child, prefix + "SESSION", value);
      assert.equal(child[preferred + "SESSION"], value, "writer sets preferred identity including empty");
      assert.equal(child[prefix + "SESSION"], value, "writer sets legacy identity including empty");
    }
    if (file === "st-omp") {
      env.FRACTAL_OMP_PENDING_ASK = "";
      env.ST3_OMP_PENDING_ASK = "old-ask";
      assert.equal(context.contract.readEnv("ST3_OMP_PENDING_ASK"), "", "preferred empty masks pending ask bridge");
      context.contract.removeEnv("ST3_OMP_PENDING_ASK");
      assert.equal(env.FRACTAL_OMP_PENDING_ASK, undefined);
      assert.equal(env.ST3_OMP_PENDING_ASK, undefined);
      const child = {};
      context.contract.writeEnv(child, "ST3_OMP_PENDING_ASK", "ask");
      assert.equal(child.FRACTAL_OMP_PENDING_ASK, "ask");
      assert.equal(child.ST3_OMP_PENDING_ASK, "ask");
    }
  }
}
console.log("preferred, legacy, conflicting, empty, dual-written, cleared, and reloaded hook environments passed");
