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

- `AttentionTray/V4`: the shipped design, the native round (n1–n4): a Control Center module with
  tiles, one-line inbox rows and pages drilled into within the panel. `Overview` lists the round's
  decisions, the K decisions it keeps, the Control Center benchmark and V3's heights next to V4's;
  `OpenVariants` the open questions with each option rendered (answer placement D1/D2, panel width
  W1 320/W2 336/W3 360 pt, tiles T1 capsules/T2 Control Center grid; shipped D1 · W3 · T1);
  `TopLevel`, `AskPages` and `TilePages` every scenario.
- `AttentionTray/Panel`: the shipped design per scenario, plus `AllStates` (light and dark).
- `AttentionTray/MenuBar`: the status item per state, plus `AllStates` (light and dark).
- `AttentionTray/V3`: history. `Locked` lists Johannes' picks (L1 A3 H3 S2 M3) and K1–K13 as #9344
  shipped them, with the frozen V3 renders; `LightAndDark` puts the busy inbox side by side. Its
  views are gone.
- `AttentionTray/V2`: history. `Exploration` shows the V2 options per dimension (L, A, H, S, M) as
  frozen renders, the pick marked; the variant code that drew them is gone.
- `AttentionTray/Guidelines`: the HIG/WWDC sources the design follows, first-party benchmarks and a
  pass/partial/fail checklist for the shipped design.

## Renders

`renders/` holds four sets, each with its manifest:

| Set           | Files                                                                              | Committed | Regenerate (on a Mac)                                                                                              |
| ------------- | ---------------------------------------------------------------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------ |
| Fallback      | `<scenario>-<appearance>.png`, `menubar-<state>-<appearance>.png`, `manifest.json` | yes       | `devenv tasks run attention-tray:gallery`                                                                          |
| Real captures | `capture/*.png`, `capture.json`                                                    | when run  | `devenv tasks run attention-tray:gallery-capture-vm` (headless macOS VM; `capture.json` records the guest's macOS) |
| V2 history    | `v2/*.png`, `v2/v2.json`                                                           | yes       | never: frozen from the V2 variant code                                                                             |
| V3 history    | `v3/*.png`, `v3/v3.json`                                                           | yes       | never: frozen from the V3 views (#9344)                                                                            |

On this Mac's own screen instead (unattended Mac only):
`ATTENTION_TRAY_CAPTURE_CONSENT=1 devenv tasks run attention-tray:gallery-capture`.

Real captures are committed when a consented capture run produces them; without them every story
shows the flat fallback. A render missing from a manifest or from disk shows a labelled placeholder
instead of failing the build.

## Run

```sh
pnpm exec storybook dev -p 6023   # or: devenv up → storybook-flakes-attention-tray-design-6023
env -u NODE_PRESERVE_SYMLINKS pnpm exec storybook build
```

Repo build (serialized with the other Storybooks, published to the Netlify preview):
`devenv tasks run storybook:build:flakes-attention-tray-design --mode single`.
