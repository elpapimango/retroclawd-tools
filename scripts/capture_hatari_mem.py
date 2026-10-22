#!/usr/bin/env python3
"""Captures a window of Hatari's memory at a given VBL: the reference result of an Atari ST timing probe.

    capture_hatari_mem.py ROM FLOPPY VBLS ADDR LEN OUT.bin [--hatari PATH] [--memsize MIB] [--video-timing WS]
                          [--monitor rgb|mono] [--extra "--option value ..."]

Hatari (2.6.1+) runs as a black box (GPL; nothing of it is copied) with `--parse FILE`, whose debugger commands set a
breakpoint `b VBL = VBLS :trace :once :file DUMP`; DUMP holds `savebin OUT ADDR LEN` and `quit` (Hatari manual
`doc/debugger.html`, "Breakpoint options" and the `savebin` command). Options: `--machine st --memsize 1 --sound off
--fast-forward on --confirm-quit off --statusbar off --drive-led off`, cycle exact CPU (Hatari's default for the ST,
`--cpu-exact true` is passed explicitly), the wake-up state `--video-timing` (default `ws3`, Hatari's own default),
and a throw-away `--configfile` so the user's configuration cannot change the result. ADDR and LEN are decimal or
`0x`. The options used are printed to stderr. Standard library only.
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("rom")
    ap.add_argument("floppy")
    ap.add_argument("vbls", type=int)
    ap.add_argument("addr")
    ap.add_argument("length")
    ap.add_argument("out")
    ap.add_argument("--hatari", default=shutil.which("hatari") or "hatari")
    ap.add_argument("--memsize", default="1")
    ap.add_argument("--video-timing", default="ws3")
    ap.add_argument("--monitor", default="rgb", choices=["rgb", "mono"])
    ap.add_argument("--extra", default="", help="more Hatari options, space separated")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    if os.path.exists(out):
        os.remove(out)
    with tempfile.TemporaryDirectory() as tmp:
        dump = os.path.join(tmp, "dump.ini")
        start = os.path.join(tmp, "start.ini")
        cfg = os.path.join(tmp, "hatari.cfg")
        open(cfg, "w", encoding="ascii").close()  # Hatari insists that the file exists
        with open(dump, "w", encoding="ascii") as f:
            f.write(f"savebin {out.replace(os.sep, '/')} {a.addr} {a.length}\nquit\n")
        with open(start, "w", encoding="ascii") as f:
            f.write(f"b VBL = {a.vbls} :trace :once :file {dump.replace(os.sep, '/')}\n")
        cmd = [
            a.hatari,
            "--configfile", cfg,
            "--machine", "st",
            "--tos", os.path.abspath(a.rom),
            "--memsize", a.memsize,
            "--cpu-exact", "true",
            "--video-timing", a.video_timing,
            "--monitor", a.monitor,
            "--sound", "off",
            "--fast-forward", "on",
            "--confirm-quit", "off",
            "--statusbar", "off",
            "--drive-led", "off",
            "--disk-a", os.path.abspath(a.floppy),
            "--parse", start,
        ] + a.extra.split()
        print("capture_hatari_mem: " + " ".join(cmd[1:]), file=sys.stderr)
        subprocess.run(cmd, check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=900)
    if not os.path.exists(out):
        print("Hatari wrote no dump (is VBLS too large, or the floppy unreadable?)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
