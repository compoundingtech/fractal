const { test } = require('node:test');
const assert = require('node:assert/strict');

// Node 24 loads the generated TypeScript directly, without a transpilation copy.
const modules = Promise.all([import('effect'), import('./Schema.generated.ts')]);

test('harness questions retain canonical private metadata and open response enums', async () => {
    const [{ DateTime, Option, Schema }, Rich] = await modules;
    const wire = {
        kind: 'harness-question', id: 'harness-question/00ca25ee5d0da6af852c81a5ee4949df', revision: 'claim/ada-observed',
        updated_at: '2026-10-26T10:00:00.000Z', agent: 'agent/ada', session: 'ada-session',
        native_id: 'sha256:' + 'a'.repeat(64), runtime_incarnation: 'ada-incarnation', observed_at: 1793008800000,
        questions: [{ id: 'q0', options: 64, multi: true, required: true }],
        resolution: { status: 'answered_elsewhere', revision: 'claim/ada-observed',
            answers: { q0: { positions: [0, 63], timed_out: false } }, resolved_at: 1793008801000 },
        answer: { state: 'unavailable', reason: 'resolved' },
    };
    const decode = Rich.decodeUnknownSync(Rich.HarnessQuestion, 'strict');
    for (const value of [wire, { ...wire, resolution: null, answer: { state: 'available' } }]) {
        const decoded = decode(value);
        assert.equal(DateTime.toEpochMillis(decoded.observed_at), value.observed_at);
        assert.equal(Option.isSome(decoded.resolution), value.resolution !== null);
        assert.deepEqual(Schema.encodeSync(Rich.HarnessQuestion)(decoded), value);
    }
    const future = { ...wire, resolution: { ...wire.resolution, status: 'future_status' },
        answer: { state: 'future_state', reason: 'future_reason' } };
    const decodedFuture = Rich.decodeUnknownSync(Rich.HarnessQuestion)(future);
    assert.deepEqual(decodedFuture.answer.state, { _tag: 'Unknown', raw: 'future_state' });
    assert.deepEqual(Schema.encodeSync(Rich.HarnessQuestion)(decodedFuture), future);
    assert.throws(() => decode(future));
    for (const value of [
        { ...wire, id: 'harness-question/native' }, { ...wire, native_id: 'raw-native-id' },
        { ...wire, session: 'has whitespace' }, { ...wire, session: '' },
        { ...wire, observed_at: Number.MAX_SAFE_INTEGER + 1 },
        { ...wire, resolution: { ...wire.resolution, resolved_at: Number.MAX_SAFE_INTEGER + 1 } },
        { ...wire, questions: [] }, { ...wire, questions: [{ ...wire.questions[0], id: 'q16' }] },
        { ...wire, questions: [{ ...wire.questions[0], options: 65 }] },
        { ...wire, answer: { state: 'unavailable' } },
        { ...wire, questions: [{ ...wire.questions[0], text: 'not public' }] },
    ]) assert.throws(() => decode(value));
    assert.deepEqual(Schema.encodeSync(Rich.Capability)(Rich.decodeUnknownSync(Rich.Capability, 'strict')(
        { id: 'harness.answer', version: 0, state: 'unavailable' })), { id: 'harness.answer', version: 0, state: 'unavailable' });
    assert.throws(() => Rich.decodeUnknownSync(Rich.Capability, 'strict')({ id: 'harness.answer', version: 0, state: 'unavailable', reason: 'adapter_missing' }));
});

test('harness answer requests constrain canonical data keys, positions, fences, and action ids', async () => {
    const [{ Schema }, Rich] = await modules;
    const question = 'harness-question/00ca25ee5d0da6af852c81a5ee4949df';
    const wire = { api_version: 'st3.client.v0', id: 'action/answer', type: 'harness.answer',
        idempotency_key: 'alex-harness-answer-0001', fence: { snapshot_id: 'snapshot/fixture',
            subject_revisions: { [question]: 'claim/ada-observed' }, runtime_incarnation: 'ada-incarnation' },
        parameters: { question, answers: { q0: { positions: [0, 63] }, ['sha256:' + 'b'.repeat(64)]: { positions: [] } } } };
    const decode = Rich.decodeUnknownSync(Rich.ActionRequest, 'strict');
    assert.deepEqual(Schema.encodeSync(Rich.ActionRequest)(decode(wire)), wire);
    const input = Rich.decodeUnknownSync(Rich.HarnessAnswerInput, 'strict');
    for (const positions of [[64], [-1], [0.5], [0, 0], Array.from({ length: 65 }, (_, i) => i)])
        assert.throws(() => input({ positions }));
    for (const extra of ['note', 'other', 'timed_out', 'actor', 'credential'])
        assert.throws(() => input({ positions: [0], [extra]: false }));
    for (const answers of [{ actor: { positions: [0] } }, { credential: { positions: [0] } },
        { q16: { positions: [0] } }, Object.fromEntries(Array.from({ length: 17 }, (_, i) => ['sha256:' + i.toString(16).padStart(64, '0'), { positions: [] }]))])
        assert.throws(() => decode({ ...wire, parameters: { question, answers } }));
    assert.throws(() => decode({ ...wire, id: 'action/' + 'a'.repeat(250) }));
    assert.throws(() => decode({ ...wire, actor: 'person/other' }));
    const { runtime_incarnation, ...withoutRuntime } = wire.fence;
    assert.throws(() => decode({ ...wire, fence: withoutRuntime }));
    // Schema maxLength is a character bound; the runtime separately enforces UTF-8 bytes (R1).
    const bounded = { ...wire, id: 'action/' + 'a'.repeat(249) };
    assert.deepEqual(Schema.encodeSync(Rich.ActionRequest)(decode(bounded)), bounded);
});

