import * as React from 'react'
import { flatten, parseQuery, pushRecent, rank, sectionJump, targetFromEvent, type OpenTarget, type PaletteItem, type PaletteMode, type PaletteSection, type RankedGroup, type Ranker, type Leads } from './core.ts'

/**
 * Where palette items come from. A source can be synchronous (a list already in memory) or async (a
 * search), and can wait for a minimum query length, as full-text search does from three characters.
 */
export interface PaletteSource<A> {
  readonly id: string
  /** Characters the query needs before this source is asked. */
  readonly minQuery?: number | undefined
  readonly debounceMs?: number | undefined
  readonly items: (query: string, signal: AbortSignal) => readonly PaletteItem<A>[] | Promise<readonly PaletteItem<A>[]>
}

export interface UsePaletteOptions<A> {
  readonly sections: readonly PaletteSection[]
  readonly sources: readonly PaletteSource<A>[]
  readonly onSelect: (item: PaletteItem<A>, target: OpenTarget) => void
  /** A name typed in naming mode, for what the mode was opened for. */
  readonly onName?: ((mode: Extract<PaletteMode, { _tag: 'Naming' }>, name: string) => void) | undefined
  readonly onClose?: (() => void) | undefined
  readonly mode?: PaletteMode | undefined
  readonly onModeChange?: ((mode: PaletteMode) => void) | undefined
  /** Open targets this surface offers; a phone offers only here and tab. */
  readonly targets?: readonly OpenTarget[] | undefined
  readonly ranker?: Ranker | undefined
  /** Section ids in the order a typed query ranks them; `sections` stays the order of the empty list and of Alt+digit. */
  readonly typedOrder?: readonly string[] | undefined
  /** Section ids in the order the empty query lists them. */
  readonly emptyOrder?: readonly string[] | undefined
  readonly leads?: Leads<A> | undefined
  /** Ids most recent first: an item's recency is its place here. */
  readonly recent?: readonly string[] | undefined
  readonly 'aria-label'?: string | undefined
}

export interface PaletteState<A> {
  readonly query: string
  readonly setQuery: (query: string) => void
  readonly mode: PaletteMode
  readonly groups: readonly RankedGroup<A>[]
  readonly results: readonly PaletteItem<A>[]
  readonly selected: PaletteItem<A> | undefined
  /** Some source is still answering the current query. */
  readonly loading: boolean
  /** The section the query is scoped to, by prefix or by Alt+digit. */
  readonly section: string | undefined
  readonly getInputProps: () => React.InputHTMLAttributes<HTMLInputElement> & { readonly ref: React.RefObject<HTMLInputElement | null> }
  readonly getListProps: () => React.HTMLAttributes<HTMLElement>
  readonly getSectionProps: (section: PaletteSection) => React.HTMLAttributes<HTMLElement> & { readonly 'data-section': string }
  readonly getItemProps: (item: PaletteItem<A>) => React.HTMLAttributes<HTMLElement> & { readonly 'data-selected'?: string }
}

const useSourceItems = <A,>(sources: readonly PaletteSource<A>[], query: string): { readonly items: readonly PaletteItem<A>[]; readonly loading: boolean } => {
  const [answers, setAnswers] = React.useState<Readonly<Record<string, readonly PaletteItem<A>[]>>>({})
  const [pending, setPending] = React.useState<ReadonlySet<string>>(new Set())
  React.useEffect(() => {
    const controller = new AbortController()
    const timers: ReturnType<typeof setTimeout>[] = []
    for (const source of sources) {
      if (query.trim().length < (source.minQuery ?? 0)) {
        setAnswers(current => source.id in current ? Object.fromEntries(Object.entries(current).filter(([id]) => id !== source.id)) : current)
        continue
      }
      const ask = () => {
        const answer = source.items(query, controller.signal)
        if (!(answer instanceof Promise)) {
          setAnswers(current => ({ ...current, [source.id]: answer }))
          return
        }
        setPending(current => new Set([...current, source.id]))
        answer.then(
          items => { if (!controller.signal.aborted) setAnswers(current => ({ ...current, [source.id]: items })) },
          () => {},
        ).finally(() => {
          if (!controller.signal.aborted) setPending(current => new Set([...current].filter(id => id !== source.id)))
        })
      }
      if (source.debounceMs === undefined || source.debounceMs <= 0) ask()
      else timers.push(setTimeout(ask, source.debounceMs))
    }
    return () => {
      controller.abort()
      for (const timer of timers) clearTimeout(timer)
      setPending(new Set())
    }
  }, [sources, query])
  const items = React.useMemo(() => sources.flatMap(source => answers[source.id] ?? []), [sources, answers])
  return { items, loading: pending.size > 0 }
}

/**
 * A headless command palette: one input over sectioned, ranked results. An empty query lists each
 * section by recency; a query ranks by section, then match, then recency. A prefix character scopes
 * to a section, Alt+1 to Alt+9 jump to one (plain digits stay text). Enter opens here, Mod+Enter in a
 * tab, Alt+Enter in a split right, Alt+Shift+Enter below. Naming mode takes a name instead.
 */
