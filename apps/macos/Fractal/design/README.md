# Fractal design Storybook

Storybook over real SwiftUI renders of Fractal. Nothing here re-creates the UI in HTML:
the stories frame PNGs that the `FractalMacGallery` executable produces from fixture adapters
(`../README.md`, Design gallery).

Every image is labelled with its source:

- **Real capture**: the actual `MenuBarExtra` window and status item captured by ScreenCaptureKit,
  so Liquid Glass, vibrancy and the menu-bar attachment are real.
- **Fallback · no materials**: the panel views drawn by `cacheDisplay`; layout, type and symbols are
  exact, translucent surfaces are flat and controls draw in their inactive look (the default
  button is not tinted). The status item's fallback is its template image tinted on a flat bar.

Stories follow the viewer's macOS appearance (manager and canvas switch live); the toolbar's
Appearance menu forces Light or Dark.

## Stories

- `Fractal/V4`: the shipped design, the native round (n1–n4): a Control Center module with
  tiles, one-line inbox rows and pages drilled into within the panel. `Overview` lists the round's
  decisions, the K decisions it keeps, the Control Center benchmark and V3's heights next to V4's;
  `SelectedDesign` renders D2 · W3 · T1: ask detail has a header, scrolling request and conversation,
  then fixed answer controls; the panel is 360 pt wide with compact status capsules. Answer
  submission and required-text validation are unchanged. The capped inbox and More row keep
  the signal grid below it in view. Availability history retains its H1 week strips/H2 by hour/H3
  last 24 hours picker (H1 default). `TopLevel`, `AskPages` and `TilePages` cover every scenario.
- `Fractal/Panel`: the shipped design per scenario, plus `AllStates` (light and dark).
- `Fractal/MenuBar`: the status item per state, plus `AllStates` (light and dark).
- `Fractal/V3`: history. `Locked` lists the picks (L1 A3 H3 S2 M3) as #9344
  shipped them, with the frozen V3 renders; `LightAndDark` puts the busy inbox side by side. Its
  views are gone.
- `Fractal/V2`: history. `Exploration` shows the V2 options per dimension (L, A, H, S, M) as
  frozen renders, the pick marked; the variant code that drew them is gone.
- `Fractal/Guidelines`: the HIG/WWDC sources the design follows, first-party benchmarks and a
  pass/partial/fail checklist for the shipped design.

## Renders

`renders/` holds four sets, each with its manifest. Existing PNGs and manifests retain their
historical provenance; they are not rewritten as part of the package move. Rendering is manual.
Run these commands from the repository root on a Mac:

| Set | Files | Regenerate |
| --- | --- | --- |
| Fallback | `<scenario>-<appearance>.png`, `menubar-<state>-<appearance>.png`, `manifest.json` | `swift run --package-path apps/macos/Fractal FractalMacGallery apps/macos/Fractal/design/renders` |
| Real captures | `capture/*.png`, `capture.json` | `bash apps/macos/Fractal/scripts/capture-vm.sh` (headless macOS VM; first provision with the same script's `provision` command) |
| V2 history | `v2/*.png`, `v2/v2.json` | Never: frozen from the V2 variant code |
| V3 history | `v3/*.png`, `v3/v3.json` | Never: frozen from the V3 views |

On an unattended Mac's own screen instead, only after obtaining consent:

```sh
swift run --package-path apps/macos/Fractal FractalMacGallery apps/macos/Fractal/design/renders --capture --consent
```

Real captures are committed when a consented capture run produces them; without them every story
shows the flat fallback. A render missing from a manifest or from disk shows a labelled placeholder
instead of failing the build.

## Run

From the repository root, after the normal root `pnpm install`:

```sh
pnpm --filter @compoundingtech/fractal-mac-design storybook
pnpm --filter @compoundingtech/fractal-mac-design build-storybook
```

The package follows Fractal's existing Storybook script convention. Development binds to
`127.0.0.1:6023`; the build writes `apps/macos/Fractal/design/storybook-static/`.

The gallery belongs to the root workspace, like `apps/fractal-web`, `apps/ios` and
`packages/fractal-ui`. Its shared `@overeng/utils-storybook` dependency is materialized by the
root's source-input staging and frozen lockfile. A separate install root would duplicate that
dependency-materialization contract and lock ownership; do not install dependencies in this directory.
