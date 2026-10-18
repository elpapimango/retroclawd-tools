# CLAUDE.md

Guidance for Claude Code working in this repository.

## Testing approach (decided 2026-10-17)

The point of these suites is **acid tests for an emulator**: programs and facts from outside (third-party diagnostics, published
documents, other emulators we read) that show the emulator behaves like the real machine *as far as it is worth checking* -- not proof of
real hardware, which we will never have and do not aim for. If games, demos and applications load and run, that is the goal. So:
a case earns its place by being judged from outside (a third-party program, a document's number, a behaviour a game can see); a case that
only checks our own plumbing, or repeats what another case already shows, does not. Pass 1 of the review (2026-10-17) cut the 101 cases to
69: merged cases keep every check of the cases they replace, removed ones are listed in `TODO.md` of the host repository. Acid-style suites
(CPC Acid Tests Suite, cpctech `test.zip`, Acid800 as a model for how an acid test is laid out) are added as cases with a known-gaps list. Pass 2 (2026-10-17) applied the same rules to every unit and integration test of every repository (this repository's own tools went 145 -> 90); see `TODO.md` in the host repository.

Acid tests fetched so far (copied by the host's `tools/fetch_acid.sh --from <folder>`, never committed): Patrik Rak's z80test on the 48K (`spectrum_z80test`; all variants pass). CPC: the self-checking programs of Kevin Thacker's suite (`cpc_acid_tests`; every program has a recorded state in `BASELINE`, regenerate with `RC_ACID_PRINT=1`; `RC_ACID_ONLY=name` runs one). Our own acid programs: `spectrum_timing` (T-state counts against the WOS48 contention rules), `spectrum_io`, `c64_hardware` (cases name their document in the file header; `.gap(..)` marks what the emulator does not do yet).

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

## Emulator suites (`crates/rc-suite`)

A harness and per-machine suites that run the real `retroclawd` headless and check memory, registers and the picture against
**hardware facts that each case cites** (a datasheet, a manual, the ROM itself), never against the emulator's earlier output.
`tests/zx81.rs`, `tests/cpc.rs`, `tests/c128.rs`, `tests/spectrum.rs` and `tests/c64.rs` hold the cases; a case belongs to an `Area` so a failure names what is wrong. A feature the
emulator lacks is a *gap*: `Case::gap("why")` -- it must fail, and the run fails when it starts passing, so gaps only shrink on
purpose. Everything skips unless `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS` (a folder with
`sinclair/ amstrad/ commodore/`, each with its `roms/`) are set, so public CI stays green without firmware.

* Run: `RETROCLAWD_BIN=... RETROCLAWD_MACHINE_PLUGIN_DIRS=... RC_ROMS=.../machines cargo test -p rc-suite -- --nocapture`.
* `--frames N` runs whole frames like the window (a frame interrupt each, no stop at `HALT`); `--max-cycles` stops at the first
  `HALT`, which a ROM waiting for the next frame reaches during boot.
* `z80_program` / `z80_image` assemble a test program (`--program`). A `--program` run does not boot the firmware, so a case that
  needs video timing programs the CRTC itself.
* `Resident` runs code on a **firmware-booted** CPC: it boots, saves a snapshot, writes the code into the snapshot's RAM, points
  the program counter at it and resumes. The firmware's jump block and interrupts are live, so the code can call AMSDOS (`CAS IN
  OPEN` 0xBC77 / `CAS IN DIRECT` 0xBC83), which is how disks the tools made are checked headless (no keyboard needed).
* Writing a test program as a string: the program text goes in a plain string with real newlines.
* `--type TEXT` (with `--frames`, `--type-after N`) types on the emulated keyboard like the paste hotkey, so BASIC can be driven and
  the keyboard path is part of the test: `PRINT 12*34` is checked on the C128 screen, and a disk is loaded with `LOAD"NAME",8,1`.
  A C128 with a real drive (`--true-drive`) probes it for a boot sector first: type from frame 500, not 150.
  `--type-delay MS` sets the time between key events (default 60): the ZX81 ROM debounces its keys and loses them that fast, use 160.
  The ZX81 keyboard cases read the expected matrix positions from the ROM's own key tables (0x007E unshifted, 0x00A5 shifted).
  The ZX81 ROM scans the keyboard once a frame and debounces it: type with `--type-delay 160` (the default 60 ms loses keys). The ZX81
  suite checks every key against the ROM's own key tables (0x007E unshifted, 0x00A5 shifted) with a program that reads all eight
  half rows and keeps the AND of what it saw, so exactly the typed key's matrix line shows low.
* `c128_8502_program` runs 6502 code on a C128: it powers up on its Z80, so a Z80 stub sets the memory map, points the 8502 reset
  vector at the code and sets the processor-select bit of the MMU's mode register (a Z80 reaches the I/O chips with `IN`/`OUT`, not
  memory writes; in Z80 mode `$1000-$13FF` is colour RAM). The KERNAL is not booted in such a run; to use it, boot normally and type.
* `mos6502_program` / `Env::c64()` run 6502 code on a plain C64 (`--program` starts the CPU at the load address with nothing
  initialised; the KERNAL is not booted). To measure a frame on the C64 use a CIA timer between two raster line 0s -- the
  host's `--frames` batch (985248 / 50 cycles) is pacing, not the VIC's frame -- and read `$D012` *before* `$D011`, or the
  255 -> 256 line change shows up as a false line 0. The Spectrum cases need `--no-border` for a 256x192 picture.
* Read hardware values through the CPU, not the dump: `PRINT PEEK(1)` found that the C64 port read 247, which the dump
  path would also have shown but is easy to dismiss as a debug view.
* `tools/fetch_roms.sh` in the host repository fetches the firmware these suites read from your RomM (the files of every
  case, drive DOS ROMs included); the C64 KERNAL must be revision 3 for the boot cases to match their baseline.
* **Third-party diagnostics**: `tools/fetch_diagnostics.sh` (host repository) downloads pinned release images (SHA-256 checked) into
  `machines/<family>/software/diagnostics/` (git-ignored; they are other people's software, never committed). The Spectrum case runs
  brendanalford/zx-diagnostics v0.37's `testrom.bin` (LGPL-3.0) as the machine's ROM and reads the result from the screen colours its
  source documents (`PASS` in green ink, a bright-green "48K RAM Tests Passed" bar, nothing red) -- about 100 s of machine time, 5000 frames.
  llopis/amstrad-diagnostics (MIT, v1.3; `AmstradDiag.cpr` and `.dsk`, fetched the same way) has five cases in `tests/cpc.rs`
  (`cpc_diagnostics`, about 35 s): the cartridge boots on a GX4000 in colour and shows `LOWER RAM: PASSED` in its success colour (cyan, nothing
  in the error colour); its keyboard test paints every key it reads as pressed from the matrix, so two runs type the odd and the even
  half of the 48 unshifted keys and check that exactly those cells light up (a key wired to a neighbour's matrix position fails it --
  swapping `,` and `.` was mutation-checked); on a CPC6128 the disk (`RUN"DIAG`, then `q` padding typed while it loads, then the menu key)
  passes its upper RAM test and its ROM test names the lower ROM, BASIC and AMSDOS by checksum (read from how far each line's text reaches).
  The cartridge needed two fixes: its `.cpr` is malformed (`parse_lenient`) and a CPC+ must honour classic Gate Array ink writes (it showed
  black). The first screen of the keyboard test ignores the first two keys, hence the sacrificial `z`. Five more cases (2026-10-16, `cpc_diagnostics`
  now about 90 s in a debug build): the keyboard test's joystick cells (`--at N:key_down|key_up=Joy1*`, 10 frames per press, two
  complementary runs: exactly the pressed cells light), the menu's JOYSTICK result after leaving the test by holding fire 1 (cyan `PASSED`
  with up/down/left/right/fire 1 read; error-colour `FAILED` when right never was; the untouched keyboard is `FAILED`), and the 6128 disk's
  soak test (`SOAK ITERATION N` in the title: banner still there, no error colour and a changed counter digit at frame 2000 versus 3800, taken
  with `--at 2000:screenshot=FILE`; `rc_suite::Screen::read` is public for that). Mutation-checked in the amstrad plugin: swapped Up/Down
  bits, Left moved to bit 7, bit 7 stuck on writes to RAM banks 4+ (aliasing banks or byte pairs did NOT fail it: the program fills and verifies one
  pattern, so an alias is consistent to it). Not done: the sound test, the ROM test for a 464/664.
  The +3 case `a +3 reads a sector from each of its two drives` (`--disk` + `--disk-b`) drives the FDC ports directly with a hand-made
  one-sector DSK per drive (unit in the second command byte of Read Data); mutation-checked (drive B inserted into unit 0 fails it).
  The unified diagnostic cartridge (crystalct's board: the modified dead test in Ultimax mode plus a `$DE00` flip-flop that turns it into an ordinary 8K cartridge running the standard diagnostic rev 586220++) is the second case in `c64_diagnostics`, from the 32K EPROM image `tools/fetch_diagnostics.sh` fetches (`unified_32k.bin`, local use only). The `.crt` is built by the test (retroclawd's own hardware type 0xC640). By frame 8000 (30 s in a release build, about 5 min in a debug one) the dead test has finished and the cartridge switched (between frames 5000 and 6000), and the standard diagnostic shows its RAM/ROM tests OK, the ROM checksums (`B6C70A`, `043D56`, `F3F7F8`), TIMER1/2 A/B and INTERRUPT OK; the ports and cassette read BAD without the diagnostic's test plugs and are not asserted. An earlier note here that the dead test's RAM test takes half an hour was wrong (Ultimax leaves only the low 4K; a whole cycle is about 5000 frames).
  AmstradDiag's own sound test (repository head 7ac70fc, `[T]` in the menu; the v1.3 release has none) is `cpc_diagnostics_head` in `tests/cpc.rs`,
  skipped unless `AmstradDiagHead.dsk` exists (`tools/build_amstradiag_head.sh` in the host repository builds it with sjasmplus and puts the program
  on a blank AMSDOS disk with `rcdisk`). It runs `RUN"DIAG`, types `t`, and reads the `--dump-audio` capture: the first loud sound (the CPC's key click for
  the typed letters is quieter) starts the clock, then noise A, A+B, A+B+C (louder with each channel, no chord tone in it), tones 110/131/165 Hz
  from the step that turns each channel on, then muted. Mutation-checked (PSG clock divisor). The C64 unified cartridge's standard diagnostic reads
  the 6581 BAD without a paddle in the control ports (its test plug is what puts POTX/POTY in `$50-$8F`) and OK with `--mouse paddle-port2`: two cases.
  Later the same day: the disk on a CPC664 (two cases: system information with the FDC detected, ROM test by checksum), eight PSG cases
  (`--dump-audio` through `rc_suite::Audio`: each channel's tone with sub-harmonic checks -- a square wave a third too slow still shows its
  third harmonic at the right frequency, which a first version missed and a mutation caught --, chord, mixer mute, noise, volume, register 1)
  and one case reading `B:SAME.BIN` from drive B (`--disk-b`) through AMSDOS on a 6128. Note for programs run with `--program`: set SP below
  `$C000`, the upper ROM shows there and a stack in high RAM reads back as ROM.
  The C64 dead test (crystalct/C64UnifDiagCart's modified rev 781220 ROM, a raw 8K Ultimax image fetched from a pinned commit; CERN-OHL-W-2.0
  repository, Commodore's code, so local use only) has one case in `tests/c64.rs` (`c64_diagnostics`, about 130 s in a debug build): wrapped in an
  Ultimax `.crt` it boots with no KERNAL involved and, by frame 4000, prints OK next to ZERO PAGE, STACK PAGE, SCREEN RAM and COLOR RAM and
  starts the main RAM test (which is short: the whole dead test, sound test included, ends by about frame 5000). The Spectrum suite also checks that both
  16K halves of every 128K-family ROM variant appear at 0x0000 exactly as the file holds them.
* Joysticks and sound are testable headless now: `--at N:key_down=Joy1Up` and `--dump-audio` (see above).

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
