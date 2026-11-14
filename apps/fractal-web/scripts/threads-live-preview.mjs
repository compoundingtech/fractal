#!/usr/bin/env node
/**
 * Connected, controlled Threads preview: the real web app against a real, isolated Fractal.
 *
 *   node apps/fractal-web/scripts/threads-live-preview.mjs serve        # up + seed + serve on loopback
 *   node apps/fractal-web/scripts/threads-live-preview.mjs reply THREAD [atlas|manager] TEXT
 *   node apps/fractal-web/scripts/threads-live-preview.mjs link-ask THREAD TITLE   # atlas asks again, linked to THREAD
 *   node apps/fractal-web/scripts/threads-live-preview.mjs offline|online|revoke|status|down
 *
 * `serve` makes a throwaway instance (`threadspreview`) under its own HOME in PREVIEW_ROOT
 * (default /var/tmp/fractal-threadspreview), so neither `~/.st-instance` nor the default install
 * is touched. The daemon opts into thread writes and names agent/example/manager as the explicit
 * `inbox_fallback_agent`. It declares person/avery and two program seats (atlas and manager), pairs
 * one full-control browser device for avery (its bearer stays in PREVIEW_ROOT/authorization,
 * mode 0600) and seeds three Inbox sources:
 *   - "Review the release checklist", asked by atlas, with one earlier thread and an atlas reply;
 *   - "Confirm the rollout window", asked by atlas, with no thread (the empty state);
 *   - the human gate of mission example/release-notes, which has no originating agent, so
 *     discussion goes to the configured Manager fallback and its threads belong to that mission.
 * It then serves the app with Vite on 127.0.0.1:PREVIEW_PORT (default 5317) through the existing
 * WF_ST_GATEWAY/WF_ST_AUTHORIZATION paired gateway. The browser authenticates only through that path.
 *
 * Agents in this instance are program seats: they never write on their own. `reply` is the only way
 * an agent message appears, so every agent line on screen was typed by the reviewer through Fractal's
 * own agent-side command. `offline` stops the daemon (the app shows its paused and reconnecting
 * states), `online` starts it again, `revoke` revokes the browser pairing (the app falls back to its
 * refused state; `serve` pairs a new device on the next start), and `down` removes everything.
 *
 * Needs: systemd user manager; FRACTAL_BINARY (a fractal build from this checkout: the installed one
 * may predate threads) and PTY_BINARY (default `pty` on PATH). Run from `nix develop .#web`.
 */
import { execFileSync, spawnSync } from 'node:child_process'
import { randomBytes } from 'node:crypto'
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
const authorizationFile = join(root, 'authorization')
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

/** A transient user unit with the instance's clean environment; the daemon attributes a local caller by ST_AGENT. */
const isolated = (argv, agent) => [env, '-i', `HOME=${home}`, `PATH=${tools}:${dirname(sleep)}:/usr/bin:/bin`, `USER=${process.env.USER ?? 'preview'}`,
  'LANG=C.UTF-8', 'SHELL=/bin/sh', ...(agent === undefined ? [] : [`ST_AGENT=${agent}`]), ...argv]
const fractal = (args, { check = true, agent, input } = {}) => {
  const result = spawnSync('systemd-run', ['--user', '--wait', '--pipe', '--collect', '--quiet', ...isolated([join(tools, 'fractal'), '--instance', instance, ...args], agent)],
    { encoding: 'utf8', ...(input === undefined ? {} : { input }) })
  if (check && result.status !== 0) throw new Error(`fractal ${args.join(' ')} exited ${result.status}: ${result.stderr || result.stdout}`)
  return result
}
const fractalJson = (args, options) => JSON.parse(fractal(['--json', ...args], options).stdout)
const alive = () => existsSync(gateway()) && fractal(['--daemon-wait', '0', 'agents', 'ls'], { check: false }).status === 0
const marker = (name) => join(root, name)

const gatewayPost = (path, body) => new Promise((resolve, reject) => {
  const payload = JSON.stringify(body)
  const req = request({ socketPath: gateway(), path, method: 'POST', headers: { host: '127.0.0.1', 'content-type': 'application/json', 'content-length': Buffer.byteLength(payload) } }, (res) => {
    let text = ''
    res.on('data', (chunk) => { text += chunk })
    res.on('end', () => res.statusCode === 200 ? resolve(JSON.parse(text)) : reject(new Error(`POST ${path}: ${res.statusCode} ${text}`)))
  })
  req.on('error', reject)
  req.end(payload)
})

/** The daemon reads its policy from the instance's config file at startup. */
function configure() {
  const directory = ['fractal', 'st3'].map((name) => join(instanceDir, 'config', name)).find((path) => existsSync(join(path, 'config.toml')))
    ?? join(instanceDir, 'config', 'st3')
  mkdirSync(directory, { recursive: true })
  const file = join(directory, 'config.toml')
  const current = existsSync(file) ? readFileSync(file, 'utf8') : ''
  const lines = [['thread_writes', 'thread_writes = true'], ['inbox_fallback_agent', `inbox_fallback_agent = "${agents.manager}"`]]
    .filter(([key]) => !new RegExp(`^${key}\\s*=`, 'm').test(current)).map(([, line]) => line)
  // Root keys must precede the first table.
  if (lines.length > 0) writeFileSync(file, `${lines.join('\n')}\n${current}`)
}

