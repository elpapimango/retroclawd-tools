#!/usr/bin/env python3
"""Captures the VIC-20 manifest's goldens (machines/commodore/vic20.manifest.toml) with VICE's xvic.

    capture_vic20_goldens.py KIND REGION        KIND: s1 s2 program tape disk cartridge s7-cartridge s7-kernal
                                                REGION: pal ntsc

Each run is bounded by `-limitcycles` (no wall-clock waiting), must log "cycle limit reached", and writes into the
goldens folder: S1/S2 as the 176x184 logical-pixel text image (VICE's doubled columns removed), S3 as the 8x8 glyph,
S7 as the raw 1024-byte dump of $0000-$03ff. Print the SHA-256 and compare it with the manifest's `golden_sha256`.

Paths (all overridable): RC_VICE_XVIC (xvic executable), RC_REPO (the retroclawd checkout, default ../retroclawd),
RC_GOLDENS (default ../goldens). Needs the VIC-20 ROMs in machines/commodore/roms/vic20, the scenario media
(machines/commodore/scenarios/build.sh) and, for S7, tools/fetch_diagnostics.sh. Tested with VICE 3.10, whose pinned
text goldens are byte-identical to the 3.9 ones; two options differ from 3.9: `+trapdeviceN` is the old `+virtualdevN`.
Standard library only. Pairs with capture_vice_screen.py (the PNG codec)."""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from capture_vice_screen import decode_png, encode_png  # noqa: E402

REPO = Path(os.environ.get("RC_REPO", HERE.parent.parent / "retroclawd"))
GOLDENS = Path(os.environ.get("RC_GOLDENS", REPO.parent / "goldens")) / "vic20"
XVIC = os.environ.get("RC_VICE_XVIC", "xvic")
COMMODORE = REPO / "machines" / "commodore"
ROMS = COMMODORE / "roms" / "vic20"
MEDIA = COMMODORE / "software" / "scenarios"
DIAGNOSTICS = COMMODORE / "software" / "diagnostics"
DOS_1541 = COMMODORE / "roms" / "1541" / "dos1541-325302-01+901229-05.bin"

BOOT = {"pal": 3987360, "ntsc": 3053700}  # 180 frames
CYCLES = {
    "s1": BOOT,
    "s2": {"pal": 11076000, "ntsc": 8482500},  # 500 frames
    "program": BOOT,
    "cartridge": BOOT,
    "tape": {"pal": 48734400, "ntsc": 37323000},
    "disk": {"pal": 26582400, "ntsc": 20358000},
    "s7-cartridge": {"pal": 44304000, "ntsc": 33930000},
    "s7-kernal": {"pal": 44304000, "ntsc": 33930000},
}
TEXT_ORIGIN = {"pal": (96, 48), "ntsc": (40, 22)}  # the 176x184 text area in the raw screenshot
GLYPH_ORIGIN = (96, 48)  # the 8x8 test glyph is at the same raw position in both regions
DRIVES_OFF = ["-drive8type", "0", "-drive9type", "0", "-drive10type", "0", "-drive11type", "0"]


def logical(raw: Path, origin, width: int, height: int) -> bytes:
    """The crop of the raw screenshot with each identical pair of columns reduced to one."""
    left, top = origin
    _, _, rows = decode_png(raw.read_bytes())
    out = []
    for row in rows[top : top + height]:
        row = row[3 * left : 3 * (left + 2 * width)]
        pixels = [row[x : x + 3] for x in range(0, len(row), 3)]
        if pixels[::2] != pixels[1::2]:
            sys.exit("the doubled columns differ: the crop is not on the picture")
        out.append(b"".join(pixels[::2]))
    return encode_png(width, height, out)


