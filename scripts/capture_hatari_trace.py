#!/usr/bin/env python3
"""Captures a reference CPU trace from Hatari (2.6.1+): boots an ST with the given TOS image and writes the address
of each of the first N instructions it executes, one `{pc:06X}` per line, to OUT.txt. The Atari ST machine's
`tests/emutos.rs` (retroclawd-atari) compares its own trace with it.

    capture_hatari_trace.py ROM N OUT.txt [--hatari PATH] [--vbls K] [--memsize MIB]

Hatari runs as a black box (GPL; nothing of it is copied): `--trace cpu_disasm --trace-file FILE --run-vbls K`
(Hatari manual, "Debug options" and "Misc system options"; `hatari --trace help` lists the flags), with
`--machine st --memsize 1 --sound off --fast-forward on --confirm-quit off`. A disassembly line starts with the
8-digit instruction address. `--hatari` defaults to `hatari` on PATH; `--vbls` to 5 (more than enough for the first
few thousand instructions). Standard library only.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

LINE = re.compile(r"^([0-9a-fA-F]{8}) ")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("rom")
    ap.add_argument("count", type=int)
    ap.add_argument("out")
    ap.add_argument("--hatari", default=shutil.which("hatari") or "hatari")
    ap.add_argument("--vbls", type=int, default=5)
    ap.add_argument("--memsize", default="1")
    a = ap.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        trace = os.path.join(tmp, "trace.txt")
        cmd = [
            a.hatari,
            "--machine", "st",
            "--tos", os.path.abspath(a.rom),
            "--memsize", a.memsize,
            "--sound", "off",
            "--fast-forward", "on",
            "--confirm-quit", "off",
            "--trace", "cpu_disasm",
            "--trace-file", trace,
            "--run-vbls", str(a.vbls),
        ]
        subprocess.run(cmd, check=False, timeout=600, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if not os.path.exists(trace):
            print("Hatari wrote no trace", file=sys.stderr)
            return 1
        pcs = []
        with open(trace, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = LINE.match(line)
                if m:
                    pcs.append(int(m.group(1), 16) & 0xFFFFFF)
                    if len(pcs) == a.count:
                        break
    if len(pcs) < a.count:
        print(f"only {len(pcs)} instructions traced; raise --vbls", file=sys.stderr)
    with open(a.out, "w", encoding="ascii", newline="\n") as f:
        f.writelines(f"{pc:06X}\n" for pc in pcs)
    print(f"{a.out}: {len(pcs)} instructions")
    return 0


if __name__ == "__main__":
    sys.exit(main())
