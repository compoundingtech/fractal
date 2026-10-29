---
title: "From intent to mission"
description: "Draft, preview, and approve a mission from a plain-language request."
order: 9
---

# From intent to mission

**Launch** turns a plain-language request into a reviewed mission. A planner drafts the work; you see what would run before approving it.

## The flow

1. Write what should happen and why in a Markdown request.
2. Start a planning conversation.
3. Let the planner draft a description and mission KDL.
4. Preview the exact candidate, including dependencies, changes, warnings, and blockers.
5. Give feedback or compare variants.
6. Approve publication, or deliberately approve and launch.

```sh
fractal launch start --id release-mission request.md --workspace ./project --as person/ada
fractal launch ls
```

Use the returned session ID with `fractal launch show SESSION` and `fractal launch preview SESSION`. `fractal launch revise SESSION feedback.md --as person/ada` requests a new candidate. Consult each command's `--help` for approval options and the preview token it requires.

Approval is tied to the current preview. If the candidate changes after review, review it again rather than approving unseen work. Publication alone does not start a run; approve-and-launch explicitly requests both.

## Durable planning

The request, candidates, feedback, preview, and decision are graph state, not a temporary chat transcript. Launch can also propose a new generation for an existing run without silently replacing its current plan.

Approvals appear in [Now](attention.md). For exact command options, see [CLI reference](../reference/cli.md).
