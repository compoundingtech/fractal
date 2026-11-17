export HOME=$TMPDIR/home
mkdir -p "$HOME" source
cp -r "$manifests"/. source
chmod -R u+w source
cd source
# Stage the workspace's file: packages using only the projected effect-utils closure.
mkdir repos
ln -s "$effectUtilsSources" repos/effect-utils
node -e '
  const { readFileSync } = require("node:fs");
  const { spawnSync } = require("node:child_process");
  const paths = JSON.parse(readFileSync("pnpm-source-inputs.json", "utf8")).sourceInputPaths;
  const result = spawnSync(process.execPath, [
    process.argv[1], "publish", process.cwd(), ".devenv/pnpm-source-inputs", ...paths,
  ], { stdio: "inherit" });
  process.exit(result.status ?? 1);
' "$effectUtilsSources/nix/devenv-modules/tasks/shared/stage-pnpm-source-inputs.mjs"
pnpm install --frozen-lockfile --store-dir "$TMPDIR/store"
# Both name the install directory and the time; neither is needed to run the workspace.
find . -name node_modules -prune -exec rm -f '{}/.modules.yaml' '{}/.pnpm-workspace-state-v1.json' ';'
# pacquet can leave atomic-write staging files with PID/time/counter names.
find . -type f -name 'package.json_pacquet-stage_*' -delete
mkdir "$out"
find . -name node_modules -prune -print | while IFS= read -r directory; do
  mkdir -p "$out/$(dirname "$directory")"
  cp -a "$directory" "$out/$directory"
done
