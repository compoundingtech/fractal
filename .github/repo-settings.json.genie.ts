import { githubRepoSettings, githubRuleset } from '../repos/effect-utils/genie/external.ts'

// Landing goes through GitHub's merge queue. This file declares the live ruleset: fractal-web
// joined the required checks after both PR and merge-group runs had emitted it successfully.
// Every required job must run on merge_group, or the queue never receives its checks and merges freeze.
export default githubRepoSettings({
  repository: { allow_auto_merge: true, delete_branch_on_merge: true },
  rulesets: [githubRuleset({
    name: 'main',
    target: 'branch',
    enforcement: 'active',
    bypass_actors: [],
    conditions: { ref_name: { include: ['~DEFAULT_BRANCH'], exclude: [] } },
    rules: [
      { type: 'deletion' },
      { type: 'non_fast_forward' },
      {
        type: 'pull_request',
        parameters: {
          required_approving_review_count: 0,
          dismiss_stale_reviews_on_push: false,
          required_reviewers: [],
          require_code_owner_review: false,
          dismissal_restriction: { enabled: false, allowed_actors: [] },
          require_last_push_approval: false,
          required_review_thread_resolution: false,
          require_extra_approval_for_unattributed_changes: true,
          allowed_merge_methods: ['merge', 'squash', 'rebase'],
        },
      },
      {
        type: 'required_status_checks',
        parameters: {
          // The merge queue tests every entry on top of the current main and the entries ahead of it,
          // so a pull request no longer has to be rebased onto the latest main before it can be queued.
          strict_required_status_checks_policy: false,
          do_not_enforce_on_create: false,
          required_status_checks: [
            { context: 'linux-gate', integration_id: 15368 },
            { context: 'genie-freshness', integration_id: 15368 },
            { context: 'typescript-client', integration_id: 15368 },
            { context: 'fractal-web', integration_id: 15368 },
          ],
        },
      },
      {
        type: 'merge_queue',
        parameters: {
          // The repository merges with merge commits today (the st train did).
          merge_method: 'MERGE',
          // The live queue's tuning (ruleset 20563764, 2026-10-22): a group merges when its head is green.
          grouping_strategy: 'HEADGREEN',
          // Start three groups with Namespace overflow; retain shared capacity for PRs.
          max_entries_to_build: 3,
          max_entries_to_merge: 6,
          min_entries_to_merge: 1,
          min_entries_to_merge_wait_minutes: 1,
          // A finished stage must not expire merely because its dependent gate was waiting for a runner:
          // two hours, as the live rule has it.
          check_response_timeout_minutes: 120,
        },
      },
    ],
  }), githubRuleset({
    // Evidence records are tags (docs/st3/gate-evidence.md, "Records as tags"): once written, a record is never moved, which
    // catches an accident that would rewrite one. Creation stays open: producers create tags. Deletion is open too, so
    // scripts/evidence-gc deletes dead trees with its host's own login and no window: we do not guard against an adversary who
    // deletes a newer failure yet (Nathan, 2026-10-26); the signed evidence log reports a record that disappears without GC's line.
    name: 'evidence-tags',
    target: 'tag',
    enforcement: 'active',
    bypass_actors: [],
    conditions: { ref_name: { include: ['refs/tags/evidence/**'], exclude: [] } },
    rules: [{ type: 'update' }, { type: 'non_fast_forward' }],
  })],
})
