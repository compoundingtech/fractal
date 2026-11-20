import { workflowConditionConjuncts } from '../../repos/effect-utils/packages/@overeng/genie/src/runtime/mod.ts'

/** Drafts allocate no runners, including metadata and always-running collectors.
 * Keep each existing status predicate: failed dependencies still reach their gate.
 */
export const readyPullRequestTypes = ['opened', 'synchronize', 'reopened', 'ready_for_review', 'converted_to_draft'] as const

export const readyPRJobs = <T extends { jobs: Record<string, any> }>(workflow: T): T => ({
  ...workflow,
  jobs: Object.fromEntries(Object.entries(workflow.jobs).map(([name, job]) => {
    const predicate = job.if == null ? 'success()' : String(job.if).trim().replace(/^\$\{\{\s*|\s*\}\}$/g, '')
    return [name, { ...job, if: `\${{ (github.event_name != 'pull_request' || github.event.pull_request.draft == false) && (${predicate}) }}` }]
  })),
}) as T

/** Job admission is defense in depth, not a boundary against fork-controlled YAML.
 * Repository approval for all external contributors is the security boundary.
 * Runner-group settings stay unchanged; do not add a main-ref allowlist.
 */
export const withoutForkPRJobs = <T extends { jobs: Record<string, any> }>(workflow: T): T => ({
  ...workflow,
  jobs: Object.fromEntries(Object.entries(workflow.jobs).map(([name, job]) => {
    const predicate = job.if == null ? 'success()' : String(job.if).trim().replace(/^\$\{\{\s*|\s*\}\}$/g, '')
    // Keep positive main/event conjuncts visible to the generator's cache-publisher policy.
    // A disjunction stays grouped; splitting it would change admission semantics.
    const conjuncts = workflowConditionConjuncts(predicate) ?? [predicate]
    const original = conjuncts.map(part => `(${part})`).join(' && ')
    const sameRepository = "(github.event_name != 'pull_request' && github.event_name != 'pull_request_target') || github.event.pull_request.head.repo.full_name == github.repository"
    const sameOrigin = "github.event_name != 'workflow_run' || github.event.workflow_run.head_repository.full_name == github.repository"
    return [name, { ...job, if: `\${{ (${sameRepository}) && (${sameOrigin}) && ${original} }}` }]
  })),
}) as T
