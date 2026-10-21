#!/usr/bin/env python3
"""Captures a reference trace from Hatari (2.6.1+): boots an ST with the given TOS image and writes the first N lines
of one kind to OUT.txt. The Atari ST machine's `tests/emutos.rs` (retroclawd-atari) compares its own run with it.

- `--kind pc` (default): the address of each instruction executed, one `{pc:06X}` per line.
- `--kind io`: each I/O register access, Hatari's `--trace io_all` lines verbatim
  (`IO write.b $ffff8001 = $0a pc=fc0188`), the format of the machine's `io_trace_file`.

    capture_hatari_trace.py ROM N OUT.txt [--kind pc|io] [--hatari PATH] [--vbls K] [--memsize MIB]
                            [--disk-a IMAGE] [--trace-extra FLAGS]

`--disk-a` puts a floppy image in drive A (Hatari's `--disk-a`). `--trace-extra` adds Hatari trace flags (comma
separated, e.g. `ikbd_cmds,fdc`; `hatari --trace help` lists them); every traced line that is not part of OUT.txt goes
to `OUT.extra.txt`, verbatim, in trace order.

`--memsize` is Hatari's: MiB 1-14, 0 for 512 KiB, other values KiB (256 = 256 KiB, 2560 = 2.5 MiB).

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
    ap.add_argument("--kind", choices=["pc", "io"], default="pc")
    ap.add_argument("--disk-a")
    ap.add_argument("--trace-extra", default="")
    a = ap.parse_args()
    flags = ["cpu_disasm" if a.kind == "pc" else "io_all"] + [f for f in a.trace_extra.split(",") if f]
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
            "--trace", ",".join(flags),
            "--trace-file", trace,
            "--run-vbls", str(a.vbls),
        ]
        if a.disk_a:
            cmd += ["--disk-a", os.path.abspath(a.disk_a)]
        subprocess.run(cmd, check=False, timeout=600, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if not os.path.exists(trace):
            print("Hatari wrote no trace", file=sys.stderr)
            return 1
        lines = []
        extra = []
        with open(trace, encoding="utf-8", errors="replace") as f:
            for line in f:
                m = LINE.match(line) if a.kind == "pc" else None
                if m:
                    lines.append(f"{int(m.group(1), 16) & 0xFFFFFF:06X}")
                elif a.kind == "io" and line.startswith("IO "):
                    lines.append(line.rstrip("\r\n"))
                elif a.trace_extra:
                    extra.append(line.rstrip("\r\n"))
                if len(lines) == a.count:
                    break
    if len(lines) < a.count:
        print(f"only {len(lines)} lines traced; raise --vbls", file=sys.stderr)
    with open(a.out, "w", encoding="ascii", newline="\n") as f:
        f.writelines(line + "\n" for line in lines)
    print(f"{a.out}: {len(lines)} lines")
    if a.trace_extra:
        extra_out = os.path.splitext(a.out)[0] + ".extra.txt"
        with open(extra_out, "w", encoding="utf-8", newline="\n") as f:
            f.writelines(line + "\n" for line in extra)
        print(f"{extra_out}: {len(extra)} lines")
    return 0


if __name__ == "__main__":
    sys.exit(main())
