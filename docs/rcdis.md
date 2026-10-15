# rcdis

```
rcdis [options] <file>
  -c, --cpu <6502|z80>        required
      --org <addr>            address of the first byte (raw files; a .prg brings its own)
      --prg                   the first two bytes are the load address (implied by a .prg extension)
      --skip <n>, --length <n>   use part of the file
  -e, --entry <addr>          follow the flow of control from here (repeatable)
      --follow                follow the flow from the first address
      --data <start-end>      addresses that are data (repeatable)
  -s, --symbols <file>        name addresses (repeatable)
      --addresses             add `; ADDR: BYTES` to every line
  -o, --output <file>  --force
```

The output is `rcasm` source -- assembling it reproduces the input bytes. rcdis checks each instruction, and
writes `.byte` where the assembler could not reproduce an encoding (the six implied `*NOP`s of the 6502, a Z80
`DD` prefix in front of an instruction that ignores it).

* **Linear** (no `--entry` / `--follow`): every byte is decoded in order. Fine for ROMs; data shows up as
  odd instructions.
* **Flow**: starting at the entry points, `JMP`/`JP`, `JSR`/`CALL`, `RST`, branches and `JR`/`DJNZ` are followed;
  execution stops at `RTS`/`RTI`/`BRK`/`JAM`, `RET`/`RETI`/`RETN`, unconditional jumps and `JP (HL)`. Bytes never
  reached are `.byte` lines. A target in the middle of another instruction gets no label.
* **Labels**: jump targets and memory operands inside the image become `L_xxxx` / `D_xxxx`; symbol files give
  names (inside the image) and are used for operands outside it, which then appear as `NAME = $ADDR` lines at
  the top. A 6502 absolute operand with a value below 256 keeps its digits (naming it could turn it into zero
  page).
* Undocumented 6502 opcodes print with a `*` and a `.illegal` line is added.
* Input: raw binaries and `.prg`. Tape, disk and snapshot input will come with `rctape` / `rcdisk` / `rcsnap`.
