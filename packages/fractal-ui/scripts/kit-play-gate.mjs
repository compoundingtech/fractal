import { runKitPlayGate } from './kit-play-gate-runner.mjs'
import { FULL_CASE_OVERRIDES } from './kit-play-commit-set.mjs'

// Production case requirements are reviewed source, never process configuration.
if (process.env.KIT_PLAY_CASE_OVERRIDES !== undefined) {
  throw new Error('KIT_PLAY_CASE_OVERRIDES is not supported: production kit plays must use the reviewed source policy; fixture tests inject policy through runKitPlayGate')
}
const [buildDirectory, outputDirectory] = process.argv.slice(2)
if (!buildDirectory || !outputDirectory) throw new Error('Usage: node kit-play-gate.mjs <storybook-static> <receipt-directory>')
const receipt = await runKitPlayGate({ buildDirectory, outputDirectory, overrides: FULL_CASE_OVERRIDES })
if (!receipt.pass) process.exitCode = 1
