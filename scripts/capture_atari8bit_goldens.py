#!/usr/bin/env python3
"""Captures the Atari 8-bit manifests' goldens (machines/atari/atari-{800,800xl,65xe,130xe}.manifest.toml) with Atari800 7.2.1.

    capture_atari8bit_goldens.py MACHINE [GOLDEN_SUBSTRING]

For every distinct golden of the manifest (the first scenario that uses it decides region, frames, media and memory range) it runs
capture_atari800_screen.py and compares the SHA-256 of the result with the manifest's `golden_sha256`, printing MATCH or DIFFERS.
Paths (overridable): RC_REPO (the retroclawd checkout, default ../retroclawd), RC_GOLDENS (default ../goldens), RC_ATARI800 (atari800.exe).
Needs the replacement ROMs in machines/atari/roms (atari800_roms_from_source.py), the scenario media (machines/atari/scenarios/build.sh) and,
for S7, software/diagnostics/acid800.atr. Windows only, like the capture adapter. Standard library only."""
import hashlib
import os
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = Path(os.environ.get("RC_REPO", HERE.parent.parent / "retroclawd"))
GOLD = Path(os.environ.get("RC_GOLDENS", ROOT.parent / "goldens"))
A800 = os.environ.get("RC_ATARI800", "atari800.exe")
ATARI = ROOT / "machines" / "atari"
CAPTURE = HERE / "capture_atari800_screen.py"


machine = sys.argv[1]
only = sys.argv[2] if len(sys.argv) > 2 else ""
manifest = tomllib.loads((ATARI / f"{machine}.manifest.toml").read_text())
rom_dir = "atari800" if machine == "atari-800" else "atarixl"
os_rom = ATARI / "roms" / rom_dir / ("altirraos_800.rom" if machine == "atari-800" else "altirraos_xl.rom")
basic = ATARI / "roms" / rom_dir / "altirra_basic.rom"
media = ROOT / "machines"

seen = set()
for s in manifest["scenario"]:
    golden = s.get("golden")
    if not golden or golden in seen or only not in golden:
        continue
    seen.add(golden)
    args = s.get("args", [])
    region = args[args.index("--region") + 1] if "--region" in args else "pal"
    cmd = [sys.executable, "-I", str(CAPTURE), str(os_rom), "", "--machine", machine, "--basic", str(basic),
           "--atari800", A800, "--region", region, "--frames", str(s["frames"])]
    work = Path(tempfile.mkdtemp(prefix="a8gold-"))
    target = GOLD / golden
    target.parent.mkdir(parents=True, exist_ok=True)
    files = s.get("files", {})
    if "program" in files:
        cmd += ["--program", str(media / files["program"]), "--org", args[args.index("--org") + 1]]
    if "disk" in files:
        cmd += ["--disk", str(media / files["disk"])]
    if "cartridge" in files:
        cmd += ["--cartridge", str(media / files["cartridge"])]
    if s["kind"] == "memory":
        cmd[cmd.index("") ] = str(work / "shot.png")
        cmd += ["--memory", f"{s['start']}:{s['len']}", "--memory-out", str(target)]
    else:
        cmd[cmd.index("")] = str(target)
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=1800, cwd=work)
    ok = target.is_file()
    got = hashlib.sha256(target.read_bytes()).hexdigest() if ok else "-"
    pin = s.get("golden_sha256", "-")
    print(f"{golden:36} {'MATCH' if got == pin else 'DIFFERS' if ok else 'NO FILE'}", "" if got == pin else f"{got[:12]} vs {pin[:12]} {r.stderr[-200:].strip()}")
