# rcasm

```
rcasm [options] <source.asm>
  -c, --cpu <6502|z80|huc6280>  (or `.cpu` in the source; 6510 / 8502 are 6502; `pce` is huc6280)
  -o, --output <file>         what to write ...
  -f, --format <bin|prg|segments>
      --fill <byte>           between segments in bin / prg (default 0)
  -s, --symbols <file>        symbol file;  --symbols-style rc|vice;  --all-symbols
  -l, --listing <file>
      --illegal               allow the undocumented 6502 opcodes
      --syntax <native|oldstyle|std>   rcasm's own (default), or vasm's oldstyle / std modules
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
| `.incbin "file"[,skip[,len]]`, `.include "file"` | relative to the including file, then `-I`, then the `.incdir` folders |
| `.incdir "path"` | another folder to search (relative to the source file, else to the working folder) |
| `.exitmacro` | ends the current macro expansion here (its open `.if`s end with it) |
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

### HuC6280 (PC Engine / TurboGrafx-16)

`.cpu huc6280` (or `-c huc6280`, `pce`). The 65C02 instruction set with the Rockwell bit instructions (`RMB0-7`, `SMB0-7`, `BBR0-7`, `BBS0-7`),
`BRA`, `TSB`, `TRB`, `STZ`, `PHX`/`PHY`/`PLX`/`PLY`, and the HuC6280's own: `CSH`/`CSL` (clock speed), `SET` (the next ALU instruction works on zero page
through X), `CLA`/`CLX`/`CLY`, `SAX`/`SAY`/`SXY`, `ST0`/`ST1`/`ST2 #n` (video registers), `TAM #mask` / `TMA #mask` (mapper registers), the block transfers
`TII`, `TDD`, `TIN`, `TIA`, `TAI src,dst,len` (three 16-bit values), `TST #n,addr[,X]`, `BSR`. The encodings come from the core's disassembler
(`rc-huc6280`), so everything the debugger prints assembles. Addresses are the 16-bit logical addresses the CPU sees; a value up to `$FF`
(or less than three hex digits written) uses the zero-page form, which the CPU maps to logical `$2000-$20FF`. `BBRn zp,target` and the
branches take the target address and are range checked (-128 to 127 from the next instruction).


## Objects (`-f object`)

`rcasm -f object -c 6502 main.asm -o main.rco` writes a relocatable object instead of an image (6502, Z80, HuC6280; the 68000
follows). Code goes into named **segments**; the linker (`rcld`) places them, so `.org` is an error here. A label is an offset into
its segment. Everything the linker has to decide is written into the object as a relocation: an operand that depends on a label of
a placed segment, on an import, or on `$`. The difference of two labels of one segment, and a branch inside one segment, are
constants and need none.

| Directive | Meaning |
|---|---|
| `.segment "NAME"` | switch to (or start) the segment `NAME`; the first one is `CODE`. Options: `bss` (space only, no bytes: `.res n`), `zeropage`, `align=N`, `at=ADDR` (a fixed address) |
| `.import a, b` / `.importzp a` | names defined in other objects; `importzp` ones are zero page, so `lda a` is the two-byte form |
| `.export a, b` / `.exportzp a` | names other objects may use |
| `.global a, b` | an export if this object defines the name, an import if not |

A name must be defined, imported or exported explicitly; an undefined name is an error. A constant (`NAME = 40`) may be exported
but not computed from addresses of different segments (`size = end - start` is fine inside one segment). Instructions whose size
depends on an address use the 16-bit form unless the symbol is zero page. The bytes of a relocated field in the object hold the value
assembled with the segment starting at 0; the linker overwrites them. The format is in `docs/formats.md`.

```
.import putc
.export start
start:  ldx #0
loop:   lda msg,x
        beq done
        jsr putc
        inx
        bne loop
done:   rts
.segment "RODATA"
msg:    .byte "HI", 0
```

## The 65816 and the SPC700 (`--cpu 65816`, `--cpu spc700`)

Both back ends take their encoding table from the CPU crates' disassemblers (`rc-w65c816`, `rc-spc700`), so every instruction the debugger shows assembles.
Addresses are 32-bit in the assembler: a 65816 program may `.org $808000`; the other CPUs still stop at `$FFFF`.

