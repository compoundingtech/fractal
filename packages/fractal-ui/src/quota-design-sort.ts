import type { QuotaAccount, QuotaDrawerProps, QuotaSort, QuotaSortKey, QuotaUnit, QuotaWindow } from './quota-design-types.ts'

export const quotaSortKeys: readonly QuotaSortKey[] = ['governing', 'comparison', 'account', 'reset', 'more']

export const isQuotaSortKey = (key: string | number): key is QuotaSortKey => quotaSortKeys.some(value => value === key)

/** Unknown and unreported readings are not zero. Exhaustion is separate evidence. */
export const quotaWindowFraction = (window: QuotaWindow): number | undefined =>
  window.reported && window.fraction !== undefined && Number.isFinite(window.fraction) && window.fraction >= 0
    ? window.fraction
    : undefined

export const quotaRoleWindow = (account: QuotaAccount, role: 'governing' | 'comparison'): QuotaWindow | undefined =>
  account.windows.find(window => window[role])

type QuotaView = Pick<QuotaDrawerProps, 'variant' | 'columns'>

/** The governing window is always visible; matching secondary windows share columns. */
export const quotaVisibleWindows = (account: QuotaAccount, view: QuotaView): readonly QuotaWindow[] => {
  if (view.variant === 'stacked') return account.windows
  const governing = quotaRoleWindow(account, 'governing')
  if (view.variant === 'columns') return account.windows.filter(window => window.id === governing?.id || view.columns.some(column => column.id === window.column))
  const chips = account.windows.filter(window => window.id !== governing?.id).slice(0, 2)
  return governing === undefined ? chips : [governing, ...chips]
}

export const quotaOmittedWindowCount = (account: QuotaAccount, view: QuotaView, visible: readonly QuotaWindow[] = quotaVisibleWindows(account, view)): number =>
  account.windows.length - visible.length

/** Reset sorting and display are anchored to the governing window, not inventory minima. */
export const quotaGoverningResetAt = (account: QuotaAccount): number | undefined => {
  const resetAt = quotaRoleWindow(account, 'governing')?.resetAt
  if (resetAt === undefined) return undefined
  const value = Date.parse(resetAt)
  return Number.isFinite(value) ? value : undefined
}

const canonicalCompare = (a: string, b: string): number => a < b ? -1 : a > b ? 1 : 0
const numericCompare = (a: number | undefined, b: number | undefined, direction: QuotaSort['direction']): number => {
  if (a === undefined) return b === undefined ? 0 : 1
  if (b === undefined) return -1
  return direction === 'ascending' ? a - b : b - a
}

export const compareQuotaAccounts = (a: QuotaAccount, b: QuotaAccount, sort: QuotaSort, view: QuotaView): number => {
  let order: number
  if (sort.key === 'governing' || sort.key === 'comparison') {
    const left = quotaRoleWindow(a, sort.key)
    const right = quotaRoleWindow(b, sort.key)
    const leftFraction = left === undefined ? undefined : quotaWindowFraction(left)
    const rightFraction = right === undefined ? undefined : quotaWindowFraction(right)
    const leftKnown = left?.exhausted === true || leftFraction !== undefined
    const rightKnown = right?.exhausted === true || rightFraction !== undefined
    const direction = sort.direction === 'ascending' ? 1 : -1
    const exhaustedOrder = direction * (Number(left?.exhausted === true) - Number(right?.exhausted === true))
    // Unknown state stays last; exhaustion reverses, but never becomes a fake fraction.
    order = leftKnown !== rightKnown
      ? Number(rightKnown) - Number(leftKnown)
      : exhaustedOrder !== 0 ? exhaustedOrder : numericCompare(leftFraction, rightFraction, sort.direction)
  } else if (sort.key === 'account') {
    order = canonicalCompare(a.id, b.id) * (sort.direction === 'ascending' ? 1 : -1)
  } else if (sort.key === 'reset') {
    order = numericCompare(quotaGoverningResetAt(a), quotaGoverningResetAt(b), sort.direction)
  } else {
    order = numericCompare(quotaOmittedWindowCount(a, view), quotaOmittedWindowCount(b, view), sort.direction)
  }
  return order === 0 ? canonicalCompare(a.id, b.id) : order
}

export const filterQuotaAccounts = (accounts: readonly QuotaAccount[], filters: Pick<QuotaDrawerProps, 'provider' | 'plan' | 'state'>): readonly QuotaAccount[] =>
  accounts.filter(account =>
    (filters.provider === '' || account.provider === filters.provider) &&
    (filters.plan === '' || account.plan === filters.plan) &&
    (filters.state === '' || account.state === filters.state),
  )

export const sortQuotaAccounts = (accounts: readonly QuotaAccount[], sort: QuotaSort, view: QuotaView): readonly QuotaAccount[] =>
  [...accounts].sort((a, b) => compareQuotaAccounts(a, b, sort, view))

export interface QuotaAmount { readonly unit: QuotaUnit; readonly value?: number }

const knownAmount = (value: number | undefined): number | undefined => value !== undefined && Number.isFinite(value) && value >= 0 ? value : undefined

/** USD is shown only with coverage and a known amount; otherwise use reported tokens. */
export const quotaAccountAmount = (account: QuotaAccount, unit: QuotaUnit): QuotaAmount => {
  const usd = knownAmount(account.usd)
  return unit === 'usd' && account.usdCoverage && usd !== undefined
    ? { unit: 'usd', value: usd }
    : { unit: 'tokens', value: knownAmount(account.tokens) }
}

export interface QuotaSubtotal {
  readonly usd: number
  readonly tokens: number
  readonly usdAccounts: number
  readonly tokenAccounts: number
  readonly unknownAccounts: number
}

/** Mixed-unit fallbacks stay separate, and incomplete subtotals are explicitly counted. */
export const quotaProviderSubtotal = (accounts: readonly QuotaAccount[], unit: QuotaUnit): QuotaSubtotal => {
  let usd = 0
  let tokens = 0
  let usdAccounts = 0
  let tokenAccounts = 0
  let unknownAccounts = 0
  for (const account of accounts) {
    const amount = quotaAccountAmount(account, unit)
    if (amount.value === undefined) unknownAccounts += 1
    else if (amount.unit === 'usd') { usd += amount.value; usdAccounts += 1 }
    else { tokens += amount.value; tokenAccounts += 1 }
  }
  return { usd, tokens, usdAccounts, tokenAccounts, unknownAccounts }
}