test('harness refusal details stay closed while result statuses remain lossless', async () => {
    const [{ Schema }, Rich] = await modules;
    const decode = Rich.decodeUnknownSync(Rich.ErrorEnvelope, 'strict');
    const base = { api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/answer', message: '', retryable: false };
    for (const [code, reasons] of [
        ['invalid-answer', ['unknown_option', 'too_many', 'required_missing', 'empty_single', 'too_long', 'unknown_question', 'other_not_allowed', 'note_not_allowed']],
        ['not-answerable', ['adapter_missing', 'adapter_unavailable']],
    ]) {
        for (const reason of reasons) {
            const value = { ...base, code, details: { reason } };
            assert.deepEqual(Schema.encodeSync(Rich.ErrorEnvelope)(decode(value)), value);
        }
        for (const details of [{}, { reason: 'future_reason' }, { reason: 'resolved' }])
            assert.throws(() => decode({ ...base, code, details }));
    }
    const future = { ...base, code: 'future-error', details: { future: 1 } };
    assert.deepEqual(Schema.encodeSync(Rich.ErrorEnvelope)(Rich.decodeUnknownSync(Rich.ErrorEnvelope)(future)), future);
    assert.throws(() => decode(future));
    for (const status of ['accepted', 'already_resolved', 'future_status']) {
        const value = { status, resolution: { status: 'future_resolution', revision: 'claim/observed', answers: {}, resolved_at: 1793008801000 } };
        assert.deepEqual(Schema.encodeSync(Rich.HarnessAnswerResult)(Rich.decodeUnknownSync(Rich.HarnessAnswerResult)(value)), value);
        assert.throws(() => Rich.decodeUnknownSync(Rich.HarnessAnswerResult, 'strict')(value));
    }
    const known = { status: 'accepted', resolution: { status: 'answered', revision: 'claim/observed', answers: {}, resolved_at: 1793008801000 } };
    assert.deepEqual(Schema.encodeSync(Rich.HarnessAnswerResult)(Rich.decodeUnknownSync(Rich.HarnessAnswerResult, 'strict')(known)), known);
});
test('revisioned agents opt-in and frames are strict disjoint branches with lossless provenance', async () => {
    const [{ Schema }, Rich] = await modules;
    const command = require('../../../docs/st3/client-v0/fixtures/agents-publication-subscribe.json');
    const chunk = require('../../../docs/st3/client-v0/fixtures/agents-publication-chunk.json');
    const delta = require('../../../docs/st3/client-v0/fixtures/agents-publication-changes.json');
    const error = require('../../../docs/st3/client-v0/fixtures/agents-publication-row-too-large.json');
    const decodeCommand = Rich.decodeUnknownSync(Rich.CollectionCommand, 'strict');
    const decodeFrame = Rich.decodeUnknownSync(Rich.CollectionFrame, 'strict');
    assert.deepEqual(Schema.encodeSync(Rich.CollectionCommand)(decodeCommand(command)), command);
    for (const field of ['limit', 'window', 'person', 'actor', 'agent', 'status', 'filters']) {
        assert.throws(() => decodeCommand({ ...command, [field]: field === 'limit' ? 100 : null }));
    }
    assert.throws(() => decodeCommand({ ...command, agents_publication_version: 2 }));
    assert.throws(() => decodeCommand({ ...command, collection: 'work' }));
    const legacyCommand = { kind: 'subscribe', id: 'legacy', collection: 'agents', limit: 100 };
    assert.deepEqual(Schema.encodeSync(Rich.CollectionCommand)(decodeCommand(legacyCommand)),
        { ...legacyCommand, actor: null, person: null, agent: null, status: null });
    // Existing nullable Agent fields encode omitted values as null; frame provenance stays exact.
    const normalizedRows = Schema.encodeSync(Schema.Array(Rich.Agent))(
        Rich.decodeUnknownSync(Schema.Array(Rich.Agent), 'strict')(chunk.items));
    const normalizedChunk = { ...chunk, items: normalizedRows };
    for (const wire of [normalizedChunk, delta, error]) {
        assert.deepEqual(Schema.encodeSync(Rich.CollectionFrame)(decodeFrame(wire)), wire);
    }
    assert.equal(Rich.decodeUnknownSync(Rich.ErrorCode, 'strict')(error.code), error.code);
    const { publication, chunk_index, chunk_count, ...legacy } = normalizedChunk;
    assert.deepEqual(Schema.encodeSync(Rich.CollectionFrame)(decodeFrame(legacy)), legacy);
    for (const field of ['publication', 'chunk_index', 'chunk_count']) {
        const invalid = { ...chunk };
        delete invalid[field];
        assert.throws(() => decodeFrame(invalid));
    }
    assert.throws(() => decodeFrame({ ...chunk, chunk_count: 0 }));
    assert.throws(() => decodeFrame({ ...chunk, base_revision: 42 }));
    assert.throws(() => decodeFrame({ ...chunk, has_more: true }));
    assert.throws(() => decodeFrame({ ...chunk, collection: 'missions' }));
    assert.throws(() => decodeFrame({ ...delta, chunk_index: 0 }));
    assert.throws(() => decodeFrame({ ...delta, base_revision: 0 }));
    const unpublished = structuredClone(chunk);
    delete unpublished.snapshot.published_at;
    assert.throws(() => decodeFrame(unpublished));
    for (const field of ['chunk_index', 'chunk_count', 'base_revision']) {
        assert.throws(() => decodeFrame({ ...legacy, [field]: 1 }));
    }
});
test('push registration schema bounds secret input and exposes only safe device status', async () => {
    const [{ Schema }, Rich] = await modules;
    const decode = Rich.decodeUnknownSync(Rich.PushRegistration, 'strict');
    const request = { token: 'ab'.repeat(32), environment: 'development', bundle_id: 'example.push', app_version: '1.0' };
    assert.deepEqual(Schema.encodeSync(Rich.PushRegistration)(decode(request)), request);
    for (const invalid of ['', 'a', 'abc', 'zz', 'ab'.repeat(257)]) {
        assert.throws(() => decode({ ...request, token: invalid }));
    }
    for (const patch of [{ environment: 'sandbox' }, { bundle_id: '' }, { bundle_id: 'invalid topic' }, { app_version: '' }, { app_version: '\n' }]) {
        assert.throws(() => decode({ ...request, ...patch }));
    }
    const status = Rich.decodeUnknownSync(Rich.PushRegistrationStatus, 'strict');
    assert.deepEqual(Schema.encodeSync(Rich.PushRegistrationStatus)(status({ registered: false })), { registered: false });
    const registered = { registered: true, environment: 'production' };
    assert.deepEqual(Schema.encodeSync(Rich.PushRegistrationStatus)(status(registered)), registered);
    assert.throws(() => status({ registered: true }));
    assert.throws(() => status({ registered: false, environment: 'production' }));
});


test('diagnostic runtime versions match the server metadata grammar in both decode modes', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = {
        event_id: '00000000-0000-4000-8000-000000000001',
        launch_id: '00000000-0000-4000-8000-000000000002',
        launch_id_basis: 'process', sequence: 1,
        occurred_at_unix_ms: 1, occurrence_time_basis: 'exact', captured_at_unix_ms: 1,
        app_version: '0.1.0', native_build: '1', runtime_version: '0.1.0(1)',
        update_id: 'embedded', platform: 'ios', os_version: '27.0',
        severity: 'error', capture_source: 'js',
        payload: { kind: 'js-error', name: 'TypeError', fatal: false, frames: [] },
    };
    for (const mode of ['strict', 'tolerant']) {
        const decode = Rich.decodeUnknownSync(Rich.ClientDiagnosticEvent, mode);
        for (const runtime of ['0.1.0(1)', 'ios-42', 'v_1.0(build-2_3.4)', 'x'.repeat(128)]) {
            const result = Schema.encodeSync(Rich.ClientDiagnosticEvent)(decode({ ...wire, runtime_version: runtime }));
            assert.equal(result.runtime_version, runtime);
        }
        for (const runtime of ['', '(1)', '0.1.0()', '0.1.0(1)(2)', '0.1.0(1', '0.1.0(1))',
            '-version', 'version(-1)', 'version/1', 'version(é)', 'version\n', 'version(1)\n', `${'x'.repeat(126)}(1)`]) {
            assert.throws(() => decode({ ...wire, runtime_version: runtime }), `${mode}: ${runtime}`);
        }
    }
});

test('every wire version receiver accepts exactly the deployed and fractal names', async () => {
    const [{ Schema }, Rich] = await modules;
    const contract = require('../../../docs/st3/client-v0/schemas/client-v0.schema.json');
    let receivers = 0;
    const branches = schema => schema.members ? schema.members.flatMap(branches) : [schema];
    for (const [name, definition] of Object.entries(contract.$defs)) {
        for (const [field, specification] of Object.entries(definition.properties ?? {})) {
            if (!specification.enum?.some(value => typeof value === 'string' && value.startsWith('st3.'))) continue;
            receivers++;
            const [oldName, newName] = specification.enum;
            assert.deepEqual(specification.enum, [oldName, oldName.replace(/^st3\./, 'fractal.')]);
            for (const branch of branches(Rich[name])) {
                const receiver = branch.fields[field];
                assert(receiver, `${name}.${field} must exist on every receiver branch`);
                for (const mode of ['strict', 'tolerant']) {
                    const decode = Rich.decodeUnknownSync(receiver, mode);
                    for (const value of [oldName, newName]) {
                        assert.equal(Schema.encodeSync(receiver)(decode(value)), value, `${name}.${field}`);
                    }
                    for (const value of [oldName.replace(/^st3\./, 'smalltalk.'), `${newName}.unknown`,
                        `${oldName},${newName}`, [oldName, newName]]) {
                        assert.throws(() => decode(value), `${name}.${field} must reject ${JSON.stringify(value)}`);
                    }
                }
            }
        }
    }
    assert.equal(receivers, 14);
    assert.equal(Rich.API_VERSION, 'st3.client.v0');
    assert.equal(contract['x-st-integers'], 'json-safe');
    assert.equal(contract['x-fractal-integers'], undefined);
});

test('assistant senders and reply recipients decode without widening actors or signers', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = {
        id: 'timeline-entry/assistant-message', sequence: 1, revision: 1,
        timestamp: '2026-10-17T10:00:00Z', role: 'user', type: 'message', final: true,
        body: {
            message_id: 'message/assistant', from: 'assistant/chatgpt', to: 'agent/example',
            sender_class: 'external-assistant', assistant_grant_id: 'assistant-grant/example',
        },
    };
    for (const mode of ['tolerant', 'strict']) {
        const decode = Rich.decodeUnknownSync(Rich.TimelineEntry, mode);
        const decoded = decode(wire);
        const encoded = Schema.encodeSync(Rich.TimelineEntry)(decoded);
        assert.equal(encoded.body.from, 'assistant/chatgpt');
        assert.equal(encoded.body.to, 'agent/example');
        assert.equal(encoded.body.sender_class, 'external-assistant');
        assert.equal(encoded.body.assistant_grant_id, 'assistant-grant/example');
        for (const from of ['agent/example', 'person/example', 'step/example']) {
            assert.equal(Schema.encodeSync(Rich.TimelineEntry)(decode({
                ...wire, body: { ...wire.body, from },
            })).body.from, from);
        }
        for (const from of ['assistant/', 'assistant/with space', 'daemon/example']) {
            assert.throws(() => decode({ ...wire, body: { ...wire.body, from } }));
        }
        const reply = decode({
            ...wire, role: 'assistant',
            body: {
                message_id: 'message/reply', reply_to: wire.body.message_id,
                from: 'agent/example', to: 'assistant/chatgpt',
            },
        });
        const encodedReply = Schema.encodeSync(Rich.TimelineEntry)(reply);
        assert.equal(encodedReply.body.to, 'assistant/chatgpt');
        assert.equal(encodedReply.body.reply_to, wire.body.message_id);
        assert.throws(() => Rich.decodeUnknownSync(Rich.MessageProvenance, mode)({
            verdict: 'unsigned', signer: 'assistant/chatgpt',
        }));
        assert.throws(() => Rich.decodeUnknownSync(Rich.ParticipantRef, mode)('assistant/chatgpt'));
        assert.throws(() => Rich.decodeUnknownSync(Rich.ActorRef, mode)('assistant/chatgpt'));
    }
});

