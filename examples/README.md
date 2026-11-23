# Examples

Every folder is a project: `rcb build` in it makes the image under `build/`, `rcb run` starts the emulator on it.
They are skeletons made by `rcb new` (a loop or a return with the vectors the machine needs), meant to be read and grown.

| Folder | Machine | Image |
|---|---|---|
| `c64-hello` | Commodore 64 | `.prg` with a `SYS` line |
| `c64-two-files` | Commodore 64 | two source files assembled separately and linked (`separate = true`) |
| `spectrum-hello` | ZX Spectrum | `.tap` with a BASIC loader |
| `cpc-hello` | Amstrad CPC | `.dsk` |
| `atari-st-hello` | Atari ST | `.prg` |
| `pcengine-hello` | PC Engine | `.pce` |
| `nes-hello` | NES | `.nes` |
| `snes-hello` | Super Nintendo | `.sfc` |
| `genesis-hello` | Mega Drive | `.md` |
| `neogeo-hello` | Neo Geo | `.zip` set |
| `amiga-hello` | Amiga | `.adf` |
| `x68000-hello` | X68000 | `.x` |

```
$ cd examples/nes-hello
$ rcb build
$ rcb run            # needs rcd (the emulator) on the PATH; machines the emulator lacks only build
```

## CMake

`cmake/RetroClawd.cmake` is a module that adds the tools to a CMake build:

| Function | Makes |
|---|---|
| `retroclawd_object(out.rco SOURCE f CPU c [DEFINES A=1] [INCLUDES dir])` | an object from a source (`rcasm -f object`) |
| `retroclawd_library(out.rca OBJECTS a.rco b.rco)` | a library (`rcar create`) |
| `retroclawd_image(target LAYOUT l.toml OUTPUT img [DEPENDS ...] [MAP f] [SYMBOLS f])` | the image from a layout (`rcld build`), as a target that is part of `all` |
| `retroclawd_project(target DIRECTORY dir OUTPUT img)` | an `rcb` project, rebuilt when its `rcb.toml` or `src/` change |

The tools are found on the `PATH`, or in `RETROCLAWD_TOOLS_DIR` (a CMake variable or an environment variable).

- `cmake/c64-library`: a C64 program from `main.asm` and a `screen.asm` library, linked by a layout CMake fills in
  (`configure_file`):

  ```
  cmake -S examples/cmake/c64-library -B build-c64
  cmake --build build-c64            # build-c64/hello.prg, hello.map, hello.sym
  ```

- `cmake/rcb-projects`: the NES, SNES, Mega Drive and PC Engine example projects as CMake targets:

  ```
  cmake -S examples/cmake/rcb-projects -B build-all
  cmake --build build-all
  ```

Mix this with your own C/C++ host-side tools in the same `CMakeLists.txt` (a level editor, a data packer): the
images are ordinary custom-command outputs, so `DEPENDS` on a generated file works as usual.
