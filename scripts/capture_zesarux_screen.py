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
import re
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
    full = []
    for y in range(h):
        sy = y if top_down else h - 1 - y
        row = bmp[off + sy * stride : off + sy * stride + w * bpp // 8]
        if bpp == 8:
            full.append(b"".join(palette[i] for i in row))
        else:
            step = bpp // 8
            full.append(b"".join(bytes((row[i + 2], row[i + 1], row[i])) for i in range(0, w * step, step)))
    # A crop may reach past the screenshot (negative offsets, a larger size): the edge pixels are repeated, which is
    # the border colour, so a picture can be given the same canvas as another emulator's.
    cx, cy, cw, ch = crop or (0, 0, w, h)
    rows = []
    for y in range(cy, cy + ch):
        src = full[min(max(y, 0), h - 1)]
        px = b"".join(src[3 * min(max(x, 0), w - 1) : 3 * min(max(x, 0), w - 1) + 3] for x in range(cx, cx + cw))
        rows.append(b"\x00" + px)

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

    def cmd(self, c: str, quiet: float = 0.6) -> str:
        self.s.sendall((c + "\n").encode())
        return self.read(quiet)


def type_keys(z, keys: str, key_ms: int, after: float) -> None:
    """Types `keys` into the machine; see the notes below for the `{NAME}` and `{SHIFT+c}` forms."""
    # Plain characters are typed with send-keys-ascii (its first argument is the time between keys, in ms), but
    # that cannot press punctuation that needs Shift on the CPC, or the cursor keys, and treats LF as Enter.
    # So `{NAME}` presses a special key (ENTER, DOWN, UP, LEFT, RIGHT, PERIOD, COMMA) and `{SHIFT+c}` a character
    # with Shift held, both through send-keys-event with ZEsarUX's own key numbers (utils.h, util_teclas).
    named = {"ENTER": 129, "LEFT": 142, "RIGHT": 143, "DOWN": 144, "UP": 145, "PERIOD": 149, "COMMA": 148}
    shift_l = 133
    steps = []
    for part in re.split(r"(\{[^}]*\})", keys.replace("\\n", "{ENTER}")):
        if part.startswith("{"):
            name = part[1:-1]
            if name.upper().startswith("SHIFT+"):
                steps.append([shift_l, ord(name[6:])])
            else:
                steps.append([named[name.upper()]])
        elif part:
            steps.append(("text", part))
    for step in steps:
        if step[0] == "text":
            z.cmd(f"send-keys-ascii {key_ms} " + " ".join(str(ord(c)) for c in step[1]))
            time.sleep(len(step[1]) * key_ms * 3 / 1000)
        else:
            # a short read wait, or the answer's 0.6 s quiet period would be the time the key is held
            for k in step:
                z.cmd(f"send-keys-event {k} 1", 0.03)
                time.sleep(key_ms / 1000)
            for k in step[::-1]:
                z.cmd(f"send-keys-event {k} 0", 0.03)
                time.sleep(key_ms / 1000)
    time.sleep(after)


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

    try:  # an older ZEsarUX still listening would answer in place of the one started below: a golden of the wrong machine
        socket.create_connection(("127.0.0.1", a.port), timeout=1).close()
    except OSError:
        pass
    else:
        raise SystemExit(f"a ZEsarUX is already listening on port {a.port}; close it (or pass --port) first")
    proc = subprocess.Popen(cmd, cwd=os.path.dirname(exe), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        z = Zrcp(a.port)
        if a.load:
            time.sleep(2)  # let the machine boot, then load like the user's File > Smart load
            z.cmd(f"smartload {os.path.abspath(a.load)}")
        time.sleep(a.seconds)
        if a.keys:
            type_keys(z, a.keys, a.key_ms, a.after)
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
