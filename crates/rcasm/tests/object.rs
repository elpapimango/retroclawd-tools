//! Object mode: sections, imports, exports and the relocations the encoders' operands turn into.

use rc_common::obj::{Class, Expr, Kind, Object};
use rcasm::{Cpu, DiskFiles, Options, assemble_object};

fn obj_for(cpu: Cpu, source: &str) -> Result<Object, String> {
    let opts = Options {
        cpu: Some(cpu),
        ..Options::default()
    };
    assemble_object("t.asm", source, &opts, &DiskFiles).map_err(|e| {
        e.iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("; ")
    })
}

fn obj(source: &str) -> Object {
    obj_for(Cpu::Mos6502, source).unwrap_or_else(|e| panic!("{e}"))
}

fn sym(o: &Object, name: &str) -> usize {
    o.symbols
        .iter()
        .position(|s| s.name == name)
        .unwrap_or_else(|| panic!("no symbol {name}"))
}

#[test]
fn code_without_addresses_has_no_relocations() {
    let o = obj("start: lda #1\n sta $d020\n rts\n");
    assert!(o.relocs.is_empty());
    assert_eq!(o.sections.len(), 1);
    assert_eq!(o.sections[0].name, "CODE");
    assert_eq!(o.sections[0].bytes, [0xA9, 1, 0x8D, 0x20, 0xD0, 0x60]);
    assert_eq!(o.cpu, "6502");
    assert_eq!(o.symbols[sym(&o, "start")].class, Class::Label);
}

#[test]
fn a_jump_to_an_import_is_an_absolute_relocation() {
    let o = obj(".import putc\n jmp putc\n lda putc+1\n");
    assert_eq!(o.sections[0].bytes, [0x4C, 0, 0, 0xAD, 1, 0]);
    assert_eq!(o.relocs.len(), 2);
    let r = &o.relocs[0];
    assert_eq!((r.offset, r.width, r.kind), (1, 2, Kind::Abs));
    assert_eq!(r.expr, Expr::Sym(sym(&o, "putc") as u32));
    let r = &o.relocs[1];
    assert_eq!((r.offset, r.width), (4, 2));
    assert_eq!(
        r.expr,
        Expr::Binary(
            3,
            Box::new(Expr::Sym(sym(&o, "putc") as u32)),
            Box::new(Expr::Num(1))
        )
    );
    assert!(o.symbols[sym(&o, "putc")].global);
    assert_eq!(o.symbols[sym(&o, "putc")].class, Class::Import);
}

#[test]
fn a_label_operand_is_absolute_even_when_small() {
    // lda there would be zero page for a constant < 256; the section offset is unknown to the linker
    let o = obj(" lda there\n there: nop\n");
    assert_eq!(o.sections[0].bytes.len(), 4);
    assert_eq!(o.relocs.len(), 1);
    assert_eq!((o.relocs[0].offset, o.relocs[0].width), (1, 2));
}

#[test]
fn an_importzp_symbol_keeps_zero_page_addressing() {
    let o = obj(".importzp tmp\n lda tmp\n sta tmp+1\n");
    assert_eq!(o.sections[0].bytes, [0xA5, 0, 0x85, 1]);
    assert_eq!(o.relocs.len(), 2);
    assert!(o.relocs.iter().all(|r| r.width == 1 && r.kind == Kind::Abs));
    assert_eq!((o.relocs[0].offset, o.relocs[1].offset), (1, 3));
}

#[test]
fn low_and_high_byte_immediates() {
    let o = obj("here: lda #<here\n ldx #>here+1\n");
    assert_eq!(o.relocs.len(), 2);
    assert_eq!((o.relocs[0].offset, o.relocs[0].width), (1, 1));
    assert_eq!((o.relocs[1].offset, o.relocs[1].width), (3, 1));
}

#[test]
fn a_branch_inside_one_section_is_a_constant() {
    let o = obj("top: nop\n bne top\n");
    assert!(o.relocs.is_empty());
    assert_eq!(o.sections[0].bytes, [0xEA, 0xD0, 0xFD]);
}