export const usePalette = <A,>(options: UsePaletteOptions<A>): PaletteState<A> => {
  const { sections, sources, onSelect, onName, onClose, onModeChange, targets, ranker, recent, typedOrder, emptyOrder, leads } = options
  const [ownMode, setOwnMode] = React.useState<PaletteMode>({ _tag: 'Search' })
  const mode = options.mode ?? ownMode
  const setMode = (next: PaletteMode) => {
    setOwnMode(next)
    onModeChange?.(next)
  }
  const [query, setQueryState] = React.useState(mode._tag === 'Naming' ? mode.initial : '')
  const [jumped, setJumped] = React.useState<string | undefined>(undefined)
  const [selectedIndex, setSelectedIndex] = React.useState(0)
  const input = React.useRef<HTMLInputElement>(null)
  const listId = React.useId()
  const modeKey = mode._tag === 'Naming' ? `${mode.intent}:${mode.initial}` : 'search'
  React.useEffect(() => {
    setQueryState(mode._tag === 'Naming' ? mode.initial : '')
    setSelectedIndex(0)
    // Only a change of mode resets the text.
  }, [modeKey])
  const parsed = parseQuery(query, sections)
  const section = parsed.section ?? jumped
  const naming = mode._tag === 'Naming'
  const { items, loading } = useSourceItems(sources, naming ? '' : parsed.text)
  const order = React.useMemo(() => new Map((recent ?? []).map((id, index) => [id, (recent?.length ?? 0) - index] as const)), [recent])
  const groups = React.useMemo(() => naming ? [] : rank(
    items.map(item => order.has(item.id) ? { ...item, recency: Math.max(item.recency ?? 0, order.get(item.id)!) } : item),
    { query: parsed.text, sections, section, ranker, typedOrder, emptyOrder, leads },
  ), [naming, items, order, parsed.text, sections, section, ranker, typedOrder, emptyOrder, leads])
  const results = React.useMemo(() => flatten(groups), [groups])
  const selected = results[Math.min(selectedIndex, results.length - 1)]
  const setQuery = (next: string) => {
    setQueryState(next)
    setSelectedIndex(0)
  }
  const optionId = (item: PaletteItem<A>) => `${listId}-${item.id}`
  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    const jump = sectionJump(event)
    if (jump !== undefined && !naming) {
      event.preventDefault()
      const target = sections[jump]
      setJumped(target === undefined || target.id === jumped ? undefined : target.id)
      setSelectedIndex(0)
      return
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      if (results.length === 0) return
      const by = event.key === 'ArrowDown' ? 1 : -1
      setSelectedIndex(index => (Math.min(index, results.length - 1) + by + results.length) % results.length)
      return
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      if (naming) setMode({ _tag: 'Search' })
      else if (jumped !== undefined) setJumped(undefined)
      else onClose?.()
      return
    }
    const target = targetFromEvent(event, targets)
    if (target === undefined) return
    event.preventDefault()
    if (mode._tag === 'Naming') {
      const name = query.trim()
      if (name !== '') onName?.(mode, name)
      return
    }
    if (selected !== undefined) onSelect(selected, target)
  }
  return {
    query,
    setQuery,
    mode,
    groups,
    results,
    selected,
    loading,
    section,
    getInputProps: () => ({
      ref: input,
      value: query,
      onChange: event => setQuery(event.currentTarget.value),
      onKeyDown,
      role: 'combobox',
      'aria-expanded': true,
      'aria-controls': listId,
      'aria-autocomplete': 'list',
      'aria-label': options['aria-label'] ?? (naming ? 'Name' : 'Search'),
      ...(selected === undefined || naming ? {} : { 'aria-activedescendant': optionId(selected) }),
      autoComplete: 'off',
      spellCheck: false,
    }),
    getListProps: () => ({ id: listId, role: 'listbox', 'aria-label': options['aria-label'] ?? 'Results' }),
    getSectionProps: row => ({ role: 'group', 'aria-label': row.label, 'data-section': row.id }),
    getItemProps: item => ({
      id: optionId(item),
      role: 'option',
      'aria-selected': item === selected,
      ...(item === selected ? { 'data-selected': '' } : {}),
      // Keep the focus in the input; a press chooses.
      onMouseDown: event => event.preventDefault(),
      onMouseMove: () => {
        const index = results.indexOf(item)
        if (index !== -1 && index !== selectedIndex) setSelectedIndex(index)
      },
      onClick: event => onSelect(item, targetFromEvent({ ...event, key: 'Enter' }, targets) ?? 'here'),
    }),
  }
}

/** Where recent ids are kept: per person and gateway in the host, in memory by default. */
export interface RecentStore {
  readonly get: () => readonly string[]
  readonly set: (ids: readonly string[]) => void
  readonly subscribe: (listener: () => void) => () => void
}

export const memoryRecentStore = (initial: readonly string[] = []): RecentStore => {
  let ids = initial
  const listeners = new Set<() => void>()
  return {
    get: () => ids,
    set: next => {
      ids = next
      for (const listener of listeners) listener()
    },
    subscribe: listener => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
  }
}

/** The recent list, newest first and bounded, and a way to push an id onto it. */
export const useRecent = ({ store, limit = 20 }: { readonly store: RecentStore; readonly limit?: number }) => {
  const recent = React.useSyncExternalStore(store.subscribe, store.get, store.get)
  const push = React.useCallback((id: string) => store.set(pushRecent(store.get(), id, limit)), [store, limit])
  return { recent, push }
}
