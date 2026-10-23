import { readFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'

export function runnerCpu(cpuinfo = '') {
  const modelName = /^model name[\t ]*:[\t ]*(.*)$/m.exec(cpuinfo)?.[1].trim() ?? ''
  let cpuClass = 'unknown'
  if (/\bAMD EPYC 7763\b/i.test(modelName)) cpuClass = 'epyc-7763'
  else if (/\bAMD EPYC 9V74\b/i.test(modelName)) cpuClass = 'epyc-9v74'
  else if (/\bIntel\b.*\bXeon\b.*\b8573C\b/i.test(modelName)) cpuClass = 'xeon-8573c'
  return { modelName, cpuClass }
}

export async function readRunnerCpu(path = '/proc/cpuinfo') {
  try { return runnerCpu(await readFile(path, 'utf8')) }
  catch (error) {
    if (error.code !== 'ENOENT') throw error
    return runnerCpu()
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  console.log(JSON.stringify(await readRunnerCpu()))
}