#[test]
fn a_branch_to_another_section_or_import_is_relative() {
    let o = obj(".import far\n bne far\n");
    assert_eq!(o.relocs.len(), 1);
    let r = &o.relocs[0];
    assert_eq!((r.offset, r.width, r.kind), (1, 1, Kind::Rel));
    assert_eq!((r.bias, r.instr_back), (2, 1));
    let o = obj(".segment \"A\"\n bne there\n.segment \"B\"\nthere: rts\n");
    assert_eq!(o.sections.len(), 3); // CODE, A, B
    assert_eq!(o.relocs.len(), 1);
    assert_eq!(o.relocs[0].kind, Kind::Rel);
    assert_eq!(o.relocs[0].section, 1);
}

#[test]
fn the_difference_of_two_labels_of_one_section_is_a_constant() {
    let o = obj("a: .byte 1,2,3\nb:\n .byte b-a\n .word b-a\n");
    assert!(o.relocs.is_empty(), "{:?}", o.relocs);
    assert_eq!(o.sections[0].bytes, [1, 2, 3, 3, 3, 0]);
}

#[test]
fn data_with_addresses_is_relocated() {
    let o = obj(".import ext\n.word ext, here\n.byte <here, >here\nhere: nop\n");
    assert_eq!(o.relocs.len(), 4);
    let fields: Vec<(u32, u8)> = o.relocs.iter().map(|r| (r.offset, r.width)).collect();
    assert_eq!(fields, [(0, 2), (2, 2), (4, 1), (5, 1)]);
}

#[test]
fn sections_have_their_own_sizes_and_labels() {
    let o = obj(
        ".segment \"CODE\"\nmain: nop\n.segment \"DATA\"\ntable: .byte 1,2\n.segment \"CODE\"\n rts\n.segment \"VARS\", bss\ncounter: .res 3\nbuf: .res 5\n",
    );
    let names: Vec<&str> = o.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["CODE", "DATA", "VARS"]);
    assert_eq!(o.sections[0].bytes, [0xEA, 0x60]);
    assert_eq!(o.sections[1].bytes, [1, 2]);
    assert!(o.sections[2].bss);
    assert_eq!(o.sections[2].size, 8);
    assert!(o.sections[2].bytes.is_empty());
    assert_eq!(o.symbols[sym(&o, "buf")].value, 3);
    assert_eq!(o.symbols[sym(&o, "buf")].section, 2);
}

#[test]
fn exports_and_constants() {
    let o = obj(".export main, LIMIT\nLIMIT = 40\nmain: lda #LIMIT\n");
    assert!(o.symbols[sym(&o, "main")].global);
    let limit = &o.symbols[sym(&o, "LIMIT")];
    assert_eq!(
        (limit.class, limit.value, limit.global),
        (Class::Const, 40, true)
    );
    assert_eq!(o.exports().collect::<Vec<_>>(), ["LIMIT", "main"]);
}

#[test]
fn global_is_an_import_when_nothing_defines_it() {
    let o = obj(".global a, b\na: nop\n jmp b\n");
    assert_eq!(o.symbols[sym(&o, "a")].class, Class::Label);
    assert!(o.symbols[sym(&o, "a")].global);
    assert_eq!(o.symbols[sym(&o, "b")].class, Class::Import);
}

#[test]
fn local_labels_are_scoped_names_in_the_table() {
    let o = obj("one: nop\n@x: bne @x\ntwo: nop\n@x: bne @x\n");
    let names: Vec<&str> = o.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"one@x") && names.contains(&"two@x"),
        "{names:?}"
    );
}

