# Phase 2a: CPU cores for the new machines

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** `retroclawd-core` gains the CPUs the NES, SNES, X68000 and Amiga need, each as a published `rc-*` crate with a core that runs real code
and a public disassembler that `rcasm`/`rcdis` (phase 3 and 4) enumerate for their encoding tables, exactly as `rc-huc6280` does.

**Architecture:** one crate per CPU family in `retroclawd-core/crates/`, package `rc-<name>`, `[lib] name = "<name>"`. Each exposes `Cpu`, a
`Bus` trait of its own (the address width and the clock hooks differ), `step(&mut self, &mut impl Bus) -> u32` (cycles consumed),
`disassemble::disassemble(bytes, addr, state) -> (String, usize)`, and `reset`, `irq`, `nmi` entry points. Instruction-exact is the bar; a per-cycle
bus is added where a public suite checks it (65816 and SPC700 SingleStepTests carry per-cycle traces).

**Tech stack:** Rust 2024; test suites are fetched, never committed (`scripts/fetch_singlesteptests.sh` style, `RC_SST` points at them).

**Spec:** `docs/superpowers/specs/2026-11-23-rcb-toolchain-design.md` (phase 2a). **Roadmap:** `2026-11-23-rcb-roadmap.md`.

## Global constraints

Clean room: behaviour from public documentation (WDC datasheets, the SNES developer wiki, the Motorola manuals) and the public SingleStepTests
data; no emulator source is read. `#![forbid(unsafe_code)]`. Each crate has `description`, `repository`, `readme`; each CPU crate publishes as `0.1.0`.
Crates.io allows five new crates at once and then one per ten minutes: publish in batches as the cores finish.

## Review focus

1. Mode-dependent widths (65816 `M`/`X` flags, emulation mode, `REP`/`SEP`, `XCE`) change instruction length: the disassembler needs the flag state (a `state` argument), and the assembler needs `.a8`/`.a16`/`.i8`/`.i16` to follow it.
2. Wrap rules: 65816 direct-page wrap in emulation mode, bank wrap of (abs,X) and stack-relative operands, `MVN`/`MVP` across banks.
3. Interrupts/`BRK`/`COP` vector tables differ between emulation and native mode; `RTI` restores a different frame length.
4. 68020+: address-error rules relax (misaligned access allowed), new addressing modes (memory indirect, scaled index), `BFxxx` bitfields, `CAS`/`CAS2`, `CHK2`/`CMP2`, `TRAPcc`, `PACK`/`UNPK`, 32-bit multiply/divide; 68030 adds the PMMU and the data/instruction caches (CACR).
5. FPU: extended-precision (80-bit) arithmetic is rounded as the 68881 does (precision and rounding mode in FPCR), not as `f64` would.

### Task 1: 2A03 variant of the 6502 core

**Files:** modify `crates/mos6502` (`cpu.rs`, `lib.rs`); tests `mos6502/tests/variant.rs`.

- [ ] Test: with `Cpu::with_decimal(false)`, `SED; LDA #$09; ADC #$01` leaves `A = $0A` (binary add) and the D flag set; the default core still gives `$10`.
- [ ] Implement the option; document that the NES CPU's APU/DMA/controller registers are the machine's job.
- [ ] Publish with the next `rc-mos6502` bump.

### Task 2: the 65816 (`rc-w65c816`)

**Files:** `crates/w65c816/{Cargo.toml,src/lib.rs,src/cpu.rs,src/opcodes.rs,src/disassemble.rs}`, tests `tests/{functional.rs,singlestep.rs,disassemble.rs}`.

**Interfaces:** `trait Bus { fn read(&mut self, addr: u32) -> u8; fn write(&mut self, addr: u32, v: u8); fn irq(&self) -> bool; fn nmi_edge(&mut self) -> bool; }`;
`Cpu { a, x, y, s, d, db, pb, pc, p, e, .. }`, `Cpu::step`, `Cpu::reset`; `disassemble(bytes: &[u8], pc: u32, m16: bool, x16: bool) -> (String, usize)`.

