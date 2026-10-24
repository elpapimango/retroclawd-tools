#!/usr/bin/env python3
"""A minimal libretro frontend: run a core on a ROM for N frames and write what it produced.

Used as the black-box oracle for the Mega Drive work (Genesis Plus GX, PicoDrive, BlastEm, clownmdemu loaded from a
RetroArch installation), and usable with any libretro core.

    libretro_run.py CORE.dll ROM --frames N [--system-dir D] [--option key=value ...]
        [--input script.txt] [--out-frame F.png] [--out-ram R.bin] [--out-audio A.wav]
        [--out-state S.bin] [--frame-at N:file.png ...] [--ram-at N:file.bin ...] [--info]

The input script has one line per change: `FRAME BUTTON[,BUTTON...]` (RetroPad names: b y select start up down left right
a x l r; `-` for none). The buttons stay held from FRAME until the next line. Port 1 only.

--out-ram writes RETRO_MEMORY_SYSTEM_RAM (see --swap16) after the last frame, --ram-at writes it after frame N (1-based).
Core, ROM and system files are untrusted input to this script: nothing in them is printed.
"""

import argparse
import ctypes
import os
import struct
import sys
import zlib

RETRO_API_VERSION = 1
PIXEL_0RGB1555, PIXEL_XRGB8888, PIXEL_RGB565 = 0, 1, 2
MEMORY_SAVE_RAM, MEMORY_RTC, MEMORY_SYSTEM_RAM, MEMORY_VIDEO_RAM = 0, 1, 2, 3

ENV_SET_ROTATION = 1
ENV_GET_OVERSCAN = 2
ENV_GET_CAN_DUPE = 3
ENV_SET_MESSAGE = 6
ENV_SHUTDOWN = 7
ENV_SET_PERFORMANCE_LEVEL = 8
ENV_GET_SYSTEM_DIRECTORY = 9
ENV_SET_PIXEL_FORMAT = 10
ENV_SET_INPUT_DESCRIPTORS = 11
ENV_SET_KEYBOARD_CALLBACK = 12
ENV_GET_VARIABLE = 15
ENV_SET_VARIABLES = 16
ENV_GET_VARIABLE_UPDATE = 17
ENV_SET_SUPPORT_NO_GAME = 18
ENV_GET_LOG_INTERFACE = 27
ENV_GET_SAVE_DIRECTORY = 31
ENV_SET_SYSTEM_AV_INFO = 32
ENV_SET_CONTROLLER_INFO = 35
ENV_SET_GEOMETRY = 37
ENV_GET_LANGUAGE = 39

BUTTON_IDS = {
    "b": 0, "y": 1, "select": 2, "start": 3, "up": 4, "down": 5, "left": 6, "right": 7,
    "a": 8, "x": 9, "l": 10, "r": 11,
}


class GameInfo(ctypes.Structure):
    _fields_ = [("path", ctypes.c_char_p), ("data", ctypes.c_void_p), ("size", ctypes.c_size_t), ("meta", ctypes.c_char_p)]


class SystemInfo(ctypes.Structure):
    _fields_ = [("name", ctypes.c_char_p), ("version", ctypes.c_char_p), ("exts", ctypes.c_char_p),
                ("need_fullpath", ctypes.c_bool), ("block_extract", ctypes.c_bool)]


class GameGeometry(ctypes.Structure):
    _fields_ = [("base_width", ctypes.c_uint), ("base_height", ctypes.c_uint), ("max_width", ctypes.c_uint),
                ("max_height", ctypes.c_uint), ("aspect_ratio", ctypes.c_float)]


class SystemTiming(ctypes.Structure):
    _fields_ = [("fps", ctypes.c_double), ("sample_rate", ctypes.c_double)]


class AvInfo(ctypes.Structure):
    _fields_ = [("geometry", GameGeometry), ("timing", SystemTiming)]


class Variable(ctypes.Structure):
    _fields_ = [("key", ctypes.c_char_p), ("value", ctypes.c_char_p)]


ENV_CB = ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_uint, ctypes.c_void_p)
VIDEO_CB = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint, ctypes.c_size_t)
AUDIO_CB = ctypes.CFUNCTYPE(None, ctypes.c_int16, ctypes.c_int16)
AUDIO_BATCH_CB = ctypes.CFUNCTYPE(ctypes.c_size_t, ctypes.POINTER(ctypes.c_int16), ctypes.c_size_t)
INPUT_POLL_CB = ctypes.CFUNCTYPE(None)
INPUT_STATE_CB = ctypes.CFUNCTYPE(ctypes.c_int16, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint)


def png_bytes(width, height, rgb):
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    rows = b"".join(b"\x00" + rgb[y * width * 3:(y + 1) * width * 3] for y in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 6)) + chunk(b"IEND", b""))


def wav_bytes(samples, rate):
    data = struct.pack("<%dh" % len(samples), *samples)
    return (b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 2, rate, rate * 4, 4, 16)
            + b"data" + struct.pack("<I", len(data)) + data)


