import type { ConversationItem, Role, Sender, TextItem } from './model.ts'

/** The media type used by Fractal's wire-compatible omp custom-message projection. */
export const ompEventMediaType = 'application/vnd.omp.event+json'

/** Read object fields without assuming a producer-specific shape. */
export const fieldsOf = (value: unknown): Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {}
const stringField = ({
  value,
  key,
}: {
  readonly value: unknown
  readonly key: string
}): string | undefined => {
  const field = fieldsOf(value)[key]
  return typeof field === 'string' ? field : undefined
}

/** Default sender labels never turn missing provenance into an invented agent identity. */
export const roleSender = ({
  role,
  agentName = 'Agent',
}: {
  readonly role: Role
  readonly agentName?: string
}): Sender =>
  role === 'assistant'
    ? { kind: 'agent', label: agentName }
    : role === 'user'
      ? { kind: 'human', label: 'You', via: 'harness' }
      : { kind: 'system', label: 'System' }

/** Fractal subjects carry provenance in their explicit subject prefix. */
export const deliverySender = (from: string | undefined): Sender =>
  from?.startsWith('daemon/') === true
    ? { kind: 'system', label: 'Fractal', via: 'daemon' }
    : from?.startsWith('agent/') === true
    ? { kind: 'st-agent', label: from.slice('agent/'.length), via: 'st' }
    : from?.startsWith('human/') === true ||
        from?.startsWith('user/') === true ||
        from?.startsWith('person/') === true
      ? { kind: 'human', label: from.slice(from.indexOf('/') + 1), via: 'wf / st' }
      : { kind: 'system', label: from ?? 'Fractal', via: 'st' }

/** Undo `xml_escape` in fractal-drivers' ding. `&amp;` goes last, so an authored `&amp;lt;` stays `&lt;`. */
const unescapeDelivered = (text: string): string =>
  text.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, '&')

/**
 * A delivery's words as the sender wrote them. A body cannot forge an `<attachment/>`, whose `<` it would have escaped. The
 * element becomes a note until web reads a delivery's images by message, as `deliveredImages` in fractal-views does.
 */
const deliveredBody = (body: string): string =>
  unescapeDelivered(body.replace(/<attachment\s[^>]*?\/>/g, (element) => {
    const attribute = (name: string): string | undefined => {
      const value = new RegExp(` ${name}="([^"]*)"`).exec(element)?.[1]
      return value === undefined ? undefined : unescapeDelivered(value)
    }
    return `[${attribute('name') ?? 'attachment'} · ${attribute('unavailable') === 'expired' ? 'expired' : attribute('media_type') ?? 'not available here'}]`
  })).trimEnd()

/** Fractal's native injection is an explicit envelope, not a human message merely because role=user. */
export const proseItem = (item: TextItem): TextItem => {
  if (item.role !== 'user' && item.role !== 'system') return item
  const envelope = /^<smalltalk-message\b([^>]*)>\s*([\s\S]*?)\s*<\/smalltalk-message>/.exec(
    item.text,
  )
  const from = envelope !== null ? /\bfrom="([^"]+)"/.exec(envelope[1]!)?.[1] : undefined
  if (envelope === null || from === undefined) return item
  const sender = deliverySender(unescapeDelivered(from))
  return { ...item, text: deliveredBody(envelope[2]!), sender, role: sender.kind === 'human' ? 'user' : 'system' }
}

/** Lines drivers write around a delivery; they are not anyone's prose once the delivery is gone. */
const DELIVERY_NOTES = new Set([
  'Reply to the person in this conversation; any new question or decision for the person goes through a native work ask.',
  'Answer the person in this conversation; people have no inbox, so do not reply with st.',
  "The person reads replies in st, not in the agent's session.",
  '(dictated by voice; it may contain transcription mistakes)',
  '[preview truncated; read the full message in st3]',
  '(silent: held without waking you until this turn)',
])
const isDeliveryNote = (line: string): boolean =>
  DELIVERY_NOTES.has(line.trim()) || /^\(\d+ older silent held; st conversations ls\)$/.test(line.trim())

/** The driver's one-line form of a body (`normalize_line` in fractal-drivers' ding): any run of whitespace or control characters is one space. */
const oneLine = (text: string): string => text.replace(/[\s\p{Cc}]+/gu, ' ').trim()
const unescapeBody = (text: string): string =>
  text.replace(/<attachment\b[^>]*\/>/g, '').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&')

