import { buckRootFile, rootBuck } from './buck2/root.ts'
import { typecheckTargets } from './buck2/typescript-package.ts'

const verdicts = Object.entries(typecheckTargets).flatMap(([packagePath, projects]) =>
  Object.keys(projects).map((name) => `        "${packagePath}/${name}": "//${packagePath}:${name}",`),
)

// `buck2 build //:typecheck` checks every admitted TypeScript project.
export default buckRootFile({
  text: rootBuck,
  extra: `load("@prelude//tests:test_toolchain.bzl", "noop_test_toolchain")
load("@prelude//toolchains:remote_test_execution.bzl", "remote_test_execution_toolchain")

# Prelude Rust tests require this sanitizer-label provider even for local execution.
noop_test_toolchain(name = "test", visibility = ["PUBLIC"])
# Required by Prelude's test API; no profiles means local execution only.
remote_test_execution_toolchain(name = "remote_test_execution", visibility = ["PUBLIC"])

filegroup(
    name = "typecheck",
    srcs = {
${verdicts.join('\n')}
    },
    visibility = ["PUBLIC"],
)
`,
})
