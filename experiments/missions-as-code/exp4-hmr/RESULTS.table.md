| run stopped while | change | verdict | the run then | ops it opened after the change |
|---|---|---|---|---|
| building | v2-text | **compatible**<br>the new version replays the history exactly | ran under v2-text: shipped, 3 rounds | review, round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| building | v2-insert | **compatible**<br>the new version replays the history exactly | ran under v2-insert: shipped, 3 rounds | review, security-scan, round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| building | v2-patched | **compatible**<br>the new version replays the history exactly | ran under v2-patched: shipped, 3 rounds | review, (patched/security-scan), security-scan, round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| building | v2-remove | **incompatible**<br>event 2, op docs: the history scheduled an op this program never asked for here | ran under v1: shipped, 3 rounds | review, round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| building | v2-rounds | **compatible**<br>the new version replays the history exactly | ran under v2-rounds: shipped, 3 rounds | review, round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| in review | v2-text | **compatible**<br>the new version replays the history exactly | ran under v2-text: shipped, 3 rounds | round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| in review | v2-insert | **compatible**<br>the new version replays the history exactly | ran under v2-insert: shipped, 3 rounds | security-scan, round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| in review | v2-patched | **compatible**<br>the new version replays the history exactly | ran under v2-patched: shipped, 3 rounds | (patched/security-scan), security-scan, round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| in review | v2-remove | **incompatible**<br>event 2, op docs: the history scheduled an op this program never asked for here | ran under v1: shipped, 3 rounds | round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| in review | v2-rounds | **compatible**<br>the new version replays the history exactly | ran under v2-rounds: shipped, 3 rounds | round-1/ci, round-1/fix, round-2/ci, round-2/fix, round-3/ci |
| fixing round 1 | v2-text | **open-op-changed**<br>still waiting on round-1/fix, started under the old definition | ran under v1: shipped, 3 rounds | round-2/ci, round-2/fix, round-3/ci |
| fixing round 1 | v2-insert | **incompatible**<br>event 9, op round-1/ci: the history scheduled an op this program never asked for here | ran under v1: shipped, 3 rounds | round-2/ci, round-2/fix, round-3/ci |
| fixing round 1 | v2-patched | **compatible**<br>the new version replays the history exactly | ran under v2-patched: shipped, 3 rounds | round-2/ci, round-2/fix, round-3/ci |
| fixing round 1 | v2-remove | **incompatible**<br>event 2, op docs: the history scheduled an op this program never asked for here | ran under v1: shipped, 3 rounds | round-2/ci, round-2/fix, round-3/ci |
| fixing round 1 | v2-rounds | **compatible**<br>the new version replays the history exactly | ran under v2-rounds: shipped, 3 rounds | round-2/ci, round-2/fix, round-3/ci |
| last CI round | v2-text | **compatible-with-changes**<br>finished ops defined differently now: round-1/fix, round-2/fix | ran under v2-text: shipped, 3 rounds | none |
| last CI round | v2-insert | **incompatible**<br>event 9, op round-1/ci: the history scheduled an op this program never asked for here | ran under v1: shipped, 3 rounds | none |
| last CI round | v2-patched | **compatible**<br>the new version replays the history exactly | ran under v2-patched: shipped, 3 rounds | none |
| last CI round | v2-remove | **incompatible**<br>event 2, op docs: the history scheduled an op this program never asked for here | ran under v1: shipped, 3 rounds | none |
| last CI round | v2-rounds | **compatible**<br>the new version replays the history exactly | ran under v2-rounds: shipped, 3 rounds | none |

- v1: sha256 a0b7960588aea312…
- v2-text: sha256 37414b4fb0a6b115…
- v2-insert: sha256 816da071bfd00230…
- v2-patched: sha256 719a2d0e9ac31a0c…
- v2-remove: sha256 d8f536293569298c…
- v2-rounds: sha256 40f4778d88b0c4e5…
