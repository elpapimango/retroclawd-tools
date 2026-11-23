# File formats

All integers in `.rco` and `.rca` are LEB128 (seven bits per byte, low group first, high bit set on all but the last byte, at most
10 bytes); signed values are zigzag coded (`(v << 1) ^ (v >> 63)`). A string is a length then that many UTF-8 bytes. A reader checks every
count and length against the bytes that are left before it allocates anything, and every index against its table.

## `.rco` object

```
"RCO1"
string cpu            "6502" | "z80" | "huc6280" | "68000"
string name           the source file the object came from
uint   nfiles ; nfiles * string      files named by relocations (index 0 is the source)
uint   nsections
  per section:  string name
                byte flags            1 bss, 2 zero page, 4 fixed address
                uint alignment        power of two, at most 65536
                uint size             at most 2^28
                [uint address]        when flag 4
                size bytes            unless bss
uint   nsymbols
  per symbol:   string name
                byte tag              low nibble class (0 label, 1 constant, 2 import); 0x10 global; 0x20 zero page
                [uint section]        labels
                [int value]           labels (offset into the section, at most the size) and constants
uint   nrelocs
  per reloc:    uint section ; uint offset
                byte tag              low nibble width 1..4; 0x10 relative; 0x20 big endian
                [int bias]            relative only
                uint instr_back       the instruction starts this many bytes before the field (at most 255)
                expression ; uint file ; uint line
```

A relocation says: "the `width` bytes at `offset` of `section` hold the value of `expression`" (absolute), or "the value of `expression` minus
(the address of the instruction plus `bias`)" (relative); `$` is the address of the instruction, which starts `instr_back` bytes before the field. The expression is a tree: a tag byte,
then 0 `int` (number), 1 `uint` (an index into the object's symbol table), 2 (`$`, the address of the instruction), 3 `byte expr` (unary:
the character `- + ~ ! < >`), 4 `byte expr expr` (binary: 0 `*`, 1 `/`, 2 `%`, 3 `+`, 4 `-`, 5 `<<`, 6 `>>`, 7 `<`, 8 `<=`, 9 `>`, 10 `>=`,
11 `==`, 12 `!=`, 13 `&`, 14 `^`, 15 `|`, 16 `&&`, 17 `||`); at most 64 deep. The linker checks that the value fits.

## `.rca` archive

```
"RCA1"
uint nmembers ; per member:  string name ; uint length ; length bytes (a complete .rco)
uint nindex   ; per entry:   string exported-name ; uint member-number
```

The index lists every name a member exports (global labels and constants), the first member winning; a reader recomputes it from the
members and rejects an archive whose stored index differs.

