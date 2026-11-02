const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const ts = require('typescript');

const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'fractal-declarations-'));
let encodeKdl;
try {
    fs.writeFileSync(path.join(temporary, 'package.json'), JSON.stringify({ type: 'commonjs' }));
    const source = fs.readFileSync(path.join(__dirname, 'index.ts'), 'utf8');
    const output = ts.transpileModule(source, {
        compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
    }).outputText;
    fs.writeFileSync(path.join(temporary, 'index.js'), output);
    ({ encodeKdl } = require(path.join(temporary, 'index.js')));
} finally {
    fs.rmSync(temporary, { recursive: true, force: true });
}

const revision = 'b283ff12e887f06fbeb2da3094b10f8fd2b9b7840f7af8031445b124829f25f7';
const editor = { id: 'docs/editor', workspace: '.', launch: { command: 'true' } };
const writer = {
    id: 'docs/writer', workspace: '.', launch: { command: 'true' },
    under: { agent: editor, reason: "The editor reviews the writer's work." },
};
const mission = {
    id: 'docs/daily-review', state: 'ready', reportTo: editor,
    goals: ['Review the documentation each day.'], completion: 'all-steps-exhausted',
    steps: [{ id: 'review', assignedTo: writer, goals: ['Review the documentation and report corrections.'] }],
};
const schedule = {
    id: 'docs/daily-review', host: 'local', every: '1d', anchor: '2026-01-01T09:00:00Z', catchUp: 'latest',
    work: { mission, revision, workspace: '.' },
};

test('the st validate daily review uses object references and matches the canonical KDL fixture exactly', () => {
    const expected = fs.readFileSync(path.join(__dirname, 'fixtures/daily-review.kdl'), 'utf8');
    assert.equal(encodeKdl({ agents: [editor, writer], missions: [mission], schedules: [schedule] }), expected);
});

test('dotted and slash-qualified agent identities match the canonical golden without a host guess', () => {
    const dotted = { ...editor, id: 'remote.editor' };
    const expected = fs.readFileSync(path.join(__dirname, 'fixtures/daily-review.kdl'), 'utf8')
        .replaceAll('docs/editor', 'remote.editor');
    const declarations = {
        agents: [dotted, { ...writer, under: { ...writer.under, agent: dotted } }],
        missions: [{ ...mission, reportTo: dotted }],
        schedules: [schedule],
    };
    assert.equal(encodeKdl(declarations), expected);
});

test('host-folded declarations and object/string references throw instead of guessing a host', () => {
    const simple = { ...editor, id: 'editor' };
    assert.throws(() => encodeKdl({ agents: [simple] }), /would be host-folded/);
    for (const reference of [simple, 'agent/editor']) {
        const declarations = [
            { agents: [{ ...writer, under: { agent: reference, reason: 'Review' } }] },
            { missions: [{ ...mission, reportTo: reference }] },
            { missions: [{ ...mission, steps: [{ ...mission.steps[0], assignedTo: reference }] }] },
        ];
        for (const document of declarations) {
            assert.throws(() => encodeKdl(document), /would be host-folded/);
        }
    }
});

test('empty and omitted optional collections add no declarations beside a nonempty collection', () => {
    assert.equal(encodeKdl({ agents: [editor], missions: [], schedules: [] }), encodeKdl({ agents: [editor] }));
});

test('argv preserves argument boundaries, order and empty arguments', () => {
    assert.equal(encodeKdl({ agents: [{
        id: 'tools/argv', workspace: '/work tree', launch: { argv: ['tool', '', 'two words', '--flag=✓'] },
    }] }), 'version 2\n\nagent "tools/argv" {\n  workspace "/work tree"\n  argv "tool" "" "two words" "--flag=✓"\n}\n');
});

