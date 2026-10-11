export type QuotaUnit = 'usd' | 'tokens'
export type QuotaSortKey = 'governing' | 'comparison' | 'account' | 'reset' | 'more'
export interface QuotaSort { readonly key: QuotaSortKey; readonly direction: 'ascending' | 'descending' }
export interface QuotaWindow {
  readonly id: string
  readonly column: string
  readonly label: string
  readonly fraction?: number
  readonly exhausted: boolean
  readonly reported: boolean
  readonly governing: boolean
  readonly comparison: boolean
  readonly resetAt?: string
  readonly scope: string
  readonly facts: readonly { readonly label: string; readonly value: string }[]
}
export interface QuotaAccount {
  readonly id: string
  readonly label: string
  readonly provider: string
  readonly plan: string
  readonly state: 'current' | 'stale' | 'failed' | 'unknown' | 'disabled'
  readonly windows: readonly QuotaWindow[]
  readonly usd?: number
  readonly tokens?: number
  readonly usdCoverage: boolean
  readonly resetCount?: number
  readonly resetCredits: readonly { readonly label: string; readonly value: string }[]
  readonly facts: readonly { readonly label: string; readonly value: string }[]
}
export interface QuotaStatus { readonly kind: 'current' | 'loading' | 'stale' | 'failed' | 'incompatible' | 'unpublished'; readonly text: string }
export interface QuotaDrawerProps {
  readonly landmarkContext?: string
  readonly accounts: readonly QuotaAccount[]
  readonly columns: readonly { readonly id: string; readonly label: string }[]
  readonly status: QuotaStatus
  readonly variant: 'columns' | 'stacked' | 'chips'
  readonly sort: QuotaSort
  readonly unit: QuotaUnit
  readonly provider: string
  readonly plan: string
  readonly state: string
  readonly grouped: boolean
  readonly collapsed: boolean
  readonly now: string
  readonly onSort: (sort: QuotaSort) => void
  readonly onUnit: (unit: QuotaUnit) => void
  readonly onFilter: (filter: { provider?: string; plan?: string; state?: string }) => void
  readonly onGrouped: (grouped: boolean) => void
  readonly onCollapsed: (collapsed: boolean) => void
  readonly onOpenUsage: (accountId: string) => void
  readonly onEscape: () => void
}
export interface QuotaHistoryValue {
  readonly id: string
  readonly label: string
  readonly accountId: string
  readonly model: string
  readonly scope: string
  readonly usd?: number
  readonly tokens?: number
  readonly coverage: number
  readonly tokenTypes: readonly { readonly label: string; readonly value?: number }[]
  readonly requests?: number
  readonly turns?: number
}
export interface QuotaHistoryDay { readonly id: string; readonly label: string; readonly partial: boolean; readonly values: readonly QuotaHistoryValue[] }
export interface QuotaEconomics {
  readonly id: string
  readonly label: string
  readonly accountId: string
  readonly limitId: string
  readonly usageValue?: number
  readonly ledgerValue?: number
  readonly price?: number
  readonly basis: 'invoiced' | 'list'
  readonly coverageDays?: number
}
export interface QuotaUsageProps {
  readonly landmarkContext?: string
  readonly accounts: readonly QuotaAccount[]
  readonly days: readonly QuotaHistoryDay[]
  readonly economics: readonly QuotaEconomics[]
  readonly status: QuotaStatus
  readonly variant: 'overview' | 'multiples' | 'heatmap'
  readonly unit: QuotaUnit
  readonly stackBy: 'account' | 'model'
  readonly range: readonly [number, number]
  readonly selectedDay?: string
  readonly selectedAccount?: string
  readonly detailTab: 'limits' | 'account' | 'history'
  readonly onUnit: (unit: QuotaUnit) => void
  readonly onStackBy: (stackBy: 'account' | 'model') => void
  readonly onRange: (range: readonly [number, number]) => void
  readonly onDay: (day?: string) => void
  readonly onAccount: (id: string) => void
  readonly onDetailTab: (tab: 'limits' | 'account' | 'history') => void
  readonly onBack?: () => void
}
