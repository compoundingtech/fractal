#!/usr/bin/env bash
# Run only at the Mac gate, never automatically deployed or launched on a person's Mac.
set -euo pipefail
if [[ $(uname -s) != Darwin ]]; then
  echo 'NOT RUN (needs Mac): SwiftUI/AppKit build, tests, probe, Instruments and memory sampling' >&2
  exit 2
fi
cd "$(dirname "$0")/.."
mode=${1:-check}
out=${TRAY_PERF_OUT:-"$PWD/tmp/perf-scroll-mac/$(date -u +%Y%m%dT%H%M%SZ)"}
mkdir -p "$out"
case "$mode" in
  check)
    : "${FRACTAL_CLIENT_PATH:?Use the attention-tray Nix development shell with the pinned FractalClient package}"
    # Nix source closures have no .git directory. Record the locked revision and
    # the actual package path rather than requiring an external client checkout.
    printf '%s\n' "$FRACTAL_CLIENT_PATH" | tee "$out/fractal-client-path.txt"
    nix eval --raw --expr '(builtins.fromJSON (builtins.readFile ./flake.lock)).nodes.fractal.locked.rev' \
      | tee "$out/fractal-client-head.txt"
    /usr/bin/swift build --product AttentionTray 2>&1 | tee "$out/build-tray.txt"
    /usr/bin/swift build --product TrayGallery 2>&1 | tee "$out/build-gallery.txt"
    /usr/bin/swift test 2>&1 | tee "$out/tests.txt"
    .build/debug/TrayGallery "$out" --scroll-probe 2>&1 | tee "$out/probe.txt"
    echo 'PASS requires four probe PASS lines (inline/pinned × 300/400 pt), stable offset and zero section writes.'
    ;;
  trace)
    pid=${2:?Usage: bash scripts/validate-scroll-mac.sh trace EXISTING_TRAY_PID}
    kill -0 "$pid"
    # Attach only: this script never launches the app or opens a window.
    xcrun xctrace list templates > "$out/templates.txt"
    echo 'For each 60s capture, manually repeat panel open → detail → scroll fully down/up → back; include a 30s refresh.'
    echo 'Use a 120 Hz display; test cold and warm Markdown/image pages, both answer placements, and Always/Automatic scrollers.'
    xcrun xctrace record --template 'Animation Hitches' --attach "$pid" --time-limit 60s --output "$out/hitches.trace"
    xcrun xctrace record --template 'Time Profiler' --attach "$pid" --time-limit 60s --output "$out/first-frame.trace"
    xcrun xctrace export --input "$out/hitches.trace" --toc --output "$out/hitches-toc.xml"
    xcrun xctrace export --input "$out/first-frame.trace" --toc --output "$out/first-frame-toc.xml"
    # Export every table (schemas vary with Xcode); retain signpost intervals and hitch/frame data.
    xcrun xctrace export --input "$out/hitches.trace" --xpath '/trace-toc/run[@number="1"]/data/table' --output "$out/hitches.xml"
    xcrun xctrace export --input "$out/first-frame.trace" --xpath '/trace-toc/run[@number="1"]/data/table' --output "$out/first-frame.xml"
    cat <<'THRESHOLDS'
Mac reviewer must inspect traces in Instruments (no automatic PASS from command success):
- Points of Interest: dev.schickling.attention-tray / NavigationFirstFrame, every drill-in/back <100 ms.
  Interval is click handler → first AppKit draw submission, NOT proof of presentation.
  Correlate with rendered-frame presentation in Animation Hitches; first presented frame must also be <100 ms.
- At 120 Hz, each scrolling frame fits 8.333 ms; zero dropped frames / zero animation hitches throughout each 60s capture.
- Repeat panel open/drill-in/back 20 times cold/warm; inspect main-thread stacks for Markdown parsing/image decoding on unchanged refresh.
- Keep panel closed for two refreshes; no page-body/render work should appear (menu-bar label updates are expected).
- Short-window probe passes with both Always and Automatic system scrollbar settings; manual bottom content/answer controls reachable.
Missing template, absent signposts, or missing presentation data is NOT RUN/blocked, never a pass.
THRESHOLDS
    ;;
  memory)
    pid=${2:?Usage: bash scripts/validate-scroll-mac.sh memory EXISTING_TRAY_PID}
    kill -0 "$pid"
    echo 'Keep panel closed and speech muted; sampling 180 seconds, separately from Instruments to avoid profiler overhead.'
    for ((i=0; i<180; i++)); do
      stamp=$(date -u +%Y%m%dT%H%M%SZ)
      ps -p "$pid" -o pid=,rss=,vsz=,pcpu= >> "$out/ps.txt"
      # footprint records physical footprint, not just RSS. Capture vmmap each second
      # too so a <5s transient ~289 MB allocation cannot fall between summary samples.
      footprint -p "$pid" > "$out/footprint-$stamp.txt" 2>&1
      vmmap -summary "$pid" > "$out/vmmap-$stamp.txt" 2>&1
      sleep 1
    done
    echo 'Compare footprint/vmmap peaks against idle baseline, correlate MALLOC/VM categories with 30s refreshes.'
    echo 'Gate remains pending if the earlier ~289 MB spike recurs without attribution; no memory-fix claim from tray changes alone.'
    ;;
  *) echo 'Usage: validate-scroll-mac.sh {check|trace PID|memory PID}' >&2; exit 2 ;;
esac
printf 'Evidence: %s\n' "$out"