/** A bounded preview ends in `…`; anything else that differs carries words the mail does not. */
const repeatsBody = (copy: string, body: string): boolean => {
  const preview = oneLine(copy)
  const full = oneLine(body)
  return preview === full || (preview.endsWith('…') && full.startsWith(preview.slice(0, -1)))
}

/** What a `<channel>` delivery says, without its marker, PING header or notes. */
const channelCopy = (block: string): string => {
  const nested = /<smalltalk-message\b[^>]*>([\s\S]*?)<\/smalltalk-message>/.exec(block)?.[1]
  if (nested !== undefined) return unescapeBody(nested)
  return block
    .replace(/^<channel\b[^>]*>|<\/channel>$/g, '').split('\n')
    .filter(line => !/^\s*\[st3-delivery:[^\]]*\]\s*$/.test(line) && !/^\s*\[PING from st3\] /.test(line) && !isDeliveryNote(line))
    .join('\n').replace(/… \[read the full message in st3\]/, '…')
}

/**
 * Mirrors st3-views' shown/delivered semantics for envelope, channel and PING delivery links.
 * That package is not a dependency here. Only explicit graph links suppress mail, never prose.
 *
 * A link alone is not enough: a GitHub watch's wake gains its comment excerpt only at delivery
 * (`add_excerpts` in fractal-github-base), so that delivery is the only copy of those words. A copy
 * goes only when `shownBodies` holds the same body for the message it names.
 * Keep unrelated text in a native turn; driver delivery notes are not user prose.
 */
export const withoutShownDeliveries = (raw: string, shownBodies: ReadonlyMap<string, string>): string => {
  let delivered = false
  const isShown = (id: string | undefined, copy: string): boolean => {
    const body = id === undefined ? undefined : shownBodies.get(id)
    return body !== undefined && repeatsBody(copy, body)
  }
  let text = raw.replace(/\r\n/g, '\n')
  text = text.replace(/<channel\b([^>]*)>[\s\S]*?<\/channel>/g, (block: string, head: string) => {
    const id = /\bmessageId="([^"]+)"/.exec(head)?.[1] ??
      /^\s*\[PING from st3\] (message\/\S+) from /m.exec(block)?.[1] ??
      /<smalltalk-message\b[^>]*\bgraph="([^"]+)"/.exec(block)?.[1]
    if (!isShown(id, channelCopy(block))) return block
    delivered = true
    return ''
  })
  text = text.replace(/<smalltalk-message\b([^>]*)>([\s\S]*?)<\/smalltalk-message>/g, (block: string, head: string, inner: string) => {
    if (!isShown(/\bgraph="([^"]+)"/.exec(head)?.[1], unescapeBody(inner))) return block
    delivered = true
    return ''
  })
  text = text.split('\n').filter((line) => {
    // A bare header repeats no body, so a shown message is enough.
    const id = /^\s*\[PING from st3\] (message\/\S+) from /.exec(line)?.[1]
    if (id === undefined || !shownBodies.has(id)) return true
    delivered = true
    return false
  }).join('\n')
  if (!delivered) return raw
  return text.split('\n').filter(line => !isDeliveryNote(line)).join('\n').trim()
}

