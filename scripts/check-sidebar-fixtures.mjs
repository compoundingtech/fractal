import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { isIP } from 'node:net';

const fixtureDirectory = fileURLToPath(new URL('../fixtures/clients/sidebar/', import.meta.url));
function readJson(path) {
  try { return JSON.parse(readFileSync(path, 'utf8')); }
  catch { throw new Error('invalid sidebar fixture JSON'); }
}
export const behaviorSchema = readJson(join(fixtureDirectory, 'behavior.schema.json'));
const keywords = new Set(['$schema', '$defs', '$ref', 'description', 'type', 'const', 'enum', 'oneOf',
  'properties', 'additionalProperties', 'required', 'items', 'minItems', 'maxItems', 'minLength', 'minimum']);

// Fail closed if the shared schema starts using a keyword this small walker cannot enforce.
export function checkSchema(schema) {
  for (const [key, value] of Object.entries(schema)) {
    assert.ok(keywords.has(key), 'unsupported behavior-schema keyword');
    if (key === 'properties' || key === '$defs') Object.values(value).forEach(checkSchema);
    else if (key === 'oneOf') value.forEach(checkSchema);
    else if (['items', 'additionalProperties'].includes(key) && typeof value === 'object') checkSchema(value);
  }
}
checkSchema(behaviorSchema);

export function checkEnvelope(value, schema = behaviorSchema) {
  if (schema.$ref) {
    assert.ok(schema.$ref.startsWith('#/$defs/'), 'only local behavior-schema references are supported');
    return checkEnvelope(value, behaviorSchema.$defs[schema.$ref.slice(8)]);
  }
  if (schema.oneOf) {
    const matches = schema.oneOf.filter(branch => {
      try { checkEnvelope(value, branch); return true; } catch { return false; }
    });
    assert.equal(matches.length, 1, 'behavior-schema union mismatch');
  }
  if (schema.const !== undefined) assert.ok(JSON.stringify(value) === JSON.stringify(schema.const), 'behavior-schema constant mismatch');
  if (schema.enum) assert.ok(schema.enum.includes(value), 'behavior-schema enum mismatch');
  const type = value === null ? 'null' : Array.isArray(value) ? 'array' : typeof value;
  if (schema.type) assert.ok([schema.type].flat().some(expected => expected === type ||
    (expected === 'integer' && Number.isInteger(value))), 'behavior-schema type mismatch');
  if (schema.minLength !== undefined) assert.ok([...value].length >= schema.minLength, 'behavior-schema string bound');
  if (schema.minimum !== undefined) assert.ok(value >= schema.minimum, 'behavior-schema numeric bound');
  if (Array.isArray(value)) {
    assert.ok(value.length >= (schema.minItems ?? 0) && value.length <= (schema.maxItems ?? Infinity), 'behavior-schema array bound');
    if (schema.items) for (const item of value) checkEnvelope(item, schema.items);
  } else if (value !== null && typeof value === 'object') {
    for (const key of schema.required ?? []) assert.ok(Object.hasOwn(value, key), 'behavior-schema required field');
    for (const [key, item] of Object.entries(value)) {
      const child = schema.properties?.[key] ?? schema.additionalProperties;
      assert.ok(child !== false && (child !== undefined || schema.additionalProperties !== false), 'behavior-schema unknown field');
      if (child && typeof child === 'object') checkEnvelope(item, child);
    }
  }
}

const folder = '00000000-0000-7000-8000-[0-9a-f]{12}';
const subject = '(?:person/test(?:-[0-9]+)?|agent/test/[^\\s/]+|machine/retained|(?:machine|host)/test-[0-9]+)';
const identities = new RegExp(`^(?:${subject}|arrangement/person/test(?:-[0-9]+)?/${folder})$`, 'u');
const folderId = new RegExp(`^${folder}$`);
const rowId = new RegExp(`^(?:folder:${folder}|subject:${subject})$`, 'u');
const legacyNames = new Set(['empty', 'nested-and-dormant-non-agent-placements',
  'multi-tombstone-lift-and-unknown-root', 'authoritative-cycle-cut-not-revision-order',
  'separate-domains-and-unicode-scalar-ties']);
