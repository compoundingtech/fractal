import { readFileSync } from 'node:fs'
import { defaultActionlintConfig, githubWorkflow, plainFlakeSetupSteps } from '../../repos/effect-utils/genie/external.ts'
import { megarepoApplyStep, readOnlyBinaryCaches } from './workspace-ci.ts'
import { auditCaches } from './cache-audit.ts'

// Independent of required checks. Keep this superset of the kit's transitive inputs in
// scripts/ci-fractal-web-changes, including the generated changelog and source-input staging.
const paths = [
  'packages/fractal-ui/**',
  'apps/fractal-web/**',
  'clients/typescript/fractal-client/**',
  'clients/typescript/fractal-views/**',
  'clients/typescript/fractal-scenarios/**',
  'docs/st3/client-v0/schemas/**',
  'fixtures/**',
  '**/package.json*',
  '**/pnpm-workspace.yaml*',
  '**/pnpm-lock.yaml',
  '**/tsconfig*.json*',
  '.npmrc',
  'pnpm-install-contract.json*',
  'pnpm-source-inputs.json*',
  'megarepo.kdl',
  'megarepo.lock',
  'flake.nix',
  'flake.lock',
  'nix/**',
  'genie/**',
  'scripts/**',
  'infra/storybook/**',
  '.github/workflows/storybook-deploy.yml*',
  '.github/workflows/workspace-ci.ts',
  '.github/workflows/cache-audit.ts',
]

const stage = "${{ github.event_name == 'pull_request' && format('pr-{0}', github.event.pull_request.number) || 'main' }}"
const configureDomain = `# Preserve the difference between an absent variable (stack default) and an empty one.
if jq -e 'has("STORYBOOK_DOMAIN")' <<< "$STORYBOOK_VARIABLES" > /dev/null; then
  export STORYBOOK_DOMAIN
  STORYBOOK_DOMAIN=$(jq -r '.STORYBOOK_DOMAIN' <<< "$STORYBOOK_VARIABLES")
fi`
const cloudflareEnv = {
  CLOUDFLARE_API_TOKEN: '${{ secrets.CLOUDFLARE_API_TOKEN }}',
  CLOUDFLARE_ACCOUNT_ID: '${{ secrets.CLOUDFLARE_ACCOUNT_ID }}',
  STORYBOOK_VARIABLES: '${{ toJSON(vars) }}',
}

const setupSteps = [
  {
    name: 'Check deployment credentials',
    id: 'credentials',
    env: {
      CLOUDFLARE_API_TOKEN: '${{ secrets.CLOUDFLARE_API_TOKEN }}',
      CLOUDFLARE_ACCOUNT_ID: '${{ secrets.CLOUDFLARE_ACCOUNT_ID }}',
    },
    run: `if [[ -n "$CLOUDFLARE_API_TOKEN" && -n "$CLOUDFLARE_ACCOUNT_ID" ]]; then
  echo 'enabled=true' >> "$GITHUB_OUTPUT"
else
  echo 'enabled=false' >> "$GITHUB_OUTPUT"
  printf '## Storybook deployment skipped\\n\\nConfigure both CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID repository secrets to enable deployment.\\n' >> "$GITHUB_STEP_SUMMARY"
fi`,
  },
  {
    uses: 'actions/checkout@v4',
    if: "steps.credentials.outputs.enabled == 'true'",
    with: {
      // An immutable PR head also works after the source branch is deleted on close.
      ref: '${{ github.event.pull_request.head.sha || github.sha }}',
      'persist-credentials': false,
    },
  },
  ...plainFlakeSetupSteps({ nix: { binaryCaches: readOnlyBinaryCaches } }).map((step) => ({
    ...step,
    if: "steps.credentials.outputs.enabled == 'true'",
  })),
  { ...megarepoApplyStep, if: "steps.credentials.outputs.enabled == 'true'" },
  {
    name: 'Install the standalone deployment stack',
    if: "steps.credentials.outputs.enabled == 'true'",
    run: 'nix develop .#web --command pnpm --dir infra/storybook install --frozen-lockfile',
  },
]