/** Deliberately decode an event envelope; arbitrary prose and JSON remain prose. */
export const structuredEvent = ({
  id,
  data,
  eventType,
  at,
}: {
  readonly id: string
  readonly data: unknown
  readonly eventType: string
  readonly at?: string
}): ConversationItem => {
  const event = fieldsOf(data)
  const kind = stringField({ value: data, key: 'kind' }) ?? eventType
  const details = fieldsOf(event['details'])
  const content = typeof event['content'] === 'string' ? event['content'] : ''
  const base = { id, ...(at !== undefined ? { at } : {}) }
  if (kind === 'irc:incoming') {
    const from = stringField({ value: details, key: 'from' })
    return {
      _tag: 'Event',
      ...base,
      kind: 'harness-message',
      title: 'Harness message',
      sender: { kind: 'harness', label: from ?? 'IRC', via: 'IRC' },
      text: stringField({ value: details, key: 'message' }) ?? content,
      data,
    }
  }
  if (kind === 'async-result') {
    const jobs = Array.isArray(details['jobs']) ? details['jobs'] : []
    const task =
      jobs.length === 1 && stringField({ value: jobs[0], key: 'type' }) === 'task'
        ? jobs[0]
        : undefined
    const output = /<(?:output|preview)>([\s\S]*?)<\/(?:output|preview)>/.exec(content)?.[1]?.trim()
    let outputSummary = output
    if (output !== undefined && /^[{[]/.test(output)) {
      try {
        outputSummary = stringField({ value: JSON.parse(output), key: 'summary' })
      } catch {
        outputSummary = undefined
      }
    }
    const schemaData = fieldsOf(fieldsOf(fieldsOf(task)['schema'])['data'])
    const summary = stringField({ value: schemaData, key: 'summary' })
    return {
      _tag: 'Event',
      ...base,
      kind: task !== undefined ? 'subagent-result' : 'harness-message',
      title: task !== undefined ? 'Subagent result' : 'Background result',
      sender:
        task !== undefined
          ? {
              kind: 'subagent',
              label:
                stringField({ value: task, key: 'label' }) ??
                stringField({ value: task, key: 'jobId' }) ??
                'Subagent',
              via: 'native',
            }
          : { kind: 'system', label: 'Harness', via: 'background' },
      text:
        task !== undefined
          ? (summary ??
            outputSummary ??
            'Result received. Open event details to inspect the native payload.')
          : jobs.length > 0
            ? `${jobs.length} background ${jobs.length === 1 ? 'job' : 'jobs'} completed.`
            : 'Background result received.',
      data,
    }
  }
  if (kind === 'resource-observed' || kind === 'resource.observed') {
    const resource = fieldsOf(
      event['data'] ?? event['details'] ?? fieldsOf(event['body'])['fields'] ?? data,
    )
    const ref =
      stringField({ value: event, key: 'subject' }) ??
      stringField({ value: resource, key: 'ref' }) ??
      stringField({ value: resource, key: 'resource_ref' }) ??
      stringField({ value: resource, key: 'subject' })
    return {
      _tag: 'Event',
      ...base,
      kind: 'resource-observed',
      title: 'Resource observed',
      sender: { kind: 'system', label: 'Fractal', via: 'daemon' },
      text: ref ?? 'Resource observation received',
      data,
    }
  }
  // User-attributed extension messages (for example skills) are actual human input.
  if (
    event['_tag'] === 'OmpEvent' &&
    event['version'] === 1 &&
    event['attribution'] === 'user' &&
    content !== ''
  ) {
    return {
      _tag: 'Text',
      id,
      at: at ?? '',
      role: 'user',
      text: content,
      attachments: [],
      streaming: false,
      sender: { kind: 'human', label: 'You', via: 'harness' },
    }
  }
  return { _tag: 'UnknownEvent', ...base, eventType: kind, data }
}

/** Explicit structured media types authorize decoding; ordinary prose never gets guessed at. */
export const contentEvent = ({
  id,
  text,
  mediaType,
  at,
}: {
  readonly id: string
  readonly text: string
  readonly mediaType: string
  readonly at: string
}): ConversationItem | undefined => {
  if (mediaType !== ompEventMediaType && mediaType !== 'application/json') return undefined
  try {
    const data: unknown = JSON.parse(text)
    const event = fieldsOf(data)
    if (mediaType === 'application/json')
      return structuredEvent({
        id,
        data,
        eventType: stringField({ value: data, key: 'type' }) ?? 'structured content',
        at,
      })
    if (event['_tag'] !== 'OmpEvent' || event['version'] !== 1 || typeof event['kind'] !== 'string')
      return { _tag: 'UnknownEvent', id, at, eventType: 'invalid omp event', data }
    return structuredEvent({ id, data, eventType: event['kind'], at })
  } catch {
    return { _tag: 'UnknownEvent', id, at, eventType: 'invalid omp event', data: { content: text } }
  }
}

/** Attribution for every renderer; authoritative item provenance takes precedence. */
export const senderOf = ({
  item,
  agentName,
}: {
  readonly item: ConversationItem
  readonly agentName: string
}): Sender => {
  if ('sender' in item && item.sender !== undefined) return item.sender
  switch (item._tag) {
    case 'Text':
      return roleSender({ role: item.role, agentName: agentName })
    case 'Message':
      return deliverySender(item.author ?? item.from)
    case 'Reasoning':
    case 'ToolCall':
      return { kind: 'agent', label: agentName }
    case 'Usage':
      return { kind: 'system', label: 'Usage' }
    case 'Status':
      return { kind: 'system', label: 'Run' }
    case 'Notice':
    case 'UnknownEvent':
    case 'Unsupported':
      return { kind: 'system', label: 'System' }
    case 'Event':
      return item.sender
  }
}
