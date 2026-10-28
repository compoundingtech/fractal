import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { test } from 'node:test';

for (const [name, legacy, fractal, enabled] of [
  ['unset', undefined, undefined, false],
  ['legacy fallback', '1', undefined, true],
  ['Fractal only', undefined, '1', true],
  ['Fractal enables over legacy', '0', '1', true],
  ['Fractal disables over legacy', '1', '0', false],
  ['empty Fractal masks legacy', '1', '', false],
]) {
  test(`push configuration: ${name}`, () => {
    const env = { ...process.env, APP_VARIANT: 'dev' };
    delete env.ST_IOS_PUSH;
    delete env.FRACTAL_IOS_PUSH;
    if (legacy !== undefined) env.ST_IOS_PUSH = legacy;
    if (fractal !== undefined) env.FRACTAL_IOS_PUSH = fractal;
    const config = JSON.parse(execFileSync(process.execPath, [
      '-e', 'console.log(JSON.stringify(require("./app.config.js")()))',
    ], { cwd: new URL('.', import.meta.url), env, encoding: 'utf8' }));
    assert.equal(config.extra?.stIosPush === true, enabled);
    assert.equal(config.plugins.some((plugin) => Array.isArray(plugin) && plugin[0] === 'expo-notifications'), enabled);
    assert.equal(config.plugins.includes('./plugins/withoutPush'), !enabled);
  });
}