test('both usage aggregation literals keep agent rows through the generated schema', async () => {
    const [{ Schema }, Rich] = await modules;
    const contract = require('../../../docs/st3/client-v0/schemas/client-v0.schema.json');
    const literals = contract.$defs.UsageSummary.properties.aggregation.enum;
    assert.deepEqual(literals, [
        'cumulative-per-incarnation-else-response-deltas',
        'rollup-per-slot-else-cumulative-per-incarnation-else-response-deltas',
    ]);
    for (const mode of ['tolerant', 'strict']) {
        const decode = Rich.decodeUnknownSync(Rich.Agent, mode);
        for (const aggregation of literals) {
            const wire = {
                id: 'agent/example/worker', kind: 'agent', name: 'Worker',
                reachability: 'local', revision: '1', runtime_ids: [], state: 'waiting',
                updated_at: '2024-02-29T00:00:00.000Z',
                usage: {
                    total_tokens: 1200, input_tokens: 900, output_tokens: 300,
                    cached_tokens: 100, incarnation_count: 2, aggregation,
                },
            };
            const result = Schema.encodeSync(Rich.Agent)(decode(wire));
            assert.equal(result.id, wire.id);
            assert.deepEqual(result.usage, wire.usage);
        }
    }
});

test('pairing challenge object siblings constrain every union branch in both decode modes', async () => {
    const [{ Schema }, Rich] = await modules;
    const assistantEnvelope = require('../../../docs/st3/client-v0/fixtures/assistant-pairing-challenge.json');
    const capabilities = require('../../../docs/st3/client-v0/fixtures/capabilities.json');
    const person = {
        kind: 'pairing-challenge', pairing_id: 'pairing/person', code: 'ABCDEFGH',
        expires_at: assistantEnvelope.value.expires_at,
    };
    for (const mode of ['tolerant', 'strict']) {
        const challenge = Rich.decodeUnknownSync(Rich.PairingChallenge, mode);
        const envelope = Rich.decodeUnknownSync(Rich.Envelope, mode);
        for (const wire of [person, { ...person, grant_kind: 'person' }, assistantEnvelope.value]) {
            const encoded = Schema.encodeSync(Rich.PairingChallenge)(challenge(wire));
            assert.equal(encoded.kind, wire.kind);
            assert.equal(encoded.pairing_id, wire.pairing_id);
            assert.equal(encoded.code, wire.code);
            assert.equal(encoded.expires_at, new Date(wire.expires_at).toISOString());
            assert.equal(envelope({ ...assistantEnvelope, value: wire }).value.kind, 'pairing-challenge');
            for (const key of ['kind', 'pairing_id', 'code', 'expires_at']) {
                const missing = { ...wire };
                delete missing[key];
                assert.throws(() => challenge(missing), `${mode}: missing ${key}`);
            }
            for (const invalid of [{ kind: 'capabilities' }, { code: 'invalid' }, { expires_at: 'invalid' }]) {
                assert.throws(() => challenge({ ...wire, ...invalid }));
            }
        }
        assert.equal(envelope(capabilities).value.kind, 'capabilities');
        assert.throws(() => challenge(capabilities.value));
        assert.throws(() => challenge({ ...person, grant_id: assistantEnvelope.value.grant_id }));
        assert.throws(() => challenge({ ...assistantEnvelope.value, person_root_fingerprint: `sha256:${'A'.repeat(43)}` }));
    }
    const future = { ...person, future_field: true };
    assert.equal(Rich.decodeUnknownSync(Rich.PairingChallenge)(future).kind, 'pairing-challenge');
    assert.throws(() => Rich.decodeUnknownSync(Rich.PairingChallenge, 'strict')(future));
});

test('prompt answers keep ordinary text bounds and optional delegation across the main carry', async () => {
    const [{ Schema }, Rich] = await modules;
    for (const mode of ['strict', 'tolerant']) {
        const decode = Rich.decodeUnknownSync(Rich.PromptRespondParameters, mode);
        for (const answer of ['allow', 'deny', 'dismiss', 'approve', 'use-all', 'trust', '{"0":[1],"1":[0,2]}', 'x'.repeat(16384)]) {
            const wire = { target_id: 'agent/example/worker', episode: 'claim/prompt', answer };
            assert.deepEqual(Schema.encodeSync(Rich.PromptRespondParameters)(decode(wire)), wire);
        }
        for (const answer of ['', 'x'.repeat(16385)]) {
            assert.throws(() => decode({ target_id: 'agent/example/worker', episode: 'claim/prompt', answer }));
        }
        const wire = { target_id: 'agent/example/worker', episode: 'claim/prompt', answer: 'dismiss',
            delegation: { person: 'person/avery', policy: 'claim/policy', message: 'message/instruction',
                quote: 'Dismiss that notice.', episode: 'claim/prompt' } };
        assert.deepEqual(Schema.encodeSync(Rich.PromptRespondParameters)(decode(wire)), wire);
    }
});

test('queued mission deadlines decode null and admission timestamps', async () => {
    const [{ Option, Schema }, Rich] = await modules;
    const wire = require('../../../docs/st3/client-v0/fixtures/person-ask-mission.json');
    for (const mode of ['strict', 'tolerant']) {
        const decode = Rich.decodeUnknownSync(Rich.Mission, mode);
        const queued = structuredClone(wire);
        queued.run_details[0].after = 'mission-run/predecessor';
        queued.run_details[0].deadline = null;
        const value = decode(queued);
        assert(Option.isNone(value.run_details[0].deadline));
        assert.equal(Schema.encodeSync(Rich.Mission)(value).run_details[0].deadline, null);
        queued.run_details[0].deadline = '2026-09-20T11:05:00.000Z';
        assert(Option.isSome(decode(queued).run_details[0].deadline));
        assert.equal(Schema.encodeSync(Rich.Mission)(decode(queued)).run_details[0].deadline,
            '2026-09-20T11:05:00.000Z');
    }
});

test('terminal refusal and unconfirmed send envelopes preserve their applied answers', async () => {
    const [{ Schema }, Rich] = await modules;
    const decode = Rich.decodeUnknownSync(Rich.ErrorEnvelope);
    for (const [code, applied, refusal] of [
        ['recipient-retired', 'none', { variant: 'Retired', recipient: 'agent/example/worker' }],
        ['lifecycle-unavailable', 'none', { variant: 'LifecycleUnavailable', recipient: 'agent/example/worker', reason: 'conflict' }],
        ['message-send-unconfirmed', 'unknown', undefined],
    ]) {
        const wire = { api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/example',
            code, message: 'The send outcome', retryable: false, applied,
            details: { applied, scope: 'control.messages', future_detail: { version: 2 }, ...(refusal ? { refusal } : {}) } };
        const roundTrip = Schema.encodeSync(Rich.ErrorEnvelope)(decode(wire));
        assert.equal(roundTrip.applied, applied);
        assert.equal(roundTrip.details.applied, applied);
        assert.equal(roundTrip.details.scope, 'control.messages');
        assert.deepEqual(roundTrip.details.future_detail, { version: 2 });
        if (refusal) assert.deepEqual(roundTrip.details.refusal, refusal);
    }
});

test('directive notes decode metadata and expiry without becoming gate evidence', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = require('../../../docs/st3/client-v0/fixtures/directive-notes.json').value;
    const decode = Rich.decodeUnknownSync(Rich.DirectiveNotes, 'strict');
    const result = Schema.encodeSync(Rich.DirectiveNotes)(decode(wire));
    assert.equal(result.notes[0].person, 'person/ada');
    assert.equal(result.notes[0].author, 'person/ada');
    assert.equal(result.notes[0].expires_at, '2026-10-12T09:00:00.000Z');
    assert.equal(result.notes[0].text, wire.notes[0].text);
    assert.deepEqual(Schema.encodeSync(Rich.DirectiveNotes)(decode({ notes: [] })), { notes: [] });
    const invalid = structuredClone(wire);
    invalid.notes[0].time = '2026-02-30T09:00:00Z';
    assert.throws(() => decode(invalid));
});

test('person-ask mission rows retain actor references through the generated decoder', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = require('../../../docs/st3/client-v0/fixtures/person-ask-mission.json');
    const decode = Rich.decodeUnknownSync(Rich.Mission, 'strict');
    const roundTrip = Schema.encodeSync(Rich.Mission)(decode(wire));
    assert.equal(roundTrip.run_details[0].current_steps[0].assignee, 'person/avery');
    assert.equal(roundTrip.run_details[0].steps[0].assignee, 'person/avery');
    assert.equal(roundTrip.run_details[0].steps[0].wake.assignee, 'person/avery');
    assert.equal(roundTrip.run_details[0].steps[0].claimant, null);
    for (const actor of ['agent/asker', 'daemon/reconciler', 'person/avery']) {
        const row = structuredClone(wire);
        row.run_details[0].current_steps[0].assignee = actor;
        row.run_details[0].current_steps[0].claimant = actor;
        row.run_details[0].steps[0].assignee = actor;
        row.run_details[0].steps[0].claimant = actor;
        row.run_details[0].steps[0].wake.assignee = actor;
        assert.equal(Schema.encodeSync(Rich.Mission)(decode(row)).run_details[0].steps[0].claimant, actor);
    }
    const invalid = structuredClone(wire);
    invalid.run_details[0].steps[0].assignee = 'mission/not-an-actor';
    assert.throws(() => decode(invalid));
});

