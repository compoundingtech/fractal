import { describe, expect, it } from 'vitest'

import appSource from '../web/App.tsx?raw'
import buildChangesSource from './BuildChangesSegment.tsx?raw'
import devbarSource from './DevBar.tsx?raw'
import preferenceSource from './devBarPreference.ts?raw'
import transportMetersSource from './transportMeters.tsx?raw'

/** These source-boundary guards run without importing browser diagnostics into Node. */
describe('composable developer diagnostics boundary', () => {
  it('loads diagnostic modules only through a statically gated dynamic import', () => {
    // A static DevBar import in App used to pull the diagnostic graph into production.
    expect(appSource).not.toMatch(/import\s+\{\s*DevBar\s*\}\s+from/)
    expect(appSource).toMatch(/import\.meta\.env\.DEV\s*\|\|\s*import\.meta\.env\.PERF\s*\?\s*React\.lazy/)
    expect(appSource).toContain("import('../telemetry/DevBar.tsx')")
  })

  it('composes the kit shell with host-owned counters, meters, and transport slots', () => {
    expect(devbarSource).toContain("import { Devbar } from '@overeng/devbar'")
    // Build identity and recent changes share one segment: the kit trigger with a host-owned disclosure.
    expect(devbarSource).toContain("import { BuildChangesSegment } from './BuildChangesSegment.tsx'")
    expect(devbarSource).toMatch(/\{\s*id: 'build',\s*priority: 1,\s*hasAlert: hasUnseen,\s*render: \(\) => \(\s*<BuildChangesSegment\b/)
    expect(devbarSource).not.toContain("id: 'version'")
    expect(devbarSource).not.toContain("id: 'changelog'")
    expect(buildChangesSource).toContain("import { BuildIdentitySegment } from '@overeng/devbar/build-identity'")
    expect(buildChangesSource).toMatch(/<BuildIdentitySegment\s+identity=\{identity\}\s+now=\{now\}\s+disclosure=\{\{/)
    expect(devbarSource).toContain("{ id: 'counters', label: 'Counters'")
    expect(devbarSource).toContain("{ id: 'meters', label: 'Meters'")
    for (const id of ['websocket', 'http-active', 'http-p50', 'http-p95', 'http-errors', 'subscriptions', 'message-rate']) {
      expect(transportMetersSource).toContain(`{ id: '${id}', label:`)
    }
    expect(devbarSource).toContain('<MetersProvider meters={meters}>')
    expect(devbarSource).toContain('openPanel={openPanel} onOpenPanelChange={setOpenPanel}')
    expect(preferenceSource).toContain("key: 'devbar.visible'")
  })
})