const forbidden = [
  /(?:\/(?:home|Users)\/|[a-z]:[\\/](?:Users|Documents and Settings)[\\/]|~\/|file:\/\/)/i,
  /\b(?:localhost|[a-z0-9-]+\.(?:internal|local|lan|corp|private|home|intranet|localdomain)|[a-z0-9.-]+\.ts\.net)\b/i,
  /(?:https?|ssh):\/\//i,
  /\b(?:\d{1,3}\.){3}\d{1,3}\b/,
  /\b(?:(?:AKIA|ASIA)[A-Z0-9]{16}|xox[baprs]-[a-z0-9-]+|eyJ[a-z0-9_-]+\.[a-z0-9_-]+\.[a-z0-9_-]+|Basic\s+[a-z0-9+/=]+)/i,
  /(?:arn:|urn:|agent:\/\/|session:\/\/|tracker\/|ticket\/|\b[A-Z]{2,}-\d+|\bsession[-_/][a-z0-9]+)/,
  /(?:\bBearer\s+\S+|\b(?:gh[pousr]_|github_pat_|sk[-_])[a-z0-9_-]+|-----BEGIN [A-Z ]*PRIVATE KEY-----|(?:password|token|secret)\s*[:=])/i,
];
function checkText(text) {
  // The schema dialect URL is public metadata, not a fixture endpoint.
  if (text === 'https://json-schema.org/draft/2020-12/schema') return;
  assert.ok(!forbidden.some(pattern => pattern.test(text)), 'forbidden private fixture text');
  assert.ok(!/(?:password|credential|authorization|api[_-]?key|private[_-]?key|access[_-]?token|\btoken\b|\bsecret\b)/i.test(text), 'credential fixture field');
  for (const token of text.split(/[^a-f0-9:.]+/i)) {
    assert.ok(!isIP(token) && !isIP(token.replace(/^\[|\]$/g, '')), 'fixture IP address');
  }
}

// Check both keys and values, after JSON decoding (including escaped strings).
export function scrub(value) {
  if (typeof value === 'string') checkText(value);
  else if (Array.isArray(value)) value.forEach(scrub);
  else if (value !== null && typeof value === 'object') {
    for (const [key, item] of Object.entries(value)) { checkText(key); scrub(item); }
  }
}
function publicStrings(value, schema = behaviorSchema, field = '') {
  if (schema.$ref) return publicStrings(value, behaviorSchema.$defs[schema.$ref.slice(8)], field);
  if (schema.oneOf && !schema.properties) return publicStrings(value, schema.oneOf.find(branch => {
    try { checkEnvelope(value, branch); return true; } catch { return false; }
  }), field);
  if (typeof value === 'string' && schema.const === undefined && !schema.enum) {
    let allowed = false;
    if (field === 'name') allowed = legacyNames.has(value) || /^synthetic-case-[0-9]+$/.test(value);
    else if (field === 'owner') allowed = /^person\/test(?:-[0-9]+)?$/.test(value);
    else if (field === 'value') allowed = /^(?:Sidebar|Folder|Synthetic (?:Folder|Resource) [0-9]+)$/.test(value);
    else if (['revision', 'arrangementRevision'].includes(field)) allowed = /^claim\/(?:head|create|name|place|position|delete|test-[0-9]+)$/.test(value);
    else if (field === 'updated_at') allowed = /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(value);
    else if (field === 'key') allowed = /^[a-zA-Z][a-zA-Z0-9]*$/.test(value);
    else if (['parent', 'folder', 'bucket', 'folders', 'deletedFolders', 'parents'].includes(field)) allowed = folderId.test(value);
    else allowed = identities.test(value) || (field === 'id' && folderId.test(value));
    assert.ok(allowed, 'non-synthetic fixture identity or label');
  } else if (Array.isArray(value)) value.forEach(item => publicStrings(item, schema.items, field));
  else if (value !== null && typeof value === 'object') {
    for (const [key, item] of Object.entries(value)) {
      const child = schema.properties?.[key];
      if (!child) {
        const ref = schema.additionalProperties.$ref;
        const allowed = ref === '#/$defs/treeNode' ? rowId.test(key)
          : ref === '#/$defs/rollup' ? key.startsWith('folder:') && rowId.test(key)
          : ref === '#/$defs/folder' || field === 'parents' ? folderId.test(key) : identities.test(key);
        assert.ok(allowed, 'non-synthetic fixture map identity');
      }
      publicStrings(item, child ?? schema.additionalProperties, child ? key : field);
    }
  }
}
export function checkFixture(value) { scrub(value); checkEnvelope(value); publicStrings(value); }

