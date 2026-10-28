import { defaultActionlintConfig, githubWorkflow, plainFlakeSetupSteps } from '../../repos/effect-utils/genie/external.ts'
import { megarepoApplyStep, readOnlyBinaryCaches } from './workspace-ci.ts'
import { auditCaches } from './cache-audit.ts'
import { scopeLinuxNixSandbox } from './nix-sandbox-scope.ts'

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
      ref: '${{ github.sha }}',
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

export default githubWorkflow(scopeLinuxNixSandbox(auditCaches({
  actionlint: defaultActionlintConfig,
  name: 'Storybook deploy',
  on: {
    push: { branches: ['main'] },
  },
  permissions: { contents: 'read' },
  // Never interrupt an infrastructure apply; serialize updates to the canonical site.
  concurrency: { group: 'storybook-deploy-main', 'cancel-in-progress': false },
  jobs: {
    deploy: {
      name: 'storybook-deploy',
      if: "github.event_name == 'push' && github.ref == 'refs/heads/main'",
      'runs-on': 'ubuntu-latest',
      'timeout-minutes': 30,
      defaults: { run: { shell: 'bash' } },
      env: { STAGE: 'main' },
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
const canonical = domain !== '' ? 'https://' + domain : fallback
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
  },
}, {
  deploy: 'Storybook uses a clean static build and read-only upstream Nix caches; no Actions build cache is configured.',
})))
