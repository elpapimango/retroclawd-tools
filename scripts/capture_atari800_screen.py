#!/usr/bin/env python3
"""Capture Atari800 7.2.1 Win64's native PNG and optional raw memory.

Uses a private config and the documented monitor's alternating BLINE 0/156
breaks: CONT alone repeatedly breaks within the same scanline, not each frame.
No ROM patch, PC change or memory injection occurs for a cold boot. --program
explicitly loads a standalone raw Program through READ and CONT address.

F10 is posted only to the reference process's SDL window. Capture occurs within
two further frames of --frames, so use a stable screen. The PNG is Atari800's
own screenshot, not a desktop grab or a reconstruction from our renderer.
Windows only; standard library only. Source for monitor commands and F10:
https://github.com/atari800/atari800/tree/63a381884175e3cbfc745ff00dfb3d70d928eb50/src
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import pathlib
import queue
import shutil
import subprocess
import sys
import tempfile
import threading


class Monitor:
    def __init__(self, process):
        self.process = process
        self.events = queue.Queue()
        self.transcript = bytearray()
        threading.Thread(target=self._read, daemon=True).start()
        self.prompt()

    def _read(self):
        pending = bytearray()
        while True:
            byte = self.process.stdout.read(1)
            if not byte:
                self.events.put((False, bytes(pending)))
                return
            pending.extend(byte)
            if pending.endswith(b"> "):
                self.events.put((True, bytes(pending)))
                pending.clear()

    def prompt(self):
        try:
            ok, text = self.events.get(timeout=15)
        except queue.Empty as error:
            raise RuntimeError("Atari800 monitor timed out") from error
        self.transcript.extend(text)
        if not ok:
            raise RuntimeError("Atari800 exited before its monitor prompt")
        return text

    def command(self, command):
        self.process.stdin.write(command.encode("ascii") + b"\n")
        self.process.stdin.flush()
        return self.prompt()

    def advance(self, frames):
        for _ in range(frames):
            self.command("BLINE 156")
            self.command("CONT")
            self.command("BLINE 0")
            self.command("CONT")


def screenshot_key(pid):
    user = ctypes.WinDLL("user32", use_last_error=True)
    user.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user.IsWindowVisible.argtypes = [wintypes.HWND]
    user.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    handles = []

    @callback_type
    def visit(handle, _):
        owner = wintypes.DWORD()
        user.GetWindowThreadProcessId(handle, ctypes.byref(owner))
        title = ctypes.create_unicode_buffer(256)
        user.GetWindowTextW(handle, title, len(title))
        if owner.value == pid and user.IsWindowVisible(handle) and title.value.startswith("Atari 800 Emulator"):
            handles.append(handle)
        return True

    if not user.EnumWindows(visit, 0) or len(handles) != 1:
        raise RuntimeError(f"expected one Atari800 SDL window, found {len(handles)}")
    for message, flags in [(0x100, 0x00440001), (0x101, 0xC0440001)]:
        if not user.PostMessageW(handles[0], message, 0x79, flags):
            raise ctypes.WinError(ctypes.get_last_error())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("os_rom", type=pathlib.Path)
    parser.add_argument("out", type=pathlib.Path)
    parser.add_argument("--basic", type=pathlib.Path)
    parser.add_argument("--atari800", default=shutil.which("atari800") or "atari800")
    parser.add_argument("--region", choices=["pal", "ntsc"], default="pal")
    parser.add_argument("--frames", type=int, default=300)
    parser.add_argument("--program", type=pathlib.Path)
    parser.add_argument("--org", type=lambda value: int(value, 0), default=0x2000)
    parser.add_argument("--memory", help="START:LEN, decimal or 0x hexadecimal")
    parser.add_argument("--memory-out", type=pathlib.Path)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("this capture adapter requires Atari800's Windows SDL build")
    if args.frames < 1 or not 0 <= args.org <= 0xffff:
        parser.error("frames must be positive and org must be a 16-bit address")
    if bool(args.memory) != bool(args.memory_out):
        parser.error("--memory and --memory-out must be specified together")
    memory = None
    if args.memory:
        try:
            start, length = (int(value, 0) for value in args.memory.split(":"))
            if not 0 <= start <= 0xffff or not 1 <= length <= 65536 - start:
                raise ValueError()
            memory = (start, length)
        except ValueError:
            parser.error("--memory requires START:LEN within the 16-bit address space")
    os_rom = args.os_rom.resolve(strict=True)
    if os_rom.stat().st_size != 10240:
        parser.error("Atari 800 OS ROM must contain exactly 10240 bytes")
    basic = args.basic.resolve(strict=True) if args.basic else None
    if basic and basic.stat().st_size != 8192:
        parser.error("BASIC ROM must contain exactly 8192 bytes")
    executable = shutil.which(args.atari800)
    if not executable:
        parser.error(f"reference executable not found: {args.atari800}")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rc-atari800-") as directory:
        work = pathlib.Path(directory)
        (work / "reference.cfg").write_text("Atari 800 Emulator, Version 7.2.1\n", encoding="ascii")
        command = [
            executable, "-config", str(work / "reference.cfg"), "-no-autosave-config",
            "-atari", "-" + args.region, "-osb_rom", str(os_rom), "-800-rev", "custom",
            "-nopatchall", "-turbo", "-nosound", "-monitor", "-windowed",
            "-screenshots", "reference.png",
        ]
        command += ["-basic", "-basic_rom", str(basic), "-basic-rev", "custom"] if basic else ["-nobasic"]
        process = subprocess.Popen(command, cwd=work, stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        monitor = None
        try:
            monitor = Monitor(process)
            if args.program:
                program = args.program.resolve(strict=True)
                if program.stat().st_size > 65536 - args.org:
                    raise ValueError("raw Program wraps the 16-bit address space")
                shutil.copyfile(program, work / "program.bin")
                response = monitor.command(f"READ program.bin {args.org:x}")
                if b"Read " not in response:
                    raise RuntimeError("reference did not confirm loading the raw Program")
            monitor.command("BLINE 0")
            monitor.command(f"CONT {args.org:x}" if args.program else "CONT")
            monitor.advance(args.frames)
            if memory:
                start, length = memory
                monitor.command(f"WRITE {start:x} {start + length - 1:x} memory.bin")
                raw = (work / "memory.bin").read_bytes()
                if len(raw) != length:
                    raise RuntimeError("reference memory output has the wrong length")
                args.memory_out.parent.mkdir(parents=True, exist_ok=True)
                args.memory_out.write_bytes(raw)
            screenshot_key(process.pid)
            monitor.advance(2)
            capture = (work / "reference.png").read_bytes()
            if not capture.startswith(b"\x89PNG\r\n\x1a\n"):
                raise RuntimeError("reference did not write a PNG screenshot")
            process.stdin.write(b"QUIT\n")
            process.stdin.flush()
            if process.wait(timeout=10) != 0:
                raise RuntimeError("Atari800 exited unsuccessfully")
            args.out.write_bytes(capture)
            print(f"{args.out}: sha256 {hashlib.sha256(capture).hexdigest()}")
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            if monitor:
                args.out.with_suffix(".monitor.txt").write_bytes(monitor.transcript)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"capture failed: {error}", file=sys.stderr)
        sys.exit(1)
