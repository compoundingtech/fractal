# Codex minimal access

This paid black-box eval declares a Codex seat with only a model, in a daemon whose
`access_mode` is `auto`, so the seat starts in the managed `st-managed` profile with no
permission arguments. The seat claims one step and must edit its workspace, commit and push a
branch, reach the network (git over https and gh), install a package, search the web, and be
refused a write to the home directory. Held-out gates check each result; the runner
(`scripts/st3-codex-minimal-eval/run`) also records the provider's argv, which must carry the
`st-managed` profile and no bypass flag, and whether the seat's transcript holds a web search call.
Commits and package installs are refused by the sandbox and succeed only through the escalation
that `on-request` approval with `auto_review` answers; the write outside the workspace must not.
