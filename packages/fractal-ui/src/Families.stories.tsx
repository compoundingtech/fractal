import type { Meta, StoryObj } from '@storybook/react-vite'
import { unifiedDiff } from '@fractal/scenarios'
import { useScenarioSlice, type WireSlice } from '@fractal/scenarios/react'
import { scenarioArgTypes } from '@fractal/scenarios/storybook'
import { useArgs } from 'storybook/preview-api'
import { Families, type FamiliesProps } from './Families'
import type { Theme } from './kit'

const changedFiles: FamiliesProps['changedFiles'] = [
  {
    path: 'src/session.ts',
    before: 'export const session = openSession()\n',
    after: 'export const session = openSession()\nexport const sessionId = session.id\n',
    status: { state: 'ready', label: 'Ready' } satisfies FamiliesProps['changedFiles'][number]['status'],
  },
  {
    path: 'src/long-filename-that-truncates-in-narrow-panes.test.ts',
    before: 'expect(session).toBeDefined()\n',
    after: 'expect(session).toBeDefined()\nexpect(session.id).toBe(sessionId)\n',
    status: { state: 'building', label: 'Checking' } satisfies FamiliesProps['changedFiles'][number]['status'],
  },
  {
    path: 'src/legacy-session.ts',
    before: 'export const legacySession = openLegacySession()\n',
    after: '// Legacy session entry point removed.\n',
    status: { state: 'error', label: 'Failed' } satisfies FamiliesProps['changedFiles'][number]['status'],
  },
].map(({ path, before, after, status }): FamiliesProps['changedFiles'][number] => {
  const lines = unifiedDiff(path, before, after).split('\n')
  return {
    id: path, path,
    added: lines.filter(line => line.startsWith('+') && !line.startsWith('+++')).length,
    removed: lines.filter(line => line.startsWith('-') && !line.startsWith('---')).length,
    status,
  }
})

const actorState = (state: WireSlice<'roster'>['state']['agents'][number]['state']): FamiliesProps['actorStates'][number] => {
  switch (state) {
    case 'running': return { state: 'building', label: state }
    case 'waiting': return { state: 'ready', label: state }
    case 'desired': case 'starting': return { state: 'queued', label: state }
    case 'failed': return { state: 'error', label: state }
    case 'down': case 'stopped': case 'suspended': case 'retired': return { state: 'canceled', label: state }
    default: return { state: 'queued', label: state }
  }
}

const rosterFamilies = ({ loading, state }: WireSlice<'roster'>): Pick<FamiliesProps, 'actors' | 'actorStates'> => {
  if (loading) return { actors: [], actorStates: [] }
  const agents = state.agents
  return {
    actors: agents.map(agent => ({ name: agent.name, detail: agent.state, tone: agent.state === 'failed' ? 'danger' : agent.state === 'waiting' ? 'warning' : 'neutral' })),
    actorStates: agents.map(agent => actorState(agent.state)),
  }
}
type FamiliesStoryProps = Omit<FamiliesProps, 'actors' | 'actorStates'>

const FamiliesStory = (args: FamiliesStoryProps) => {
  const roster = useScenarioSlice('roster', rosterFamilies)
  return <Families {...args} {...roster} />
}


const meta = {
  title: 'Fractal UI/Component families',
  component: FamiliesStory,
  args: { direction: 'folio', scheme: 'light', density: 'comfortable',
    changedFiles,
    // Johannes decision step-run/ask-ce5bd35de7577106aa6b657fd44c7d25/ask: keep this local default.
    worktree: 'isolated/example',
  },
  parameters: { scenario: { _tag: 'slices', slices: ['roster'] },
    a11y: {
      options: {
        rules: {
          // CommandMenu options show a description and a shortcut chip beside the label. Each
          // option's accessible name is exactly its visible label, which meets WCAG 2.5.3. The
          // description is still exposed through aria-describedby. axe's heuristic, however,
          // requires all visible text, including aria-hidden text, to appear in the name.
          'label-content-name-mismatch': { enabled: false },
        },
      },
    },
  },
  argTypes: {
    ...scenarioArgTypes(['roster']),
    direction: { control: 'radio', options: ['folio', 'relay', 'orbit'] },
    scheme: { control: 'radio', options: ['light', 'dark'] },
    density: { control: 'radio', options: ['compact', 'comfortable'] },
    onThemeChange: { table: { disable: true } },
  },
  render: function Render(args: FamiliesStoryProps) {
    const [, updateArgs] = useArgs()
    return <FamiliesStory {...args} onThemeChange={(change: Partial<Theme>) => updateArgs(change)} />
  },
} satisfies Meta<typeof FamiliesStory>
export default meta

type Story = StoryObj<typeof meta>
export const AllFamilies: Story = {}
export const FamiliesDark: Story = { args: { scheme: 'dark' } }