export default githubWorkflow(auditCaches({
  actionlint: defaultActionlintConfig,
  name: 'Storybook deploy',
  on: {
    push: { branches: ['main'] },
    // Close must always be delivered, even after all Storybook changes are reverted.
    pull_request: { types: ['opened', 'synchronize', 'reopened', 'ready_for_review', 'converted_to_draft', 'closed'] },
  },
  permissions: { contents: 'read' },
  // Never interrupt an apply/destroy; a close queues cleanup behind an in-flight deploy.
  concurrency: { group: `storybook-deploy-${stage}`, 'cancel-in-progress': false },
  jobs: {
    changes: {
      name: 'storybook-changes',
      if: "(github.event_name != 'pull_request' || github.event.pull_request.draft == false) && (github.event_name == 'push' || (github.event.action != 'closed' && github.event.pull_request.head.repo.full_name == github.repository))",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 5,
      permissions: { contents: 'read', 'pull-requests': 'read' },
      outputs: { changed: '${{ steps.scope.outputs.changed }}' },
      steps: [{
        name: 'Detect Storybook input changes',
        id: 'scope',
        env: { GH_TOKEN: '${{ github.token }}', STORYBOOK_PATHS: JSON.stringify(paths) },
        run: `python3 - <<'STORYBOOK_CHANGES_PY'\n${readFileSync(new URL('../../scripts/ci-storybook-changes', import.meta.url), 'utf8')}\nSTORYBOOK_CHANGES_PY`,
      }],
    },
    deploy: {
      name: 'storybook-deploy',
      needs: ['changes'],
      if: "(github.event_name != 'pull_request' || github.event.pull_request.draft == false) && (needs.changes.outputs.changed == 'true' && (github.event_name == 'push' || (github.event.action != 'closed' && github.event.pull_request.head.repo.full_name == github.repository)))",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 30,
      defaults: { run: { shell: 'bash' } },
      env: { STAGE: stage },
      steps: [
        ...setupSteps,
        {
          name: 'Install and build Storybook without deployment credentials',
          if: "steps.credentials.outputs.enabled == 'true'",
          // The web shell publishes pnpm-source-inputs before the frozen root install.
          run: `nix develop .#web --command bash -euo pipefail -c '
pnpm install --frozen-lockfile
node apps/fractal-web/scripts/changelog-registry.mjs
pnpm --filter @fractal/ui build-storybook
'`,
        },
        {
          name: 'Deploy Storybook and publish its URLs',
          if: "steps.credentials.outputs.enabled == 'true'",
          env: cloudflareEnv,
          run: `${configureDomain}
nix develop .#web --command pnpm --dir infra/storybook run deploy --stage "$STAGE" --yes
# Alchemy v2 prints human-oriented outputs, not JSON. Read the authoritative account
# subdomain instead of scraping CLI logs. Keep the API token out of logs and summaries.
SUBDOMAIN=$(curl --fail --silent --show-error \\
  --header "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \\
  "https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/workers/subdomain" \\
  | jq -er 'select(.success == true) | .result.subdomain | select(type == "string" and test("^[a-z0-9-]+$"))')
WORKERS_URL="https://smalltalk-storybook-$STAGE.$SUBDOMAIN.workers.dev"
export WORKERS_URL
node --input-type=module <<'STORYBOOK_SUMMARY'
import { appendFileSync } from 'node:fs'
const fallback = process.env.WORKERS_URL
const domain = process.env.STORYBOOK_DOMAIN ?? 'storybook.compounding.tech'
// Only main owns the canonical domain; previews always use their stage-specific worker.
const canonical = process.env.STAGE === 'main' && domain !== '' ? 'https://' + domain : fallback
for (const value of [canonical, fallback]) {
  const url = new URL(value)
  if (url.protocol !== 'https:' || !/^[a-z0-9.-]+$/i.test(url.host) || url.pathname !== '/' || url.search || url.hash || url.username || url.password) {
    throw new Error('Unexpected Storybook deployment URL')
  }
}
appendFileSync(process.env.GITHUB_STEP_SUMMARY,
  '## Storybook deployment\\n\\n' +
  '- Storybook: <' + canonical + '>\\n' +
  '- workers.dev fallback: <' + fallback + '>\\n')
STORYBOOK_SUMMARY`,
        },
      ],
    },
    destroy: {
      name: 'storybook-destroy',
      // Cleanup must run even when a previously deployed PR is closed as a draft.
      if: "github.event_name == 'pull_request' && github.event.action == 'closed' && github.event.pull_request.head.repo.full_name == github.repository",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 30,
      defaults: { run: { shell: 'bash' } },
      env: { STAGE: stage },
      steps: [
        ...setupSteps,
        {
          name: 'Destroy the closed pull request preview',
          if: "steps.credentials.outputs.enabled == 'true'",
          env: cloudflareEnv,
          run: `${configureDomain}
nix develop .#web --command pnpm --dir infra/storybook run destroy --stage "$STAGE" --yes
printf '## Storybook preview removed\\n\\nStage: %s\\n' "$STAGE" >> "$GITHUB_STEP_SUMMARY"`,
        },
      ],
    },
  },
}, {
  changes: 'Reads GitHub PR file metadata only; no checkout, install or build.',
  deploy: 'Storybook uses a clean static build and read-only upstream Nix caches; no Actions build cache is configured.',
  destroy: 'Destroys remote infrastructure only; no Storybook build or build cache.',
}))