#[test]
fn z80_and_huc6280_relocate_too() {
    let o = obj_for(
        Cpu::Z80,
        ".import func\n call func\n ld hl,here\n here: ret\n",
    )
    .unwrap();
    assert_eq!(o.cpu, "z80");
    assert_eq!(o.relocs.len(), 2);
    assert!(o.relocs.iter().all(|r| r.width == 2 && r.kind == Kind::Abs));
    assert_eq!((o.relocs[0].offset, o.relocs[1].offset), (1, 4));
    let o = obj_for(Cpu::Z80, ".import func\n jr func\n").unwrap();
    assert_eq!(o.relocs[0].kind, Kind::Rel);
    assert_eq!((o.relocs[0].bias, o.relocs[0].instr_back), (2, 1));

    let o = obj_for(
        Cpu::Huc6280,
        ".import a, b, c\n tii a,b,c\n bbr3 $20,a\n bsr b\n",
    )
    .unwrap();
    let fields: Vec<(u32, u8, Kind)> = o
        .relocs
        .iter()
        .map(|r| (r.offset, r.width, r.kind))
        .collect();
    assert_eq!(
        fields,
        [
            (1, 2, Kind::Abs),
            (3, 2, Kind::Abs),
            (5, 2, Kind::Abs),
            (9, 1, Kind::Rel),
            (11, 1, Kind::Rel),
        ]
    );
    // bbr3 zp,rel: the relative field is the third byte, three bytes after the start
    assert_eq!((o.relocs[3].bias, o.relocs[3].instr_back), (3, 2));
}

#[test]
fn mistakes_are_errors() {
    assert!(
        obj_for(Cpu::Mos6502, " jmp nowhere\n")
            .unwrap_err()
            .contains("nowhere")
    );
    assert!(
        obj_for(Cpu::Mos6502, ".org $1000\n")
            .unwrap_err()
            .contains("object")
    );
    assert!(obj_for(Cpu::Mos6502, "x: nop\n.import x\n").is_err());
    assert!(
        obj_for(Cpu::Mos6502, ".export nothing\n")
            .unwrap_err()
            .contains("never defined")
    );
    assert!(
        obj_for(Cpu::Mos6502, ".segment \"V\", bss\n .byte 1\n")
            .unwrap_err()
            .contains("bss")
    );
    assert!(
        obj_for(Cpu::Mos6502, "a: nop\nsize = a\n")
            .unwrap_err()
            .contains("linker")
    );
    assert!(obj_for(Cpu::Mos6502, ".segment \"A\"\n.segment \"A\", bss\n").is_err());
    // flat mode refuses the object directives
    let flat = rcasm::assemble(
        "t",
        ".segment \"A\"\n",
        &Options {
            cpu: Some(Cpu::Mos6502),
            ..Options::default()
        },
        &DiskFiles,
    );
    assert!(flat.is_err());
}

#[test]
fn an_object_survives_encoding() {
    let o = obj(".import f\n.export main\nmain: jsr f\n bne main\n rts\n");
    assert_eq!(Object::decode(&o.encode()).unwrap(), o);
}

/// Mangled sources never panic in object mode; whatever assembles gives an object the reader accepts.
#[test]
fn mutated_sources_never_panic() {
    let seeds = [
        ".import putc\n.export start\nstart: jsr putc\n bne start\n lda #<start\n.word start, putc+2\n.segment \"V\", bss\nx: .res 4\n",
        ".importzp t\n lda t\n sta t+1\n.byte >here, <here\nhere: rts\n",
    ];
    let alphabet = b" \n\t.,:;@#$()<>+-*/\"'abcxyz019";
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut accepted = 0;
    for round in 0..4000 {
        let mut text = seeds[round % seeds.len()].as_bytes().to_vec();
        for _ in 0..(1 + next() % 4) {
            let at = (next() as usize) % text.len();
            match next() % 3 {
                0 => text[at] = alphabet[(next() as usize) % alphabet.len()],
                1 => {
                    text.remove(at);
                }
                _ => text.insert(at, alphabet[(next() as usize) % alphabet.len()]),
            }
            if text.is_empty() {
                text.push(b'\n');
            }
        }
        let Ok(source) = String::from_utf8(text) else {
            continue;
        };
        for cpu in [Cpu::Mos6502, Cpu::Z80, Cpu::Huc6280] {
            let opts = Options {
                cpu: Some(cpu),
                ..Options::default()
            };
            if let Ok(o) = assemble_object("m.asm", &source, &opts, &DiskFiles) {
                accepted += 1;
                assert!(Object::decode(&o.encode()).is_ok(), "{source}");
            }
        }
    }
    assert!(accepted > 100, "only {accepted} mutants assembled");
}
