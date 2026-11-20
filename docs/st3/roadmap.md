# Fractal product roadmap

This file records accepted future work that is not built yet. Active fleet work belongs in Fractal missions.

## Agent continuity

- [ ] Suspend a quiet seat that holds no work after a member-configured idle time, and resume it
  on its exact session when a wake-class message arrives, as specified in [idle residency](idle-residency.md).
- [ ] Suspend an idle seat automatically at a long graph wait or human gate, and resume it when the
  graph makes its work ready again. Manual [suspend and resume](suspend.md) exist today. This
  needs the parked-work lease contract in [idle residency DQ4](idle-residency.md#design-questions).

## User interfaces

- [ ] Hold a product design session for Fractal and the Fractal mobile application.

## Runtime containment

- [ ] Add optional CPU, memory, process, and disk limits for each mission-owned runtime.
- [ ] Evaluate the macOS sandbox and resource-limit surfaces before selecting an enforcement contract.

## Harness accounts

- [ ] Extend [model accounts](accounts.md) to OMP seats.
- [ ] Offer round-robin selection within a pool, without moving active conversations.
- [ ] Evaluate [Comradex](https://github.com/nicosuave/comradex) as a reference for Codex account routing.
