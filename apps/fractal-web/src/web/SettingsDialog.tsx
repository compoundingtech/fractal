import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { useAtom, useAtomValue } from '@effect/atom-react'
import { ChromeControl } from '../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { ThemePortal } from '../../../../packages/fractal-ui/src/assistant-ui/taste/ThemePortal.tsx'
import { accentVars as accent, borderVars as border, elevationVars as elevation, geometryVars as g, interactionVars as interaction, motionVars as m, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { AvailabilityPanel } from '../availability/AvailabilityPanel.tsx'
import { useConnection, useDataSource } from '../data/react.tsx'
import { devBarAvailable, toggleDevBar, useDevBarVisible } from '../telemetry/devBarPreference.ts'
import { deviceAccess, type SettingsBuild } from './settingsModel.ts'
import { shortcutCaps, shortcutList, type ShortcutDefinition } from './shortcuts.ts'
import { systemEventsPreference } from './conversationPreferences.ts'

export type SettingsScheme = 'dark' | 'light' | 'system'

export interface SettingsControlProps {
  readonly scheme: SettingsScheme
  readonly onSchemeChange: (scheme: SettingsScheme) => void
  readonly build?: SettingsBuild | undefined
}

/**
 * The sidebar header's gear and the modal it opens. The trigger owns open state and focus return, and the portal stays
 * inside the themed workspace, or inside the phone drawer's focus scope, so the sheet keeps the colour scheme and contains focus.
 * Contents and their reads exist only while open.
 */
export const SettingsControl = ({ scheme, onSchemeChange, build }: SettingsControlProps) =>
  <Aria.DialogTrigger>
    <ChromeControl icon="gear" label="Settings" popup="dialog" edge="trail" />
    <ThemePortal>
      <Aria.ModalOverlay isDismissable {...stylex.props(styles.overlay)}>
        <Aria.Modal {...stylex.props(styles.modal)}>
          <Aria.Dialog aria-label="Settings" data-testid="settings-dialog" data-shortcuts-paused="" {...stylex.props(styles.dialog)}>
            {({ close }) => <>
              {/* The close control's open tooltip would otherwise take the first Escape for itself. */}
              <header onKeyDownCapture={event => { if (event.key === 'Escape') close() }} {...stylex.props(styles.header)}>
                <Aria.Heading slot="title" {...stylex.props(styles.title)}>Settings</Aria.Heading>
                <ChromeControl icon="x" label="Close settings" edge="trail" onPress={close} />
              </header>
              <div {...stylex.props(styles.body)}>
                <AvailabilitySection />
                <Section title="Appearance"><ThemeRow scheme={scheme} onSchemeChange={onSchemeChange} /></Section>
                <Section title="Keyboard shortcuts"><ShortcutList /></Section>
                <Section title="Dev tools"><SystemEventsRow /><DevBarRow /></Section>
                <Section title="Device and connection"><DeviceFacts build={build} /></Section>
              </div>
            </>}
          </Aria.Dialog>
        </Aria.Modal>
      </Aria.ModalOverlay>
    </ThemePortal>
  </Aria.DialogTrigger>

const Section = ({ title, children }: { readonly title: string; readonly children: React.ReactNode }) => {
  const id = React.useId()
  return <section aria-labelledby={id} {...stylex.props(styles.section)}>
    <h3 id={id} {...stylex.props(styles.sectionTitle)}>{title}</h3>
    {children}
  </section>
}

/** Present only where the source reaches a gateway for a person; fixture sources have no port and a replay page drops it. */
const AvailabilitySection = () => {
  const port = useDataSource().availability
  return port === undefined ? null : <Section title="Availability"><AvailabilityPanel port={port} /></Section>
}

const ThemeRow = ({ scheme, onSchemeChange }: Pick<SettingsControlProps, 'scheme' | 'onSchemeChange'>) => {
  const touch = React.useContext(NarrowLayoutContext)
  return <Aria.RadioGroup value={scheme} onChange={value => onSchemeChange(value === 'light' || value === 'system' ? value : 'dark')} orientation="horizontal" {...stylex.props(styles.row)}>
    <Aria.Label {...stylex.props(styles.rowLabel)}>Theme</Aria.Label>
    <div {...stylex.props(styles.segments)}>
      {(['light', 'dark', 'system'] as const).map(option => <Aria.Radio key={option} value={option} {...stylex.props(styles.segment, touch && styles.touch, scheme === option && styles.segmentSelected)}>
        {option === 'light' ? 'Light' : option === 'dark' ? 'Dark' : 'System'}
      </Aria.Radio>)}
    </div>
  </Aria.RadioGroup>
}

const ShortcutList = () =>
  <dl {...stylex.props(styles.facts)}>
    {shortcutList(devBarAvailable()).map(shortcut => <div key={shortcut.action} {...stylex.props(styles.fact)}>
      <dt {...stylex.props(styles.factLabel)}>{shortcut.action}</dt>
      <dd {...stylex.props(styles.factValue, styles.keys)}><ShortcutKeys shortcut={shortcut} /></dd>
    </div>)}
  </dl>

const ShortcutKeys = ({ shortcut }: { readonly shortcut: ShortcutDefinition }) =>
  shortcutCaps(shortcut).map((chord, alternative) => <React.Fragment key={alternative}>
    {alternative === 0 ? null : <span {...stylex.props(styles.keyOr)}>or</span>}
    {chord.flat().map((cap, index) => <kbd key={index} {...stylex.props(styles.key)}>{cap}</kbd>)}
  </React.Fragment>)

const SystemEventsRow = () => {
  const [visible, setVisible] = useAtom(systemEventsPreference)
  const touch = React.useContext(NarrowLayoutContext)
  return <Aria.Switch isSelected={visible} onChange={setVisible} {...stylex.props(styles.row, styles.switchRow, touch && styles.touch)}>
    <span {...stylex.props(styles.rowLabel)}>Show all system events</span>
    <span aria-hidden="true" {...stylex.props(styles.track, visible && styles.trackOn)}><span {...stylex.props(styles.thumb, visible && styles.thumbOn)} /></span>
  </Aria.Switch>
}

const DevBarRow = () => {
  const visible = useDevBarVisible()
  const touch = React.useContext(NarrowLayoutContext)
  if (!devBarAvailable()) return <p {...stylex.props(styles.note)}>This build does not include the devbar.</p>
  return <Aria.Switch isSelected={visible} onChange={toggleDevBar} {...stylex.props(styles.row, styles.switchRow, touch && styles.touch)}>
    <span {...stylex.props(styles.rowLabel)}>Show devbar</span>
    <span aria-hidden="true" {...stylex.props(styles.track, visible && styles.trackOn)}><span {...stylex.props(styles.thumb, visible && styles.thumbOn)} /></span>
  </Aria.Switch>
}

/** Only what the client already holds: the granted capabilities, the host it reaches, and the injected build. Never the credential. */
const DeviceFacts = ({ build }: { readonly build?: SettingsBuild | undefined }) => {
  const source = useDataSource()
  const access = deviceAccess(useAtomValue(source.grants))
  const connected = useConnection()._tag === 'Live'
  return <>
    <dl {...stylex.props(styles.facts)}>
      <Fact label="Access">{access.title}{connected ? '' : ' (last known, not connected)'}</Fact>
      <Fact label="May">{access.allowed.join(', ')}</Fact>
      {access.refused.length === 0 ? null : <Fact label="May not">{access.refused.join(', ')}</Fact>}
      <Fact label="Connection">{connected ? 'Connected' : 'Not connected'}</Fact>
      <Fact label="Gateway" mono>{source.gateway ?? 'Not reported'}</Fact>
      <Fact label="Version" mono>{build === undefined ? 'Not reported' : `${build.version}${build.revision === undefined ? '' : ` · ${build.revision.slice(0, 9)}`}${build.dirty === true ? ' · dirty' : ''}`}</Fact>
    </dl>
    <p {...stylex.props(styles.note)}>
      Access is set when this device is paired and can't be changed here. Pair it again with <code {...stylex.props(styles.code)}>fractal devices --as &lt;person&gt; pair --full-control</code> or <code {...stylex.props(styles.code)}>--read-only</code>, then give the app's server the new grant through <code {...stylex.props(styles.code)}>WF_ST_AUTHORIZATION_FILE</code>. <code {...stylex.props(styles.code)}>WF_ST_GATEWAY</code> selects the gateway. The version changes when the app is rebuilt.
    </p>
  </>
}

const Fact = ({ label, mono = false, children }: { readonly label: string; readonly mono?: boolean; readonly children: React.ReactNode }) =>
  <div {...stylex.props(styles.fact)}>
    <dt {...stylex.props(styles.factLabel)}>{label}</dt>
    <dd {...stylex.props(styles.factValue, mono && styles.mono)}>{children}</dd>
  </div>

const phone = '@media (max-width: 899px)'

const styles = stylex.create({
  overlay: { position: 'fixed', insetInline: s.zero, insetBlockStart: s.zero, insetBlockEnd: 'var(--wf-devbar-space, 0px)', zIndex: 100, display: 'flex', alignItems: { default: 'center', [phone]: 'stretch' }, justifyContent: 'center', backgroundColor: surface.scrim },
  modal: {
    display: 'flex', boxSizing: 'border-box', outlineStyle: 'none',
    width: { default: '560px', [phone]: '100%' }, maxWidth: { default: 'calc(100vw - 32px)', [phone]: '100%' }, maxHeight: { default: 'min(640px, calc(100% - 32px))', [phone]: '100%' },
    paddingBlockStart: { default: s.zero, [phone]: 'env(safe-area-inset-top)' }, paddingBlockEnd: { default: s.zero, [phone]: 'env(safe-area-inset-bottom)' }, paddingInlineStart: { default: s.zero, [phone]: 'env(safe-area-inset-left)' }, paddingInlineEnd: { default: s.zero, [phone]: 'env(safe-area-inset-right)' },
    borderWidth: { default: g.hairline, [phone]: s.zero }, borderStyle: 'solid', borderColor: border.borderStrong, borderRadius: { default: r.lg, [phone]: r.none },
    backgroundColor: surface.raised, color: ink.fg, boxShadow: elevation.dialog, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading,
  },
  dialog: { display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: s.zero, minHeight: s.zero, outlineStyle: 'none' },
  header: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexShrink: 0, minHeight: g.band, boxSizing: 'border-box', paddingInline: s.xl, borderBlockEndWidth: g.hairline, borderBlockEndStyle: 'solid', borderBlockEndColor: border.border },
  title: { margin: s.zero, fontSize: t.headingSize, lineHeight: t.headingLeading, fontWeight: t.weightMedium },
  body: { flexGrow: 1, minHeight: s.zero, overflowY: 'auto', paddingBlock: s.md, paddingInline: s.xl },
  section: { display: 'flex', flexDirection: 'column', gap: s.md, paddingBlock: s.lg },
  sectionTitle: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, color: ink.fgMuted },
  row: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: s.lg },
  rowLabel: { color: ink.fg },
  segments: { display: 'inline-flex', gap: s.xs2, padding: s.xs2, borderRadius: r.control, backgroundColor: surface.controlFill },
  segment: {
    display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box', minWidth: '64px', minHeight: g.controlMd, paddingInline: s.lg,
    borderRadius: r.sm, color: ink.fgMuted, cursor: 'pointer', outlineStyle: 'none',
    ':hover': { color: ink.fg },
    ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset },
  },
  segmentSelected: { backgroundColor: surface.rowActive, color: ink.fg },
  touch: { minHeight: g.touchTarget },
  switchRow: { cursor: 'pointer', minHeight: g.controlMd, borderRadius: r.sm, outlineStyle: 'none', ':is([data-focus-visible])': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: g.focusOffset } },
  track: { display: 'inline-flex', alignItems: 'center', flexShrink: 0, boxSizing: 'border-box', width: '32px', height: '18px', padding: s.xs2, borderRadius: r.full, backgroundColor: border.rail, transitionProperty: 'background-color', transitionDuration: m.fast, transitionTimingFunction: m.ease },
  trackOn: { backgroundColor: accent.primary },
  thumb: { width: '14px', height: '14px', borderRadius: r.full, backgroundColor: ink.fg, transitionProperty: 'transform', transitionDuration: m.fast, transitionTimingFunction: m.ease },
  thumbOn: { transform: 'translateX(14px)' },
  facts: { display: 'flex', flexDirection: 'column', gap: s.sm, margin: s.zero },
  fact: { display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: s.lg },
  factLabel: { flexShrink: 0, color: ink.fgSoft },
  factValue: { margin: s.zero, minWidth: s.zero, textAlign: 'end', overflowWrap: 'anywhere', color: ink.fg },
  mono: { fontFamily: t.fontMono, fontSize: t.metaSize },
  keys: { display: 'inline-flex', alignItems: 'center', justifyContent: 'flex-end', flexWrap: 'wrap', gap: s.xs },
  keyOr: { fontSize: t.metaSize, color: ink.fgMuted },
  key: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', boxSizing: 'border-box', minWidth: g.controlSm, height: g.controlSm, paddingInline: s.sm, borderRadius: r.keyboard, backgroundColor: surface.controlFill, color: ink.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize },
  note: { margin: s.zero, fontSize: t.metaSize, lineHeight: t.metaLeading, color: ink.fgMuted },
  code: { fontFamily: t.fontMono, color: ink.fgSoft, overflowWrap: 'anywhere' },
})