test('assistant answer records strictly preserve respondent and separate authorization provenance', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = {
        ask: 'work/person-ask', respondent: 'assistant/chatgpt',
        answered_at_unix_ms: 1792195200000, status: 'completed', summary: 'Use the first option.',
        sender_class: 'external-assistant', assistant_grant_id: 'assistant-grant/example',
        assistant_owner: 'person/avery', assistant_session_actor: 'assistant/chatgpt/session/example',
    };
    const decode = Rich.decodeUnknownSync(Rich.PersonAnswerRecord, 'strict');
    const roundTrip = Schema.encodeSync(Rich.PersonAnswerRecord)(decode(wire));
    assert.deepEqual(roundTrip, wire);
    assert.equal(roundTrip.respondent, 'assistant/chatgpt');
    assert.notEqual(roundTrip.respondent, roundTrip.assistant_owner);
    assert.notEqual(roundTrip.respondent, roundTrip.assistant_session_actor);
    for (const field of ['assistant_owner', 'assistant_session_actor', 'assistant_grant_id']) {
        assert.throws(() => decode({ ...wire, [field]: 42 }), `invalid ${field}`);
    }
    const person = {
        ask: wire.ask, respondent: 'person/avery', answered_at_unix_ms: wire.answered_at_unix_ms,
        status: 'completed', summary: wire.summary,
    };
    assert.deepEqual(Schema.encodeSync(Rich.PersonAnswerRecord)(decode(person)), person);
});

test('timestamp codecs reject normalized invalid calendar dates and preserve instants', async () => {
    const [{ DateTime, Schema }, Rich] = await modules;
    const decode = Rich.decodeUnknownSync(Rich.Timestamp);
    assert.equal(Schema.encodeSync(Rich.Timestamp)(decode('2024-02-29T12:00:00+02:00')), '2024-02-29T10:00:00.000Z');
    for (const invalid of ['2024-02-30T12:00:00Z', '1900-02-29T00:00:00Z', '2024-04-31T00:00:00Z', '2024-01-01T24:00:00Z']) {
        assert.throws(() => decode(invalid));
    }
    assert.equal(DateTime.formatIso(decode('2000-02-29T00:00:00Z')), '2000-02-29T00:00:00.000Z');
    assert.throws(() => Schema.encodeSync(Rich.Timestamp)(DateTime.makeUnsafe('+010000-01-01T00:00:00Z')));
});

test('unknown cases round-trip tolerantly and reject in both strict decoding APIs', async () => {
    const [{ Effect, Schema }, Rich] = await modules;
    const future = Rich.decodeUnknownSync(Rich.ErrorCode)('future-error');
    assert.deepEqual(future, { _tag: 'Unknown', raw: 'future-error' });
    assert.equal(Schema.encodeSync(Rich.ErrorCode)(future), 'future-error');
    assert.equal(Rich.decodeUnknownSync(Rich.ErrorCode, 'strict')('not-found'), 'not-found');
    assert.equal(Rich.decodeUnknownSync(Rich.ErrorCode, 'strict')('agent-roster-cut-not-ready'), 'agent-roster-cut-not-ready');
    assert.equal(Schema.encodeSync(Rich.ErrorCode)('agent-roster-cut-not-ready'), 'agent-roster-cut-not-ready');
    assert.throws(() => Rich.decodeUnknownSync(Rich.ErrorCode, 'strict')('future-error'));
    assert.deepEqual(Effect.runSync(Rich.decodeUnknownEffect(Rich.ErrorCode)('future-error')), future);
    assert.throws(() => Effect.runSync(Rich.decodeUnknownEffect(Rich.ErrorCode, 'strict')('future-error')));
    assert.throws(() => Schema.encodeSync(Rich.ErrorCode)({ _tag: 'Unknown', raw: 'not-found' }));
});

test('optional agent lifecycle preserves known values and absence while future values keep roster rows', async () => {
    const [{ Effect, Schema }, Rich] = await modules;
    const wire = {
        id: 'agent/example/worker', kind: 'agent', name: 'Worker',
        reachability: 'local', revision: '1', runtime_ids: [], state: 'waiting',
        updated_at: '2024-02-29T00:00:00.000Z',
    };
    for (const mode of ['tolerant', 'strict']) {
        const decode = Rich.decodeUnknownSync(Rich.Agent, mode);
        const absent = decode(wire);
        assert.equal(Object.hasOwn(absent, 'lifecycle'), false);
        assert.equal(Object.hasOwn(Schema.encodeSync(Rich.Agent)(absent), 'lifecycle'), false);
        for (const lifecycle of ['standing', 'owner', 'bounded']) {
            const decoded = decode({ ...wire, lifecycle });
            assert.equal(decoded.lifecycle, lifecycle);
            assert.equal(Schema.encodeSync(Rich.Agent)(decoded).lifecycle, lifecycle);
            assert.equal(Effect.runSync(Rich.decodeUnknownEffect(Rich.Agent, mode)({ ...wire, lifecycle })).lifecycle, lifecycle);
        }
    }

    const future = { ...wire, lifecycle: 'future-kind' };
    const unknown = { _tag: 'Unknown', raw: 'future-kind' };
    const decoded = Rich.decodeUnknownSync(Rich.Agent)(future);
    assert.deepEqual(decoded.lifecycle, unknown);
    assert.equal(Schema.encodeSync(Rich.Agent)(decoded).lifecycle, 'future-kind');
    assert.deepEqual(Effect.runSync(Rich.decodeUnknownEffect(Rich.Agent)(future)).lifecycle, unknown);
    assert.throws(() => Rich.decodeUnknownSync(Rich.Agent, 'strict')(future));
    assert.throws(() => Effect.runSync(Rich.decodeUnknownEffect(Rich.Agent, 'strict')(future)));
    assert.throws(() => Schema.encodeSync(Rich.Agent)({ ...decoded, lifecycle: { _tag: 'Unknown', raw: 'standing' } }));

    const roster = {
        kind: 'page', collection: 'agents', filters: {},
        items: [wire, future], page: { limit: 50, has_more: false },
    };
    const page = Rich.decodeUnknownSync(Rich.Page)(roster);
    assert.equal(page.items.length, 2);
    assert.equal(Object.hasOwn(page.items[0], 'lifecycle'), false);
    assert.equal(page.items[1].id, wire.id);
    assert.equal(page.items[1].name, wire.name);
    assert.equal(page.items[1].state, wire.state);
    assert.deepEqual(page.items[1].lifecycle, unknown);
    const encoded = Schema.encodeSync(Rich.Page)(page);
    assert.equal(encoded.items.length, 2);
    assert.equal(Object.hasOwn(encoded.items[0], 'lifecycle'), false);
    assert.equal(encoded.items[1].lifecycle, 'future-kind');
    assert.throws(() => Rich.decodeUnknownSync(Rich.Page, 'strict')(roster));
    assert.throws(() => Effect.runSync(Rich.decodeUnknownEffect(Rich.Page, 'strict')(roster)));
});

test('recursive Glass layouts enforce child bounds and strict nested keys', async () => {
    const [{ Schema }, Rich] = await modules;
    const layout = { split: 'right', children: [{ tabs: [{ pane: 'left' }] }, { tabs: [{ pane: 'right' }] }] };
    assert.deepEqual(Rich.decodeUnknownSync(Rich.GlassLayout, 'strict')(layout), layout);
    assert.deepEqual(Schema.encodeSync(Rich.GlassLayout)(layout), layout);
    assert.throws(() => Rich.decodeUnknownSync(Rich.GlassLayout)({ split: 'right', children: [{ tabs: [] }] }));
    const extra = { tabs: [{ pane: 'left', future: true }] };
    assert.deepEqual(Rich.decodeUnknownSync(Rich.GlassLayout)(extra), { tabs: [{ pane: 'left' }] });
    assert.throws(() => Rich.decodeUnknownSync(Rich.GlassLayout, 'strict')(extra));
});

test('nullable Work timing codecs equate missing and null while rejecting unsafe wire integers', async () => {
    const [{ DateTime, Duration, Option, Schema }, Rich] = await modules;
    const wire = {
        id: 'work/test', kind: 'work', revision: '1', updated_at: '2024-02-29T00:00:00Z',
        mission_run_id: 'mission-run/test', generation_id: 'run-generation/test', definition_id: '1',
        path: 'test', state: 'ready', attempt: 0, readiness_epoch: 0, goals: [], constraints: [],
    };
    const decode = Rich.decodeUnknownSync(Rich.Work, 'strict');
    const absent = decode(wire);
    const nullable = decode({ ...wire, claim_expires_at_unix_ms: null, execution_started_at_unix_ms: null, timeout_ms: null });
    assert.deepEqual(absent, nullable);
    assert(Option.isNone(absent.claim_expires_at_unix_ms));
    assert.equal(Schema.encodeSync(Rich.Work)(absent).claim_expires_at_unix_ms, null);
    const timed = decode({ ...wire, claim_expires_at_unix_ms: 1000, execution_elapsed_ms: 12, timeout_ms: 15 });
    assert(Option.isSome(timed.claim_expires_at_unix_ms));
    assert.equal(DateTime.toEpochMillis(timed.claim_expires_at_unix_ms.value), 1000);
    assert.equal(Duration.toMillis(timed.execution_elapsed_ms), 12);
    assert(Option.isSome(timed.timeout_ms));
    assert.equal(Duration.toMillis(timed.timeout_ms.value), 15);
    for (const invalid of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
        assert.throws(() => decode({ ...wire, execution_elapsed_ms: invalid }));
    }
    assert.throws(() => decode({ ...wire, claim_expires_at_unix_ms: 253402300800000 }));
    assert.throws(() => Schema.encodeSync(Rich.Work)({ ...timed, attempt: Number.MAX_SAFE_INTEGER + 1 }));
    assert.throws(() => Schema.encodeSync(Rich.Work)({ ...timed, execution_elapsed_ms: Duration.infinity }));
    const tolerated = Rich.decodeUnknownSync(Rich.Work)({ ...wire, agentless: null });
    assert.equal(Object.hasOwn(tolerated, 'agentless'), false);
    assert.equal(Object.hasOwn(Schema.encodeSync(Rich.Work)(tolerated), 'agentless'), false);
    assert.throws(() => decode({ ...wire, agentless: null }));
});

