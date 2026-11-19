import { sha256 } from '@noble/hashes/sha2.js'
import { bytesToHex } from '@noble/hashes/utils.js'
import { Schema } from 'effect'

const matchingString = (pattern: RegExp, message: string): Schema.Codec<string, string> =>
  Schema.String.pipe(Schema.refine((value): value is string => pattern.test(value), { message }))
const Head = matchingString(/^[0-9a-f]{40}$/, 'Expected an exact commit SHA')
const Digest = matchingString(/^sha256:[0-9a-f]{64}$/, 'Expected a SHA-256 digest')
const Seat = matchingString(/^[A-Za-z0-9][A-Za-z0-9._-]*$/, 'Expected a local reviewer seat ID')

/** Immutable material to assess, not the material attached to a person ask by Review.subject. */
export const ReviewSubject = Schema.Union([
  Schema.Struct({
    kind: Schema.Literal('pullRequest'),
    repo: matchingString(/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/, 'Expected owner/repository'),
    number: Schema.Number.pipe(Schema.refine((value): value is number => Number.isSafeInteger(value) && value > 0)),
    head: Head,
    format: Schema.Literals(['q19', 'fractal']),
  }),
  Schema.Struct({ kind: Schema.Literal('artifact'), ref: Schema.NonEmptyString, digest: Digest }),
])
export type ReviewSubject = typeof ReviewSubject.Type

export const ReviewRubric = Schema.Struct({ ref: Schema.NonEmptyString, digest: Digest })
export type ReviewRubric = typeof ReviewRubric.Type

/** The child mission must declare this run-owned seat with native fresh-context. */
export const ReviewerSeat = Schema.Struct({
  mission: Schema.NonEmptyString,
  seat: Seat,
  product: Schema.NonEmptyString,
  freshContext: Schema.Literal(true),
  input: Schema.optionalKey(Schema.Record(Schema.String, Schema.String)),
})
export type ReviewerSeat = typeof ReviewerSeat.Type

export const ReviewFinding = Schema.Struct({
  message: Schema.NonEmptyString,
  blocking: Schema.Boolean,
  path: Schema.optionalKey(Schema.String),
  line: Schema.optionalKey(Schema.Number),
})
export type ReviewFinding = typeof ReviewFinding.Type

const VerdictFields = {
  verdict: Schema.Literals(['GO', 'NO-GO']),
  findings: Schema.Array(ReviewFinding),
  /** Commit SHA for a PR, or the exact sha256: digest for an artifact. */
  head: Schema.String,
}
export const ReviewVerdict = Schema.Struct(VerdictFields).pipe(Schema.refine((value): value is typeof value =>
  (value.verdict === 'GO') === !value.findings.some((finding) => finding.blocking),
{ message: 'A review verdict must agree with its blocking findings' }))
export type ReviewVerdict = typeof ReviewVerdict.Type

/** A PR reviewer publishes from its own seat and returns that immutable comment's ID. */
export const ReviewResult = Schema.Struct({
  ...VerdictFields,
  commentId: Schema.optionalKey(Schema.Number.pipe(Schema.refine(
    (value): value is number => Number.isSafeInteger(value) && value > 0,
  ))),
}).pipe(Schema.refine((value): value is typeof value =>
  (value.verdict === 'GO') === !value.findings.some((finding) => finding.blocking),
{ message: 'A review verdict must agree with its blocking findings' }))
export type ReviewResult = typeof ReviewResult.Type

export class ReviewError extends Schema.TaggedError<ReviewError>()('ReviewError', {
  reason: Schema.Literals(['invalid-input', 'stale-head', 'invalid-verdict', 'not-independent',
    'github-unavailable', 'publication-conflict']),
  message: Schema.String,
}) {}

export interface ReviewOptions {
  readonly id: string
  readonly subject: ReviewSubject
  readonly rubric: ReviewRubric
  readonly reviewer: ReviewerSeat
  /** Known author identities, in addition to the calling actor, may never assess this subject. */
  readonly authors?: readonly string[]
}

export const reviewHead = (subject: ReviewSubject): string =>
  subject.kind === 'pullRequest' ? subject.head : subject.digest

/** Stable within a program attempt, including all interpretation and reviewer inputs. */
export const reviewKey = (options: ReviewOptions): string => {
  const subject = options.subject.kind === 'pullRequest'
    ? ['pullRequest', options.subject.repo, options.subject.number, options.subject.head, options.subject.format]
    : ['artifact', options.subject.ref, options.subject.digest]
  const input = Object.entries(options.reviewer.input ?? {}).sort(([left], [right]) =>
    left < right ? -1 : left > right ? 1 : 0)
  const fields = [options.id, subject, [options.rubric.ref, options.rubric.digest],
    [options.reviewer.mission, options.reviewer.seat, options.reviewer.product, options.reviewer.freshContext, input],
    [...(options.authors ?? [])].sort()]
  return `review-${bytesToHex(sha256(new TextEncoder().encode(JSON.stringify(fields))))}`
}

