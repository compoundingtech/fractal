#!/usr/bin/env bash
# Real window-server captures of the tray inside a headless macOS VM (README.md, Design gallery).
#
#   capture-vm.sh provision          once per VM host: pull the guest, SSH key, Screen Recording
#   capture-vm.sh [capture] [args…]  build TrayGallery here, capture in the guest, write
#                                    design/renders/capture/ + capture.json (args go to --capture:
#                                    --only <text>)
#   capture-vm.sh stop               stop a guest left running (ATTENTION_TRAY_CAPTURE_KEEP=1)
#
# The guest runs on ATTENTION_TRAY_CAPTURE_HOST (an SSH host, or `local` for this Mac) under tart
# with no window anywhere: `--no-graphics` plus CI=1, otherwise tart opens Screen Sharing on the
# host's display. The gallery uses the guest's own window server, so nothing appears on anyone's
# screen and no consent or idle wait applies. The guest's macOS is the captures' macOS;
# capture.json records it.
set -euo pipefail

vm=${ATTENTION_TRAY_CAPTURE_VM:-attention-tray-capture}
host=${ATTENTION_TRAY_CAPTURE_HOST:-mbp2021}
state_dir=.local/state/attention-tray-capture # under $HOME on the VM host
# Pinned like the tray's own nixpkgs input (flakes/attention-tray/flake.nix).
nixpkgs=github:NixOS/nixpkgs/4ba99f3209788ed04f01af38f77ddb803ad6ec63

log() { printf 'capture-vm: %s\n' "$*" >&2; }
die() {
  log "$*"
  exit 1
}

# ---------------------------------------------------------------------------------------------
# Host side: runs on the VM host, from the copy `on_host` places in the state directory.

host_main() {
  cd "$HOME/$state_dir"
  tools
  local cmd=$1
  shift
  case $cmd in
    provision) host_provision ;;
    capture) host_capture "$@" ;;
    stop) ./tart/bin/tart stop "$vm" >/dev/null 2>&1 || true ;;
    *) die "unknown host command $cmd" ;;
  esac
}

tools() {
  [[ -x tart/bin/tart ]] || NIXPKGS_ALLOW_UNFREE=1 nix build --impure --quiet -o tart "$nixpkgs#tart" >&2
  [[ -x sshpass/bin/sshpass ]] || nix build --quiet -o sshpass "$nixpkgs#sshpass" >&2
  [[ -x vncdo/bin/vncdo ]] || nix build --quiet -o vncdo "$nixpkgs#python3Packages.vncdotool" >&2
  # A non-default output links as <name>-<output>.
  [[ -x jq-bin/bin/jq ]] || nix build --quiet -o jq "$nixpkgs#jq.bin" >&2
  [[ -f id_ed25519 ]] || ssh-keygen -q -t ed25519 -N '' -C "$vm" -f id_ed25519
}

vms() { ./tart/bin/tart list --source local --format json | ./jq-bin/bin/jq -e --arg vm "$vm" "$1" >/dev/null; }
# shellcheck disable=SC2016 # jq variables
exists() { vms '.[] | select(.Name == $vm)'; }
# shellcheck disable=SC2016
running() { vms '.[] | select(.Name == $vm and .Running)'; }

ssh_options=(-q -T -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o LogLevel=ERROR -o ConnectTimeout=5)
guest() { ssh "${ssh_options[@]}" -i id_ed25519 "admin@$(./tart/bin/tart ip "$vm")" "$@"; }