test('second durations preserve safe wire units without millisecond precision loss', async () => {
    const [{ Duration, Option, Schema }, Rich] = await modules;
    const wire = { host_id: 'host/test', peer_only_envelopes: 0, local_only_envelopes: 0 };
    for (const seconds of [0, 2, Number.MAX_SAFE_INTEGER - 2, Number.MAX_SAFE_INTEGER]) {
        const decoded = Rich.decodeUnknownSync(Rich.SyncPeer, 'strict')({ ...wire, estimated_catch_up_seconds: seconds });
        assert(Option.isSome(decoded.estimated_catch_up_seconds));
        assert.equal(Duration.toNanosUnsafe(decoded.estimated_catch_up_seconds.value), BigInt(seconds) * 1_000_000_000n);
        assert.equal(Schema.encodeSync(Rich.SyncPeer)(decoded).estimated_catch_up_seconds, seconds);
    }
    const decoded = Rich.decodeUnknownSync(Rich.SyncPeer)(wire);
    assert.equal(Schema.encodeSync(Rich.SyncPeer)({ ...decoded, estimated_catch_up_seconds: Option.some(Duration.millis(2500)) }).estimated_catch_up_seconds, 3);
    assert.throws(() => Rich.decodeUnknownSync(Rich.SyncPeer)({ ...wire, estimated_catch_up_seconds: Number.MAX_SAFE_INTEGER + 1 }));
    assert.throws(() => Schema.encodeSync(Rich.SyncPeer)({ ...decoded, estimated_catch_up_seconds: Option.some(Duration.nanos((BigInt(Number.MAX_SAFE_INTEGER) + 1n) * 1_000_000_000n)) }));
});

test('strict mode distinguishes real unknown enums from arbitrary same-shaped JSON bags', async () => {
    const [{ Schema }, Rich] = await modules;
    const payload = Schema.Struct({
        details: Schema.Unknown,
        codes: Schema.Array(Rich.ErrorCode),
        optional: Schema.OptionFromOptionalNullOr(Rich.ErrorCode, { onNoneEncoding: null }),
    });
    const raw = { _tag: 'Unknown', raw: 'user-data' };
    const cyclic = { ...raw };
    cyclic.self = cyclic;
    assert.equal(Rich.containsUnknownCase(raw), false);
    assert.equal(Rich.containsUnknownCase(cyclic), false);
    assert.equal(Rich.decodeUnknownSync(payload, 'strict')({ details: cyclic, codes: ['not-found'] }).details, cyclic);
    const unknown = Rich.decodeUnknownSync(payload)({ details: raw, codes: ['future-error'] });
    assert.equal(Rich.containsUnknownCase(unknown), true);
    assert.throws(() => Rich.decodeUnknownSync(payload, 'strict')({ details: raw, codes: ['future-error'] }));
    assert.throws(() => Rich.decodeUnknownSync(payload, 'strict')({ details: raw, codes: [], optional: 'future-error' }));
});

test('optional string null is absent only tolerantly, while required boolean null always fails', async () => {
    const [{ Schema }, Rich] = await modules;
    const absent = { pane: 'test' };
    const tolerated = Rich.decodeUnknownSync(Rich.GlassTab)({ ...absent, title: null });
    assert.deepEqual(tolerated, absent);
    assert.deepEqual(Schema.encodeSync(Rich.GlassTab)(tolerated), absent);
    assert.throws(() => Rich.decodeUnknownSync(Rich.GlassTab, 'strict')({ ...absent, title: null }));
    const glass = {
        id: 'glass/test', kind: 'glass', revision: '1', updated_at: '2024-02-29T00:00:00Z',
        body: null, deleted: null, base_revision: null, replaced_revision: null,
    };
    for (const mode of ['tolerant', 'strict']) {
        const decode = Rich.decodeUnknownSync(Rich.Glass, mode);
        assert.equal(decode({ ...glass, deleted: false }).deleted, false);
        assert.throws(() => decode(glass));
    }
});

test('timeline discrimination preserves required headers and conditional bodies', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = {
        id: 'timeline-entry/test', sequence: 1, revision: 1, timestamp: '2024-02-29T00:00:00.000Z',
        role: 'assistant', type: 'status', final: false, body: { status: 'running' },
    };
    for (const mode of ['tolerant', 'strict']) {
        const decode = Rich.decodeUnknownSync(Rich.TimelineEntry, mode);
        assert.equal(decode(wire).body.status, 'running');
        assert.throws(() => decode({ ...wire, body: {} }));
        assert.throws(() => decode({ ...wire, id: undefined }));
        assert.throws(() => decode({ ...wire, type: 'message', body: {} }));
    }
    const future = { ...wire, type: 'future-entry', body: { new_field: true } };
    const decoded = Rich.decodeUnknownSync(Rich.TimelineEntry)(future);
    assert.deepEqual(decoded.type, { _tag: 'Unknown', raw: 'future-entry' });
    assert.deepEqual(Schema.encodeSync(Rich.TimelineEntry)(decoded), future);
    assert.throws(() => Rich.decodeUnknownSync(Rich.TimelineEntry, 'strict')(future));
});

test('subject references validate complete family identities while brands preserve wire strings', async () => {
    const [{ Schema }, Rich] = await modules;
    const decode = Rich.decodeUnknownSync(Rich.MissionRunId, 'strict');
    const id = 'mission-run/test/nested';
    assert.equal(Schema.encodeSync(Rich.MissionRunId)(decode(id)), id);
    for (const invalid of ['mission/test', 'mission-run/', 'mission-run/test value']) {
        assert.throws(() => decode(invalid));
    }
    const cursor = 'opaque-cursor';
    assert.equal(Schema.encodeSync(Rich.Cursor)(Rich.decodeUnknownSync(Rich.Cursor)(cursor)), cursor);
    assert.throws(() => Rich.decodeUnknownSync(Rich.Cursor)('short'));
});

test('paired credentials redact decoded values and restore the original wire secret', async () => {
    const [{ Redacted, Schema }, Rich] = await modules;
    const wire = {
        kind: 'paired-session', device_id: 'device/test', person_id: 'person/test',
        session_actor: 'session/test', credential: 'synthetic-credential-00000000000000',
        scopes: ['read'], expires_at: '2024-02-29T00:00:00.000Z',
    };
    const decoded = Rich.decodeUnknownSync(Rich.PairedSession, 'strict')(wire);
    assert(Redacted.isRedacted(decoded.credential));
    assert.equal(Redacted.value(decoded.credential), wire.credential);
    assert.equal(String(decoded.credential).includes(wire.credential), false);
    assert.deepEqual(Schema.encodeSync(Rich.PairedSession)(decoded), wire);
    assert.throws(() => Rich.decodeUnknownSync(Rich.PairedSession)({ ...wire, credential: 'short' }));
    assert.throws(() => Schema.encodeSync(Rich.PairedSession)({ ...decoded, credential: Redacted.make('short') }));
});

test('future resource kinds cannot conceal malformed known resources', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = {
        id: 'future-resource/test', kind: 'future-resource', revision: '1',
        updated_at: '2024-02-29T00:00:00.000Z',
    };
    const decoded = Rich.decodeUnknownSync(Rich.Resource)(wire);
    assert.deepEqual(decoded.kind, { _tag: 'Unknown', raw: 'future-resource' });
    assert.deepEqual(Schema.encodeSync(Rich.Resource)(decoded), wire);
    assert.throws(() => Rich.decodeUnknownSync(Rich.Resource, 'strict')(wire));
    for (const kind of ['work', 'mission', 'glass']) {
        assert.throws(() => Rich.decodeUnknownSync(Rich.Resource)({ ...wire, kind }));
    }
});

test('mission generation maps require complete run identities as keys', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = {
        id: 'mission/test', kind: 'mission', revision: '1', updated_at: '2024-02-29T00:00:00.000Z',
        title: 'Test', state: 'ready', mission_revision: '1', runs: ['mission-run/test'],
        run_generations: { 'mission-run/test': 'run-generation/test' }, visualization: null, usage: null,
    };
    const decode = Rich.decodeUnknownSync(Rich.Mission, 'strict');
    assert.deepEqual(Schema.encodeSync(Rich.Mission)(decode(wire)), wire);
    for (const key of ['mission-run/', 'mission/test', 'mission-run/has space']) {
        assert.throws(() => decode({ ...wire, run_generations: { [key]: 'run-generation/test' } }));
    }
});

