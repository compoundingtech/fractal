# Demo scripts as Effect programs

`release-day.kdl` and `an-alert-needs-an-answer.kdl` are snapshots of the demo-scripts mission's
example demos, taken from its working tree on 2026-10-13 before it merged. Its format may change;
the copies here are what the programs were checked against. Every name in them is that mission's
invented name.

```sh
node --test exp6-port/demo/*.test.ts                                   # programs == files
node exp6-port/demo/compile.ts exp6-port/demo/release-day.ts           # print the beats block
node exp6-port/demo/compile.ts exp6-port/demo/an-alert-needs-an-answer.ts now
```

See section 4 of [../COMPARISON.md](../COMPARISON.md).