async function start() {
  mkdirSync(tools, { recursive: true })
  mkdirSync(join(root, 'workspace'), { recursive: true })
  if (!existsSync(join(tools, 'fractal'))) {
    const binary = process.env.FRACTAL_BINARY
    if (binary === undefined) throw new Error('Set FRACTAL_BINARY to a fractal built from this checkout')
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
  spawnSync('systemctl', ['--user', 'reset-failed', unit], { stdio: 'ignore' })
  execFileSync('systemd-run', ['--user', `--unit=${unit}`, '--collect', '--quiet', ...isolated([join(tools, 'fractal'), '--instance', instance, 'up', '--pty-binary', join(tools, 'pty')])])
  for (let i = 0; i < 120 && !alive(); i++) await pause(500)
  if (!alive()) throw new Error(`The preview daemon did not start; see journalctl --user -u ${unit}`)
  if (!existsSync(marker('setup-done'))) {
    fractal(['setup', '--person', person.slice('person/'.length), '--yes', '--service', 'false', '--install', 'false', '--harness', 'none', '--claude-channel', 'false', '--assistant', 'false'])
    writeFileSync(marker('setup-done'), '')
  }
}

async function up() {
  if (!alive()) await start()
  if (!existsSync(marker('declared'))) {
    // Program seats exist in the roster and run; they never write unless `reply` speaks for them.
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
  if (!existsSync(authorizationFile)) {
    // Pair as a web device would: begin locally for avery, complete on the paired gateway.
    const challenge = fractalJson(['devices', 'pair', 'Threads preview browser', '--full-control', '--as', person]).value
    const session = await gatewayPost(`/v1/client/pairings/${encodeURIComponent(challenge.pairing_id.replace(/^pairing\//, ''))}/complete`, {
      api_version: 'st3.client.v0', code: challenge.code, device_public_key: `threads-preview-${randomBytes(24).toString('base64url')}`,
    })
    const credential = session.value?.credential ?? session.credential
    if (!credential) throw new Error('Pairing returned no credential')
    writeFileSync(authorizationFile, `Bearer ${credential}`, { mode: 0o600 })
    writeFileSync(marker('pairing'), challenge.pairing_id)
  }
  if (!existsSync(marker('seeded'))) seed()
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
  reply(thread, 'atlas', 'Not yet. I will add the migration and its rollback as separate checklist items.')
  writeFileSync(marker('seeded'), `${thread}\n`)
}

function reply(thread, who, text) {
  const agent = agents[who]
  if (agent === undefined) throw new Error(`reply speaks for ${Object.keys(agents).join(' or ')}`)
  process.stdout.write(fractal(['conversations', 'reply-in-thread', thread, '-m', text], { agent }).stdout)
}

/** A new ask from atlas, attached to an existing (possibly closed) thread as its requesting agent. */
function linkAsk(thread, title) {
  const key = `link-${randomBytes(4).toString('hex')}`
  const subject = ask(key, title, 'Following up in the thread we already have.')
  const events = fractalJson(['trace', 'show', subject, '--as', agents.atlas]).value?.items ?? []
  const claim = events.find((event) => event.kind === 'work.person-asked')?.claim_id
  if (claim === undefined) throw new Error(`no work.person-asked claim in the trace of ${subject}`)
  process.stdout.write(fractal(['work', 'link-thread', subject, '--thread', thread, '--episode', claim, '--as', agents.atlas]).stdout)
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
    label: 'connected preview: real fractal-web and a real isolated Fractal daemon through the paired client gateway',
    url: `http://127.0.0.1:${port}/w/inbox`,
    buildRev: execFileSync('git', ['-C', app, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
    agents, fallback: agents.manager, mission,
    threads: fractalJson(['attention', 'ls', '--as', person], { check: false })?.value?.items?.length ?? null,
  }, null, 2))
}

function revoke() {
  const pairing = readFileSync(marker('pairing'), 'utf8').trim()
  process.stdout.write(fractal(['devices', 'revoke', pairing, '--as', person]).stdout)
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

const [command = 'serve', ...rest] = process.argv.slice(2)
if (command === 'serve') await serve()
else if (command === 'up') await up()
else if (command === 'reply') {
  const [thread, who, ...text] = rest
  if (thread === undefined || text.length === 0) throw new Error('usage: threads-live-preview.mjs reply THREAD atlas|manager TEXT')
  reply(thread, who, text.join(' '))
}
else if (command === 'offline') spawnSync('systemctl', ['--user', 'stop', unit], { stdio: 'inherit' })
else if (command === 'online') await start()
else if (command === 'revoke') revoke()
else if (command === 'status') console.log(JSON.stringify(fractalJson(['attention', 'ls', '--as', person], { check: false }), null, 2))
else if (command === 'link-ask') {
  const [thread, ...title] = rest
  if (thread === undefined || title.length === 0) throw new Error('usage: threads-live-preview.mjs link-ask THREAD TITLE')
  linkAsk(thread, title.join(' '))
}
else if (command === 'down') down()
else throw new Error('usage: threads-live-preview.mjs [serve|up|reply|link-ask|offline|online|revoke|status|down]')
