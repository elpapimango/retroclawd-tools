# Phase 1: the object model

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** `rcasm` assembles a source to a relocatable `.rco` object, `rcar` bundles objects into a `.rca` library, `rcld` links objects and libraries
into the same images it writes today.

**Architecture:** the object formats live in `rc-common::obj`. `rcasm` keeps its two-pass whole-program mode (flat output, unchanged) and gains an
*object mode* (`Options::object`): code goes into named sections, labels are section offsets, and an operand that mentions a label, an import or
`$` is *relocatable*. The encoders are not touched: relocation fields are found by **probing**: the instruction is encoded again with the
relocatable operand's value moved (and, for relative fields, with the instruction's address moved), and the bytes that change are the field.
The object stores the operand's expression text, the linker evaluates it with final addresses. `rcld` gets inputs `obj`/`lib` and `[[segment]]`
placement rules; the linker returns the `Piece`s and symbols the existing machine writers already take, so every output format is reused.

**Tech stack:** Rust 2024, `rc-common`, `rcasm`, `rcld`, new binary crate `rcar`.

**Spec:** `docs/superpowers/specs/2026-11-23-rcb-toolchain-design.md` (phase 1). **Roadmap:** `2026-11-23-rcb-roadmap.md`.

## Global constraints

Those of the roadmap. `#![forbid(unsafe_code)]`; object, archive and layout readers never panic on any bytes (fuzz targets); every existing test of
`rcasm` and `rcld` passes unchanged; flat mode output is byte-identical to 0.2.0; no code from cc65/vasm.

## Review focus

1. A relocatable operand in a 6502 instruction that could be zero page: object mode always chooses absolute unless the symbol was declared `.importzp`/`.exportzp` or is an absolute constant.
2. `.byte <label`, `.byte >label`, `.word label+2`, `.word label-label2` (difference of two labels of one section is a constant; of two sections is a linker-time value) all link to the right bytes.
3. A relative branch to an import or to a label in another section: the linker checks the range and names symbol, file and distance.
4. Two objects defining the same global: error naming both files. An import nothing defines: error naming the symbol and every object that wants it.
5. A library member is pulled in only when it defines a symbol that is still undefined; members pulled in by members resolve (order independent within one library).
6. Section larger than its region, or segment with no rule: error names the section and object.
7. Corrupt or truncated `.rco`/`.rca`: an error with the offset, never a panic or an allocation of the claimed size.

## File structure

| Path | Responsibility |
|---|---|
| `rc-common/src/obj.rs` | `Object`, `Section`, `Symbol`, `Reloc`, `Archive`; `encode`/`decode`; limits |
| `docs/formats.md` | byte-level specification of `.rco` and `.rca` (written first, test vectors in it) |
| `rcasm/src/object.rs` | object mode: sections, symbol classes, probing, `assemble_object` |
| `rcasm/src/asm.rs`, `cpu.rs` | hooks: `Options::object`, section state, `Eval` reports relocatable terms |
| `rcar/` | `rcar` binary: `create`, `list`, `extract`, `add`, `delete` |
| `rcld/src/link.rs` | linker: symbol resolution, archive member selection, placement, relocation |
| `rcld/src/layout.rs`, `lib.rs` | `[[input]] obj/lib`, `[[segment]]`, `load` calls the linker |

## Formats (decided here, specified in `docs/formats.md` before use)

`.rco` is `"RCO1"` then little-endian LEB128 integers and length-prefixed UTF-8 strings: CPU name, source name, sections (name, flags
`abs|zeropage|bss`, alignment, size, optional fixed address, bytes unless bss), symbols (name, class `label|const|import|importzp`, scope
`global|local`, section index + value or constant), relocations (section, offset, width 1..4, endian, kind `abs|rel`, `rel_bias`, expression
text, symbol-use lines for diagnostics: file index, line), file table. `.rca` is `"RCA1"`, a member count, per member (name, byte length, the
object bytes) and an index (export name -> member number) that readers verify against the members. Limits: 64 MB per file, 65 535 sections,
1 M symbols, 4 M relocations; a length that exceeds the bytes left is an error before any allocation.

### Task 1: the formats

**Files:** Create `rc-common/src/obj.rs`, `docs/formats.md`; modify `rc-common/src/lib.rs`; test `rc-common/tests/obj.rs`.

**Interfaces:** produces `rc_common::obj::{Object, Section, Symbol, Reloc, Archive, Class, Kind}` with `Object::encode(&self) -> Vec<u8>`,
`Object::decode(&[u8]) -> Result<Object, String>`, `Archive::{new, add(name, &Object), members(), decode, encode, find_export(&str) -> Option<usize>}`.

- [ ] Tests: encode/decode round trip of an object with every field kind; archive round trip with index; each truncation of an encoded object (every prefix) is an `Err` and none panics; a length field claiming 2^40 bytes is an `Err` without allocating; unknown magic and an index that disagrees with the members are `Err`.
- [ ] Implement; write `docs/formats.md` with one hex example of a 6-byte object.
- [ ] `cargo test -p rc-common`; commit.

### Task 2: object mode in the assembler

