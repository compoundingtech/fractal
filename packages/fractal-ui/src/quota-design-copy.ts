import type { QuotaAccount } from './quota-design-types.ts'

export const quotaStateLabels: Readonly<Record<QuotaAccount['state'], string>> = {
  current: 'Current',
  stale: 'Stale',
  failed: 'Failed',
  unknown: 'Unreported',
  disabled: 'Disabled',
}