const prose = (value: string): string => value
  .replace(/[\r\n]/g, ' ')
  .replaceAll('<!--', '&lt;!--')
  .replace(/NO-GO/gi, 'changes-required')

const validVerdict = Schema.is(ReviewVerdict)

/** Only this trusted renderer can emit admission lines or an identity footer. */
export const reviewComment = (options: {
  readonly subject: Extract<ReviewSubject, { readonly kind: 'pullRequest' }>
  readonly rubric: ReviewRubric
  readonly verdict: ReviewVerdict
  readonly reviewer: string
  readonly publication: string
}): string => {
  const { subject, rubric, verdict } = options
  if (!validVerdict(verdict) || verdict.head !== subject.head)
    throw new ReviewError({
      reason: 'invalid-verdict', message: 'A comment must bind a consistent verdict to the exact requested head',
    })
  if (!/^agent\/[A-Za-z0-9._/-]+$/.test(options.reviewer) ||
    options.reviewer.split('/').some((part) => part === '' || part === '.' || part === '..') ||
    !/^review-[0-9a-f]{64}$/.test(options.publication))
    throw new ReviewError({
      reason: 'invalid-input', message: 'A comment needs a valid run-owned actor and publication marker',
    })
  const lines = [
    `## Review: ${verdict.verdict === 'GO' ? 'accept' : 'changes-required'}`,
    `Head: ${subject.head}`,
    `Rubric: ${prose(rubric.ref)} (${rubric.digest})`,
    '', '### Findings',
    ...(verdict.findings.length === 0 ? ['None.'] : verdict.findings.map((finding) =>
      `- ${finding.blocking ? 'Blocking' : 'Note'}: ${prose(finding.message)}${finding.path === undefined
        ? '' : ` (${prose(finding.path)}${finding.line === undefined ? '' : `:${finding.line}`})`}`)),
    '',
  ]
  if (subject.format === 'q19' && verdict.verdict === 'GO')
    lines.push(`q19-go v1 head=${subject.head} verdict=GO`, '')
  // The native consumer's value is ReviewVerdict, not the older git-review JSON protocol.
  if (subject.format === 'fractal')
    lines.push('```json', JSON.stringify({
      verdict: verdict.verdict,
      head: verdict.head,
      findings: verdict.findings.map((finding) => ({
        ...finding,
        message: prose(finding.message),
        ...(finding.path === undefined ? {} : { path: prose(finding.path) }),
      })),
    }), '```', '')
  lines.push(`<!-- fractal-review:${options.publication} -->`, '',
    '<!-- agent-footer:begin v=3 -->', '| Field | Value |', '| --- | --- |',
    `| \`agent_identity\` | ${options.reviewer} |`, '<!-- agent-footer:end -->')
  return lines.join('\n')
}

/**
 * Managed publishers may enrich the v3 footer, but never the assessment or reviewer identity.
 * Admission lines remain renderer-owned even inside that metadata block.
 */
export const reviewCommentMatches = (actual: string, expected: string): boolean => {
  const begin = '<!-- agent-footer:begin v=3 -->'
  const end = '<!-- agent-footer:end -->'
  const start = actual.indexOf(begin)
  const finish = actual.indexOf(end)
  const expectedStart = expected.indexOf(begin)
  if (actual.includes('\r') || start < 0 || expectedStart < 0 || finish < start ||
    actual.lastIndexOf(begin) !== start || actual.lastIndexOf(end) !== finish ||
    actual.slice(0, start) !== expected.slice(0, expectedStart))
    return false
  const tail = actual.slice(finish + end.length)
  if (tail !== '' && tail !== '\n') return false
  const identityRow = /^\|[ \t]*`agent_identity`[ \t]*\|[^\r\n]*$/gm
  const identities = actual.slice(start, finish).match(identityRow) ?? []
  const expectedIdentities = expected.slice(expectedStart).match(identityRow) ?? []
  if (identities.length !== 1 || expectedIdentities.length !== 1 ||
    identities[0] !== expectedIdentities[0])
    return false
  // Match the consumer's literal LF splitting, not JavaScript's broader multiline anchors.
  const admissions = actual.match(/(?:^|\n)q19-go[^\n]*/g) ?? []
  const expectedAdmissions = expected.match(/(?:^|\n)q19-go[^\n]*/g) ?? []
  return expectedAdmissions.length <= 1 && admissions.length === expectedAdmissions.length &&
    admissions.every((line, index) => line === expectedAdmissions[index]) &&
    (expectedAdmissions.length === 0 || !/NO-GO/i.test(actual))
}
