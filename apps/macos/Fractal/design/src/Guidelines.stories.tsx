import type { Meta, StoryObj } from '@storybook/react'

/** A guideline the tray follows, with the Apple source it comes from. */
interface Source {
  readonly topic: string
  readonly title: string
  readonly url: string
  readonly rule: string
}

const sources: readonly Source[] = [
  {
    topic: 'Menu bar extras',
    title: 'HIG: The menu bar › Menu bar extras',
    url: 'https://developer.apple.com/design/human-interface-guidelines/the-menu-bar#Menu-bar-extras',
    rule: 'Black-and-clear symbol the system recolors; menu unless the content is too complex (then a window).',
  },
  {
    topic: 'Menu bar extras',
    title: 'SwiftUI MenuBarExtraStyle.window',
    url: 'https://developer.apple.com/documentation/SwiftUI/MenuBarExtraStyle/window',
    rule: 'A popover-like window whose controls lay out as in a normal window.',
  },
  {
    topic: 'Materials / Liquid Glass',
    title: 'HIG: Materials',
    url: 'https://developer.apple.com/design/human-interface-guidelines/materials',
    rule: 'Liquid Glass is the control and navigation layer; never the content layer; regular glass for text-heavy surfaces.',
  },
  {
    topic: 'Materials / Liquid Glass',
    title: 'WWDC25 219 Meet Liquid Glass',
    url: 'https://developer.apple.com/videos/play/wwdc2025/219/',
    rule: 'No glass on glass; tint only primary actions; use fills and vibrancy on top of glass.',
  },
  {
    topic: 'Materials / Liquid Glass',
    title: 'Adopting Liquid Glass',
    url: 'https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass',
    rule: 'Remove custom backgrounds from popovers and let the system material show.',
  },
  {
    topic: 'Materials / Liquid Glass',
    title: 'WWDC25 310 Build an AppKit app with the new design',
    url: 'https://developer.apple.com/videos/play/wwdc2025/310/',
    rule: 'Taller mini–medium controls, capsule large controls; concentric shapes; status text not on glass.',
  },
  {
    topic: 'Materials / Liquid Glass',
    title: 'WWDC26 289 (AppKit, macOS 27)',
    url: 'https://developer.apple.com/videos/play/wwdc2026/289/',
    rule: 'Menu-bar extras get expanded-interface sessions; interactive glass only for controls.',
  },
  {
    topic: 'Typography',
    title: 'HIG: Typography › macOS built-in text styles',
    url: 'https://developer.apple.com/design/human-interface-guidelines/typography#macOS-built-in-text-styles',
    rule: 'Headline 13 bold, body 13, callout 12, subheadline 11, footnote 10; 10 pt minimum; use styles, not sizes.',
  },
  {
    topic: 'SF Symbols',
    title: 'HIG: SF Symbols',
    url: 'https://developer.apple.com/design/human-interface-guidelines/sf-symbols',
    rule: 'Hierarchical rendering for depth; variable color for change, not depth; badges via symbol variants.',
  },
  {
    topic: 'Color & vibrancy',
    title: 'HIG: Color',
    url: 'https://developer.apple.com/design/human-interface-guidelines/color',
    rule: 'Semantic label colors; the person’s accent color; never color alone for essential information.',
  },
  {
    topic: 'Layout & controls',
    title: 'SwiftUI GroupedFormStyle',
    url: 'https://developer.apple.com/documentation/SwiftUI/GroupedFormStyle',
    rule: 'Grouped sections, leading labels, trailing controls: the System Settings look.',
  },
  {
    topic: 'Layout & controls',
    title: 'HIG: Toggles / Buttons / Popovers',
    url: 'https://developer.apple.com/design/human-interface-guidelines/popovers',
    rule: 'No Close button in popovers; save work when they close; one prominent action; ellipsis for buttons that open more UI.',
  },
  {
    topic: 'Layout & controls',
    title: 'macOS 27 release notes',
    url: 'https://developer.apple.com/documentation/macos-release-notes/macos-27-release-notes',
    rule: 'controlSize and button border shape reset inside popovers: set them on the panel root.',
  },
  {
    topic: 'Charts',
    title: 'HIG: Charts',
    url: 'https://developer.apple.com/design/human-interface-guidelines/charts',
    rule: 'Show the trend, mark thresholds, describe marks for VoiceOver; Swift Charts BarMark/RuleMark.',
  },
  {
    topic: 'Accessibility',
    title: 'HIG: Accessibility',
    url: 'https://developer.apple.com/design/human-interface-guidelines/accessibility',
    rule: 'Reduce Transparency and Increase Contrast make system materials frostier/bordered; keyboard reaches every control.',
  },
]

