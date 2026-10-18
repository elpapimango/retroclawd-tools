#!/usr/bin/env python3
"""Captures a golden picture from VICE: runs EMULATOR (x64sc, x128 ...) for CYCLES emulated CPU cycles (deterministic,
no wall-clock waiting: `-limitcycles`, `-warp`), takes its exit screenshot and writes it as a PNG, optionally cropped
onto the canvas of a retroclawd picture. Then pin it:  rc-scenarios hash OUT.png  and put the hash in the manifest's
`golden_sha256`, with the VICE version and this command in the manifest's [reference].

    capture_vice_screen.py EMULATOR CYCLES OUT.png [--keybuf TEXT] [--crop X,Y,W,H] [-- extra VICE arguments, e.g. -8 disk.d64]

`--crop` may reach past the screenshot (negative X or Y, a larger size): the edge pixels are repeated, which is the
border colour. A PAL C64 frame is 19656 cycles. Needs the VICE `bin` folder on PATH. Standard library only.
"""
import argparse
import os
import shutil
import struct
import subprocess
import tempfile
import zlib


def decode_png(data: bytes):
    """Returns (width, height, rows of RGB bytes). Handles the colour types and filters VICE writes."""
    pos, idat, pal = 8, b"", None
    while pos < len(data):
        n, tag = struct.unpack(">I4s", data[pos : pos + 8])
        body = data[pos + 8 : pos + 8 + n]
        if tag == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
        elif tag == b"PLTE":
            pal = body
        elif tag == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    chans = {0: 1, 2: 3, 3: 1, 6: 4}[ctype]
    stride = (w * chans * depth + 7) // 8
    bpp = max(1, chans * depth // 8)
    prev = bytearray(stride)
    rows, at = [], 0
    for _ in range(h):
        ftype, line = raw[at], bytearray(raw[at + 1 : at + 1 + stride])
        at += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if ftype == 1:
                line[x] = (line[x] + a) & 255
            elif ftype == 2:
                line[x] = (line[x] + b) & 255
            elif ftype == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif ftype == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        prev = line
        if ctype == 3:
            if depth == 8:
                idx = list(line[:w])
            elif depth == 4:
                idx = [(line[i // 2] >> (4 * (1 - i % 2))) & 15 for i in range(w)]
            else:
                raise SystemExit(f"unsupported palette depth {depth}")
            rows.append(b"".join(pal[3 * i : 3 * i + 3] for i in idx))
        elif ctype == 2:
            rows.append(bytes(line[: 3 * w]))
        elif ctype == 6:
            rows.append(b"".join(bytes(line[4 * i : 4 * i + 3]) for i in range(w)))
        else:
            rows.append(b"".join(bytes((v, v, v)) for v in line[:w]))
    return w, h, rows


def encode_png(w: int, h: int, rows) -> bytes:
    def chunk(tag: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + r for r in rows)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def crop_rows(w, h, rows, crop):
    cx, cy, cw, ch = crop
    out = []
    for y in range(cy, cy + ch):
        src = rows[min(max(y, 0), h - 1)]
        out.append(b"".join(src[3 * min(max(x, 0), w - 1) : 3 * min(max(x, 0), w - 1) + 3] for x in range(cx, cx + cw)))
    return cw, ch, out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("emulator")
    ap.add_argument("cycles", type=int)
    ap.add_argument("out")
    ap.add_argument("--keybuf", help="typed into the keyboard buffer; \\n is Return")
    ap.add_argument("--crop", help="X,Y,W,H of the screenshot to keep, so the golden has the canvas of a retroclawd picture")
    ap.add_argument("--vicii", action="store_true", help="x128: take the VIC-II (40 column) screen; the default screenshot is the VDC's")
    ap.add_argument("extra", nargs="*")
    a = ap.parse_args()

    exe = shutil.which(a.emulator)
    if not exe:
        raise SystemExit(f"{a.emulator} is not on PATH")
    with tempfile.TemporaryDirectory() as d:
        shot = os.path.join(d, "shot.png")
        cmd = [exe, "-default", "+sound", "-warp", "-limitcycles", str(a.cycles), ("-exitscreenshotvicii" if a.vicii else "-exitscreenshot"), shot]
        if a.keybuf:
            cmd += ["-keybuf", a.keybuf]  # VICE itself reads \n in the text as Return
        cmd += a.extra
        subprocess.run(cmd, cwd=d, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300)
        if not os.path.exists(shot):
            raise SystemExit("VICE wrote no screenshot")
        w, h, rows = decode_png(open(shot, "rb").read())
    if a.crop:
        w, h, rows = crop_rows(w, h, rows, tuple(int(v) for v in a.crop.split(",")))
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    open(a.out, "wb").write(encode_png(w, h, rows))
    print(f"{a.out}: {w}x{h}")


if __name__ == "__main__":
    main()
