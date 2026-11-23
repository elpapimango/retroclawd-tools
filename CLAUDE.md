# CLAUDE.md

Guidance for Claude Code working in this repository.

retroclawd-tools is an independent tool suite: `rcasm` (6502, Z80, 68000 assembler), `rcdis`, `rcdisk`, `rctape`, `rcsnap`, `rcld`, `rcchd` and the
shared `rc-common`. It is released on its own (crates.io and GitHub releases), separately from the emulator
[retroclawd](https://github.com/elpapimango/retroclawd), which uses the tools but is not needed to build or run them.

- The CPU cores and file formats come from [retroclawd-core](https://github.com/elpapimango/retroclawd-core), published as `rc-<name>`
  (`z80 = { package = "rc-z80", version = "0.1" }`; the code says `use z80::...`). To work on both at once, put a `[patch.crates-io]` section
  in an untracked `.cargo/config.toml` pointing at a local checkout.
- The integration suite and scenario runner (`rc-scenarios`, `rc-suite`) run the emulator and stay in the retroclawd repository.
- Test approach: a test earns its place by being judged from outside (a third-party program, a published document, another emulator read as
  a black box); a case that only checks our own plumbing does not. Fuzz targets are in `fuzz/` (cargo-fuzz, excluded from the workspace).
- Release order (dependencies first): `rc-common`, `rcasm`, then `rcdis`, `rcdisk`, `rctape`, `rcsnap`, `rcchd`, `rcld`.
- Commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.