/** One HIG checklist item judged against the current design. */
interface Check {
  readonly item: string
  readonly verdict: 'pass' | 'partial' | 'fail'
  readonly evidence: string
}

const checklist: readonly Check[] = [
  {
    item: 'Status item is a template symbol (no color), sized for the 24 pt bar',
    verdict: 'pass',
    evidence:
      'TrayMenuBarLabel draws an isTemplate image: one symbol and the open-ask count as plain text, like the battery percentage; no second (gauge) glyph.',
  },
  {
    item: 'Window-style extra only because the content is too complex for a menu',
    verdict: 'pass',
    evidence: 'Asks need inline answers and text entry; a menu cannot hold them.',
  },
  {
    item: 'Panel draws no background; the system Liquid Glass is the surface',
    verdict: 'pass',
    evidence:
      'TrayPanel has no .background/glassEffect; the V1 captures showed the menu-bar window’s glass behind the grouped sections.',
  },
  {
    item: 'No glass on glass; no popover cascading from the panel',
    verdict: 'pass',
    evidence:
      'Status chips are toggles that expand their section in place inside the same Form (no .popover); the chart tooltip uses a content-layer fill (.background.secondary), not a material.',
  },
  {
    item: 'One leading edge for title, sections, rows and footer',
    verdict: 'pass',
    evidence:
      'PanelMetrics.inset is the grouped Form’s content inset (30 pt), shared by the header, the status strip and the footer.',
  },
  {
    item: 'Text styles only, no fixed point sizes; clear hierarchy',
    verdict: 'pass',
    evidence:
      'TrayViews uses headline/body/callout/subheadline/footnote/caption only. Section headers are subheadline semibold secondary under the one headline title; footers are secondary metadata.',
  },
  {
    item: 'Status never carried by color alone',
    verdict: 'pass',
    evidence:
      'Symbols + text for connection, objectives, availability; urgency is a symbol plus semibold text; offline swaps the menu-bar glyph. Histogram hours without a bar differ in shape (spent: cross, unmeasured: ring); a key under the chart names them and the dashed nearly-spent rule.',
  },
  {
    item: 'SF Symbols chosen per job; system symbols only',
    verdict: 'pass',
    evidence:
      'Ask kind and availability symbols use .hierarchical; the urgent bubble is .monochrome so it does not wash out; status symbols use filled variants.',
  },
  {
    item: 'One prominent action per view',
    verdict: 'partial',
    evidence:
      'Only the first answerable ask’s recommended answer is .borderedProminent (never a recommended decline), and only while nothing is typed or focused; the note editor’s Save takes that role while open. No answer is the Return default: answers are irreversible. Flat fallbacks draw it untinted (inactive window).',
  },
  {
    item: 'No hover-only controls',
    verdict: 'pass',
    evidence:
      'Mark Read, Open in Fractal, Read Aloud and an open ask’s answers are always in the hierarchy; a closed ask opens with its disclosure from the keyboard; no row reflows under the pointer (the full update summary is the help text).',
  },
  {
    item: 'Control size set on the panel root (macOS 27 resets it in popovers)',
    verdict: 'pass',
    evidence: 'TrayPanel sets controlSize on its root; answer buttons use .small for density.',
  },
  {
    item: 'Keyboard reaches every control; frequent actions have shortcuts',
    verdict: 'partial',
    evidence:
      '⌘R refresh, ⌘, Settings, ⌘Q quit, Return sends the focused field or saves the note; Tab reaches every control. No arrow-key navigation between asks.',
  },
  {
    item: 'Reduce Transparency / Increase Contrast respected',
    verdict: 'partial',
    evidence:
      'Inherited from system materials and semantic colors; not yet captured with those settings on.',
  },
  {
    item: 'Text size (macOS has no Dynamic Type)',
    verdict: 'partial',
    evidence:
      'Text styles are ready for the Accessibility › Text size setting; macOS does not scale third-party text styles today.',
  },
  {
    item: 'Charts: Swift Charts, visible threshold, per-bar accessibility, detail',
    verdict: 'partial',
    evidence:
      'BarMark per hour with labelled bars for VoiceOver; dashed rule at nearly spent; spent hours carry a cross and unmeasured hours a ring on the baseline, with a key under the chart. The reading and time per bar show on pointer hover only.',
  },
  {
    item: 'Work survives the panel closing',
    verdict: 'pass',
    evidence:
      'Typed answers, the picked option and the note draft live on AttentionModel; a commit or the card leaving Fractal clears them.',
  },
  {
    item: 'People can hide the extra (isInserted, Settings)',
    verdict: 'pass',
    evidence:
      'MenuBarExtra(isInserted:) bound to Show in Menu Bar in Settings…; opening the app again re-inserts it.',
  },
  {
    item: 'Light and dark verified on the real window server',
    verdict: 'partial',
    evidence:
      'The panel and the status item show flat fallbacks until the headless VM capture pipeline renders them.',
  },
]

