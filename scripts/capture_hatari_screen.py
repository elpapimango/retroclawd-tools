#!/usr/bin/env python3
"""Captures Hatari's screen at a given VBL as a PNG: the reference picture of an Atari ST scenario (S1).

    capture_hatari_screen.py ROM VBLS OUT.png [--memsize MIB] [--disk-a IMAGE] [--hatari PATH]

Hatari (2.6.1+) runs as a black box (GPL; nothing of it is copied) with `--parse FILE`, whose debugger commands set a
breakpoint `b VBL = VBLS :trace :once :file SHOT` (Hatari manual `doc/debugger.html`, "Breakpoint options" and
"Chaining breakpoints and other actions"); SHOT holds `screenshot OUT.png` and `quit`. Options: `--machine st
--memsize 1 --sound off --fast-forward on --confirm-quit off --borders off --statusbar off --drive-led off` (the
picture without borders or Hatari's status bar, as retroclawd draws it). `--memsize` is Hatari's (MiB, 0 = 512 KiB, other values
KiB). Standard library only.
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
    ap.add_argument("vbls", type=int)
    ap.add_argument("out")
    ap.add_argument("--hatari", default=shutil.which("hatari") or "hatari")
    ap.add_argument("--memsize", default="1")
    ap.add_argument("--disk-a")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    if os.path.exists(out):
        os.remove(out)
    with tempfile.TemporaryDirectory() as tmp:
        shot = os.path.join(tmp, "shot.ini")
        start = os.path.join(tmp, "start.ini")
        with open(shot, "w", encoding="ascii") as f:
            f.write(f"screenshot {out}\nquit\n")
        with open(start, "w", encoding="ascii") as f:
            f.write(f"b VBL = {a.vbls} :trace :once :file {shot}\n")
        cmd = [
            a.hatari,
            "--machine", "st",
            "--tos", os.path.abspath(a.rom),
            "--memsize", a.memsize,
            "--sound", "off",
            "--fast-forward", "on",
            "--confirm-quit", "off",
            "--borders", "off",
            "--statusbar", "off",
            "--drive-led", "off",
            "--parse", start,
        ]
        if a.disk_a:
            cmd += ["--disk-a", os.path.abspath(a.disk_a)]
        subprocess.run(cmd, check=False, timeout=600, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if not os.path.exists(out):
        print("Hatari wrote no screenshot", file=sys.stderr)
        return 1
    print(f"{out}: {os.path.getsize(out)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
