#!/usr/bin/env python3
"""Captures a golden picture from ZEsarUX: boots MACHINE headless, waits SECONDS of wall time, optionally types
keys, saves the screen over ZRCP and writes it as a PNG. Then pin it:  rc-scenarios hash OUT.png  and put the
hash in the manifest's `golden_sha256`, with the ZEsarUX version and this command in the manifest's [reference].

    capture_zesarux_screen.py MACHINE SECONDS OUT.png [--keys TEXT] [--after SECONDS] [-- extra zesarux args]

ZEsarUX has no deterministic frame stop, so SECONDS is wall time at its own 50 Hz pacing: use a value well after
the picture settles. `--realvideo` is always passed: without it save-screen writes an all-black picture (ZX81 and Spectrum). Needs `zesarux` on PATH. Standard library only.
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
import zlib


def bmp_to_png(bmp: bytes, crop=None) -> bytes:
    assert bmp[:2] == b"BM", "not a BMP"
    off = struct.unpack_from("<I", bmp, 10)[0]
    _, w, h, _, bpp, comp, _, _, _, ncol, _ = struct.unpack_from("<IiiHHIIiiII", bmp, 14)
    assert comp == 0 and bpp in (8, 24, 32), f"unsupported BMP: bpp={bpp} compression={comp}"
    top_down = h < 0
    h = abs(h)
    palette = []
    if bpp == 8:
        n = ncol or 256
        palette = [bmp[54 + 4 * i : 54 + 4 * i + 3][::-1] for i in range(n)]  # BGR -> RGB
    stride = (w * bpp // 8 + 3) & ~3
    rows = []
    cx, cy, cw, ch = crop or (0, 0, w, h)
    for y in range(cy, cy + ch):
        sy = y if top_down else h - 1 - y
        row = bmp[off + sy * stride : off + sy * stride + w * bpp // 8]
        if bpp == 8:
            px = b"".join(palette[i] for i in row)
        else:
            step = bpp // 8
            px = b"".join(bytes((row[i + 2], row[i + 1], row[i])) for i in range(0, w * step, step))
        px = px[3 * cx : 3 * (cx + cw)]
        rows.append(b"\0" + px)

    def chunk(tag: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", cw, ch, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(b"".join(rows), 9))
        + chunk(b"IEND", b"")
    )


class Zrcp:
    def __init__(self, port: int):
        deadline = time.time() + 20
        while True:
            try:
                self.s = socket.create_connection(("127.0.0.1", port), timeout=2)
                break
            except OSError:
                if time.time() > deadline:
                    raise SystemExit("ZEsarUX remote protocol did not come up")
                time.sleep(0.3)
        self.read()

    def read(self, quiet: float = 0.6) -> str:
        self.s.settimeout(quiet)
        buf = b""
        try:
            while True:
                d = self.s.recv(65536)
                if not d:
                    break
                buf += d
        except OSError:
            pass
        return buf.decode(errors="replace")

    def cmd(self, c: str) -> str:
        self.s.sendall((c + "\n").encode())
        return self.read()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("machine")
    ap.add_argument("seconds", type=float)
    ap.add_argument("out")
    ap.add_argument("--keys", help="typed (send-keys-ascii) after the first wait; \n is Enter")
    ap.add_argument("--key-ms", type=int, default=160, help="milliseconds between typed keys")
    ap.add_argument("--after", type=float, default=3.0, help="wait after typing before the picture")
    ap.add_argument("--crop", help="X,Y,W,H of the screenshot to keep (the ZX81 picture is 256x192 at 50,41), so the golden has "
                    "the same geometry as a retroclawd picture without its border")
    ap.add_argument("--load", help="a program (.p, .tap, .z80 ...) loaded with smartload before the wait")
    ap.add_argument("--port", type=int, default=10000)
    ap.add_argument("extra", nargs="*")
    a = ap.parse_args()

    exe = shutil.which("zesarux")
    if not exe:
        raise SystemExit("zesarux is not on PATH")
    cmd = [exe, "--noconfigfile", "--nosplash", "--nowelcomemessage", "--disable-all-first-aid", "--machine", a.machine,
           "--ao", "null", "--enable-remoteprotocol", "--remoteprotocol-port", str(a.port),
           "--exit-after", str(int(a.seconds + a.after + 30))]
    cmd += ["--realvideo"] + a.extra  # without it save-screen writes an all-black picture

    proc = subprocess.Popen(cmd, cwd=os.path.dirname(exe), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        z = Zrcp(a.port)
        if a.load:
            time.sleep(2)  # let the machine boot, then load like the user's File > Smart load
            z.cmd(f"smartload {os.path.abspath(a.load)}")
        time.sleep(a.seconds)
        if a.keys:
            # \n = Enter (13), \d = cursor down (10), \u = up (11), \l = left (8), \r = right (9)
            special = {"n": 13, "d": 10, "u": 11, "l": 8, "r": 9}
            text, i = [], 0
            while i < len(a.keys):
                if a.keys[i] == "\\" and i + 1 < len(a.keys) and a.keys[i + 1] in special:
                    text.append(special[a.keys[i + 1]])
                    i += 2
                else:
                    text.append(ord(a.keys[i]))
                    i += 1
            codes = " ".join(map(str, text))
            z.cmd(f"send-keys-ascii {a.key_ms} {codes}")  # the first argument is the time between keys, in ms
            time.sleep(len(text) * a.key_ms * 3 / 1000 + a.after)
        with tempfile.TemporaryDirectory() as d:
            bmp = os.path.join(d, "shot.bmp")
            z.cmd(f"save-screen {bmp}")
            if not os.path.exists(bmp):
                raise SystemExit("ZEsarUX wrote no screen file")
            png = bmp_to_png(open(bmp, "rb").read(), tuple(int(v) for v in a.crop.split(",")) if a.crop else None)
        os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
        open(a.out, "wb").write(png)
        print(f"{a.out}: {len(png)} bytes; {z.cmd('get-registers').splitlines()[0]}")
        z.cmd("exit-emulator")
    finally:
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    sys.exit(main())
