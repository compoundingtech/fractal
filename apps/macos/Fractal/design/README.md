# Attention tray design Storybook

Storybook over real SwiftUI renders of the Attention tray panel. Nothing here re-creates the UI in
HTML: the stories frame PNGs that the `TrayGallery` executable produces from fixture adapters by
rendering the actual panel views into an `NSHostingView` and capturing it with `cacheDisplay`.

That capture path draws the view hierarchy only. Window-server effects (vibrancy, materials,
Liquid Glass, the menu-bar attachment) are not captured, so translucent surfaces appear flat. Judge
those on a running tray, not here.

## Renders

`renders/` holds `<scenario>-<appearance>.png` (@2x, `light`/`dark`) and `manifest.json` (generator,
command, tray source revision, scale, scenario titles/descriptions, per-render pixel sizes). Both are
generated; regenerate on a Mac:

```sh
devenv tasks run attention-tray:gallery
```

A render missing from the manifest or from disk shows a labelled placeholder instead of failing the
build.

## Run

```sh
pnpm exec storybook dev -p 6023   # or: devenv up → storybook-flakes-attention-tray-design-6023
env -u NODE_PRESERVE_SYMLINKS pnpm exec storybook build
```

Repo build (serialized with the other Storybooks, published to the Netlify preview):
`devenv tasks run storybook:build:flakes-attention-tray-design --mode single`.