/** First-party menu-bar extras the panel is compared against. */
const benchmarks = [
  'Control Center modules: bold headline title, platters on glass, a trailing control per row — the tray’s grouped sections match this rhythm.',
  'Wi-Fi / Bluetooth: section headers small and secondary, rows of symbol + title + detail, settings link last — the tray’s section headers follow (subheadline semibold, secondary).',
  'Battery: a percentage beside the glyph in the menu bar — the precedent for the plain count beside the tray’s symbol.',
  'Focus: the active mode as the row’s value with a menu to change it — the tray’s availability rows use the same pop-up pattern.',
  'Control Center: a module expands in place rather than opening another window — the tray’s status chips do the same.',
]

const GuidelinesView = () => (
  <div className="at-gallery">
    <section className="at-gallery-row">
      <h2 className="at-gallery-heading">HIG checklist · current design</h2>
      <table className="at-checklist">
        <thead>
          <tr>
            <th>Guideline</th>
            <th>Verdict</th>
            <th>Evidence</th>
          </tr>
        </thead>
        <tbody>
          {checklist.map((check) => (
            <tr key={check.item}>
              <td>{check.item}</td>
              <td className="at-verdict" data-verdict={check.verdict}>
                {check.verdict}
              </td>
              <td>{check.evidence}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
    <section className="at-gallery-row">
      <h2 className="at-gallery-heading">First-party benchmarks</h2>
      <ul className="at-sources">
        {benchmarks.map((benchmark) => (
          <li key={benchmark}>{benchmark}</li>
        ))}
      </ul>
    </section>
    <section className="at-gallery-row">
      <h2 className="at-gallery-heading">Sources followed</h2>
      <ul className="at-sources">
        {sources.map((source) => (
          <li key={source.url}>
            <strong>{source.topic}</strong> —{' '}
            <a href={source.url} target="_blank" rel="noreferrer">
              {source.title}
            </a>
            : {source.rule}
          </li>
        ))}
      </ul>
    </section>
  </div>
)

const meta = {
  title: 'AttentionTray/Guidelines',
  component: GuidelinesView,
  parameters: {
    docs: {
      description: {
        component:
          'The Apple guidance the tray follows (HIG, WWDC25/26, SwiftUI/AppKit docs), first-party benchmarks, and a pass/partial/fail checklist for the current design.',
      },
    },
  },
} satisfies Meta<typeof GuidelinesView>

export default meta
type Story = StoryObj<typeof meta>

/** Checklist, benchmarks and cited sources. */
export const HigChecklist: Story = {}