def main() -> None:
    if len(sys.argv) != 3 or sys.argv[1] not in CYCLES or sys.argv[2] not in BOOT:
        sys.exit(__doc__)
    kind, region = sys.argv[1], sys.argv[2]
    work = Path(tempfile.mkdtemp(prefix="vic20-golden-"))
    raw, log = work / "raw.png", work / "vice.log"
    kernal = ROMS / ("kernal-901486-07.bin" if region == "pal" else "kernal-901486-06.bin")
    cmd = [
        XVIC, "-default", "-console", f"-{region}",
        "-basic", str(ROMS / "basic-901486-01.bin"), "-chargen", str(ROMS / "chargen-901460-03.bin"),
        "+sound", "-warp", "-limitcycles", str(CYCLES[kind][region]),
        "-exitscreenshot", str(raw), "-logfile", str(log),
    ]
    for name in ("vic20-pattern.prg", "vic20-pattern.a0", "vic20-pattern.d64", f"vic20-pattern-{region}.tap"):
        if (MEDIA / name).is_file():
            shutil.copy(MEDIA / name, work / name)
    dump = work / "memory.bin"
    if kind == "s1":
        cmd += ["-kernal", str(kernal)] + DRIVES_OFF
    elif kind == "s2":
        cmd += ["-kernal", str(kernal)] + DRIVES_OFF + ["-keybuf", "print 12*34\n"]
    elif kind == "program":
        (work / "program.mon").write_text('load "vic20-pattern.prg" 0\ng 1000\n')
        cmd += ["-kernal", str(kernal)] + DRIVES_OFF + ["-initbreak", "ready", "-moncommands", "program.mon"]
    elif kind == "tape":
        (work / "cassette.mon").write_text("tapectrl 1\ng\n")
        cmd += ["-kernal", str(kernal)] + DRIVES_OFF + [
            "+trapdevice1", "-1", f"vic20-pattern-{region}.tap",
            "-initbreak", "ready", "-moncommands", "cassette.mon",
            "-keybuf", 'load"pattern",1,1\nsys4608\n',
        ]
    elif kind == "disk":
        cmd += ["-kernal", str(kernal)] + [
            "-drive8type", "1541", "-drive9type", "0", "-drive10type", "0", "-drive11type", "0",
            "-dos1541", str(DOS_1541), "-drive8truedrive", "+trapdevice8", "-8", "vic20-pattern.d64",
            "-keybuf", 'load"pattern",8,1\nsys4608\n',
        ]
    elif kind == "cartridge":
        cmd += ["-kernal", str(kernal)] + DRIVES_OFF + ["-cartA", "vic20-pattern.a0"]
    else:  # the dead test; VICE is stopped at its success loop only to read memory
        (work / "dump.mon").write_text(f'bsave "{dump.as_posix()}" 0 $0000 $03ff\ndelete 1\nx\n')
        cmd += DRIVES_OFF + ["-moncommands", str(work / "dump.mon")]
        if kind == "s7-cartridge":
            cmd += ["-kernal", str(kernal), "-cartA", str(DIAGNOSTICS / "vic20-dead-test-v1.1.01.a0"), "-initbreak", "0xa1ca"]
        else:
            cmd += ["-kernal", str(DIAGNOSTICS / f"vic20-dead-test-v1.1.01.{region}.e0"), "-memory", "all", "-initbreak", "0xe1c1"]
    # The cycle limit makes VICE exit with a nonzero status; the log says whether it was the limit.
    subprocess.run(cmd, capture_output=True, text=True, timeout=1800, cwd=work, check=False)
    text = log.read_text(errors="replace") if log.is_file() else ""
    if "cycle limit reached" not in text:
        sys.exit(f"VICE did not reach the cycle limit:\n{text[-500:]}")
    GOLDENS.mkdir(parents=True, exist_ok=True)
    if kind in ("s1", "s2"):
        target, data = GOLDENS / f"S{kind[1]}-{region}.png", logical(raw, TEXT_ORIGIN[region], 176, 184)
    elif kind.startswith("s7"):
        if not dump.is_file() or dump.stat().st_size != 1024:
            sys.exit("the monitor did not write the 1024-byte dump")
        target, data = GOLDENS / f"S7-{kind[3:]}-{region}.bin", dump.read_bytes()
    else:
        target, data = GOLDENS / f"S3-{kind}-{region}.png", logical(raw, GLYPH_ORIGIN, 8, 8)
    target.write_bytes(data)
    print(target, hashlib.sha256(data).hexdigest())


main()
