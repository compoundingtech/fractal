import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

const script = readFileSync(new URL('./scripts/landing-screenshots', import.meta.url), 'utf8');
const captureStart = script.indexOf('for view in now conversation; do\n');
assert.notEqual(captureStart, -1, 'capture loop must remain covered by these checks');
const capture = script.slice(captureStart);
const png = readFileSync(new URL('./assets/icon.png', import.meta.url));

function capturePair(t, identicalView) {
  const directory = mkdtempSync(join(tmpdir(), 'fractal-landing-capture-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const out = join(directory, 'capture output');
  const fixtures = join(directory, 'fixtures');
  mkdirSync(out);
  mkdirSync(fixtures);
  for (const view of ['now', 'conversation']) {
    writeFileSync(join(fixtures, `${view}-light.png`), png);
    // The guard compares file bytes, not decoded pixels: trailing bytes suffice
    // to make a distinct fixture without introducing an image dependency.
    writeFileSync(join(fixtures, `${view}-dark.png`), view === identicalView
      ? png : Buffer.concat([png, Buffer.from(view)]));
  }
  // Run the production capture loop with only macOS/interactive capture commands
  // replaced. File creation, cmp, diagnostics and fatal shell exit remain real.
  const result = spawnSync('bash', ['-c', `
set -Eeuo pipefail
out="$1"
fixtures="$2"
sim=test-simulator
sleep() { :; }
run() {
  case "$1" in
    maestro) : ;;
    xcrun)
      if [[ "$3" = io ]]; then
        local destination="${'${@: -1}'}"
        cp "$fixtures/${'${destination##*/}'}" "$destination"
      fi
      ;;
    *) return 1 ;;
  esac
}
${capture}`, 'landing-capture-test', out, fixtures], { encoding: 'utf8' });
  assert.ifError(result.error);
  return { ...result, out, files: readdirSync(out).filter(name => name.endsWith('.png')).sort() };
}

for (const view of ['now', 'conversation']) {
  test(`landing capture rejects byte-identical ${view} light/dark PNGs`, t => {
    const result = capturePair(t, view);
    assert.equal(result.status, 1, result.stderr);
    assert.ok(result.stderr.includes(`FAILED: ${view} light/dark screenshots are byte-identical:`));
    assert.ok(result.stderr.includes(join(result.out, `${view}-light.png`)));
    assert.ok(result.stderr.includes(join(result.out, `${view}-dark.png`)));
    assert.doesNotMatch(result.stdout, /Screenshots saved/);
    assert.deepEqual(result.files, view === 'now'
      ? ['now-dark.png', 'now-light.png']
      : ['conversation-dark.png', 'conversation-light.png', 'now-dark.png', 'now-light.png']);
  });
}

test('landing capture accepts distinct PNG bytes for both views', t => {
  const result = capturePair(t);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr, '');
  assert.ok(result.stdout.includes(`Screenshots saved to ${result.out}`));
  assert.deepEqual(result.files, ['conversation-dark.png', 'conversation-light.png', 'now-dark.png', 'now-light.png']);
});