**65816.** Immediates follow the register widths: `.a8`/`.a16` (accumulator), `.i8`/`.i16` (index registers); in `.smart` mode (the default,
`.smart off` switches it off) `REP #$30` and `SEP #$20` with a constant change them too, from the next instruction on. An 8-bit register given a 16-bit value is an
error that says so. Address sizes: a literal's digits decide (`$12` direct page, `$1234` absolute, `$123456` long), else the value when it is known, else absolute; the
suffixes `.b`, `.w`, `.l` decide outright (`lda.l $12`). `jmp`/`jsr` take a label of their own bank without its bank byte; another bank is an error that names `jml`/`jsl`.
Forms: `lda ($12),y`, `lda [$12],y`, `lda $12,s`, `lda ($12,s),y`, `jmp ($1234,x)`, `jml [$1234]`, `mvn $7E,$7F` (source, destination; the bytes store the destination first), `per`, `brl`.
Branches stay in their bank. In object mode 3-byte (long) address fields relocate like the others.

**SPC700.** `mov a,#$12`, `mov a,$12` (direct page), `mov a,!$1234` (absolute; a plain operand above `$FF` is absolute too), `mov a,$12+x`, `mov a,[$12+x]`, `mov a,[$12]+y`,
`mov a,(x)+`, `mov $22,$11` (destination first), `mov $12,#$34`, `set1 $12.3`, `bbs $12.3,target`, `or1 c,/$1234.5` (a 13-bit address and a bit number),
`tcall 5`, `pcall $f0`, `movw ya,$12`. The direct page flag is the program's business: the assembler writes the byte.

## ca65-style source (scopes, unnamed labels, `.repeat`)

Enough of ca65's language to assemble typical cc65-era projects as they are:
- `.proc name` ... `.endproc` (a label, then a scope) and `.scope name` ... `.endscope`; inside, names are `scope::name`; a reference finds the innermost scope that has the name, then the global one; `outer::inner` and `::global` name a scope explicitly. Scoped names are objects' symbols (`util::run`).
- Unnamed labels: a line starting with `:` defines one; `:+`, `:++` mean the next ones, `:-`, `:--` the previous ones.
- `.repeat COUNT[, VAR]` ... `.endrep` (nestable; VAR counts from 0).
- `.addr` (= `.word`), `.dbyt` (a big-endian word), `.setcpu "65816"`, `.p02`, `.pc02`, `.psc02`, `.p816`, and the housekeeping directives `.feature`, `.list`, `.nolist`, `.macpack`, `.listbytes`, `.autoimport`, `.case` (accepted, no effect).
- Together with `.segment`, `.import`/`.export`/`.global`, `.importzp`/`.exportzp`, `.res`, `.macro`/`.endmacro`, `.if`/`.ifdef`, `.include`, `.incbin` and `@cheap` locals, which were there already.

Also: `.define NAME text` and `.define NAME(a, b) text` (replaced outside strings, `.undef NAME` removes; `.ifdef` sees them),
`.struct Name` ... `.endstruct` (members `.byte .word .addr .faraddr .dword .res N .tag Other`; `Name::member` is the offset, `.sizeof(Name)` the size),
`.enum [Name]` ... `.endenum` (`A`, `B = 5`, counting up; `Name::A` when named), `.local a, b` in a macro (names private to each expansion) and `.ifblank` / `.ifnblank`,
`.union Name` ... `.endunion` (the same members, all at offset 0; the size is the largest), `.ifconst EXPR` (true when the expression has a value at that point of the first pass:
a label defined further down is not constant, in both passes, so the sizes agree), `.ifref NAME` / `.ifnref NAME` (true when an expression used NAME earlier in the
pass, the library idiom of assembling a routine only when something called it; `.ifdef` is not a use; decided in the first pass and replayed) and `.charmap INDEX, VALUE` (from there on, the byte INDEX of a string literal in
`.byte`/`.text`/`.asciiz` and their aliases becomes VALUE; character constants in expressions are not mapped).

Since 0.6.2 also `.set NAME, VALUE` (a symbol that may be given a new value later; `=` and `equ` still refuse a second definition) and
`.dsect` ... `.dend` (an offset section: labels inside get addresses from where the last one ended, 0 at first, `.org` moves them; nothing is written).
Since 0.6.3: `.phase ADDR` ... `.dephase` (the labels and `*` count from ADDR while the bytes go on where the code is: code that is
copied elsewhere before it runs) and `.ifc A, B` / `.ifnc A, B` (the two texts after macro arguments were put in, quotes around
either left out).

Not done: nested anonymous `.struct`/`.union` members, `.mac` tricks with `.concat`.

## vasm's oldstyle and std syntaxes (`--syntax oldstyle|std`, the 8-bit CPUs)

Written from vasm's manual, not its code (vasm is not run here to compare with: `acp_dynawave.asm` of a public GameTank project,
built with `vasm6502_oldstyle -dotdir -wdc02`, gives the same 4096 bytes as the image in that project; use `--cpu huc6280` for the 65C02 instructions).
Each line is rewritten into rcasm's syntax before the passes (`crates/rcasm/src/vasm.rs`); a directive that is not translated becomes an error naming it at its line.
In a layout: `syntax = "oldstyle"`.

