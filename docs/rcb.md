# rcb: the project driver

```
rcb new NAME --machine c64|spectrum|cpc|atari|atari-st|pcengine|nes|snes|genesis|neogeo|amiga|x68000
rcb build        assemble, link, write build/NAME.<ext>, build/NAME.map and build/NAME.sym
rcb check        the same, writing nothing
rcb run          build, then `rcd <machine> ...` on the image
rcb debug        build, then `rcdd <machine> ... --symbols build/NAME.sym`
rcb clean        remove build/
rcb targets      the machines and what each project makes
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
(a loop or a return with the vectors the machine needs). Hardware kits (register and BIOS include files) are planned; `rcb kits` lists none yet.