**Files:** Create `rcasm/src/object.rs`; modify `asm.rs`, `cpu.rs`, `lib.rs`, `main.rs`; tests `rcasm/tests/object.rs`.

**Interfaces:** `Options.object: bool`; `pub fn assemble_object(name, source, &Options, &dyn Files) -> Result<rc_common::obj::Object, Vec<Diag>>`;
directives `.segment "NAME"[, zeropage|bss]`, `.export a, b`, `.import a, b`, `.importzp`, `.exportzp`, `.global a` (import if undefined, export if defined),
`.scope NAME`/`.endscope` and `.proc NAME`/`.endproc` (a scope is a prefix `NAME::`; `NAME::sym` reaches into it); `.org` is an error in object mode
("the linker places sections").  Consumes `Eval` from `cpu.rs`.

- [ ] Tests (6502, Z80, HuC6280 each): two sections get their own bytes and sizes; `jmp label` yields an `abs` 2-byte reloc against the expression `label`; `lda import`/`lda <import`/`lda #>label+1`; `bne far_label` in another section yields `rel` with the right bias; `tii a,b,c` with three relocatable words yields three relocs at offsets 1, 3, 5; `.word label2-label1` in one section is a constant; undefined symbol without `.import` is an error; `.import` of a defined label is an error; `.exportzp` keeps zero page addressing.
- [ ] `Eval` gains a recorder: each `value(text)` call returns the value and records `(expression text, relocatable: bool)`; `relocatable` when the expression names a non-constant symbol or uses `$`. A relocatable operand sets `unresolved` for width choice, except `importzp`/`exportzp` symbols.
- [ ] Probing in `object.rs`: for an instruction with recorded relocatable operands, for each of them re-run the encoder with that operand's value `+1`, `+0x100`, `+0x10000` (skip a probe the encoder rejects); the changed bytes' union is the field (width <= 4); then re-run with the instruction's address `+0x10`: if the field moves, kind `rel` and `rel_bias = value0 - field0`. Data directives (`.byte`, `.word`, `.dword`, `.fill` with relocatable values) write their field directly.
- [ ] Flat mode untouched: `cargo test -p rcasm` passes.
- [ ] `rcasm -c file.asm -o file.rco`; docs/rcasm.md "Objects" section; commit.

### Task 3: archives (`rcar`)

**Files:** Create `crates/rcar/{Cargo.toml,src/main.rs,src/lib.rs}`, tests `rcar/tests/cli.rs`, `docs/rcar.md`.

- [ ] Tests: `rcar create lib.rca a.rco b.rco` then `rcar list` prints members and exports; `extract`, `add` (replaces a member of the same name), `delete`; refusing to overwrite without `--force`; a non-object member is an error naming it.
- [ ] Implement over `Archive`; `README` and crates.io description; commit.

### Task 4: the linker

**Files:** Create `rcld/src/link.rs`; modify `layout.rs`, `lib.rs`; tests `rcld/tests/link.rs`.

**Interfaces:** layout `[[input]] obj = ["a.rco"]`, `lib = ["lib.rca"]`; `[[segment]] name = "CODE" region = "ram" [align = n] [load = ...]`
(a segment without a rule goes into the first region that fits, in input order; with no regions at all, sections of the layout's `at`/first free
address are packed from `start` of the machine default); `pub fn link(inputs: &[LinkInput], rules: &[Rule], regions: &[Region]) -> Result<Linked, Vec<String>>`
with `Linked { pieces: Vec<Piece>, symbols: Vec<(String, i64)>, map: String }`.

- [ ] Tests for review-focus items 3-6 and the two-object, one-library C64 example whose `.prg` is byte-equal to the same program assembled whole.
- [ ] Algorithm: collect objects, pull library members by undefined globals to a fixed point, place sections (rule order, then input order), compute symbol addresses, apply relocations by evaluating the stored expression with `rcasm::expr`, writing the field (range-checked, error names symbol, file, line), emit pieces.
- [ ] `load` handles `obj`/`lib`; existing layouts unchanged; docs/rcld.md "Objects and libraries"; commit.

### Task 5: 68000 objects

**Files:** modify `rcasm/src/m68k.rs`, `rcasm/src/object.rs`; tests `rcasm/tests/object68k.rs`.

- [ ] The 68000 assembler's text/data/bss become sections, its relocations become `abs` 4-byte big-endian and `rel` word/long (PC-relative displacements); `.import/.export` as above; a program linked by `rcld` for `atari-st` equals the whole-program `.prg`.
- [ ] Commit.

### Task 6: finish

- [ ] Fuzz targets `rco_decode`, `rca_decode`, `link_layout`; `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace` green on three OSes (CI).
- [ ] Version 0.3.0 of all tools (`rcar` is new, published as `rcar`), tag `v0.3.0`; roadmap note; phase 2a plan next.

## Self-review

Spec coverage: objects, archives, relocatable rcasm, linker over objects with TOML layouts (tasks 1-4), 68000 objects for Genesis/Neo Geo use (5). Not here
on purpose: ld65 `.cfg` (phase 4), banked-card linking for the PC Engine (phase 3), debug info (phase 7). Names are consistent across tasks.
