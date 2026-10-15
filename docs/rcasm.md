# rcasm

```
rcasm [options] <source.asm>
  -c, --cpu <6502|z80>        (or `.cpu` in the source; 6510 / 8502 are 6502)
  -o, --output <file>         what to write ...
  -f, --format <bin|prg|segments>
      --fill <byte>           between segments in bin / prg (default 0)
  -s, --symbols <file>        symbol file;  --symbols-style rc|vice;  --all-symbols
  -l, --listing <file>
      --illegal               allow the undocumented 6502 opcodes
  -D NAME[=VALUE]             define a constant (VALUE defaults to 1)
  -I <dir>                    search for .include / .incbin
      --check                 assemble, report errors, write nothing
      --force                 overwrite output files that exist
```

Errors read `file:line: message`; the exit status is 1 when there are any. An existing output file is never
overwritten without `--force`.

## Output

* `bin` -- one image from the lowest to the highest address, gaps filled with `--fill`.
* `prg` -- the same with the load address in front (little endian), the C64 `.prg` form.
* `segments` -- one file per segment, `<output>.<ADDR>.bin` (what a linker places).
* `-s` -- `NAME = $C000` per label (`--all-symbols` adds constants). `retroclawd --symbols`, `rcdis -s` and
  `rcasm` itself read this form; `--symbols-style vice` writes `al C:c000 .NAME` for the VICE monitor. Local
  labels appear as `scope_name`.
* `-l` -- address, up to four bytes, source.

## Source

```
label:   instruction operands   ; comment
name     = expression           ; constant   (also: name equ expression, name .equ expression)
@local:                         ; local label, scoped to the last global label
* = $C000                       ; same as .org $C000
```

A word at the start of a line without a colon is a label unless it is an instruction, directive or macro
(write `name:` for a label that looks like one). Several labels and an instruction can share a line.

### Numbers and expressions

`$FF`, `0xFF`, `0FFh`, `%1010`, `0b1010`, `1234`, `'A'`; `$` or `*` alone is the address of the instruction;
`_` may separate digits. Operators with C precedence: `* / %`, `+ -`, `<< >>`, `< <= > >=`, `== !=`, `&`, `^`,
`|`, `&&`, `||`, unary `- + ~ !`, and `<x` / `>x` for the low / high byte. Symbols are case sensitive;
mnemonics and registers are not.

### Directives (with or without the dot for the bare names `org db dw defb defw defs ds equ end include incbin`)

| Directive | Meaning |
|---|---|
| `.org addr` | set the address; starts a new segment unless it continues the last one |
| `.byte a,b,"text"` (`db`, `defb`) | bytes; strings are ASCII with `\n \r \t \0 \\ \" \xNN` |
| `.word a,b` (`dw`, `defw`) | 16-bit little endian |
| `.text "s"`, `.ascii`, `.asciiz` | text; `.asciiz` adds a 0 |
| `.fill n[,v]` (`ds`, `defs`, `res`) | n bytes of v (default 0); `n` must be known |
| `.align n[,v]` | pad to a multiple of n |
| `.incbin "file"[,skip[,len]]`, `.include "file"` | relative to the including file, then `-I` |
| `.macro name p1,p2` ... `.endm` | text macro; parameters replace whole words; `@labels` inside are per expansion |
| `.if expr` / `.ifdef sym` / `.ifndef sym` / `.else` / `.endif` | conditionals; `expr` must be known on that line |
| `.cpu name`, `.illegal [on|off]` | select the CPU; allow undocumented 6502 opcodes |
| `.assert expr[,"msg"]`, `.error "msg"` | stop with a message |
| `.end` | stop reading this file |

### 6502

`lda #$10`, `lda $10`, `lda $1234,x`, `lda ($10,x)`, `lda ($10),y`, `jmp ($0300)`, `asl a` / `asl`; branches take
the target address and are range checked. A value below 256 uses zero page when the instruction has that form
and the value is known on that line; a symbol defined *later* gives the absolute form in both passes (so the
size cannot change between passes). A literal with three or more hex digits (`$0010`) always means absolute.
Undocumented opcodes need `--illegal` / `.illegal`; `*nop` selects the undocumented twin of an instruction.

### Z80

Zilog syntax: `ld a,(ix+5)`, `ld (hl),n`, `jp (hl)`, `ex af,af'`, `out (c),a`, `bit 3,(iy-1)`, `rst 0x38`, `im 1`.
`jr` / `djnz` take the target address. `add b`, `sub a,b`, `in f,(c)` and `sl1` are accepted. The IXH / IXL / IYH
/ IYL halves, `SLL`, `IN (C)` and the `RLC (IX+d),B` register-copy forms assemble. An operand written entirely
inside parentheses is a memory reference.

## How it works

Two passes over the same statement stream (includes, macros and conditionals are expanded during the walk): the
first learns label addresses, the second evaluates strictly and emits. Instruction encodings are not tabulated
here: the 6502 renders the evaluated operand as text for `mos6502::disassemble::assemble_one`; the Z80 builds
its table at start-up by *enumerating `z80::disassemble`*, so everything the debugger can display assembles, and
`tests/roundtrip.rs` proves it for every opcode.
