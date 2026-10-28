// Replace only the annotation entry, not the addon, its options, rules or reporter.
export * from '@storybook/addon-a11y/preview'
import { afterEach as checkAccessibility } from '@storybook/addon-a11y/preview'

export async function afterEach(context) {
  const mark = phase => window.dispatchEvent(new CustomEvent('kit-a11y-timing', {
    detail: { phase, storyId: context.id, at: performance.now() },
  }))
  mark('start')
  try {
    return await checkAccessibility(context)
  } finally {
    mark('end')
  }
}