test('resource pages decode only through ResourcesPage, never the generic Page', async () => {
    const [, Rich] = await modules;
    const page = (collection) => ({ kind: 'page', collection, filters: {}, items: [], page: { limit: 50, has_more: false } });
    assert.equal(Rich.decodeUnknownSync(Rich.Page, 'strict')(page('missions')).collection, 'missions');
    assert.throws(() => Rich.decodeUnknownSync(Rich.Page)(page('resources')));
});

test('usage pricing provenance round-trips while legacy rows remain readable', async () => {
    const [{ Schema }, Rich] = await modules;
    const counts = { total_tokens: 1100, input_tokens: 1000, output_tokens: 100,
        cache_write_tokens: 0, cache_write_1h_tokens: 0, cached_tokens: 0,
        cost_microusd: 3000, reported_cost_microusd: 0, unpriced_tokens: 0 };
    const legacy = { agent: 'agent/example.usage', pricing: 'old-label', ...counts };
    const decode = Rich.decodeUnknownSync(Rich.UsageRow, 'strict');
    assert.deepEqual(Schema.encodeSync(Rich.UsageRow)(decode(legacy)), legacy);
    const wire = { ...legacy, native_session_id: 'native-example', pricing_provenance: [{
        price_table_id: 'st.api-list', price_table_version: 'example-version', cost_source: 'computed',
        rates_usd_per_million_tokens: { input: 2, output: 10, cache_read: 0.1, cache_write_5m: 2.5, cache_write_1h: 2.5 },
        ...counts,
    }] };
    assert.deepEqual(Schema.encodeSync(Rich.UsageRow)(decode(wire)), wire);
    for (const cost_source of ['provider_reported', 'unpriced']) {
        const value = { ...legacy, pricing_provenance: [{ cost_source, ...counts }] };
        assert.deepEqual(Schema.encodeSync(Rich.UsageRow)(decode(value)), value);
    }
    assert.throws(() => decode({ ...wire, pricing_provenance: [{ ...wire.pricing_provenance[0], cost_source: 'invented' }] }));
    assert.throws(() => decode({ ...wire, pricing_provenance: [{ ...wire.pricing_provenance[0], rates_usd_per_million_tokens: { input: 2 } }] }));
});

test('usage identity metadata is additive and independent of quota age', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = { account: 'claude/unknown', driver: 'claude', measured_at_unix_ms: 1000, measured_by: 'agent/example/worker', host: 'alder', seats: [] };
    const decode = Rich.decodeUnknownSync(Rich.UsageLimit, 'strict');
    assert.equal(Object.hasOwn(decode(wire), 'identified'), false);
    for (const identified of [false, true]) {
        const decoded = decode({ ...wire, identified });
        assert.equal(decoded.identified, identified);
        assert.equal(Schema.encodeSync(Rich.UsageLimit)(decoded).identified, identified);
    }
    assert.throws(() => decode({ ...wire, identified: 'stale' }));
});

test('exact agent subscriptions preserve the selector and reject other collections', async () => {
    const [{ Schema, Option }, Rich] = await modules;
    const command = { kind: 'subscribe', id: 'details', collection: 'agents', agent: 'agent/example/worker', limit: 1 };
    const decode = Rich.decodeUnknownSync(Rich.CollectionCommand, 'strict');
    assert.deepEqual(Schema.encodeSync(Rich.CollectionCommand)(decode(command)), {
        ...command, actor: null, person: null, status: null,
    });
    assert(Option.isNone(decode({ ...command, agent: null }).agent));
    assert(Option.isNone(decode({ ...command, agent: undefined }).agent));
    for (const agent of ['worker', 'person/worker', 'agent/', 'agent/bad id']) {
        assert.throws(() => decode({ ...command, agent }));
    }
    for (const collection of ['missions', 'attention', 'work', 'glasses', 'arrangements']) {
        assert.throws(() => decode({ ...command, collection }));
    }
});

test('arrangement subscriptions require a concrete owner and placements preserve explicit root', async () => {
    const [{ Schema, Option }, Rich] = await modules;
    const command = { kind: 'subscribe', id: 'sidebar', collection: 'arrangements', person: 'person/ada', limit: 100 };
    const decodeCommand = Rich.decodeUnknownSync(Rich.CollectionCommand, 'strict');
    assert.equal(decodeCommand(command).person, 'person/ada');
    for (const person of [undefined, null, 'agent/ada', 'person/ada/other']) {
        assert.throws(() => decodeCommand({ ...command, person }));
    }
    const operation = { op: 'subject.place', subject: 'agent/ada/worker', folder: null, key: 'a0' };
    const decoded = Rich.decodeUnknownSync(Rich.ArrangementOperation, 'strict')(operation);
    assert(Option.isNone(decoded.folder));
    assert.deepEqual(Schema.encodeSync(Rich.ArrangementOperation)(decoded), operation);
    assert.throws(() => Rich.decodeUnknownSync(Rich.ArrangementOperation, 'strict')({ ...operation, folder: 'not-a-folder-id' }));
    for (const subject of ['pty/person/ada/019a0000-0000-7000-8000-000000000001', 'session/worker']) {
        assert.throws(() => Rich.decodeUnknownSync(Rich.ArrangementOperation, 'strict')({ ...operation, subject }));
    }
    assert.throws(() => Rich.decodeUnknownSync(Rich.ArrangementOperation, 'strict')({ ...operation, parent: null }));
    assert.throws(() => Rich.decodeUnknownSync(Rich.ArrangementTombstoneRegister, 'strict')({ value: false, revision: 'claim/deleted' }));
});

test('arrangement resources keep deleted folder positions and missing-seat placements distinct from resolved locations', async () => {
    const [{ Schema }, Rich] = await modules;
    const fixture = require('../../../fixtures/clients/arrangements-v1.json');
    const encoded = Schema.encodeSync(Rich.Arrangement)(Rich.decodeUnknownSync(Rich.Arrangement, 'strict')(fixture.resource));
    const deletedID = Object.keys(encoded.body.folders).find(id => encoded.body.folders[id].tombstone !== null);
    assert.equal(encoded.body.folders[deletedID].position.value.key, 'a0');
    assert.equal(encoded.body.placements['agent/ada/worker'].value.folder, deletedID);
    assert.equal(encoded.resolved.folders['agent/ada/worker'], null);
    assert.equal(encoded.body.placements['agent/ada/offline'].value.key, 'a1');
    assert.equal(encoded.body.folders[deletedID].tombstone.value, true);
});

test('normalized conversation fallbacks and open block payloads survive schema round trips', async () => {
    const [{ Schema }, Rich] = await modules;
    const fixture = require('../../../docs/st3/client-v0/fixtures/normalized-conversation-legacy.json');
    for (const entry of fixture.value.items) {
        const decoded = Rich.decodeUnknownSync(Rich.TimelineEntry, 'strict')(entry);
        assert.deepEqual(Schema.encodeSync(Rich.TimelineEntry)(decoded), { ...entry, timestamp: new Date(entry.timestamp).toISOString() });
    }
    const block = { id: 'native-1/0', kind: 'future-kind', source_type: 'native-future', payload: { raw: { token: 'invented-token', nested: { arguments: ['all', 'values'] } } } };
    const decoded = Rich.decodeUnknownSync(Rich.TimelineBlock, 'strict')(block);
    assert.deepEqual(Schema.encodeSync(Rich.TimelineBlock)(decoded), block);
});

test('ordered membership root/place/remove/page and strict versioned bodies round trip', async () => {
    const [{ Schema, Option }, Rich] = await modules;
    const fixture = require('../../../fixtures/clients/ordered-memberships-v2.json');
    const roundTrip = (schema, value) => Schema.encodeSync(schema)(Rich.decodeUnknownSync(schema, 'strict')(value));
    const operation = Rich.decodeUnknownSync(Rich.OrderedMembershipOperation, 'strict')(fixture.edit.parameters.operations[0]);
    assert(Option.isNone(operation.bucket));
    assert.deepEqual(roundTrip(Rich.OrderedMembershipOperation, fixture.edit.parameters.operations[0]), fixture.edit.parameters.operations[0]);
    assert.deepEqual(roundTrip(Rich.OrderedMembershipOperation, fixture.edit.parameters.operations[1]), fixture.edit.parameters.operations[1]);
    assert.deepEqual(roundTrip(Rich.OrderedMembershipPage, fixture.page), {
        ...fixture.page,
        page: { ...fixture.page.page, next_cursor: null, cursor_expires_at: null, omitted_items: null },
    });
    assert.deepEqual(roundTrip(Rich.ActionRequest, fixture.edit), fixture.edit);
    assert.deepEqual(roundTrip(Rich.CollectionCommand, fixture.subscribe), fixture.subscribe);
    assert.throws(() => roundTrip(Rich.CollectionCommand, { ...fixture.subscribe, subject: undefined }));
    assert.deepEqual(roundTrip(Rich.ArrangementBody, fixture.body), fixture.body);
    const legacy = require('../../../fixtures/clients/arrangements-v1.json');
    assert.deepEqual(roundTrip(Rich.ArrangementBody, legacy.resource.body), legacy.resource.body);
    assert.throws(() => roundTrip(Rich.ArrangementBody, { ...fixture.body, version: 3 }));
    assert.throws(() => roundTrip(Rich.ArrangementBody, { ...fixture.body, placements: {} }));
    for (const code of ['unsupported-membership-container', 'membership-owner-forbidden', 'invalid-membership-member', 'invalid-membership-bucket', 'invalid-membership-key', 'invalid-membership-operations', 'membership-edit-too-large', 'invalid-arrangement-version', 'invalid-arrangement-body']) {
        assert.equal(roundTrip(Rich.ErrorCode, code), code);
    }
});