def to_rgb(data, width, height, pitch, fmt):
    raw = ctypes.string_at(data, pitch * height)
    out = bytearray(width * height * 3)
    if fmt == PIXEL_XRGB8888:
        for y in range(height):
            row = raw[y * pitch:y * pitch + width * 4]
            o = y * width * 3
            out[o:o + width * 3:3] = row[2::4]
            out[o + 1:o + width * 3:3] = row[1::4]
            out[o + 2:o + width * 3:3] = row[0::4]
    elif fmt == PIXEL_RGB565:
        for y in range(height):
            vals = struct.unpack_from("<%dH" % width, raw, y * pitch)
            o = y * width * 3
            for x, v in enumerate(vals):
                r, g, b = (v >> 11) & 31, (v >> 5) & 63, v & 31
                out[o + 3 * x] = (r << 3) | (r >> 2)
                out[o + 3 * x + 1] = (g << 2) | (g >> 4)
                out[o + 3 * x + 2] = (b << 3) | (b >> 2)
    else:
        for y in range(height):
            vals = struct.unpack_from("<%dH" % width, raw, y * pitch)
            o = y * width * 3
            for x, v in enumerate(vals):
                r, g, b = (v >> 10) & 31, (v >> 5) & 31, v & 31
                out[o + 3 * x] = (r << 3) | (r >> 2)
                out[o + 3 * x + 1] = (g << 3) | (g >> 2)
                out[o + 3 * x + 2] = (b << 3) | (b >> 2)
    return bytes(out)


def parse_input(path):
    steps = []
    if not path:
        return steps
    for line in open(path, encoding="utf-8"):
        line = line.split("#")[0].strip()
        if not line:
            continue
        frame, buttons = line.split(None, 1)
        mask = 0
        if buttons.strip() != "-":
            for name in buttons.replace(" ", "").split(","):
                mask |= 1 << BUTTON_IDS[name.lower()]
        steps.append((int(frame), mask))
    return sorted(steps)


class Frontend:
    def __init__(self, core_path, system_dir, options):
        os.add_dll_directory(os.path.dirname(os.path.abspath(core_path)))
        self.core = ctypes.CDLL(os.path.abspath(core_path))
        self.system_dir = os.path.abspath(system_dir).encode()
        self.save_dir = self.system_dir
        self.options = {k: v.encode() for k, v in options.items()}
        self.pixel_format = PIXEL_0RGB1555
        self.frame = None
        self.audio = []
        self.pad = 0
        self.keep = []
        self.swap16 = False
        self.core.retro_get_memory_data.restype = ctypes.c_void_p
        self.core.retro_get_memory_size.restype = ctypes.c_size_t
        self.core.retro_serialize_size.restype = ctypes.c_size_t
        self.core.retro_get_memory_data.argtypes = [ctypes.c_uint]
        self.core.retro_get_memory_size.argtypes = [ctypes.c_uint]
        self.core.retro_serialize.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        self.core.retro_serialize.restype = ctypes.c_bool

        @ENV_CB
        def env(cmd, data):
            cmd &= 0xFFFF
            if cmd == ENV_GET_SYSTEM_DIRECTORY:
                ctypes.cast(data, ctypes.POINTER(ctypes.c_char_p))[0] = self.system_dir
                return True
            if cmd == ENV_GET_SAVE_DIRECTORY:
                ctypes.cast(data, ctypes.POINTER(ctypes.c_char_p))[0] = self.save_dir
                return True
            if cmd == ENV_SET_PIXEL_FORMAT:
                self.pixel_format = ctypes.cast(data, ctypes.POINTER(ctypes.c_int))[0]
                return True
            if cmd == ENV_GET_VARIABLE:
                var = ctypes.cast(data, ctypes.POINTER(Variable))[0]
                value = self.options.get(var.key.decode())
                if value is None:
                    return False
                self.keep.append(value)
                var.value = value
                ctypes.cast(data, ctypes.POINTER(Variable))[0] = var
                return True
            if cmd == ENV_GET_VARIABLE_UPDATE:
                ctypes.cast(data, ctypes.POINTER(ctypes.c_bool))[0] = False
                return True
            if cmd in (ENV_GET_CAN_DUPE,):
                ctypes.cast(data, ctypes.POINTER(ctypes.c_bool))[0] = True
                return True
            if cmd in (ENV_SET_VARIABLES, ENV_SET_INPUT_DESCRIPTORS, ENV_SET_CONTROLLER_INFO, ENV_SET_GEOMETRY,
                       ENV_SET_SYSTEM_AV_INFO, ENV_SET_PERFORMANCE_LEVEL, ENV_SET_SUPPORT_NO_GAME, ENV_SET_ROTATION,
                       ENV_SET_MESSAGE, ENV_SET_KEYBOARD_CALLBACK):
                return True
            if cmd == ENV_GET_LANGUAGE:
                ctypes.cast(data, ctypes.POINTER(ctypes.c_uint))[0] = 0
                return True
            return False

        @VIDEO_CB
        def video(data, width, height, pitch):
            if data:
                self.frame = (width, height, to_rgb(data, width, height, pitch, self.pixel_format))

        @AUDIO_CB
        def audio(left, right):
            self.audio.extend((left, right))

        @AUDIO_BATCH_CB
        def audio_batch(data, frames):
            self.audio.extend(data[i] for i in range(frames * 2))
            return frames

        @INPUT_POLL_CB
        def poll():
            pass

        @INPUT_STATE_CB
        def state(port, device, index, ident):
            if port == 0 and device == 1:
                return (self.pad >> ident) & 1
            return 0

        self.callbacks = (env, video, audio, audio_batch, poll, state)
        self.core.retro_set_environment(env)
        self.core.retro_set_video_refresh(video)
        self.core.retro_set_audio_sample(audio)
        self.core.retro_set_audio_sample_batch(audio_batch)
        self.core.retro_set_input_poll(poll)
        self.core.retro_set_input_state(state)
        self.core.retro_init()

    def load(self, rom_path):
        info = SystemInfo()
        self.core.retro_get_system_info(ctypes.byref(info))
        game = GameInfo()
        self.rom_bytes = open(rom_path, "rb").read()
        self.rom_buf = ctypes.create_string_buffer(self.rom_bytes, len(self.rom_bytes))
        game.path = os.path.abspath(rom_path).encode()
        if not info.need_fullpath:
            game.data = ctypes.cast(self.rom_buf, ctypes.c_void_p)
            game.size = len(self.rom_bytes)
        if not self.core.retro_load_game(ctypes.byref(game)):
            raise SystemExit("the core refused the ROM")
        av = AvInfo()
        self.core.retro_get_system_av_info(ctypes.byref(av))
        self.av = av

    def ram(self, kind=MEMORY_SYSTEM_RAM):
        ptr = self.core.retro_get_memory_data(kind)
        size = self.core.retro_get_memory_size(kind)
        data = ctypes.string_at(ptr, size) if ptr and size else b""
        if self.swap16:
            swapped = bytearray(data)
            swapped[0::2], swapped[1::2] = data[1::2], data[0::2]
            data = bytes(swapped)
        return data

    def state(self):
        size = self.core.retro_serialize_size()
        buf = ctypes.create_string_buffer(size)
        if not self.core.retro_serialize(buf, size):
            return b""
        return buf.raw

    def run(self):
        self.core.retro_run()


