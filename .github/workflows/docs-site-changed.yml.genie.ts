import { auditCaches } from './cache-audit.ts'
import { defaultActionlintConfig, githubWorkflow } from '../../repos/effect-utils/genie/external.ts'

// SITE_CI_APP_ID and SITE_CI_APP_PRIVATE_KEY identify an App installed on
// compoundingtech/private with Contents: write for repository dispatches.
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
          id: 'site-token',
          name: 'Mint website dispatch token',
          uses: 'actions/create-github-app-token@v3',
          with: {
            'app-id': '${{ vars.SITE_CI_APP_ID }}',
            'private-key': '${{ secrets.SITE_CI_APP_PRIVATE_KEY }}',
            owner: 'compoundingtech',
            repositories: 'private',
            'permission-contents': 'write',
          },
        },
        {
          name: 'Dispatch docs change',
          env: { GH_TOKEN: '${{ steps.site-token.outputs.token }}' },
          run: 'gh api --method POST repos/compoundingtech/private/dispatches -f event_type=fractal-docs-changed',
        },
      ],
    },
  },
}, { notify: 'Sends a repository dispatch without a checkout, build or cache.' }))
