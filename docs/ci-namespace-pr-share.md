# Namespace PR capacity

Issue [#2918](https://github.com/compoundingtech/smalltalk/issues/2918) requests a fairer share of the shared Namespace workspace. The reported 24 running and 137 queued jobs are the reporter's snapshot, not a measurement independently verified by this change.

Use a dedicated runner profile for this repository's unqueued PR jobs, with **15 concurrent runners**. This limits simultaneous runners rather than sampling a count before dispatch. [Namespace's profile limit](https://namespace.so/docs/solutions/github-actions/runner-controls/concurrent-runners) keeps excess jobs queued until profile capacity becomes available.

The workflow keeps its existing choices of idle owned general runners. It sends the remaining Workspace Linux jobs to the PR profile, including isolation-vm and optional perf-cost. Required jobs retain priority 2; optional cost retains priority 3. The profile does not add owned capacity, loan reserved workers, alter test arguments or skip any checks.

Merge-group jobs never select this profile. An exact PR head already in the merge queue at the hosted selection step also retains existing routing. Main, schedule, manual and release events keep their existing paths. GitHub-hosted lightweight checks stay on GitHub.

## Provision and activate

The repository variable `CI_PR_NAMESPACE_PROFILE` defaults to empty. Merging the source does **not** create a profile, set a limit, or activate a cap.

1. The Namespace workspace administrator creates a profile dedicated to this repository, for example `smalltalk-pr-linux-x86-64`. In Advanced Settings, set **Concurrent runners per profile = 15**. Record its actual identity, access scope, image, machine shape, limit and existing settings. Do not change the shared workspace quota or a profile used by another repository.
2. Use Ubuntu 24.04 / AMD64 / 8 vCPU / 16 GiB with the tools, permissions, nested virtualization and cache setup the current Linux jobs require. In particular, the existing isolation-vm KVM probe and both VM tests must actually pass on this profile; the old profile's result does not qualify a new profile. Preserve fork isolation and the repository's no-Docker rule. This source contains no image or container job change.
3. Qualify the source through normal exact-head review and hosted checks. Record an explicit profile qualification before treating its tools/KVM as proven. Keep the variable empty while that qualification is missing. A separately authorized manual qualification can target the new profile; this PR does not dispatch one.
4. Confirm #2898 has landed and stopped new automatic Performance/Nix PR demand. Existing jobs keep their frozen labels. This hook covers Workspace Linux jobs; it does not retrofit old runs or remove the separate perf-cost signal.
5. Capture queued/running **jobs**, their event, profile/shape, assigned runner and wait ages immediately before activation. Set `CI_PR_NAMESPACE_PROFILE` to the provisioned name only, without `namespace-profile-` or label controls. The script validates the name and emits inline priority and run-ID controls. No new secret is required: its queue lookup uses the job's read-only repository token, separate from the org runner-status token.
6. After activation, verify real PR assignments use the profile and that its active runners remain at or below 15. Record the same job counts and PR/merge-group start-wait distribution after one hour, with unstarted jobs kept as pending observations. Verify merge-group assignments did not use the PR profile. Do not infer a speedup, a reserved share for another repo or a whole-workspace cap from this routing change alone.

## What a waiting job does

A job beyond the provider's limit remains queued with the original run, head and attempt. It starts when profile capacity is freed; it needs no rerun, branch push, placeholder runner label, sleeping job or custom controller. Run-ID affinity remains inline in the label. Namespace documents [scheduled-order assignment with run affinity and numeric job priority](https://namespace.so/docs/solutions/github-actions/runner-controls/job-ordering); required PR work precedes optional cost. Parallel jobs can still finish in a different order.

The merge-queue read is advisory and fails open to the existing routing on a timeout, API error, malformed data, missing read token or changed head. There is no live capacity-count dependency. This deliberately preserves CI availability, so the policy is not an unconditional repository-wide upper bound during lookup failures or while old uncapped runs are active. Summaries distinguish selected-profile and unchanged-routing decisions.

The queue exemption is a snapshot at selection. GitHub freezes a job's requested labels; entering the merge queue later does not rewrite labels of an existing PR run. Required checks normally finish before queue admission; the separate merge-group run is always exempt. There is no promise of retroactive migration or preemption.

## Roll back

Clear `CI_PR_NAMESPACE_PROFILE` to restore the existing routing for newly selected jobs. Already queued jobs retain the profile, so keep the profile available until they finish. Do not delete it or cancel required jobs as rollback. Restore only the owned variable value; do not overwrite another writer's changes. No profile, variable or job is changed by this source PR.

## Reduce refresh demand

Both published merge-main mission definitions now explicitly say to stop without pushing on MERGEABLE heads, regardless of behind count; refresh only CONFLICTING/DIRTY heads, and conflicting contract heads only when next in line. At the 2026-10-15 source read their revisions were `69ff661118812de1c49d2ca9bfdf05f7b044cbfe4ef11783d1b950189127ec17` and `e33a9fadfb4d6910a6343d0b5af9b4bb038dd6ac36713f5d2ce3589ae031acd4`.

Factory reports resolver-1877 follows the later instruction. Its completed resolver step's recorded definition `822aeedc4ce9439f272805545e2a584340a079964df502ac376c4d8d93c97f6f` does not contain it. Curator confirmed there is no later resolver definition to cite and is asking Factory to add the rule before any restart. Resolver coverage remains unverified; no new refresh or run is requested.
