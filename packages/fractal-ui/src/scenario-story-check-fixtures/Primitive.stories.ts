import * as React from 'react'
import type { Meta, StoryObj } from '@storybook/react'

const Example = () => React.createElement('span', {}, 'Example')
const meta = {
  title: 'Scenario check fixture/TypeScript CSF',
  component: Example,
  parameters: { scenario: { _tag: 'primitive', reason: 'Tests CSF discovery without JSX or domain data.' } },
} satisfies Meta<typeof Example>
export default meta
export const AllStates: StoryObj<typeof meta> = {}