test('ordered membership frames and envelopes decode only as membership variants', async () => {
    const [{ Schema, Option }, Rich] = await modules;
    const fixture = require('../../../fixtures/clients/ordered-memberships-v2.json');
    const decode = (schema, value) => Rich.decodeUnknownSync(schema, 'strict')(value);
    const snapshot = decode(Rich.CollectionFrame, fixture.snapshot_frame);
    assert.equal(snapshot.items[1].member, 'mission/m1');
    assert(Option.isNone(snapshot.items[0].position.bucket));
    assert.equal(Option.getOrThrow(snapshot.items[1].position.bucket), '019a0000-0000-7000-8000-000000000010');
    assert.equal(Schema.encodeSync(Rich.CollectionFrame)(snapshot).items[0].position.bucket, null);
    const changes = decode(Rich.CollectionFrame, fixture.changes_frame);
    assert.deepEqual(changes.removes, ['agent/ada/worker']);
    assert.equal(changes.upserts[0].position.key, 'Zz');
    assert.deepEqual(snapshot.membership, fixture.snapshot_frame.membership);
    assert.deepEqual(Schema.encodeSync(Rich.CollectionFrame)(snapshot).membership, fixture.snapshot_frame.membership);
    assert.deepEqual(Schema.encodeSync(Rich.OrderedMembershipState)(decode(Rich.OrderedMembershipState, fixture.changes_frame.membership)), fixture.changes_frame.membership);
    const outside = decode(Rich.CollectionFrame, fixture.outside_window_changes_frame);
    assert.deepEqual([outside.upserts, outside.removes, outside.order, outside.has_more], [[], [], changes.order, changes.has_more]);
    assert(outside.membership.changed_index > changes.membership.changed_index);
    for (const frame of [fixture.snapshot_frame, fixture.changes_frame]) {
        const { membership, ...missingState } = frame;
        assert.throws(() => decode(Rich.CollectionFrame, missingState));
        for (const field of ['container', 'live_count', 'changed_index']) {
            const { [field]: _, ...partial } = membership;
            assert.throws(() => decode(Rich.CollectionFrame, { ...frame, membership: partial }));
        }
        assert.throws(() => decode(Rich.CollectionFrame, { ...frame, membership: { ...membership, live_count: -1 } }));
        assert.throws(() => decode(Rich.CollectionFrame, { ...frame, membership: { ...membership, extra: true } }));
    }
    const { membership: _state, ...resourceShape } = fixture.snapshot_frame;
    const empty = { ...fixture.snapshot_frame, items: [], order: [] };
    assert.equal(decode(Rich.CollectionFrame, empty).collection, 'ordered-memberships');
    assert.equal(decode(Rich.CollectionFrame, { ...resourceShape, items: [], order: [], collection: 'agents' }).collection, 'agents');
    assert.throws(() => decode(Rich.CollectionFrame, { ...empty, collection: 'agents' }));
    assert.throws(() => decode(Rich.CollectionFrame, { ...fixture.snapshot_frame, collection: 'agents' }));
    const missingBucket = structuredClone(fixture.changes_frame);
    delete missingBucket.upserts[0].position.bucket;
    assert.throws(() => decode(Rich.CollectionFrame, missingBucket));
    assert.equal(decode(Rich.Envelope, fixture.envelope).value.collection, 'ordered-memberships');
    assert.equal(decode(Rich.Envelope, { ...fixture.envelope, value: fixture.page }).value.items[0].member, 'agent/ada/worker');
    assert.equal(decode(Rich.Envelope, { ...fixture.envelope, value: { ...fixture.envelope.value, collection: 'agents' } }).value.collection, 'agents');
    assert.throws(() => decode(Rich.Page, fixture.envelope.value));
});

test('assistant error views preserve exact native diagnostics in both decoder modes', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = {
        type: 'assistant_error', status: 'failed', presentation: 'full',
        is_error: true, label: 'native diagnostic', message: 'native diagnostic',
        native: {
            stopReason: 'future-stop', errorMessage: 'native\n\u001bdiagnostic',
            errorStatus: { future: [false, null, 429] }, errorId: null,
            retryRecovery: { attempt: 2, future: ['unchanged'] },
            future: { retained: true },
        },
    };
    for (const mode of ['strict', 'tolerant']) {
        const decoded = Rich.decodeUnknownSync(Rich.TimelineViewAssistantError, mode)(wire);
        assert.deepEqual(Schema.encodeSync(Rich.TimelineViewAssistantError)(decoded), wire);
        const { native, ...legacy } = wire;
        assert.deepEqual(
            Schema.encodeSync(Rich.TimelineViewAssistantError)(
                Rich.decodeUnknownSync(Rich.TimelineViewAssistantError, mode)(legacy)),
            legacy,
        );
    }
});


test('relay omission fixture keeps absent fields, nullable values, and unknown bodies', async () => {
    const [{ Schema }, Rich] = await modules;
    const fixture = require('../../../docs/st3/client-v0/fixtures/timeline-relay-omission.json');
    for (const entry of fixture.value.items) {
        const typed = Rich.decodeUnknownSync(Rich.TimelineEntry)(entry);
        assert.deepEqual(Schema.encodeSync(Rich.TimelineEntry)(typed), {
            ...entry, timestamp: new Date(entry.timestamp).toISOString(),
        });
    }
    assert.equal(fixture.value.items[0].body.reply_to, null);
});

test('phone observation schema keeps histogram pairs and live-share as separate closed types', async () => {
    const [{Schema},Rich] = await modules;
    const decode=Rich.decodeUnknownSync(Rich.ObservationReport,'strict');
    const interval={carrier:'lan',interval_start:'2026-10-10T15:00:00Z',interval_end:'2026-10-10T15:01:00Z'};
    const report={report_id:'019a1234-0000-7000-8000-000000000001',samples:[
        {...interval,kind:'latency',target:'ios-connect',count:100,over_target:1,max_ms:2000,buckets:[[103,99],[2047,1]]},
        {...interval,kind:'live-share',target:'ios-live-share',foreground_ms:10000,live_ms:9900},
    ]};
    assert.deepEqual(Schema.encodeSync(Rich.ObservationReport)(decode(report)),report);
    for (const samples of [
        [{...report.samples[0],buckets:[[103,99,1]]}],
        [{...report.samples[0],buckets:[[103,1000001]]}],
        [{...report.samples[1],kind:'latency'}],
        [{...report.samples[1],carrier:'unknown'}],
    ]) assert.throws(()=>decode({...report,samples}));
});

test('native delivery links are optional message references and round trip', async () => {
    const [{ Schema }, Rich] = await modules;
    const items = require('../../../docs/st3/client-v0/fixtures/timeline-delivery-links.json').value.items;
    for (const entry of items) {
        const typed = Rich.decodeUnknownSync(Rich.TimelineEntry, 'strict')(entry);
        assert.deepEqual(Schema.encodeSync(Rich.TimelineEntry)(typed), {
            ...entry, timestamp: new Date(entry.timestamp).toISOString(),
        });
    }
    for (const delivers of [null, 'native-user-1', 'agent/fixture', 'message/with space']) {
        assert.throws(() => Rich.decodeUnknownSync(Rich.TimelineEntry, 'strict')({ ...items[0], delivers }));
    }
});

test('producer conformance fixtures decode strictly without losing tool results', async () => {
    const [{ Schema }, Rich] = await modules;
    // arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
    const replay = require(process.env.FRACTAL_TIMELINE_CONFORMANCE_OUTPUT ?? process.env.ST_TIMELINE_CONFORMANCE_OUTPUT
        ?? '../../../docs/st3/client-v0/fixtures/timeline-conformance.json');
    const fixtures = Array.isArray(replay) ? replay : replay.items;
    assert.ok(fixtures.length > 0);
    const decode = Rich.decodeUnknownSync(Rich.TimelineEntry, 'strict');
    for (const entry of fixtures) {
        assert.doesNotThrow(() => Schema.encodeSync(Rich.TimelineEntry)(decode(entry)), entry.id);
    }
});

test('known daemon sender schema gap remains an explicit rejection (issue #1957)', async () => {
    // https://github.com/compoundingtech/fractal/issues/1957 — pending sender amendment.
    const [, Rich] = await modules;
    const entry = {
        id: 'timeline-entry/ada', sequence: 0, revision: 1, final: true,
        timestamp: '2026-10-12T00:00:00.000Z', role: 'system', type: 'message',
        body: { message_id: 'message/tester', from: 'daemon/runtime', to: 'agent/ada' },
    };
    assert.throws(() => Rich.decodeUnknownSync(Rich.TimelineEntry, 'strict')(entry));
});

