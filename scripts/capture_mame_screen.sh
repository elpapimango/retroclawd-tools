#!/usr/bin/env bash
# Captures a golden picture from MAME: runs a driver headless for SECONDS of emulated time, takes a snapshot
# and copies it to OUT. Then pin it:  rc-scenarios hash OUT   and put the hash in the manifest's `golden_sha256`,
# with the MAME version and this command in the manifest's [reference].
#
#   scripts/capture_mame_screen.sh <driver> <seconds> <out.png> [extra mame arguments, e.g. -rompath DIR -cass FILE]
#
# NOT YET RUN: no MAME is installed on the machine this was written on. Check the first capture by eye, and if
# `-video none` produces no snapshot with your MAME build, change it to `-video soft -window`.
set -euo pipefail
[[ $# -ge 3 ]] || { sed -n '2,10p' "$0"; exit 2; }
driver="$1"; seconds="$2"; out="$3"; shift 3
command -v mame >/dev/null || { echo "mame is not on PATH" >&2; exit 1; }

work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
cat > "$work/snap.lua" <<LUA
local stop = tonumber(os.getenv("SNAP_SECONDS"))
emu.register_frame_done(function()
  if manager.machine.time:as_double() >= stop then
    manager.machine.video:snapshot()
    manager.machine:exit()
  end
end)
LUA

SNAP_SECONDS="$seconds" mame "$driver" -video none -sound none -nothrottle -skip_gameinfo \
  -snapshot_directory "$work/snap" -snapname snapshot -autoboot_script "$work/snap.lua" "$@" >/dev/null
shot="$(find "$work/snap" -name '*.png' | head -1)"
[[ -n "$shot" ]] || { echo "mame wrote no snapshot" >&2; exit 1; }
mkdir -p "$(dirname "$out")"
cp "$shot" "$out"
echo "wrote $out (mame $(mame -help 2>/dev/null | head -1))"
