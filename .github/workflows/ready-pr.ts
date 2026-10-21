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
