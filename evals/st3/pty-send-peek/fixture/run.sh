#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
subject="pty/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/psp"
for _ in $(seq 1 100); do
  st3 terminals peek "$subject" >pre.txt 2>/dev/null && grep -Fq READY pre.txt && break
  sleep 0.05
done
grep -Fq READY pre.txt
! grep -Fq ACK:PSP-9f3a7c2e pre.txt
st3 terminals send "$subject" PSP-9f3a7c2e >/dev/null
for _ in $(seq 1 100); do
  st3 terminals peek "$subject" >post.txt
  grep -Fq ACK:PSP-9f3a7c2e post.txt && exit 0
  sleep 0.05
done
exit 1
