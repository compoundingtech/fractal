# Workspace primitives

This module holds unstyled, headless building blocks for a split workspace: panes, splits, tabs, drag and drop, a per-pane side panel, and a command palette. They work the way React Aria Components does: state is shown through render props and `data-*` attributes, and `className` and `style` accept values or functions, so all styling is the host's. The host also supplies the data, the entity descriptions and each kind's content.

The module imports only `effect`, `react`, `react-dom`, `react-aria` and `react-stately`, plus its own files. `boundary.unit.test.ts` enforces that.

Run its tests with `pnpm -C packages/fractal-ui test:workspace`.

| Import | Contents |
|---|---|
| `@fractal/ui/workspace` | The pure model (`transition`), selectors, geometry, drop resolution, the aside resolver, the body format and the codec |
| `@fractal/ui/workspace/store` | The Effect store: persistence, following other windows, and the `SpaceSync` seam |
| `@fractal/ui/workspace/react` | `WorkspaceProvider`, `Workspace`, `SplitView`, `Divider`, `Pane`, `TabStrip`, `Tab`, `PaneBody`, `PaneContent`, `PaneAside`, drag and drop, and keys |
| `@fractal/ui/workspace/palette` | The palette core (fuzzy ranking, prefixes, targets) plus `usePalette` and `useRecent` |

## Model

A workspace is a list of named **spaces**. Each space has a layout tree: **splits** (`axis: 'row' | 'column'`, with sizes as shares that sum to 1) whose leaves are **panes** of ordered **tabs**. A tab holds the host's opaque `ref`.

What a space shares and what stays on the client:

