#!/usr/bin/env python3
"""Builds the three Altirra replacement ROMs the Atari 8-bit manifests use from the Atari800 7.2.1 C sources.

    atari800_roms_from_source.py SRC_DIR OUT_DIR

SRC_DIR holds altirraos_800.c, altirraos_xl.c and altirra_basic.c from
https://raw.githubusercontent.com/atari800/atari800/63a381884175e3cbfc745ff00dfb3d70d928eb50/src/roms/ (fetch them with curl into
a folder of their own). Each array is read as bytes and written only if its size and SHA-1 equal the pins of
machines/atari/firmware.toml, so a wrong or altered download writes nothing. OUT_DIR is machines/atari/roms: the OS 800 and BASIC go
to atari800/, the XL/XE OS and BASIC to atarixl/. Avery Lee's permissive licence (copying with the notice kept) covers these files.
Standard library only."""
import hashlib
import re
import sys
from pathlib import Path

ROMS = [  # source, size, sha1, destinations under OUT_DIR
    ("altirraos_800.c", 10240, "24b16053b5a11ed2f35f208c9a061d973b8be3e7", ["atari800/altirraos_800.rom"]),
    ("altirraos_xl.c", 16384, "738b9590a29f424ad2c9d3f9cb1d170de79241f7", ["atarixl/altirraos_xl.rom"]),
    ("altirra_basic.c", 8192, "95778af5688ec180baa9db364577d113bfc189ea", ["atari800/altirra_basic.rom", "atarixl/altirra_basic.rom"]),
]


def array_bytes(path: Path) -> bytes:
    text = path.read_text(errors="replace")
    body = text[text.index("{", text.index("UBYTE const")) :]
    return bytes(int(h, 16) for h in re.findall(r"0x([0-9a-fA-F]{2})", body[: body.index("};")]))


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    src, out = Path(sys.argv[1]), Path(sys.argv[2])
    bad = False
    for name, size, sha1, targets in ROMS:
        data = array_bytes(src / name)
        ok = len(data) == size and hashlib.sha1(data).hexdigest() == sha1
        print(f"{name}: {len(data)} bytes, {hashlib.sha1(data).hexdigest()} {'OK' if ok else 'MISMATCH'}")
        if not ok:
            bad = True
            continue
        for target in targets:
            (out / target).parent.mkdir(parents=True, exist_ok=True)
            (out / target).write_bytes(data)
    sys.exit(1 if bad else 0)


main()
