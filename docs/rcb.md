# rcb: the project driver

```
rcb new NAME --machine c64|spectrum|cpc|atari|atari-st|pcengine|nes|snes|genesis|neogeo|segacd|amiga|x68000
rcb build        assemble, link, write build/NAME.<ext>, build/NAME.map and build/NAME.sym
rcb check        the same, writing nothing
rcb run          build, then `rcd <machine> ...` on the image
rcb debug        build, then `rcdd <machine> ... --symbols build/NAME.sym`
rcb clean        remove build/
rcb targets      the machines and what each project makes
rcb kits         the hardware kits;  rcb kit c64   prints one
rcb -C DIR ...   work in another project folder
```

A project is a folder with `rcb.toml`: an [rcld layout](rcld.md) (machine, entry, `[options]`, `[[file]]`, `[define]`, `[[segment]]` ...) followed by a `[project]` table (keep it last: keys after a table header belong to it).

```toml
machine = "pcengine"
entry = "reset"

[project]
name = "hello"
sources = ["src/*.asm"]     # a * in the file name; they become the first [[input]]
separate = false            # true: each source is assembled to an object and linked (rcld `src`)
output = "build/hello.pce"  # default build/<name>.<the machine's usual extension>
```

`[run] args = ["pcengine", "--rom", "{output}"]` replaces the emulator arguments `rcb run` guesses (`{output}` is the image); `RCB_EMULATOR=program` replaces `rcd`/`rcdd`.
Machines the emulator does not have yet (NES, SNES, Amiga, X68000) build, but `rcb run` says so unless `[run]` is given. `rcb new` makes a skeleton for each machine that builds as it is
and shows that it runs in the smallest way the machine allows (a border or background colour, a line of text), with the vectors and header the machine needs. The 68000 ones follow the
Chibi Akumas lessons (chibiakumas.com): the Mega Drive turns the screen red, the Atari ST prints a line and waits for a key, the Neo Geo turns its backdrop red once `[options] security` gives it SNK's
security code (see rcld's Neo Geo section; without it the BIOS shows its crosshatch), and the Mega-CD's initial program turns the backdrop behind the BIOS's licence screen red once
`[options] security` names a disc of your own of the same region (see rcld's Mega-CD section; without it the BIOS stays in its menu). Checked in the emulator 2026-11-26 (Mega-CD 2026-11-27). Hardware kits: `rcb kits` lists them, `rcb kit MACHINE` prints one, and `rcb new` writes the machine's kit to `src/kit.inc` and includes it from the skeleton
(`.include "src/kit.inc"`). A kit is constants only (registers, system call numbers, colours, joypad bits): it emits no bytes. The values are from public hardware documentation
and tutorials; the SNES, Mega Drive, Atari ST, Amiga, X68000 and Neo Geo kits were checked against the Chibi Akumas platform pages (2026-11-26), and what those pages do not
show is marked `[H?]` or named as coming from other documentation.

`rcb build` also writes `build/NAME.rcdbg` (which source line made which bytes) for every machine. On the 8-bit/16-bit machines and the 68000 machines with a flat image (Mega Drive, Neo Geo, Mega-CD) the addresses are where the code runs; the
Atari ST, Amiga and X68000 load a program where the system puts it, so their addresses are offsets from the start of the text, with `base text gemdos` (ST) or `base text scan` and an
`anchor` (the text's first 32 bytes, `??` for the bytes the loader relocates) by which the debugger finds the program in memory (checked live on the ST 2026-11-27; `rcdd --source-base ADDR` names the address instead). `rcb debug` passes it on:
`rcdd ... --symbols build/NAME.sym --source-info build/NAME.rcdbg`. The debugger then shows the source line at the PC (`source_line` in the state of debug-ui), sets breakpoints by line
(debug-zrcp: `set-breakpoint 0 main.asm:12`; debug-ui: `POST /api/breakpoint/source {"file": "main.asm", "line": 12}`) and steps by line (`step-line` / `next-line` in debug-zrcp,
`POST /api/step-line` / `/api/next-line` in debug-ui; `next-line` steps over calls).