vnc() {
  local url password port
  url=$(grep -o 'vnc://[^ ]*' run.log | tail -1)
  password=${url#vnc://:}
  password=${password%@*}
  port=${url##*:}
  ./vncdo/bin/vncdo -s "127.0.0.1::$port" -p "$password" "$@"
}

# Boots the guest headless. Its VNC server is Virtualization.framework's own, on the host's
# loopback, used only to grant Screen Recording during provisioning.
started=0
start() {
  running && return
  started=1
  (CI=1 nohup ./tart/bin/tart run --no-graphics --no-audio --no-clipboard --vnc-experimental "$vm" \
    >run.log 2>&1 </dev/null &)
  ./tart/bin/tart ip --wait 180 "$vm" >/dev/null
  for _ in $(seq 60); do
    ./sshpass/bin/sshpass -p admin ssh "${ssh_options[@]}" "admin@$(./tart/bin/tart ip "$vm")" true </dev/null 2>/dev/null && return
    sleep 2
  done
  die "the guest booted but SSH did not answer"
}

stop_if_started() {
  if ((started)) && [[ -z ${ATTENTION_TRAY_CAPTURE_KEEP:-} ]]; then ./tart/bin/tart stop "$vm" >/dev/null; fi
}

# The guest macOS: the newest the host can run (Virtualization.framework runs macOS 27 guests on
# hosts from 26.6.2), unless ATTENTION_TRAY_CAPTURE_IMAGE names one.
image() {
  if [[ -n ${ATTENTION_TRAY_CAPTURE_IMAGE:-} ]]; then
    echo "$ATTENTION_TRAY_CAPTURE_IMAGE"
    return
  fi
  local major minor patch
  IFS=. read -r major minor patch _ <<<"$(sw_vers -productVersion).0.0"
  if ((major >= 27 || (major == 26 && (minor > 6 || (minor == 6 && patch >= 2))))); then
    echo ghcr.io/cirruslabs/macos-golden-gate-vanilla:27.0
  else
    echo "ghcr.io/cirruslabs/macos-tahoe-vanilla:$major.$minor"
  fi
}

host_provision() {
  if ! exists; then
    local from policy="$HOME/.config/reclaim-disk/policy.json" floor=0 available need
    from=$(image)
    # A pulled guest takes ~30 GB (26.5: 28 GB); a clone of one already pulled shares its blocks.
    need=$((40 << 30))
    ./tart/bin/tart list --source oci --quiet | grep -qxF "$from" && need=$((5 << 30))
    [[ -f $policy ]] && floor=$(./jq-bin/bin/jq -r '.floorBytes // 0' "$policy")
    available=$(($(df -k "$HOME" | awk 'NR == 2 {print $4}') << 10))
    ((available - need > floor)) ||
      die "$((available >> 30)) GiB free: the guest (~$((need >> 30)) GiB) would cross this host's disk floor ($((floor >> 30)) GiB)"
    log "cloning $from (a first pull downloads ~25-30 GB)"
    ./tart/bin/tart clone "$from" "$vm" >&2
    # 2x like a Retina display, tall enough for the longest panel.
    ./tart/bin/tart set "$vm" --display 1600x1200pt --no-display-refit >&2
  fi
  start
  # shellcheck disable=SC2016 # expands in the guest
  ./sshpass/bin/sshpass -p admin ssh "${ssh_options[@]}" "admin@$(./tart/bin/tart ip "$vm")" \
    'key=$(cat); mkdir -p ~/.ssh; grep -qxF "$key" ~/.ssh/authorized_keys 2>/dev/null || echo "$key" >>~/.ssh/authorized_keys' \
    <id_ed25519.pub
  if ! guest 'screencapture -x /tmp/probe.png && rm /tmp/probe.png' 2>/dev/null; then
    # SIP protects the TCC database and macOS 27 lets nothing grant TCC programmatically, so add
    # sshd-keygen-wrapper (the responsible process of every SSH command) the way a person would,
    # over VNC: Settings › Screen & System Audio Recording › + › admin password › Go to
    # /usr/libexec/sshd-keygen-wrapper › Open. Coordinates fit the 1600x1200pt display, macOS 26.5.
    # A password typed into a just-opened sheet is sometimes not submitted, so the file panel's
    # XPC service (alive only while a panel is open) confirms each attempt.
    log "granting Screen Recording to sshd-keygen-wrapper in the guest"
    guest 'killall "System Settings" com.apple.appkit.xpc.openAndSavePanelService 2>/dev/null; sleep 1
      open "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"'
    sleep 6
    vnc move 1384 614 click 1
    sleep 6
    local clear=() attempt
    for _ in $(seq 40); do clear+=(key bsp); done
    for attempt in 1 2 3; do
      vnc "${clear[@]}" type admin pause 1 key enter
      sleep 5
      guest 'pgrep -q -f openAndSavePanelService' && break
      ((attempt < 3)) || log "the password sheet did not accept the password"
    done
    vnc type / pause 2 type usr/libexec/sshd-keygen-wrapper pause 2 key enter pause 3 key enter pause 4 \
      capture provision.png
    guest 'killall "System Settings"' || true
    guest 'screencapture -x /tmp/probe.png && rm /tmp/probe.png' ||
      die "Screen Recording is still missing in the guest; see ~/$state_dir/provision.png on $(hostname -s) and finish it over $(grep -o 'vnc://[^ ]*' run.log | tail -1) there"
  fi
  log "provisioned $vm: macOS $(guest 'sw_vers -productVersion'), Screen Recording granted"
  stop_if_started
}

# stdin: tar with TrayGallery and renders/capture.json; stdout: tar with capture.json + capture/.
host_capture() {
  local rev=$1
  shift
  exists || die "no $vm VM on $(hostname -s): run capture-vm.sh provision first"
  start
  guest 'rm -rf ~/gallery && mkdir ~/gallery && tar -x -C ~/gallery -f -'
  log "capturing on macOS $(guest 'sw_vers -productVersion') ($vm on $(hostname -s))"
  # Nobody uses the guest's screen: consent is implied and there is no idle wait.
  guest "cd ~/gallery && TRAY_GALLERY_REV=$(printf %q "$rev") ./TrayGallery renders --capture --consent --away 0 $(printf '%q ' "$@")" >&2
  guest 'cd ~/gallery/renders && tar -c -f - capture.json capture'
  stop_if_started
}

if [[ ${1:-} == host ]]; then
  shift
  host_main "$@"
  exit
fi

# ---------------------------------------------------------------------------------------------
# Local side: builds the gallery and drives the host.

root=$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
renders="$root/flakes/attention-tray/design/renders"

on_host() {
  if [[ $host == local ]]; then
    mkdir -p "$HOME/$state_dir"
    cp "${BASH_SOURCE[0]}" "$HOME/$state_dir/capture-vm.sh"
    ATTENTION_TRAY_CAPTURE_VM=$vm bash "$HOME/$state_dir/capture-vm.sh" host "$@"
  else
    # shellcheck disable=SC2029 # expands on the host
    ssh -o BatchMode=yes "$host" "mkdir -p $state_dir && cat >$state_dir/capture-vm.sh" <"${BASH_SOURCE[0]}"
    # shellcheck disable=SC2029
    ssh -o BatchMode=yes "$host" \
      "$(printf '%q=%q ' ATTENTION_TRAY_CAPTURE_VM "$vm" ATTENTION_TRAY_CAPTURE_KEEP "${ATTENTION_TRAY_CAPTURE_KEEP:-}" \
        ATTENTION_TRAY_CAPTURE_IMAGE "${ATTENTION_TRAY_CAPTURE_IMAGE:-}")bash $state_dir/capture-vm.sh host $(printf '%q ' "$@")"
  fi
}

command=${1:-capture}
[[ $# -gt 0 ]] && shift
case $command in
  provision) on_host provision ;;
  stop) on_host stop ;;
  capture)
    gallery=$(nix build --no-link --print-out-paths "$root/flakes/attention-tray#gallery")
    rev=$(sed -n 's/^TRAY_GALLERY_REV=\([^ ]*\) .*/\1/p' "$gallery/bin/attention-tray-gallery")
    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT
    mkdir -p "$stage/seed/renders" "$stage/out"
    cp "$gallery/libexec/TrayGallery" "$stage/seed/"
    # The gallery merges into the manifest it finds: a --only run keeps every other entry.
    [[ -f $renders/capture.json ]] && cp "$renders/capture.json" "$stage/seed/renders/"
    tar -c -f - -C "$stage/seed" . | on_host capture "$rev" "$@" | tar -x -f - -C "$stage/out"
    [[ -f $stage/out/capture.json ]] || die "the guest returned no capture.json"
    mkdir -p "$renders/capture"
    find "$stage/out/capture" -name '*.png' -exec cp {} "$renders/capture/" \;
    cp "$stage/out/capture.json" "$renders/capture.json"
    log "wrote $(find "$stage/out/capture" -name '*.png' | wc -l | tr -d ' ') captures to ${renders#"$root"/}/capture/"
    ;;
  *) die "usage: capture-vm.sh [provision | capture [--only <text>] | stop]" ;;
esac
