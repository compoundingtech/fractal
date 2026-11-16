import { recordValue } from './blocks.ts'

type RecordedStSend = { readonly to: string; readonly body: string; readonly subject?: string }

/** Literal first command plus file/FD redirections and bounded output consumers, never shell execution. */
const literalArgv = (source: string): readonly string[] | undefined => {
  const command = source.trim(), pipeline: string[][] = []
  const outputConsumers: Readonly<Record<string, true | undefined>> = { cat: true, head: true, tail: true, sort: true, uniq: true, wc: true, cut: true, tr: true, tee: true }
  let argv: string[] = [], word = '', started = false, descriptor = true
  let quote: "'" | '"' | undefined, redirect: '<' | '>' | '>>' | '<&' | '>&' | undefined
  const flush = (): boolean => {
    if (!started) return true
    if (redirect !== undefined) {
      if (word.length === 0 || ((redirect === '<&' || redirect === '>&') && word !== '-' && !/^[0-9]+$/.test(word))) return false
      redirect = undefined
    } else argv.push(word)
    word = ''
    started = false
    descriptor = true
    return true
  }
  for (let index = 0; index < command.length; index++) {
    const char = command[index]!
    if (quote === "'") {
      if (char === "'") quote = undefined
      else word += char
      continue
    }
    if (char === '\\') {
      const next = command[++index]
      if (next === undefined) return undefined
      if (next === '\n') continue
      if (quote === '"' && !['$', '`', '"', '\\'].includes(next)) word += '\\'
      word += next
      started = true
      descriptor = false
      continue
    }
    if (quote === '"') {
      if (char === '"') quote = undefined
      else if (char === '$' || char === '`') return undefined
      else word += char
      continue
    }
    if (char === "'" || char === '"') {
      quote = char
      started = true
      descriptor = false
    } else if (char === ' ' || char === '\t') {
      if (!flush()) return undefined
    } else if (char === '<' || char === '>') {
      if (redirect !== undefined && (!started || !flush())) return undefined
      if (started && descriptor && /^[0-9]+$/.test(word)) {
        word = ''
        started = false
      } else if (!flush()) return undefined
      const next = command[index + 1]
      if (next === '&') {
        redirect = char === '<' ? '<&' : '>&'
        index++
      } else if (char === '>' && next === '>') {
        redirect = '>>'
        index++
      } else redirect = char
    } else if (char === '|') {
      if (!flush() || redirect !== undefined || argv.length === 0 || command[index + 1] === '|') return undefined
      pipeline.push(argv)
      argv = []
    } else if ('\n\r\0$`;&()#*?[]~{}!'.includes(char)) return undefined
    else {
      word += char
      started = true
    }
  }
  if (quote !== undefined || !flush() || redirect !== undefined || argv.length === 0) return undefined
  pipeline.push(argv)
  if (pipeline.slice(1).some(command => !Object.hasOwn(outputConsumers, command[0]!))) return undefined
  return pipeline[0]
}

const commandArgv = (facts: Readonly<Record<string, unknown>>): readonly string[] | undefined => {
  if (facts.argv !== undefined) {
    if (!Array.isArray(facts.argv) || !facts.argv.every((value): value is string => typeof value === 'string')) return undefined
    if (facts.executable !== undefined) return typeof facts.executable === 'string' ? [facts.executable, ...facts.argv] : undefined
    return facts.argv
  }
  if (Array.isArray(facts.command)) return facts.command.every((value): value is string => typeof value === 'string') ? facts.command : undefined
  return typeof facts.command === 'string' ? literalArgv(facts.command) : undefined
}

/** Legacy producer adapter for MessageSendArgs and global Cli flags in crates/st3/src/main.rs. */
export const recordedStSendFor = (item: { readonly input?: unknown; readonly view?: Readonly<Record<string, unknown>>; readonly result?: { readonly view?: Readonly<Record<string, unknown>> } }): RecordedStSend | undefined => {
  const view = item.view ?? item.result?.view, input = recordValue(item.input)
  const facts = view?.command !== undefined || view?.argv !== undefined ? view : input
  if (facts === undefined) return undefined
  const argv = commandArgv(facts)
  if (argv?.[0] !== 'st') return undefined
  const values = new Map<string, string>()
  const globals: Readonly<Record<string, true | undefined>> = { '--endpoint': true, '--catalog': true, '--daemon-wait': true }
  let index = 1, to: string | undefined, sending = false
  while (index < argv.length) {
    const argument = argv[index++]!
    if (!sending && argument === 'conversations' && argv[index] === 'send') {
      index++
      sending = true
      continue
    }
    if (argument === '--json') {
      if (values.has(argument)) return undefined
      values.set(argument, '')
      continue
    }
    if (argument === '--' && sending) {
      if (to !== undefined || index !== argv.length - 1) return undefined
      to = argv[index++]
      continue
    }
    if (!argument.startsWith('-')) {
      if (!sending || to !== undefined) return undefined
      to = argument
      continue
    }
    const equals = argument.indexOf('=')
    let flag = equals < 0 ? argument : argument.slice(0, equals)
    let value = equals < 0 ? undefined : argument.slice(equals + 1)
    if (argument.startsWith('-m') && argument !== '-m') {
      flag = '-m'
      value = argument.slice(argument[2] === '=' ? 3 : 2)
    }
    if (flag === '-m') flag = '--body'
    if (flag === '--as') flag = '--from'
    if (!Object.hasOwn(globals, flag) && (!sending || !['--body', '--subject', '--in-reply-to', '--tags', '--from', '--attach', '--idempotency-key'].includes(flag))) return undefined
    const separateValue = value === undefined
    value ??= argv[index++]
    if (value === undefined || (separateValue && value.startsWith('-'))) return undefined
    if (values.has(flag) && flag !== '--tags' && flag !== '--attach') return undefined
    values.set(flag, value)
  }
  const body = values.get('--body')
  return sending && to !== undefined && to.length > 0 && body !== undefined && (values.get('--from')?.length ?? 0) > 0 ? { to, body, ...(values.has('--subject') ? { subject: values.get('--subject')! } : {}) } : undefined
}