// Rendered symbols/styles are presentation, not identity fields. Stable targets remain strict.
export function checkGolden(value) {
  scrub(value);
  const keys = (object, expected) => assert.deepEqual(Object.keys(object).sort(), expected.split(' ').sort());
  keys(value, 'format generatedBy size cases');
  assert.equal(value.format, 'tui-sidebar-v1');
  assert.equal(value.generatedBy, 'STUI_UPDATE_GOLDEN=1 cargo test -p fractal-tui --features fractal-daemon/test-support sidebar_golden');
  assert.deepEqual(value.size, [44, 14]);
  const stable = id => typeof id === 'string' && (rowId.test(id) ||
    /^group:(?:everything|kind\/(?:Agents|Missions|Terminals|Machines|Pull requests|Repositories|Resources|Unavailable))$/.test(id));
  const rect = r => {
    assert.ok(Array.isArray(r) && r.length === 4 && r.every(n => Number.isInteger(n) && n >= 0));
    assert.ok(r[0] + r[2] <= 44 && r[1] + r[3] <= 14);
  };
  assert.ok(Array.isArray(value.cases) && value.cases.length === 11);
  const states = new Set(['reserved', 'legacy', 'retired', 'unfocused', 'fold', 'search',
    'scroll', 'narrow', 'zero-width', 'zero-height', 'zero']);
  for (const item of value.cases) {
    keys(item, 'state area arrangement selected rows cells hits contexts');
    assert.ok(states.delete(item.state), 'unknown or repeated golden state');
    rect(item.area);
    assert.ok(item.arrangement === null || identities.test(item.arrangement));
    assert.ok(item.selected === null || stable(item.selected));
    assert.ok(Array.isArray(item.rows) && item.rows.every(stable));
    assert.ok(Array.isArray(item.cells));
    let screen = '', count = 0;
    for (const run of item.cells) {
      assert.ok(Array.isArray(run) && run.length === 2);
      const [length, cell] = run;
      assert.ok(Number.isInteger(length) && length > 0 && count + length <= 616);
      assert.ok(Array.isArray(cell) && cell.length === 2);
      assert.ok(typeof cell[0] === 'string' && cell[0].length > 0 && cell[0].length <= 8);
      assert.ok(typeof cell[1] === 'string' && cell[1].length <= 512);
      screen += cell[0].repeat(length); count += length;
    }
    assert.equal(count, 616);
    // Inspect decoded adjacent symbols too: per-cell scrubbing alone misses split secrets.
    scrub(screen);
    for (const text of [screen, ...item.cells.map(run => run[1][1])]) {
      for (const match of text.matchAll(/(?:person|agent|machine|host)\/[^\s"'·]+/g)) {
        assert.ok(identities.test(match[0]), 'non-synthetic rendered identity');
      }
    }
    assert.ok(Array.isArray(item.hits) && Array.isArray(item.contexts));
    for (const hit of item.hits) {
      assert.ok(Array.isArray(hit) && hit.length === 3); rect(hit[0]);
      assert.ok(['filter', 'new', 'row', 'add', 'menu', 'drag'].includes(hit[1]));
      assert.ok(['filter', 'new'].includes(hit[1]) ? hit[2] === null : stable(hit[2]));
    }
    for (const context of item.contexts) {
      assert.ok(Array.isArray(context) && context.length === 3); rect(context[0]);
      assert.ok(typeof context[1] === 'string' && context[1].startsWith('subject:') && rowId.test(context[1]));
      assert.ok(Number.isInteger(context[2]) && context[2] >= 0);
    }
  }
}

export function checkFixtures(directory = fixtureDirectory) {
  let count = 0;
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    assert.ok(!entry.isSymbolicLink(), 'fixture symlinks are not admitted');
    if (entry.isDirectory()) { count += checkFixtures(path); continue; }
    assert.ok(/\.(?:json|snap|snapshot)$/i.test(entry.name), 'unsupported sidebar fixture file');
    const value = readJson(path);
    if (path === join(fixtureDirectory, 'behavior.schema.json')) { scrub(value); continue; }
    if (entry.name === 'tui-v1.snapshot') checkGolden(value);
    else checkFixture(value);
    count++;
  }
  assert.ok(count > 0, 'no sidebar fixtures found');
  return count;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { console.log(`Sidebar fixture gate passed (${checkFixtures()} files).`); }
  catch { console.error('Sidebar fixture gate failed: schema or public-data policy violation.'); process.exitCode = 1; }
}
