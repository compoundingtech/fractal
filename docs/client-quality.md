# Device-local client quality

stui records client experience in `$XDG_CACHE_HOME/st3/stui/slo` (otherwise
`~/.cache/st3/stui/slo`). `stui --slo` prints the retained hours and exits before
configuration, connections or terminal setup. It creates no directory. These
measurements never leave the device. The summary contains no person, conversation
text, resource identifiers or paths. Its only identifying fields are host and build.

Each JSON line has exactly these fields:

| Field | Value |
| --- | --- |
| `version` | `1` |
| `window_start` | Unix seconds at the start of the local clock hour |
| `host` | Device hostname |
| `build` | Client build version |
| `bucket_upper_ms` | `[1,2,4,8,16,32,64,128,256,512,1024,2048,4096,8192,16384,32768,null]` |
| `counters` | Fixed names mapped to integer counts; durations use milliseconds |
| `histograms` | Fixed names mapped to seventeen integer bucket counts |
| `targets` | Initial target names mapped to numeric limits |

Buckets have inclusive upper bounds. Fractional milliseconds round upward. The
last bucket holds everything over 32768 ms. Percentiles printed by `--slo` are
bucket upper bounds, not interpolated times. The starting targets are guesses for
a week of measurement, listed together in `crates/stui/src/ui/slo.rs`.

The counters are `active_ms`, `panics`, `silent_exits`, `launches`,
`first_screen_top_bar_flips`, `first_screen_placeholder_flips`,
`conversation_opens`, `open_not_bound`, `open_remote_unavailable`,
`open_cannot_load`, `drafts_restored_unclean`, `not_live_ms`,
`direct_attach_attempts`, `direct_attach_success`, `gateway_attach`, `terminal_open_errors`,
`dead_terminal_tabs`, and `measurement_overflow`.

The histograms are `first_frame_ms`, `first_real_state_ms`,
`conversation_open_ms`, `key_to_redraw_ms`, `freshness_ms`, `reconnect_ms`,
`terminal_open_ms`, and `alert_landed_ms`.

Launch timing starts at entry to main. Real state requires `World.settled` and
successful terminal presentation. First-screen flips count changes in the live
status and attention/agent loading masks until that state is presented.
Conversation opens start with the ordinary subscription intent and finish when
entries are visible after successful presentation; cancelled opens have no success
sample. Open failures retain the feed's typed refusal or transcript-not-bound notice.
Focused input keys have individual timestamps in a fixed array. Freshness starts
when the feed receives a decoded window, before maintaining its view, and ends
after a frame displays that window's rows (or the classic alert badge). Hidden
windows produce no latency sample. Reconnects end when live state is presented.
Direct attempts count only devices with a local daemon; ordinary paired-device
gateway routes do not reduce the direct-route success percentage. Terminal opens
end when an attached native screen is visibly presented. Dead tabs
count stream loss or exit, once per transition. Alert landing requires a subsequent
non-truncated attention window to show the answered card gone or resolved, followed
by successful presentation; failed actions do not produce a success sample.

A dedicated thread wakes at the next clock-hour boundary and on shutdown. It adds
active time and drains atomics into daily JSONL files. Shutdown adds a partial hour;
a later process adds to that same hour under a disk lock. At most 336 hour rows
remain across at most fifteen daily files. Daily reads are capped at 128 KiB and
24 lines. A failed cache-directory creation disables the writer for that run.
Panic counting writes synchronously from the existing panic hook, using the same
fixed summary; the panic text stays in the existing separate panic log. Run markers
distinguish a known panic from an otherwise silent exit.

An unclean kill can lose the counters and active time since the last completed
hour, and recovery counts its stale run marker in the next launch's hour. Restored
drafts are counted only after such an unclean run. Multiple simultaneously running
clients contribute active time separately, as active client hours. A build field
names the most recent contributor to that hour. Fixed timestamp-array exhaustion
increments `measurement_overflow`; it never allocates extra hot-path storage.

Allocation verification uses stui's thread-local counting global allocator:
compare one hundred identical warmed terminal draws with and without quality
recording. The renderer's existing allocations must be identical, so the new
input and presentation accounting adds zero. Histogram boundary, fixed line
shape, unavailable directory, same-hour merging and fourteen-day rotation tests
run alongside it.