test('exchange carrier additions preserve legacy absence and future carrier values', async () => {
    const [{ Schema }, Rich] = await modules;
    const wire = { host_id: 'host/birch', peer_only_envelopes: 5, local_only_envelopes: 0, last_exchange_at: null, estimated_catch_up_seconds: null, diverged_since: null };
    const decode = Rich.decodeUnknownSync(Rich.SyncPeer, 'strict');
    assert.deepEqual(Schema.encodeSync(Rich.SyncPeer)(decode(wire)), wire);
    const observed = {
        ...wire,
        last_successful_exchange: { at: '2026-10-13T12:00:00.000Z', carrier: 'loopback' },
        last_failed_exchange: { at: '2026-10-13T11:59:00.000Z', carrier: 'fabric', fabric_route: 'unknown' },
    };
    assert.deepEqual(Schema.encodeSync(Rich.SyncPeer)(decode(observed)), observed);
    const future = { ...observed, last_successful_exchange: { ...observed.last_successful_exchange, carrier: 'future-carrier' } };
    assert.deepEqual(Schema.encodeSync(Rich.SyncPeer)(decode(future)), future);
    assert.throws(() => decode({ ...observed, last_failed_exchange: { carrier: 'fabric' } }));
});

test('stream sync notice frames decode in both modes, and a bad notice never reads as level', async () => {
    const [, Rich] = await modules;
    const fixture = require('../../../fixtures/clients/stream-sync/frames.json');
    // What a reader makes of a decoded frame's sync: only exactly level, or a catching-up or
    // diverged with peers, is known; anything else is unknown.
    const read = (sync) => sync === undefined ? 'unknown'
        : sync.state === 'level' && !sync.peers ? 'level'
        : (sync.state === 'catching-up' || sync.state === 'diverged') && sync.peers?.length ? sync.state
        : 'unknown';
    for (const [name, { frame, expect }] of Object.entries(fixture.valid)) {
        for (const mode of ['tolerant', 'strict']) {
            const decoded = Rich.decodeUnknownSync(Rich.CollectionFrame, mode)(frame);
            assert.equal(read(decoded.sync), expect, `${name} ${mode}`);
        }
        // A key a newer daemon adds is ignored when tolerant, the production default.
        const newer = { ...frame, a_key_from_a_newer_daemon: { any: 'value' } };
        assert.equal(read(Rich.decodeUnknownSync(Rich.CollectionFrame, 'tolerant')(newer).sync), expect, `${name} newer`);
    }
    for (const [name, { frame, expect }] of Object.entries(fixture.invalid)) {
        assert.throws(() => Rich.decodeUnknownSync(Rich.CollectionFrame, 'strict')(frame), undefined, `${name} strict`);
        assert.equal(expect, 'unknown');
        // Tolerant decoding never turns a bad notice into level: it fails the frame, or reads it
        // as something the reader rule calls unknown.
        let decoded;
        try { decoded = Rich.decodeUnknownSync(Rich.CollectionFrame, 'tolerant')(frame); } catch { decoded = undefined; }
        if (decoded) assert.notEqual(read(decoded.sync), 'level', `${name} tolerant`);
    }
});

test('message provenance retains its serving model and reads historical absence strictly', async () => {
    const [{ Schema }, Rich] = await modules;
    const decode = Rich.decodeUnknownSync(Rich.MessageProvenance, 'strict');
    for (const authorization_model of ['legacy', 'candidate']) {
        const wire = { verdict: 'unsigned', authorization_model };
        assert.deepEqual(Schema.encodeSync(Rich.MessageProvenance)(decode(wire)), wire);
    }
    const historical = { verdict: 'unsigned' };
    assert.deepEqual(Schema.encodeSync(Rich.MessageProvenance)(decode(historical)), historical);
    assert.throws(() => decode({ ...historical, authorization_model: 'unqualified-model' }));
    assert.throws(() => decode({ ...historical, authorization_model: null }));
    assert.throws(() => decode({ ...historical, undeclared_authority: true }));
});


test('schedule schema tolerates future state strings and retains raw declaration fields', async () => {
    const fs = require('node:fs'); const path = require('node:path');
    const [{ Schema }, Rich] = await modules;
    const wire = JSON.parse(fs.readFileSync(path.join(__dirname, '../../../docs/st3/client-v0/fixtures/schedules-page.json'), 'utf8')).value;
    const decoded = Rich.decodeUnknownSync(Rich.SchedulesPage, 'tolerant')(wire);
    // PageInfo.omitted_items is optional on the wire and encodes as null when the daemon leaves it out.
    assert.deepEqual(Schema.encodeSync(Rich.SchedulesPage)(decoded), { ...wire, page: { omitted_items: null, ...wire.page } });
});

test('an attention presentation carries a launch preview, an unavailable launch or a prompt', async () => {
    const [{ Schema }, Rich] = await modules;
    const decode = Rich.decodeUnknownSync(Rich.AttentionPresentation, 'strict');
    const base = { tier: 'today', kind: 'launch', agent: null, raised_by: null, waiting: null };
    const roundTrips = presentation => assert.deepEqual(Schema.encodeSync(Rich.AttentionPresentation)(decode(presentation)), presentation);
    roundTrips({ ...base, launch: {
        variant: 'variant/default', goals: ['Audit the build.'], asks_you: 1,
        steps: [{ path: 'approve', title: 'Approve', assignee: { kind: 'st', agent: null, label: 'st' }, after: ['prepare'], asks_you: true }],
        agents: [{ id: 'agent/example/oak', label: 'Oak', harness: 'claude', host: null }],
    } });
    roundTrips({ ...base, launch: { unavailable: { reason: 'The planner has not proposed a mission yet.', diagnostics: [] } } });
    const call = { call_id: 'call-1', tool: 'Bash', input: '{"command":"ls"}', input_complete: true };
    const prompt = { ...base, tier: 'stopped', kind: 'prompt' };
    roundTrips({ ...prompt, prompt: { calls: [call], answers: ['allow', 'deny'], allow: true } });
    roundTrips({ ...prompt, prompt: { calls: null, answers: ['deny'], allow: false, unknown_reason: 'the timeline cannot be read' } });
    // An input is cut at 4 KiB, and a prompt names at most five calls.
    assert.throws(() => decode({ ...prompt, prompt: { calls: [{ ...call, input: 'x'.repeat(4097) }], answers: [], allow: false } }));
    assert.throws(() => decode({ ...prompt, prompt: { calls: Array(6).fill(call), answers: [], allow: false } }));
    // A row from a daemon that serves neither still decodes.
    roundTrips(base);
});

test('optional author and recipients retain closed legacy scalar types', async () => {
    const [{Schema},Rich] = await modules;
    const items = require('../../../docs/st3/client-v0/fixtures/timeline-message-participants.json').value.items;
    const decode = Rich.decodeUnknownSync(Rich.TimelineEntry,'strict');
    for (const entry of items) assert.deepEqual(Schema.encodeSync(Rich.TimelineEntry)(decode(entry)), {
        ...entry,timestamp:new Date(entry.timestamp).toISOString(),
    });
    for (const patch of [ {from:'daemon/runtime'}, {to:['agent/worker']}, {author:'step/fixture'},
        {recipients:['daemon/runtime']}, {author:null}, {recipients:null} ]) {
        assert.throws(()=>decode({...items[0],body:{...items[0].body,...patch}}));
    }
});

test('an ask resolved at its deadline decodes, and what the decoder from before deadlines refuses', async () => {
    const [{ Schema }, Rich] = await modules;
    // The answer record before deadlines: no resolved_by, and a respondent that is a sender only.
    const { resolved_by, ...before } = Rich.PersonAnswerRecord.fields;
    assert.ok(resolved_by);
    const previous = Schema.Struct({ ...before, respondent: Rich.SenderRef });
    const record = {
        ask: 'step-run/run-generation-1/ask-1', status: 'completed', respondent: 'daemon/runtime',
        answered_at_unix_ms: 1, resolved_by: 'deadline',
    };
    const withDefault = {
        ...record, summary: 'Timed out without an answer; the default applied: Keep it open (keep-open)',
        answer: { type: 'decision', outcome: 'decline', id: 'keep-open', label: 'Keep it open' },
    };
    const withoutAnswer = { ...record, summary: 'Timed out without an answer' };
    for (const wire of [withDefault, withoutAnswer]) {
        for (const mode of ['strict', 'tolerant']) {
            const current = Rich.decodeUnknownSync(Rich.PersonAnswerRecord, mode)(wire);
            assert.equal(current.respondent, 'daemon/runtime');
            assert.equal(current.resolved_by, 'deadline');
            assert.deepEqual(current.answer, wire.answer);
        }
        // An older client reads the answer itself as it always did: its outcome is one it knows
        // and it ignores resolved_by. It refuses daemon/runtime as a respondent, as it already
        // does for an ask its requester's end cancelled, so [asks] deadlines comes on only once
        // the web clients run this build.
        const old = Rich.decodeUnknownSync(previous, 'tolerant');
        assert.throws(() => old(wire));
        const read = old({ ...wire, respondent: 'person/avery' });
        assert.deepEqual(read.answer, wire.answer);
        assert.equal('resolved_by' in read, false);
    }
    // No outcome was added: the one a deadline would otherwise have needed is still refused.
    assert.throws(() => Rich.decodeUnknownSync(Rich.PersonAnswerRecord, 'strict')({
        ...withDefault, answer: { ...withDefault.answer, outcome: 'timed_out' },
    }));
});