def write(path, data):
    with open(path, "wb") as f:
        f.write(data)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("core")
    ap.add_argument("rom", nargs="?")
    ap.add_argument("--frames", type=int, default=60)
    ap.add_argument("--system-dir", default=".")
    ap.add_argument("--option", action="append", default=[])
    ap.add_argument("--input")
    ap.add_argument("--out-frame")
    ap.add_argument("--out-ram")
    ap.add_argument("--out-audio")
    ap.add_argument("--out-state")
    ap.add_argument("--frame-at", action="append", default=[])
    ap.add_argument("--ram-at", action="append", default=[])
    ap.add_argument("--info", action="store_true")
    ap.add_argument("--swap16", action="store_true",
                    help="swap the two bytes of every word of the RAM dumps (the Mega Drive cores expose the big-endian "
                         "68000 RAM with the bytes of each word swapped; this gives the 68000's own byte order)")
    args = ap.parse_args()
    options = dict(o.split("=", 1) for o in args.option)
    fe = Frontend(args.core, args.system_dir, options)
    fe.swap16 = args.swap16
    if args.info or not args.rom:
        info = SystemInfo()
        fe.core.retro_get_system_info(ctypes.byref(info))
        print(info.name.decode(), info.version.decode())
        if not args.rom:
            return
    fe.load(args.rom)
    if args.info:
        print("fps %.6f rate %.1f %dx%d" % (fe.av.timing.fps, fe.av.timing.sample_rate, fe.av.geometry.base_width,
                                            fe.av.geometry.base_height))
    steps = parse_input(args.input)
    frame_at = {int(s.split(":", 1)[0]): s.split(":", 1)[1] for s in args.frame_at}
    ram_at = {int(s.split(":", 1)[0]): s.split(":", 1)[1] for s in args.ram_at}
    for n in range(1, args.frames + 1):
        for frame, mask in steps:
            if frame <= n:
                fe.pad = mask
        fe.run()
        if n in frame_at and fe.frame:
            write(frame_at[n], png_bytes(*fe.frame[:2], fe.frame[2]))
        if n in ram_at:
            write(ram_at[n], fe.ram())
    if args.out_frame and fe.frame:
        write(args.out_frame, png_bytes(*fe.frame[:2], fe.frame[2]))
    if args.out_ram:
        write(args.out_ram, fe.ram())
    if args.out_audio:
        write(args.out_audio, wav_bytes(fe.audio, int(fe.av.timing.sample_rate)))
    if args.out_state:
        write(args.out_state, fe.state())


if __name__ == "__main__":
    sys.exit(main())