- **oldstyle**: labels at the first column, `:` optional (indented labels need it); `*` in the first column starts a comment unless it is `*=`;
  local labels `.name` and `name$`; numbers `$FF`, `%101`, `@17` (octal; binary on the Z80), `4#123`, `12d`, `17o`/`17q`, `101b`, `0x`, `0b`, `'A` and,
  in instructions, `"A"`; the Z80's `&FF`; `=` compares in conditions; blanks after commas in operands. Directives (with or without a dot):
  `byt byte db dfb defb fcb data di8 ascii asc`, `word wor wrd dw dfw defw fdb di16 addr da defp`, `dl defl` (24 bits on the 6502, 32 on the Z80), `di24 di32`,
  `asciiz string str fcc defm text fcs`, `blk bsz ds dsb defs zmb dc ds8 rmb blkw dsw ds16 blkl dsl ds24 ds32 fill reserve spc bss`, `align` (a bit count)
  `even org include incbin binary mdat`, `equ eq = set defc`, `if cond ifne ifeq ifgt ifge iflt ifle ifd ifdef ifnd ifndef ifused ifblank ifnblank else el endif endc fi ei`,
  `macro mac endm endmac endmacro` (`\1`..`\9`, `\name`, `name\@`, `\()`), `rept repeat endr endrep endrepeat`, `dsect dend`, `end assert fail`,
  `global xdef xref xlib extern`, `list nolist`, `rorg phase` ... `rend dephase` (rcasm's `.phase`), `struct structure` ... `endstruct endstructure`
  (data and space fields; `name.field` is a field's offset and `name` the size; `label name v1,,v3` lays out an instance, an empty value keeping the
  field's default, a reserved field taking none; instructions in a structure are left out; structures are known in the files included after them),
  `section NAME` (rcasm's `.segment` with `-f object`; an error in an image), `abyte OFFSET,values` (the offset added to every byte, string
  characters too), `roffs OFFSET[,FILL]` (padding up to OFFSET past the last `org`), `di64 ds64`, `inline` ... `einline` (local labels of the block's
  own), `incdir PATH` (rcasm's `.incdir`), `exitmacro exitm mexit` (rcasm's `.exitmacro`, also from inside a conditional), and `local symdepend needs`
  (no effect: they only concern object files). Not translated (an error): `weak`.
- **std**: labels end with `:`; local labels `.1`, `1$`, digit labels `1:` with `1b` / `1f`; numbers `0x`, `0b`, `017` (octal); `a!b` (or-not). Directives:
  `.byte .ascii .short .half .word .2byte .int .long .4byte .asciz .asciiz .string .space .skip .zero .align .p2align` (power of two) `.balign .org .include .incbin
  .equ .set .equiv .if .ifne .ifeq .ifgt .ifge .iflt .ifle .ifdef .ifndef .ifb .ifnb .ifc .ifnc .elseif .else .endif .macro .endm .rept .irp .irpc .endr .end .err
  .abort .global .globl .extern`, and `.list .nolist .file .size .type .stabs .stabn .stabd` (no effect). `.irp NAME, a, b` / `.irpc NAME, abc` repeat their
  block with `\NAME` replaced by each value / character. `.text .data .rodata .bss` and `.section NAME` are rcasm segments (`CODE DATA RODATA BSS`, `NAME`)
  with `-f object`; in an image the four go on in one piece and `.section` is an error. Floats, 64-bit data, `.comm` and the ELF visibility directives are errors.


## 68000 objects (`--cpu 68000 -f object`)

Motorola syntax as for the 68000 images. The object has three sections, `TEXT`, `DATA` and `BSS` (`.text`, `.data`, `.bss`), each starting at 0; `.org` is an error (the linker places the sections, see `rcld`).
`xref a,b` (also `.import`, `.extern`) names symbols of other objects; `xdef a,b` (also `.export`) and `.globl` / `.global` export symbols this object defines (a `.globl` name that is never defined is an import).
A field is settled by the assembler when it is a constant (`size equ end-start` is one: a difference of two labels of one section) or a displacement inside the section (`lea back(pc),a0`, `bra.s loop`). Anything else becomes a relocation:
an absolute long (`move.l #label,d0`, `jsr ext`, `dc.l label`), an absolute word or a displacement word (`abs.w`, `d16(an)`), a byte (a brief index displacement, an immediate byte) and, for a PC-relative field whose
target is in another section or an import, a relative one (`bsr.w ext`: the value minus the address of the extension word). The linker checks that every value fits its field and names the source line when not.
A short branch (`.s`) must stay inside the section; to anything else write `.w`. A constant is required where the instruction encodes it (a `moveq` value, a shift count, a `trap` number): an address or an import there is an error.