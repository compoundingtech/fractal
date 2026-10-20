import * as Alchemy from 'alchemy'
import * as Cloudflare from 'alchemy/Cloudflare'
import * as Config from 'effect/Config'
import * as Effect from 'effect/Effect'
import { fileURLToPath } from 'node:url'

// One domain setting, shared by local deploys and CI. Empty disables the custom domain.
const domain = Config.String('STORYBOOK_DOMAIN').pipe(Config.withDefault('storybook.compounding.tech'))

export default Alchemy.Stack(
  'SmalltalkStorybook',
  { providers: Cloudflare.providers(), state: Cloudflare.state() },
  Effect.gen(function* () {
    const stage = yield* Alchemy.Stage
    const canonicalDomain = yield* domain
    const site = yield* Cloudflare.Worker('Site', {
      name: `smalltalk-storybook-${stage}`,
      assets: { directory: fileURLToPath(new URL('../../packages/fractal-ui/storybook-static/', import.meta.url)) },
      workersDev: true,
      domain: stage === 'main' && canonicalDomain !== '' ? canonicalDomain : undefined,
    })
    return { url: site.url, urls: site.urls }
  }),
)
