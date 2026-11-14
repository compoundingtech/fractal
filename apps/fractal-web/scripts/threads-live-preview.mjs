#!/usr/bin/env node
/**
 * Connected, controlled Threads preview: the real web app against a real, isolated Fractal.
 *
 *   node apps/fractal-web/scripts/threads-live-preview.mjs serve        # up + seed + serve on loopback
 *   node apps/fractal-web/scripts/threads-live-preview.mjs up           # up + seed without serving
 *   node apps/fractal-web/scripts/threads-live-preview.mjs offline|online|revoke|status|agent-reply|link-ask|down
 *
 * `serve` makes a throwaway instance (`threadspreview`) under its own HOME in PREVIEW_ROOT
 * (default /var/tmp/fractal-threadspreview), so neither `~/.st-instance` nor the default install
 * is touched. The daemon opts into thread writes and names agent/example/manager as the explicit
 * `inbox_fallback_agent`. It declares person/avery and two program seats (atlas and manager), pairs
 * one full-control owner device for avery, and through it a browser by Fractal's own browser pairing,
 * so the daemon grants the browser `thread-browser-send` (both bearers stay in PREVIEW_ROOT, mode 0600).
 * It seeds three Inbox sources:
 *   - "Review the release checklist", asked by atlas, with one earlier thread;
 *   - "Confirm the rollout window", asked by atlas, with no thread (the empty state);
 *   - the human gate of mission example/release-notes, which has no originating agent, so
 *     discussion goes to the configured Manager fallback; one thread on it, so the Missions view lists it.
 * It then serves the app with Vite on 127.0.0.1:PREVIEW_PORT (default 5317) through the existing
 * WF_ST_GATEWAY/WF_ST_AUTHORIZATION paired gateway. The browser authenticates only through that path.
 *
 * Agents in this instance are program seats: they never write on their own. The asks name Atlas
 * as their requester via `work ask --as`, and the seeded thread messages are person-authored.
 * `agent-reply` (Atlas answers the checklist thread, the Manager the mission thread) and `link-ask`
 * (Atlas links a new follow-up ask to the checklist thread) act as those declared agents through
 * the local identity header. They need a fixture executable with declared-agent attribution; any
 * other build refuses them with 403, because the fixture disables Unix peer ancestry binding and
 * device pairing grants person authority, not agent authority. Do not read person replies as agent evidence.
 * `offline` stops the daemon (the app shows its paused and reconnecting states), `online` starts
 * it again, `revoke` revokes the browser pairing (the app falls back to its refused state;
 * `serve` pairs a new device on the next start), and `down` removes everything.
 *
 * Needs: systemd user manager; FRACTAL_BINARY, the test-support `fractal-fixture` built from this checkout
 * (`cargo build --locked -p fractal --features test-support --bin fractal-fixture`), and PTY_BINARY (default
 * `pty` on PATH). Run from `nix develop .#web`.
 *
 * What this proves, and what it does not: the app, its client SDK, the paired gateway and the daemon are real and
 * connected. The daemon is the fixture executable, which turns off host process-ancestry binding (each Fractal
 * command here runs in a transient user unit, whose service-manager ancestor the release binary cannot inspect on
 * every host) and uses its own login shell. It is not the installed runtime.
 */
