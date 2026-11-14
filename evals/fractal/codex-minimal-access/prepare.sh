#!/usr/bin/env bash
# The seat's workspace must already be a repository when its provider launches: Codex keeps a
# workspace's own .git read-only, and the managed profile grants the git directory it finds.
set -euo pipefail
git init -q repo
git -C repo -c user.email=eval@example.test -c user.name=Eval commit -q --allow-empty -m base
git init -q --bare repo/remote.git
git -C repo remote add origin "$PWD/repo/remote.git"
git -C repo config user.email eval@example.test
git -C repo config user.name Eval
