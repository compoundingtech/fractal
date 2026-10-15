import { execFileSync } from 'node:child_process'

/**
 * Production and app Storybook embed the same real checkout identity.
 * @returns {import('vite').Plugin}
 */
export const fractalBuildIdentity = () => {
  /** @param {...string} args */
  const git = (...args) => execFileSync('git', args, { encoding: 'utf8' }).trim()
  const rev = git('rev-parse', 'HEAD')
  const dirty = git('status', '--porcelain', '--untracked-files=no') !== ''
  const machineVersion = '0.1.0+' + rev.slice(0, 12)
  const buildIdentity = {
    baseVersion: '0.1.0', displayVersion: dirty ? machineVersion + '-dirty' : machineVersion, machineVersion,
    sourceKind: 'local', rev, gitRev: rev, dirty, commitTs: Number(git('show', '-s', '--format=%ct', 'HEAD')), buildTs: Math.floor(Date.now() / 1000),
  }
  return {
    name: 'fractal-build-identity',
    resolveId(id) {
      if (id === 'virtual:build-identity') return '\0fractal-build-identity'
    },
    load(id) {
      if (id !== '\0fractal-build-identity') return
      return 'export const buildIdentity=' + JSON.stringify(buildIdentity) + ';export const deploymentId=undefined;'
    },
    generateBundle() {
      // Read by server/host.mts; the host serves this identity itself.
      const { gitRev: _, ...served } = buildIdentity
      this.emitFile({ type: 'asset', fileName: 'build-identity.json', source: JSON.stringify(served) + '\n' })
    },
  }
}