import { execFileSync, spawnSync } from 'node:child_process'
import { createHash, randomBytes, randomUUID } from 'node:crypto'
import { appendFileSync, chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { request } from 'node:http'
import { basename, dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const app = fileURLToPath(new URL('..', import.meta.url))
const root = process.env.PREVIEW_ROOT ?? '/var/tmp/fractal-threadspreview'
const port = Number(process.env.PREVIEW_PORT ?? 5317)
const instance = 'threadspreview'
const person = 'person/avery'
const agents = { atlas: 'agent/example/atlas', manager: 'agent/example/manager' }
const mission = 'example/release-notes'
const unit = 'threadspreview-fractal-up'
const home = join(root, 'home')
const tools = join(root, 'tools')
const instanceDir = join(home, '.st-instance', instance)
const runDir = join(instanceDir, 'run')
const authorizationFile = join(root, 'authorization') // the browser's, granted by browser pairing
const ownerAuthorizationFile = join(root, 'owner-authorization') // avery's full-control device
const browserLogin = 'avery@preview.invalid'
const gateway = () => {
  const socket = join(runDir, 'fractal-client.sock')
  // LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-11-01-socket-names-fractal — DELETE at contraction
  const legacy = join(runDir, 'st3-client.sock')
  if (!existsSync(socket) && existsSync(legacy)) return legacy
  // LIVE-MIGRATION END arn:lmig:smalltalk:2026-11-01-socket-names-fractal
  return socket
}

const which = (name) => execFileSync('sh', ['-c', 'command -v "$1"', 'sh', name], { encoding: 'utf8' }).trim()
const env = which('env')
const sleep = which('sleep')
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

/** A transient user unit with the instance's clean environment. */
const isolated = (argv) => [env, '-i', `HOME=${home}`, `PATH=${tools}:${dirname(sleep)}:/usr/bin:/bin`, `USER=${process.env.USER ?? 'preview'}`,
  'LANG=C.UTF-8', 'SHELL=/bin/sh', ...argv]
const fractal = (args, { check = true, input } = {}) => {
  const result = spawnSync('systemd-run', ['--user', '--wait', '--pipe', '--collect', '--quiet', ...isolated([join(tools, 'fractal'), '--instance', instance, ...args])],
    { encoding: 'utf8', ...(input === undefined ? {} : { input }) })
  if (check && result.status !== 0) throw new Error(`fractal ${args.join(' ')} exited ${result.status}: ${result.stderr || result.stdout}`)
  return result
}
const fractalJson = (args, options) => JSON.parse(fractal(['--json', ...args], options).stdout)
const alive = () => existsSync(gateway()) && fractal(['--daemon-wait', '0', 'agents', 'ls'], { check: false }).status === 0
const marker = (name) => join(root, name)

const gatewayCall = (method, path, body, authorization, extra = {}) => new Promise((resolve, reject) => {
  const payload = body === undefined ? undefined : JSON.stringify(body)
  const headers = { host: '127.0.0.1', ...extra, ...(authorization === undefined ? {} : { authorization }),
    ...(payload === undefined ? {} : { 'content-type': 'application/json', 'content-length': Buffer.byteLength(payload) }) }
  const req = request({ socketPath: gateway(), path, method, headers }, (res) => {
    let text = ''
    res.on('data', (chunk) => { text += chunk })
    res.on('end', () => res.statusCode === 200 ? resolve(JSON.parse(text)) : reject(new Error(`${method} ${path}: ${res.statusCode} ${text}`)))
  })
  req.on('error', reject)
  req.end(payload)
})

/**
 * The browser's credential comes from Fractal's own browser pairing, so the daemon grants it
 * `thread-browser-send` exactly as it would a browser paired through the web server: the owner
 * device asks, avery approves the "Pair browser" ask from that device, and the browser redeems
 * its one-use request key. The approval is the click a person makes in their Inbox; here the
 * script makes it, through the same client action.
 */
async function pairBrowser(owner) {
  const requestKey = randomBytes(32).toString('hex')
  const begun = (await gatewayCall('POST', '/v1/client/pairing/browser-requests', {
    deviceLabel: 'Threads preview browser', tailnetLogin: browserLogin,
    requestKeyDigest: createHash('sha256').update(requestKey).digest('hex'),
  }, owner)).value
  const inbox = await gatewayCall('GET', '/v1/client/attention', undefined, owner)
  const approval = inbox.value.items.find((item) => item.state === 'open' && item.detail?.includes(`Request: ${begun.requestId}`))
  if (approval === undefined) throw new Error(`no "Pair browser" ask for browser request ${begun.requestId}`)
  const id = `action/${randomUUID()}`
  await gatewayCall('POST', '/v1/client/actions', {
    api_version: inbox.api_version, id, type: 'work.done', idempotency_key: id,
    fence: { snapshot_id: inbox.snapshot.id, subject_revisions: { [approval.id]: approval.revision } },
    parameters: { target_id: approval.source_id, episode: approval.episode, summary: 'Approve', answer: { id: 'approve' } },
  }, owner)
  const redeemed = (await gatewayCall('POST', `/v1/client/pairing/browser-requests/${begun.requestId}/redeem`, { requestKey }, owner)).value
  if (typeof redeemed.bearer !== 'string') throw new Error(`browser request ${begun.requestId} was not approved: ${JSON.stringify(redeemed)}`)
  return `Bearer ${redeemed.bearer}`
}

/** The daemon reads its policy from the instance's config file at startup. */
function configure() {
  const directory = ['fractal', 'st3'].map((name) => join(instanceDir, 'config', name)).find((path) => existsSync(join(path, 'config.toml')))
    ?? join(instanceDir, 'config', 'st3')
  mkdirSync(directory, { recursive: true })
  const file = join(directory, 'config.toml')
  const current = existsSync(file) ? readFileSync(file, 'utf8') : ''
  const lines = [['thread_writes', 'thread_writes = true'], ['inbox_fallback_agent', `inbox_fallback_agent = "${agents.manager}"`],
    ['browser_pairing_login', `browser_pairing_login = "${browserLogin}"`]]
    .filter(([key]) => !new RegExp(`^${key}\\s*=`, 'm').test(current)).map(([, line]) => line)
  // Root keys must precede the first table.
  if (lines.length > 0) writeFileSync(file, `${lines.join('\n')}\n${current}`)
}

async function start() {
  mkdirSync(tools, { recursive: true })
  mkdirSync(join(root, 'workspace'), { recursive: true })
  if (!existsSync(join(tools, 'fractal'))) {
    const binary = process.env.FRACTAL_BINARY
    if (binary === undefined) throw new Error('Set FRACTAL_BINARY to the fractal-fixture built from this checkout')
    copyFileSync(binary, join(tools, 'fractal'))
    copyFileSync(process.env.PTY_BINARY ?? which('pty'), join(tools, 'pty')) // Fractal finds pty beside itself
    chmodSync(join(tools, 'fractal'), 0o755); chmodSync(join(tools, 'pty'), 0o755)
    symlinkSync('fractal', join(tools, 'st3'))
  }
  mkdirSync(home, { recursive: true })
  if (!existsSync(marker('fleet-created'))) {
    fractal(['fleet', 'create', '--no-service', '--dial-out', '--name', instance])
    writeFileSync(marker('fleet-created'), '')
  }
  configure()
  await launch()
  if (!existsSync(marker('setup-done'))) {
    fractal(['setup', '--person', person.slice('person/'.length), '--yes', '--service', 'false', '--install', 'false', '--harness', 'none', '--claude-channel', 'false', '--assistant', 'false'])
    writeFileSync(marker('setup-done'), '')
    // The daemon reads its person at startup; browser pairing is the configured person's alone.
    spawnSync('systemctl', ['--user', 'stop', unit], { stdio: 'inherit' })
    await launch()
  }
}

async function launch() {
  // A daemon unit that is already running is waited for, never launched a second time.
  if (spawnSync('systemctl', ['--user', 'is-active', '--quiet', unit]).status !== 0) {
    spawnSync('systemctl', ['--user', 'reset-failed', unit], { stdio: 'ignore' })
    execFileSync('systemd-run', ['--user', `--unit=${unit}`, '--collect', '--quiet', ...isolated([join(tools, 'fractal'), '--instance', instance, 'up', '--pty-binary', join(tools, 'pty')])])
  }
  for (let i = 0; i < 120 && !alive(); i++) await pause(500)
  if (!alive()) throw new Error(`The preview daemon did not start; see journalctl --user -u ${unit}`)
}

async function up() {
  if (!alive()) await start()
  if (!existsSync(marker('declared'))) {
    // Program seats exist in the roster and run; they never write on their own.
    const seats = join(root, 'seats.kdl')
    writeFileSync(seats, ['version 2', ...Object.values(agents).map((agent) =>
      `agent "${agent.slice('agent/'.length)}" {\n  workspace "${join(root, 'workspace')}"\n  argv "${sleep}" "infinity"\n}`)].join('\n') + '\n')
    fractal(['apply', seats, '--as', person])
    const missionFile = join(root, 'mission.kdl')
    writeFileSync(missionFile, `version 2
mission "${mission}" state="ready" {
  goal "The release notes are approved for publication."
  step "approve" {
    agentless
    gate "Release notes approved" mode="approve" type="human" {
      reviewer "${person}"
      question "Are the release notes ready to publish?"
    }
  }
}
`)
    fractal(['apply', missionFile, '--as', person])
    writeFileSync(marker('declared'), '')
  }
  for (let i = 0; i < 60 && (fractal(['agents', 'ls']).stdout.match(/running/g) ?? []).length < 2; i++) await pause(500)
  if (!existsSync(ownerAuthorizationFile)) {
    // avery's own device: begin locally, complete on the paired gateway.
    const challenge = fractalJson(['devices', 'pair', 'Threads preview owner device', '--full-control', '--as', person]).value
    const session = await gatewayCall('POST', `/v1/client/pairings/${encodeURIComponent(challenge.pairing_id.replace(/^pairing\//, ''))}/complete`, {
      api_version: 'st3.client.v0', code: challenge.code, device_public_key: `threads-preview-${randomBytes(24).toString('base64url')}`,
    })
    const credential = session.value?.credential ?? session.credential
    if (!credential) throw new Error('Pairing returned no credential')
    writeFileSync(ownerAuthorizationFile, `Bearer ${credential}`, { mode: 0o600 })
  }
  if (!existsSync(authorizationFile)) {
    writeFileSync(authorizationFile, await pairBrowser(readFileSync(ownerAuthorizationFile, 'utf8').trim()), { mode: 0o600 })
  }
  if (!existsSync(marker('seeded'))) seed()
  if (!existsSync(marker('mission-thread'))) discussGate()
}

function ask(key, title, reason) {
  return fractalJson(['work', 'ask', '--for', person, '--new-run', `threads-preview-${key}`, '--title', title,
    '--as', agents.atlas, '--idempotency-key', `threads-preview-${key}`, '--reason', reason]).subject
}

function seed() {
  const review = ask('review', 'Review the release checklist', 'The checklist is drafted; tell me what to change before I finalize it.')
  ask('rollout', 'Confirm the rollout window', 'I can roll out tonight or tomorrow morning.')
  fractal(['missions', 'start', mission, '--id', 'one', '--workspace', join(root, 'workspace'), '--as', person])
  // An earlier thread on the checklist, so renaming an existing thread is part of the preview.
  const started = fractal(['attention', 'discuss', review, '-m', 'Does the checklist cover the database migration?', '--as', person]).stdout
  const thread = started.match(/thread\/[A-Za-z0-9_-]+/)?.[0]
  if (thread === undefined) throw new Error(`attention discuss printed no thread: ${started}`)
  writeFileSync(marker('seeded'), `${thread}\n`)
}

/** A thread on the mission's human gate, so the Missions view lists a thread the Manager fallback holds. */
function discussGate() {
  const gate = fractalJson(['attention', 'ls', '--as', person]).value.items.find((item) => item.mission_id === `mission/${mission}`)
  if (gate === undefined) throw new Error(`no open attention for mission/${mission}`)
  const started = fractal(['attention', 'discuss', gate.id, '-m', 'Before I approve: are the breaking changes listed first?', '--as', person]).stdout
  const thread = started.match(/thread\/[A-Za-z0-9_-]+/)?.[0]
  if (thread === undefined) throw new Error(`attention discuss printed no thread: ${started}`)
  writeFileSync(marker('mission-thread'), `${thread}\n`)
}

/**
 * A declared agent acts through the local identity header on the Unix socket. Only a fixture executable with
 * declared-agent attribution binds it as that agent; any other build answers 403, and nothing is written.
 */
async function actAs(agent, type, parameters) {
  const as = { 'x-st3-person': agent }
  const capabilities = await gatewayCall('GET', '/v1/client/capabilities', undefined, undefined, as)
  const id = `action/${randomUUID()}`
  const response = await gatewayCall('POST', '/v1/client/actions', {
    api_version: capabilities.api_version, id, type, idempotency_key: id,
    fence: { snapshot_id: capabilities.snapshot.id }, parameters,
  }, undefined, as)
  if (response.value?.status !== 'completed') throw new Error(`${type} as ${agent} was not completed: ${JSON.stringify(response)}`)
  console.log(`${type} as ${agent}: completed`)
}

/** Atlas answers the checklist thread and the Manager answers the mission gate thread. */
async function agentReplies() {
  await actAs(agents.atlas, 'thread.reply', { target_id: readFileSync(marker('seeded'), 'utf8').trim(),
    content: 'Yes: the migration is step 4, and I added the rollback right after it.' })
  await actAs(agents.manager, 'thread.reply', { target_id: readFileSync(marker('mission-thread'), 'utf8').trim(),
    content: 'They are: the breaking changes open the notes, under “Before you upgrade”.' })
}

/** Atlas asks a follow-up and links it to the checklist thread, as the ask's bound requesting agent. */
async function linkAsk() {
  const subject = ask('follow-up', 'Confirm the migration window', 'The migration takes about four minutes; which window should I use?')
  const item = fractalJson(['attention', 'ls', '--as', person]).value.items.find((each) => each.source_id === subject)
  if (item?.episode === undefined) throw new Error(`no open attention for ${subject}`)
  await actAs(agents.atlas, 'thread.link-ask', { target_id: readFileSync(marker('seeded'), 'utf8').trim(), ask_id: subject, ask_episode: item.episode })
}

async function serve() {
  await up()
  process.env.WF_ST_GATEWAY = gateway()
  process.env.WF_ST_AUTHORIZATION = readFileSync(authorizationFile, 'utf8').trim()
  const { createServer, loadConfigFromFile } = await import('vite')
  const loaded = await loadConfigFromFile({ command: 'serve', mode: 'development' }, `${app}/vite.config.ts`, app)
  const server = await createServer({ ...loaded.config, configFile: false, root: `${app}/src/web`, server: { ...loaded.config.server, host: '127.0.0.1', port, strictPort: true } })
  await server.listen()
  console.log(JSON.stringify({
    label: 'connected preview: real fractal-web and a real isolated daemon running the test-support fractal-fixture (host ancestry binding off), through the paired client gateway; not the installed runtime',
    // The workspace's main pane is a conversation; the Inbox is the Alerts tab in its sidebar.
    url: `http://127.0.0.1:${port}/w/${agents.atlas}`,
    buildRev: execFileSync('git', ['-C', app, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
    agents, fallback: agents.manager, mission,
    threads: fractalJson(['attention', 'ls', '--as', person], { check: false })?.value?.items?.length ?? null,
  }, null, 2))
}

/** The browser forgets itself the way the app's own revoke does; the next `serve` pairs it again. */
async function revoke() {
  process.stdout.write(`${JSON.stringify(await gatewayCall('POST', '/v1/client/pairing/browser-requests/revoke-current', {}, readFileSync(authorizationFile, 'utf8').trim()))}\n`)
  rmSync(authorizationFile, { force: true })
}

function down() {
  // PREVIEW_ROOT comes from the environment: act only on a directory this script made.
  if (!basename(root).startsWith('fractal-threadspreview') || !existsSync(marker('fleet-created'))) {
    console.error(`not removing ${root}: it is not named fractal-threadspreview* or has no fleet-created marker from this script`)
    process.exitCode = 1
    return
  }
  spawnSync('systemctl', ['--user', 'stop', unit], { stdio: 'inherit' })
  if (existsSync(tools)) {
    const result = fractal(['uninstall', '--yes', '--erase-local-graph', '--no-service', '--offline'], { check: false })
    process.stdout.write(result.stdout + result.stderr)
  }
  rmSync(root, { recursive: true, force: true })
  console.log(`removed ${root}`)
}

const [command = 'serve'] = process.argv.slice(2)
if (command === 'serve') await serve()
else if (command === 'up') await up()
else if (command === 'offline') spawnSync('systemctl', ['--user', 'stop', unit], { stdio: 'inherit' })
else if (command === 'online') await start()
else if (command === 'revoke') await revoke()
else if (command === 'status') console.log(JSON.stringify(fractalJson(['attention', 'ls', '--as', person], { check: false }), null, 2))
else if (command === 'agent-reply') await agentReplies()
else if (command === 'link-ask') await linkAsk()
else if (command === 'down') down()
else throw new Error('usage: threads-live-preview.mjs [serve|up|offline|online|revoke|status|agent-reply|link-ask|down]')