- **Shared:** a space's body, `{ name, layout }` without ids (`spaceBody`). This is what other windows and an external store exchange.
- **Client-local:** everything else stays on this client:
  - tab, pane and split ids;
  - each pane's shown tab;
  - focus and zoom;
  - recency and recently closed tabs;
  - per-tab views (the host's schema);
  - per-pane aside state.

`transition(state, command, config)` is pure and total: a command that does not apply returns the same reference. These invariants hold:

- A space has at most one tab per `config.identity(ref)`. Opening an entity that is already open focuses its existing tab, wherever that tab is.
- Splits made here are binary.
- A pane emptied by a local close or move collapses, and its sibling takes the space. The last pane stays, empty.
- `Receive` keeps empty panes exactly as received.
- `Receive` adopts another writer's body wholesale. Local ids, the shown tabs and the focus are kept by entity identity and position. Unknown refs are always kept.
- `FocusDirection` finds the nearest pane in a direction from the sizes alone. At an edge it does nothing, and `neighbour()` returns `undefined`, so the host can, for example, focus a sidebar.

## Store and sync

`makeWorkspaceStore` / `createWorkspaceStore` persist through a `KeyValueStore` with a debounce. They follow other windows through `WorkspacePersistence.changed`, and they decode older documents through host `legacy` decoders.

Each `SpaceSync` is an external store that keeps spaces as whole bodies. The store's behaviour toward it:

- **Publishing.** A space is published only when its structure changes. View-only changes never publish.
- **Write metadata.** Each write carries `base` (the space's last revision) and a `key` that stays the same across retries of the same body on the same base.
- **No echoes.** A body the store just sent or adopted is never sent back.
- **In-flight hold.** While a space's write is out, incoming bodies for that space are held. When the write settles, only the latest held body is applied.
- **Sizes.** With `syncsSizes: false`, bodies carry no sizes, and incoming bodies never reset the local ones.
- **Spaces appearing and disappearing.** A space that arrives is created. A removed one is retired, and its local view comes back if the space returns. Closing a space here calls `remove`.

## React

```tsx
<WorkspaceProvider store={store} describe={ref => descriptor} renderDragChip={entity => <Chip entity={entity} />}>
  <Workspace collapsed={isPhone} renderPane={pane => (
    <Pane key={pane.id} pane={pane} className={({ isFocused }) => …}>
      <TabStrip trailing={<PanelToggle />}>{tab => <Tab tab={tab}>{({ entity, isActive, close }) => …}</Tab>}</TabStrip>
      <PaneBody>
        <PaneContent renderDropOverlay={({ zone, placement }) => <div style={placement} className="drop" />}>
          {({ activeTab }) => hostSurface(activeTab)}
        </PaneContent>
        <PaneAside side="end" threshold={tab => px} defaultSize={320}>{({ activeTab }) => hostPanel(activeTab)}</PaneAside>
      </PaneBody>
    </Pane>
  )} />
</WorkspaceProvider>
```

- **Scrolling stays inside panes.**
  - Panes and their slots are `position: relative`, with `min-width: 0` and `min-height: 0`.
  - Panes are `overflow: clip`, so they cannot be scrolled programmatically.
  - The only scroll containers are `PaneContent` and the aside body (`overflow: auto`, `overscroll-behavior: contain`).
  - Nothing uses CSS `contain`, so `position: fixed` overlays still escape.
- **Tabs.**
  - `TabStrip` and `Tab` are a React Aria tab list: selection follows focus, and the arrows, Home and End move.
  - Delete or Backspace closes, and so does a middle click.
  - Alt+Shift+arrows reorder.
  - `PaneContent` is the shown tab's `tabpanel`.
- **Dividers.** `Divider` is a focusable separator built on `useMove`. Dragging and the arrows resize (Shift for larger steps), Home and End go to the extremes, and Enter or a double-click shares equally again.
- **Drag and drop** works on pointer events, not HTML5 drag.
  - Use `useEntityDrag({ ref })` to make anything draggable: a list row, a resource, a mention.
  - A tab strip slot inserts the tab there. A pane's centre opens it in that pane. A pane's edge band (25% by default) splits that side.
  - The chip that follows the pointer is live React, and the overlays are ordinary state.
  - Escape cancels.
  - Touch presses scroll unless `touch` is set.
- **The aside.**
  - `PaneAside` shows by default when the pane is at least `threshold` wide.
  - A person's toggle (`usePaneAside().toggle`) holds until the pane crosses into the other width band by more than `hysteresis` (24 px by default).
  - The TUI shows details beside a conversation from 90 columns, with width `clamp(w/3, 30, 44)`, and beside a terminal from 120 columns, with a fixed 36. Pick `threshold` per kind on that basis.
- **Keys.**
  - `useWorkspaceKeys` binds the defaults:
    - Mod+D splits right, and Mod+Shift+D splits down;
    - Alt+W closes the tab, and Alt+Shift+T reopens the last closed one;
    - Alt+arrows move the focus, Alt+O zooms, and Alt+[ and Alt+] cycle tabs;
    - Mod+K opens the palette everywhere; Ctrl+T also opens it, on a Mac only.
  - Alt+T (new tab), Alt+G (switch space) and the palette are host `handlers`.
  - `onEdge` takes focus moves past the layout's edge.
  - `ignore` lets keys through to a terminal.

## Palette

The palette core:

- `parseQuery` scopes a query to a section by its prefix character, such as `>` or `@`.
- `rank` orders results by section, then match, then recency. The empty and the typed list can each have their own section order (`emptyOrder`, `typedOrder`: the TUI's `RANK`) while Alt+digit keeps `sections`, and `leads` lifts a section whose row the query names as it begins, such as a command. It uses `fuzzyWords`, ported from the TUI: a substring or word-start match, with multi-word queries matching word by word.
- `targetFromEvent` maps Enter to here, Mod+Enter to a new tab, Alt+Enter to a split right and Alt+Shift+Enter to a split below.

`usePalette` adds the rest:

- async sources, each with its own `minQuery` and debounce;
- Alt+1 to Alt+9 to jump to a section;
- naming mode, for creating, renaming and duplicating;
- prop getters for the input, the list, the sections and the items.

`useRecent` keeps the last ids, newest first, in a host `RecentStore`.
