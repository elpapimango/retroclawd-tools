#!/usr/bin/env python3
"""Records what ZEsarUX plays: boots MACHINE, optionally loads a program and types keys, records SECONDS of its sound
(`--aofile`, raw 8-bit unsigned mono at 15600 Hz) and writes it as a 16-bit mono WAV that `rc-scenarios audio` can measure.

    capture_zesarux_audio.py MACHINE SECONDS OUT.wav [--load FILE] [--keys TEXT] [--key-ms MS] [-- extra zesarux args]

SECONDS is wall time at ZEsarUX's own pacing and counts from the start of the emulator (the recording includes the boot),
so measure a window well after the program has started. `--keys` takes the same `{NAME}` / `{SHIFT+c}` forms as
capture_zesarux_screen.py. Needs `zesarux` on PATH. Standard library only.
"""
import argparse
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from capture_zesarux_screen import Zrcp, type_keys  # noqa: E402

RATE = 15600  # ZEsarUX's own audio sample rate; the raw file is 8-bit unsigned, 0x80 the middle


def write_wav(path: str, raw: bytes) -> None:
    pcm = b"".join(struct.pack("<h", (b - 128) * 256) for b in raw)
    header = (
        b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 1, RATE, RATE * 2, 2, 16)
        + b"data" + struct.pack("<I", len(pcm))
    )
    with open(path, "wb") as f:
        f.write(header + pcm)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("machine")
    ap.add_argument("seconds", type=float)
    ap.add_argument("out")
    ap.add_argument("--load")
    ap.add_argument("--keys")
    ap.add_argument("--key-ms", type=int, default=120)
    ap.add_argument("--port", type=int, default=10000)
    ap.add_argument("extra", nargs="*")
    a = ap.parse_args()

    exe = shutil.which("zesarux")
    if not exe:
        raise SystemExit("zesarux is not on PATH")
    try:
        socket.create_connection(("127.0.0.1", a.port), timeout=1).close()
    except OSError:
        pass
    else:
        raise SystemExit(f"a ZEsarUX is already listening on port {a.port}; close it (or pass --port) first")
    with tempfile.TemporaryDirectory() as d:
        raw_path = os.path.join(d, "audio.raw")
        cmd = [exe, "--noconfigfile", "--nosplash", "--nowelcomemessage", "--disable-all-first-aid", "--machine", a.machine,
               "--ao", "sdl", "--aofile", raw_path, "--enable-remoteprotocol", "--remoteprotocol-port", str(a.port),
               "--exit-after", str(int(a.seconds + 30))] + a.extra
        proc = subprocess.Popen(cmd, cwd=os.path.dirname(exe), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            start = time.time()
            z = Zrcp(a.port)
            if a.load:
                time.sleep(2)
                z.cmd(f"smartload {os.path.abspath(a.load)}")
            if a.keys:
                time.sleep(max(0.0, 6 - (time.time() - start)))  # let the machine boot first
                type_keys(z, a.keys, a.key_ms, 0)
            time.sleep(max(0.0, a.seconds - (time.time() - start)))
            z.cmd("exit-emulator")
        finally:
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
        raw = open(raw_path, "rb").read()
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    write_wav(a.out, raw)
    print(f"{a.out}: {len(raw) / RATE:.1f} s of sound")


if __name__ == "__main__":
    main()