test('strings escape quotes, backslashes and controls using KDL-compatible quoting', () => {
    const value = 'quote" slash\\ literal\\b\\f newline\n return\r tab\t backspace\b formfeed\f nul\u0000 unicode ✓';
    const quoted = '"quote\\" slash\\\\ literal\\\\b\\\\f newline\\n return\\r tab\\t backspace\\u{8} formfeed\\u{c} nul\\u{0} unicode ✓"';
    const quotedId = `"tests/${quoted.slice(1)}`;
    assert.equal(encodeKdl({ agents: [{ id: `tests/${value}`, workspace: value, launch: { command: value } }] }),
        `version 2\n\nagent ${quotedId} {\n  workspace ${quoted}\n  command ${quoted}\n}\n`);
});

test('qualified string references match object references without adding declarations', () => {
    const strings = {
        agents: [editor, { ...writer, under: { ...writer.under, agent: 'agent/docs/editor' } }],
        missions: [{ ...mission, reportTo: 'agent/docs/editor', steps: [
            { ...mission.steps[0], assignedTo: 'agent/docs/writer' },
        ] }],
        schedules: [{ ...schedule, work: { ...schedule.work, mission: 'docs/daily-review' } }],
    };
    assert.equal(encodeKdl(strings), encodeKdl({ agents: [editor, writer], missions: [mission], schedules: [schedule] }));
    assert.equal(encodeKdl({ missions: [mission] }), `version 2\n\nmission "docs/daily-review" state="ready" report-to="agent/docs/editor" {\n  goal "Review the documentation each day."\n  completion {\n    when "all-steps-exhausted"\n  }\n  step "review" {\n    assigned-to "agent/docs/writer"\n    goal "Review the documentation and report corrections."\n  }\n}\n`);
});

test('mission states, empty step lists, and ordered goals and steps are preserved', () => {
    for (const state of ['draft', 'ready', 'retired']) {
        assert.equal(encodeKdl({ missions: [{ ...mission, state, goals: ['Work'], steps: [] }] }),
            `version 2\n\nmission "docs/daily-review" state="${state}" report-to="agent/docs/editor" {\n  goal "Work"\n  completion {\n    when "all-steps-exhausted"\n  }\n}\n`);
    }
    assert.equal(encodeKdl({ missions: [{ ...mission, goals: ['first', 'second'], steps: [
        { id: 'second', assignedTo: writer, goals: [] },
        { id: 'first', assignedTo: editor, goals: ['one', 'two'] },
    ] }] }), 'version 2\n\nmission "docs/daily-review" state="ready" report-to="agent/docs/editor" {\n  goal "first"\n  goal "second"\n  completion {\n    when "all-steps-exhausted"\n  }\n  step "second" {\n    assigned-to "agent/docs/writer"\n  }\n  step "first" {\n    assigned-to "agent/docs/editor"\n    goal "one"\n    goal "two"\n  }\n}\n');
});

test('all catch-up policies serialize a bare mission ID with its explicit revision', () => {
    for (const catchUp of ['all', 'latest', 'skip']) {
        const maxCatchUp = catchUp === 'all' ? 3 : undefined;
        assert.equal(encodeKdl({ schedules: [{ ...schedule, catchUp, maxCatchUp, work: {
            mission: 'docs/daily-review', revision, workspace: '.',
        } }] }), `version 2\n\nschedule "docs/daily-review" {\n  host "local"\n  every "1d"\n  anchor "2026-01-01T09:00:00Z"\n  catch-up "${catchUp}"\n${maxCatchUp === undefined ? '' : '  max-catch-up 3\n'}  work {\n    mission "mission/docs/daily-review@${revision}"\n    workspace "."\n  }\n}\n`);
    }
});

test('encoding neither mutates nor requires mutable declaration objects', () => {
    const agent = Object.freeze({ id: 'tools/frozen', workspace: '.', launch: Object.freeze({ argv: Object.freeze(['true']) }) });
    const declarations = Object.freeze({ agents: Object.freeze([agent]) });
    const expected = 'version 2\n\nagent "tools/frozen" {\n  workspace "."\n  argv "true"\n}\n';
    assert.equal(encodeKdl(declarations), expected);
    assert.equal(encodeKdl(declarations), expected);
});