- [ ] Tests: the SingleStepTests `65816` suite (both mode files, final state and cycle list) with the expected-failure list kept in `tests/singlestep-expected.md`; the basic flag/XCE/REP/SEP, `MVN`, `PEA/PEI/PER`, `JSL/RTL`, `BRK/COP` native and emulation vectors by hand.
- [ ] Implement all 256 opcodes in both modes, then the per-cycle bus trace (dummy reads included) the suite checks.
- [ ] Disassembler with mode state; every opcode round-trips through `rcasm` later.
- [ ] Publish `rc-w65c816 0.1.0`.

### Task 3: the SPC700 (`rc-spc700`)

Same shape: `Bus` over 16-bit addresses, 256 opcodes, `SingleStepTests/spc700`; no emulation mode; the direct page flag P selects page 0 or 1;
`DIV`, `MUL`, `DAA`/`DAS`, bit operations on `abs.bit`; `SLEEP` and `STOP` halt. Disassembler for the assembler table.

### Task 4: the SA-1 and the Super FX

- [ ] SA-1: a second 65816 (the core of Task 2 with its own `Bus`) plus the register file the cartridge exposes (`$2200-$23FF`): a `Sa1` struct the machine wires, with memory-mapping, DMA, arithmetic unit (multiply, divide, cumulative sum), bitmap register file; tests are hand-made against the documented register semantics.
- [ ] Super FX (GSU): its own 16-bit RISC instruction set (about 110 opcodes, prefix opcodes `ALT1-3`, `TO/FROM/WITH`), `PLOT`/`RPIX`, the cache, ROM/RAM buffers; disassembler; tests hand-made from the documented instruction semantics.

### Task 5: 68010, 68020, 68030

**Files:** modify `crates/mc68000` (new `variant` field on `Cpu`), new `ea` modes and instruction tables behind the variant.

- [ ] 68010: `VBR`, `MOVEC`, `MOVES`, `RTD`, `BKPT`, the loop mode of `DBcc`, the format word in exception frames, `MOVE from CCR`.
- [ ] 68020/68030: 32-bit address space, 32-bit data bus rules, new addressing modes, bitfields, `CAS/CAS2`, `CHK2/CMP2`, `TRAPcc`, `PACK/UNPK`, `MULS.L/MULU.L/DIVS.L/DIVU.L`, `EXTB`, `LINK.L`, `BKPT`, coprocessor interface (`cpGEN`), `CALLM/RTM` (68020 only), the master stack pointer, `CACR/CAAR`.
- [ ] Tests: the existing 68000 suites untouched (variant 68000 is the default); SingleStepTests where a suite exists for the variant (none is assumed): hand-made tests per instruction group against the Motorola manuals' worked results, and a differential run against the RetroArch Musashi-based core for random instruction streams (black box, as the 68000 was).

### Task 6: the 68881/68882 and the 68030 PMMU

- [ ] FPU: eight 80-bit registers, FPCR/FPSR/FPIAR, all arithmetic and transcendental instructions (`FMOVE`, `FADD`.. `FSIN`, `FATAN`, `FETOX`, `FLOGN`, `FMOVECR` constants), packed decimal and the exception conditions; extended precision through a software 64-bit-mantissa type so results match the chip, not `f64`.
- [ ] PMMU: translation control, root pointers, table walk with descriptor types, the ATC, `PFLUSH`, `PLOAD`, `PTEST`, transparent translation registers.
- [ ] Tests: published 68881 worked examples and constants (`FMOVECR` ROM table), self-consistency (`FSIN`/`FCOS` identities within the chip's stated error), table walks against hand-built page tables.

### Task 7: finish

- [ ] fmt, clippy `-D warnings`, `cargo test --workspace` green; each crate documented in its README; CI on three OSes.
- [ ] Publish the crates (batches of five, ten minutes between batches); note the state in the roadmap.

## Self-review

Covers every CPU of the spec's section 2a. The machines (video/sound/IO chips) are not here. Names (`rc-w65c816`, `rc-spc700`) are used consistently; the assembler tables
(`rcasm`) and decoders (`rcdis`) for these CPUs are phase 3/4 work that consumes the disassemblers defined here.
