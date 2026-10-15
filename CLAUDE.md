# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

`retroclawd-tools`: command-line tools on top of the retroclawd crates. A standalone Cargo workspace (it is **not** a
submodule of the application and the application never builds it). Done: `rcasm` (assembler), `rcdis`
(disassembler), `rcdisk` (disk images), `rctape` (tape images), `rcsnap` (snapshots, cartridges), `rcld` (linker / packager) (linker / packager). The original
brief, with the decisions already taken, is `docs/prompts/retroclawd-tools.md` in the
[`retroclawd`](https://github.com/elpapimango/retroclawd) repository -- except that this repository stands on its
own instead of being a fifth submodule.

## Layout

```
crates/rc-common/   Diag (file:line messages), number parsing, write_file (never overwrites without --force)
crates/rcasm/       lib + bin: expr.rs (expressions), text.rs, cpu.rs (6502 and Z80 encoders), asm.rs (passes,
                    directives, macros, conditionals, includes), lib.rs (flat/prg images, symbol file, listing)
crates/rcdis/       lib + bin: linear and flow disassembly, labels, symbols, `.byte` fallback, rcasm-checked
crates/rcdisk/      lib + bin: a `Disk` trait (lib.rs) with two adapters: cbm.rs (d64/x64/d71/d81/g64 over cbm-dos) and
                    cpm.rs (CPCEMU .dsk over dsk::cpm, AMSDOS/+3DOS headers); main.rs is the CLI
crates/rctape/      lib + bin: tzx.rs (block model: ID + body, byte-exact round trip), spectrum.rs (header, checksum, BASIC
                    lister), c64.rs (.t64, .tap facts over c64-tape), lib.rs (.tap, rows, selection, split, cleanup,
                    extract); main.rs is the CLI
crates/rcsnap/      lib + bin: spectrum.rs (one model for .z80/.sna/.szx, conversions with a list of what is lost), zx81.rs,
                    carts.rs (.crt / .cpr <-> raw), lib.rs (detection, info); main.rs is the CLI
crates/rcld/        lib + bin: layout.rs (TOML layout), place.rs (pieces, overlap / region / reserve checks, map), emit.rs (what
                    each machine and extension gets), basic.rs (the C64 `SYS` line, the Spectrum loader), templates.rs
docs/               rcasm.md, rcdis.md, rcdisk.md, rctape.md, rcsnap.md, rcld.md
```

Each tool is a library crate (reusable) plus its binary. Common code goes into `rc-common`.

## Dependencies

The retroclawd crates are git dependencies pinned by `rev` in the **root `Cargo.toml`** (`[workspace.dependencies]`):
one rev for `retroclawd-crates` (`machine`, `z80`, `mos6502`, `dsk`, later `zx-tape`) and one for `retroclawd-commodore`
(`cbm-dos`, `d64`, `d71`, `d81`, `g64`); if a tool needs something a format crate lacks, add it to that crate in its own
repository and bump the rev here -- no private copies of format logic.

To develop a tool and a crate together, point cargo at local checkouts with a git-ignored `.cargo/config.toml`:

```
[patch."https://github.com/elpapimango/retroclawd-crates"]
machine = { path = "/path/to/retroclawd/shared/crates/machine" }
z80 = { path = "/path/to/retroclawd/shared/crates/core/z80" }
mos6502 = { path = "/path/to/retroclawd/shared/crates/core/6502" }
```

then push the crates commit, put its SHA in the root `Cargo.toml`, regenerate `Cargo.lock` **without** the patch (move
`.cargo/config.toml` away, `cargo build --workspace --all-targets`; the lock written with the patch has no `source` lines
and breaks CI) and restore the patch. The
repositories are public; in CI set `CARGO_NET_GIT_FETCH_WITH_CLI=true`.

## Commands

* `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all`
* ROM-backed tests are skipped unless `RC_ROMS` points at a folder with `sinclair/ amstrad/ commodore/` (the three
  machine repositories side by side, as in the application's `machines/`). The emulator tests also need
  `RETROCLAWD_BIN` (a built `retroclawd`) and `RETROCLAWD_MACHINE_PLUGIN_DIRS`:
  `RC_ROMS=~/retroclawd/machines RETROCLAWD_BIN=~/retroclawd/target/release/retroclawd RETROCLAWD_MACHINE_PLUGIN_DIRS=~/retroclawd/target/release/plugins cargo test --workspace`

## Principles

* **One source of truth for instructions**: the CPU crates' disassemblers. `rcasm` does not keep opcode tables: the
  6502 reuses `assemble_one`, the Z80 builds its encoder by enumerating `z80::disassemble`. `rcasm/tests/roundtrip.rs`
  checks every opcode of both CPUs; `rcdis/tests/roundtrip.rs` checks random bytes and real ROMs byte for byte.
  Extend the crates, not the tools, when an instruction is missing.
* Two passes must agree: the 6502 operand width is decided in pass 1 (unknown symbol => absolute) and pinned for
  pass 2; labels that move between passes are a "phase error".
* Never overwrite an existing output without `--force`; non-zero exit on any error; messages `file:line: text`.
* Windows must keep working (paths, line endings); `.gitattributes` keeps LF where it matters.
* Commit messages end with the attribution lines of the session; do not push to master unless the owner says so.

## Checking disks against the real firmware

`rcdisk`'s CP/M side was verified by booting a CPC6128 (`retroclawd cpc6128 --rom ... --disk x.dsk --plugin
debug-ui:port=N`) under Xvfb and typing `cat`, then `run"disc`, with python-xlib XTest events. Lessons: Xvfb has no
window manager, so give the emulator window the input focus yourself (`window.set_input_focus`), hold each key about
half a second (software rendering drops short presses), and send `"` as Shift+2 (the CPC layout). A screenshot of the
root window shows the CPC screen; `GET /api/memory-bytes?addr=..&len=..` shows memory. The C64 side is an automated
test (`rcdisk/tests/emulator.rs`).

`rctape`'s emulator test (`rctape/tests/emulator.rs`) puts a loader into a saved state first (`--program` and `--tape` do
not go together, `--load-state` and `--tape` do) and lets the ROM's `LD-BYTES` read the tape. Do **not** use
`--tape-speed` above 1 with the ROM loader: the speed-up scales the pulses against the CPU clock, which the ROM's timing
loop cannot decode. `LD-BYTES` also returns an error at a block whose flag is not the one asked for, so a header block
has to be read (flag 0) before the data block.

`rcsnap` depends on the format crates of the families (`retroclawd-sinclair`, `-amstrad`, `-commodore`), pinned by `rev`
in the root `Cargo.toml`. Their Z80 snapshot types use the `retroclawd-crates` revision the families pin: keep the
`retroclawd-crates` `rev` of this workspace equal to the one the pinned family revisions use (`tools/bump_pin.sh` in the
host repository moves the families; then update the revs here), or `z80` appears twice in the graph and the `Cpu` types
differ. A 48K
`.sna` writes the PC onto the stack (the format's design), so a round trip changes two bytes of RAM; the tests know.
`--tape`/`--snapshot` and `--program` do not combine in the emulator; the snapshot tests use `--snapshot` alone.

## Checking BASIC loaders live

`rcld`'s loaders (C64 `10 SYS`, Spectrum `LOAD ""CODE`, CPC `RUN"NAME`) need the BASIC interpreter, so they were checked in
the GUI under Xvfb, with the program writing a marker byte that the debug-ui API reads back
(`--plugin debug-ui:port=N`, `GET /api/memory-bytes?addr=..&len=..`). Notes for repeating it: C64 `--program x.prg --run`
types `RUN` itself; the Spectrum needs `--keymap` with `[keys] KC_RALT = "symbol_shift"` to type `"` (J, Alt+P, Alt+P,
Enter); the CPC types `run"hello` (no dot: the `.` key typed `,` in that session). A Spectrum tape starts when the ROM's `LD-BYTES` is entered (until 2026-10-14 it started at power-on, so
the first block was gone by the time `LOAD ""` was typed and the live check needed a 30 s pause block in front; `--tape-start
any-read` brings the old rule back). Xlib scripts need `win.set_input_focus` (no window
manager) and half-second key holds.

## Open points

* `rcdis` reads raw and `.prg` input only; extracting from a disk or tape image (`rcdis game.d64:NAME`) is still to do.
* `rcdisk`: no `.g64` output, no REL/partition copying, CP/M writing only for the standard 1 KB layout and side 0, no
  BASIC tokenizer for AMSDOS `.bas` files, no zip/URL inputs.
* `rctape`: no Commodore `.tap` decoding (only facts), no `.wav`/`.csw`/`.pzx`/`.voc`, `add --type basic` needs an
  already tokenised program, loops and jumps are kept but not followed.
* `rcld`: no CPC tape / ZX81 / `.cpr` output, no 128K snapshots, sources are assembled as one program (no separate modules).
* `rcsnap`: `create` is 48K only, there is no register patching of an existing snapshot, ZX81 BASIC is not listed.
* The Z80 `DD`/`FD` prefix before an instruction that ignores it is disassembled but not assembled back (rcdis emits
  `.byte` for it).
