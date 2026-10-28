import { auditCaches } from './cache-audit.ts'
import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'

// FRACTAL_SITE_DISPATCH_TOKEN needs Contents: write on compoundingtech/private.
// The target workflow reads docs/site from main, never from dispatch payload data.
export default githubWorkflow(auditCaches({
  name: 'Publish docs change notification',
  on: { push: { branches: ['main'], paths: ['docs/site/**'] } },
  permissions: {},
  actionlint: defaultActionlintConfig,
  jobs: {
    notify: {
      name: 'Notify website deployment',
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      steps: [
        {
          name: 'Dispatch docs change',
          env: { GH_TOKEN: '${{ secrets.FRACTAL_SITE_DISPATCH_TOKEN }}' },
          run: `if [[ -z "$GH_TOKEN" ]]; then
  echo '::error::FRACTAL_SITE_DISPATCH_TOKEN is required (Contents: write on compoundingtech/private).'
  exit 1
fi
gh api --method POST repos/compoundingtech/private/dispatches -f event_type=fractal-docs-changed`,
        },
      ],
    },
  },
}, { notify: 'Sends a repository dispatch without a checkout, build or cache.' }))
