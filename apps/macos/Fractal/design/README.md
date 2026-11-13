# Attention tray design Storybook

Storybook over real SwiftUI renders of the Attention tray. Nothing here re-creates the UI in HTML:
the stories frame PNGs that the `TrayGallery` executable produces from fixture adapters
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

- `AttentionTray/Panel`: the shipped design per scenario, plus `AllStates` (light and dark).
- `AttentionTray/MenuBar`: the status item per variant (M1–M3) and state.
- `AttentionTray/V2`: `Exploration` (the locked decisions K1–K12, then pick L/A/H/S/M by ID and a
  scenario; a dot marks the shipped option, ★ the HIG review's pick) and `Current`.
- `AttentionTray/Guidelines`: the HIG/WWDC sources the design follows, first-party benchmarks and a
  pass/partial/fail checklist for the shipped design.

## Renders

`renders/` holds three generated sets, each with its manifest:

| Set                      | Files                                                           | Committed | Regenerate (on a Mac)                                                                                    |
| ------------------------ | --------------------------------------------------------------- | --------- | -------------------------------------------------------------------------------------------------------- |
| Fallback, shipped design | `<scenario>-<appearance>.png`, `menubar-*.png`, `manifest.json` | yes       | `devenv tasks run attention-tray:gallery`                                                                |
| Real captures            | `capture/*.png`, `capture.json`                                 | when run  | `ATTENTION_TRAY_CAPTURE_CONSENT=1 devenv tasks run attention-tray:gallery-capture` (unattended Mac only) |
| Exploration              | `explore/*.png`, `explore.json`                                 | no        | `devenv tasks run attention-tray:gallery-explore`                                                        |

Real captures are committed when a consented capture run produces them; without them every story
shows the flat fallback. A render missing from a manifest or from disk shows a labelled placeholder instead of failing the
build; without `explore/` the Exploration story says how to generate it.

## Run

```sh
pnpm exec storybook dev -p 6023   # or: devenv up → storybook-flakes-attention-tray-design-6023
env -u NODE_PRESERVE_SYMLINKS pnpm exec storybook build
```

Repo build (serialized with the other Storybooks, published to the Netlify preview):
`devenv tasks run storybook:build:flakes-attention-tray-design --mode single`.
