#!/usr/bin/env python3
"""Writes a ZX81 `.p` file holding one BASIC program: a PRINT of TEXT on line LINE.

    make_zx81_p.py OUT.p [--text HELLO] [--line 10]

Layout (ZXDOCS "Cassette File Images"): memory from 4009h to E_LINE-1, system variables first, VERSN = 0, NXTLIN on the
first line so the program starts by itself, then the program, the collapsed display file (25 HALTs, SLOW mode) and the
end-of-variables marker. The same layout the `rc-suite` ZX81 cases build in Rust. Standard library only.
"""
import argparse
import struct

PROGRAM = 0x407D


def zx81_code(c: str) -> int:
    if c == " ":
        return 0x00
    if c.isdigit():
        return 0x1C + int(c)
    if c.isalpha():
        return 0x26 + ord(c.upper()) - ord("A")
    raise SystemExit(f"no ZX81 code for {c!r}")


def p_file(line: int, text: str) -> bytes:
    body = bytes([0xF5, 0x0B]) + bytes(zx81_code(c) for c in text) + bytes([0x0B])  # PRINT "text"
    program = struct.pack(">H", line) + struct.pack("<H", len(body) + 1) + body + b"\x76"
    d_file = PROGRAM + len(program)
    vars_ = d_file + 25
    e_line = vars_ + 1
    f = bytearray(PROGRAM - 0x4009)

    def word(addr: int, value: int) -> None:
        f[addr - 0x4009 : addr - 0x4009 + 2] = struct.pack("<H", value)

    word(0x400C, d_file)
    word(0x400E, d_file + 1)
    word(0x4010, vars_)
    word(0x4014, e_line)
    word(0x4016, e_line - 1)
    word(0x401A, e_line + 2)
    word(0x401C, e_line + 2)
    word(0x401F, 0x405D)
    word(0x4025, 0xFFFF)
    word(0x4029, PROGRAM)
    word(0x4030, 0x0C8D)
    word(0x4034, 0xFFFF)
    f[0x4022 - 0x4009] = 2
    f[0x4028 - 0x4009] = 55
    f[0x4038 - 0x4009] = 0xBC
    f[0x403B - 0x4009] = 0x40
    return bytes(f) + program + b"\x76" * 25 + b"\x80"


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--text", default="HELLO")
    ap.add_argument("--line", type=int, default=10)
    a = ap.parse_args()
    open(a.out, "wb").write(p_file(a.line, a.text))
