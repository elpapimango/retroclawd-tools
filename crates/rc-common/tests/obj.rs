use rc_common::obj::*;

fn sample() -> Object {
    Object {
        cpu: "6502".into(),
        name: "main.asm".into(),
        files: vec!["main.asm".into(), "inc/hw.inc".into()],
        sections: vec![
            Section {
                name: "CODE".into(),
                bss: false,
                zeropage: false,
                align: 1,
                size: 6,
                addr: None,
                bytes: vec![0x4C, 0, 0, 0xD0, 0xFE, 0x60],
            },
            Section {
                name: "VARS".into(),
                bss: true,
                zeropage: false,
                align: 256,
                size: 16,
                addr: None,
                bytes: vec![],
            },
            Section {
                name: "ZP".into(),
                bss: true,
                zeropage: true,
                align: 1,
                size: 2,
                addr: Some(0x80),
                bytes: vec![],
            },
        ],
        symbols: vec![
            Symbol {
                name: "start".into(),
                class: Class::Label,
                global: true,
                zeropage: false,
                section: 0,
                value: 0,
            },
            Symbol {
                name: "LIMIT".into(),
                class: Class::Const,
                global: true,
                zeropage: false,
                section: 0,
                value: -300,
            },
            Symbol {
                name: "putc".into(),
                class: Class::Import,
                global: true,
                zeropage: false,
                section: 0,
                value: 0,
            },
            Symbol {
                name: "tmp".into(),
                class: Class::Import,
                global: true,
                zeropage: true,
                section: 0,
                value: 0,
            },
        ],
        relocs: vec![
            Reloc {
                section: 0,
                offset: 1,
                width: 2,
                big_endian: false,
                kind: Kind::Abs,
                bias: 0,
                instr_back: 0,
                expr: Expr::Binary(3, Box::new(Expr::Sym(2)), Box::new(Expr::Num(1))),
                file: 0,
                line: 3,
            },
            Reloc {
                section: 0,
                offset: 4,
                width: 1,
                big_endian: false,
                kind: Kind::Rel,
                bias: 2,
                instr_back: 1,
                expr: Expr::Sym(0),
                file: 1,
                line: 9,
            },
        ],
    }
}

#[test]
fn an_object_round_trips() {
    let o = sample();
    let bytes = o.encode();
    assert_eq!(&bytes[..4], b"RCO1");
    assert_eq!(Object::decode(&bytes).unwrap(), o);
    assert_eq!(o.exports().collect::<Vec<_>>(), ["start", "LIMIT"]);
    assert_eq!(o.imports().collect::<Vec<_>>(), ["putc", "tmp"]);
}

#[test]
fn every_truncation_is_an_error_and_none_panics() {
    let bytes = sample().encode();
    for n in 0..bytes.len() {
        assert!(Object::decode(&bytes[..n]).is_err(), "prefix {n}");
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Object::decode(&longer).is_err());
}

#[test]
fn flipping_any_byte_never_panics() {
    let bytes = sample().encode();
    for i in 0..bytes.len() {
        for flip in [0x01u8, 0x80, 0xFF] {
            let mut b = bytes.clone();
            b[i] ^= flip;
            let _ = Object::decode(&b);
        }
    }
}

#[test]
fn huge_claims_fail_before_allocating() {
    // magic, empty cpu, empty name, no files, then a section count of 2^40
    let mut b = b"RCO1".to_vec();
    b.extend([0, 0, 0]);
    b.extend([0x80, 0x80, 0x80, 0x80, 0x80, 0x40]);
    let e = Object::decode(&b).unwrap_err();
    assert!(e.contains("sections"), "{e}");
    // a string of 2^40 bytes
    let mut b = b"RCO1".to_vec();
    b.extend([0x80, 0x80, 0x80, 0x80, 0x80, 0x40]);
    assert!(Object::decode(&b).unwrap_err().contains("string"));
}

#[test]
fn bad_magic_and_bad_references_are_errors() {
    assert!(Object::decode(b"RCO2....").is_err());
    let mut o = sample();
    o.relocs[0].offset = 5; // 5 + 2 > 6
    assert!(
        Object::decode(&o.encode())
            .unwrap_err()
            .contains("outside section")
    );
    let mut o = sample();
    o.symbols[0].section = 9;
    assert!(Object::decode(&o.encode()).is_err());
    let mut o = sample();
    o.symbols[0].value = 99;
    assert!(Object::decode(&o.encode()).is_err());
}

#[test]
fn an_archive_indexes_its_exports() {
    let a = sample();
    let mut b = sample();
    b.name = "b.asm".into();
    b.symbols[0].name = "other".into();
    b.symbols[1].name = "LIMIT2".into();
    let mut lib = Archive::new();
    lib.add("a.rco", &a);
    lib.add("b.rco", &b);
    assert_eq!(lib.find_export("start").unwrap(), Some(0));
    assert_eq!(lib.find_export("other").unwrap(), Some(1));
    assert_eq!(lib.find_export("putc").unwrap(), None);
    let bytes = lib.encode().unwrap();
    let back = Archive::decode(&bytes).unwrap();
    assert_eq!(back, lib);
    assert_eq!(back.member(1).unwrap().name, "b.asm");
    // replacing keeps the position
    lib.add("a.rco", &b);
    assert_eq!(lib.len(), 2);
    assert!(lib.remove("b.rco") && !lib.remove("b.rco"));
}

#[test]
fn archive_damage_is_an_error() {
    let mut lib = Archive::new();
    lib.add("a.rco", &sample());
    let bytes = lib.encode().unwrap();
    for n in 0..bytes.len() {
        assert!(Archive::decode(&bytes[..n]).is_err(), "prefix {n}");
    }
    for i in 0..bytes.len() {
        let mut b = bytes.clone();
        b[i] ^= 0x55;
        let _ = Archive::decode(&b);
    }
    // an index that names the wrong member
    let mut other = Archive::new();
    let mut o = sample();
    o.symbols[0].name = "zzz".into();
    other.add("a.rco", &o);
    let mut b = bytes.clone();
    // swap the stored index for the other archive's: find "start" in the tail and corrupt it
    let at = b.windows(5).rposition(|w| w == b"start").unwrap();
    b[at] = b'X';
    assert!(Archive::decode(&b).unwrap_err().contains("index"));
}

#[test]
fn an_expression_naming_a_missing_symbol_or_nested_too_deep_is_an_error() {
    let mut o = sample();
    o.relocs[0].expr = Expr::Sym(99);
    assert!(
        Object::decode(&o.encode())
            .unwrap_err()
            .contains("symbol 99")
    );
    let mut e = Expr::Num(1);
    for _ in 0..200 {
        e = Expr::Unary(b'-', Box::new(e));
    }
    let mut o = sample();
    o.relocs[0].expr = e;
    assert!(Object::decode(&o.encode()).unwrap_err().contains("nested"));
}
