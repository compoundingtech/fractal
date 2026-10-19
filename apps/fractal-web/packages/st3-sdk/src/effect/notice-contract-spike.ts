// Design-only replay: no server or browser global dependency; same wire fixtures as Rust.
import { Effect, Option, Schema, Stream } from 'effect'
import { NoticeSnapshot } from '../../../../../../clients/typescript/st3-client/Schema.generated.ts'
import frames from '../../../../../../tests/notice-contract/frames.json' with { type: 'json' }

type Notice = typeof NoticeSnapshot.Type
interface Descriptor<A> {
  readonly registeredKind: string
  readonly decode: (input: unknown) => A
}
const notices: Descriptor<Notice> = {
  registeredKind: 'attention.notice-snapshot',
  decode: Schema.decodeUnknownSync(NoticeSnapshot),
}
// The production port is the extracted socket/window core. Replay provides its raw windows,
// not a pretend implementation of the proposed server watch route.
interface Windows {
  readonly windows: Stream.Stream<readonly unknown[]>
}
const watch = <A>(windows: Windows, descriptor: Descriptor<A>) => windows.windows.pipe(
  Stream.map((rows) => rows.map(descriptor.decode)),
)
// A adds one public method per domain; B carries the type at the descriptor instead.
const followNotices = (windows: Windows) => watch(windows, notices)
const windows: Windows = {
  windows: Stream.fromIterable(frames.map((frame) => frame.items ?? frame.upserts ?? [])),
}
const scenario = Effect.gen(function* () {
  const sidebarA = yield* Stream.runCollect(followNotices(windows))
  const sidebarB = yield* Stream.runCollect(watch(windows, notices))
  const tray = yield* Stream.runCollect(watch(windows, notices))
  if (JSON.stringify(sidebarA) !== JSON.stringify(sidebarB) || JSON.stringify(sidebarB) !== JSON.stringify(tray)) {
    return yield* Effect.die('A/B/sidebar/tray disagree')
  }
  if (tray[1]?.[0] === undefined || Option.getOrNull(tray[1][0].seen_at) !== '2026-10-19T12:00:01Z') {
    return yield* Effect.die('tray did not observe seen')
  }
  yield* Effect.log('A/B replay: sidebar and tray agree after seen; generated Effect codec decoded 2 windows')
})
await Effect.runPromise(scenario)
