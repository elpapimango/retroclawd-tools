#!/usr/bin/env python3
"""Captures a golden picture from Caprice32 (4.6+): boots a CPC MODEL with the given media, types TEXT, waits FRAMES
emulated frames, takes Caprice32's own screenshot and copies it to OUT.png. Then pin it:  rc-scenarios hash OUT.png
and put the hash in the manifest's `golden_sha256`, with the Caprice32 version and this command next to the scenario.

    capture_caprice_screen.py MODEL FRAMES OUT.png [--keys TEXT] [--cap32 PATH] [--rom-path DIR] [-O KEY=VALUE]... [media...]

MODEL is Caprice32's `system.model`: 0 = CPC464, 1 = CPC664, 2 = CPC6128. The wait is Caprice32's CAP32_DELAY
autotype command with `boot_time` set to FRAMES, so it counts emulated frames after the typing, not wall time, and
the emulator runs unthrottled. Caprice32's CRTC answers as a type 0 (the CPC diagnostics read 00 on its CPC6128). `--cap32` defaults to
`cap32` on PATH; `--rom-path` to the `rom/` folder next to it. `-O` passes a Caprice32 config override through. On the
CPC664 and CPC6128 AMSDOS is put in upper ROM slot 7 (`rom.slot07=amsdos.rom`): with the shipped `slot07=DEFAULT` this
build ran `RUN"` from tape. Standard library only.
"""
import argparse
import glob
import os
import shutil
import subprocess
import sys
import tempfile


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("model", type=int, choices=[0, 1, 2])
    ap.add_argument("frames", type=int)
    ap.add_argument("out")
    ap.add_argument("media", nargs="*")
    ap.add_argument("--keys", default="")
    ap.add_argument("--cap32", default=shutil.which("cap32") or "cap32")
    ap.add_argument("--rom-path")
    ap.add_argument("-O", dest="overrides", action="append", default=[])
    a = ap.parse_args()
    rom_path = a.rom_path or os.path.join(os.path.dirname(os.path.abspath(a.cap32)), "rom")
    with tempfile.TemporaryDirectory() as shots:
        cmd = [
            a.cap32,
            "-O", f"system.model={a.model}",
            "-O", f"system.boot_time={a.frames}",
            "-O", "system.limit_speed=0",
            "-O", "video.scr_fps=0",
            "-O", f"file.sdump_dir={shots}",
            "-O", f"rom.rom_path={rom_path}{os.sep}",
        ]
        if a.model in (1, 2) and not any(o.startswith("rom.slot07=") for o in a.overrides):
            cmd += ["-O", "rom.slot07=amsdos.rom"]
        for o in a.overrides:
            cmd += ["-O", o]
        if a.keys:
            cmd += ["-a", a.keys]
        cmd += ["-a", "CAP32_DELAY", "-a", "CAP32_SCRNSHOT", "-a", "CAP32_EXIT", *a.media]
        subprocess.run(cmd, check=False, timeout=600, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        found = sorted(glob.glob(os.path.join(shots, "*.png")))
        if not found:
            print("Caprice32 wrote no screenshot", file=sys.stderr)
            return 1
        shutil.copyfile(found[-1], a.out)
    print(a.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
