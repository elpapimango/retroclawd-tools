//! The format model: conversions keep the machine, and say what they lose.

use rcsnap::carts;
use rcsnap::spectrum::{self, Ay, Format, P2A3, P128, Ram, Spec};
use rcsnap::zx81;

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| {
            (i as u8)
                .wrapping_mul(31)
                .wrapping_add(seed)
                .wrapping_add((i >> 8) as u8)
        })
        .collect()
}

fn spec48() -> Spec {
    let mut s = Spec::blank48(0x8000);
    s.ram = Ram::K48(pattern(spectrum::RAM48_LEN, 1));
    s.cpu.af = 0x1234;
    s.cpu.bc = 0x2345;
    s.cpu.de = 0x3456;
    s.cpu.hl = 0x4567;
    s.cpu.af_ = 0x5678;
    s.cpu.bc_ = 0x6789;
    s.cpu.de_ = 0x789A;
    s.cpu.hl_ = 0x89AB;
    s.cpu.ix = 0x9ABC;
    s.cpu.iy = 0xABCD;
    s.cpu.sp = 0xFF00;
    s.cpu.i = 0x3F;
    s.cpu.r = 0x55;
    s.cpu.iff1 = true;
    s.cpu.iff2 = true;
    s.cpu.im = 2;
    s.border = 3;
    s
}

fn spec128() -> Spec {
    let mut s = spec48();
    s.ram = Ram::Banks(
        (0..8)
            .map(|b| pattern(spectrum::BANK_LEN, b as u8))
            .collect(),
    );
    s.p128 = Some(P128 {
        rom_bank: 1,
        ram_bank: 3,
        shadow_screen: true,
        locked: false,
    });
    s
}

fn spec2a3() -> Spec {
    let mut s = spec128();
    s.p128 = None;
    s.p2a3 = Some(P2A3 {
        ram_bank: 4,
        shadow_screen: false,
        rom_bit_lo: true,
        locked: true,
        special_mode: false,
        bits_1_2: 2,
        disk_motor: false,
    });
    s
}

const ALL: [Format; 3] = [Format::Z80, Format::Sna, Format::Szx];

#[test]
fn a_48k_snapshot_survives_every_format() {
    let s = spec48();
    for f in ALL {
        let back = spectrum::parse(f, &spectrum::write(&s, f).unwrap()).unwrap();
        if f == Format::Sna {
            // A 48K .sna keeps the PC on the stack, so the two bytes below SP now hold it.
            let mut expected = s.clone();
            expected
                .poke(s.cpu.sp - 2, &s.cpu.pc.to_le_bytes())
                .unwrap();
            assert_eq!(back, expected, "{f:?}");
            assert!(
                spectrum::losses(&s, f)
                    .unwrap()
                    .iter()
                    .any(|n| n.contains("overwritten"))
            );
        } else {
            assert_eq!(back, s, "{f:?}");
        }
    }
}

#[test]
fn a_128k_snapshot_survives_every_format() {
    let s = spec128();
    for f in ALL {
        let back = spectrum::parse(f, &spectrum::write(&s, f).unwrap()).unwrap();
        assert_eq!(back, s, "{f:?}");
    }
}

#[test]
fn a_plus3_snapshot_survives_z80_and_szx_and_not_sna() {
    let s = spec2a3();
    for f in [Format::Z80, Format::Szx] {
        let back = spectrum::parse(f, &spectrum::write(&s, f).unwrap()).unwrap();
        assert_eq!(back, s, "{f:?}");
    }
    assert!(
        spectrum::write(&s, Format::Sna)
            .unwrap_err()
            .contains("+2A/+3")
    );
}

#[test]
fn chains_of_conversions_end_where_they_started() {
    let s = spec128();
    let mut bytes = spectrum::write(&s, Format::Z80).unwrap();
    let mut format = Format::Z80;
    for next in [
        Format::Szx,
        Format::Sna,
        Format::Z80,
        Format::Sna,
        Format::Szx,
        Format::Z80,
    ] {
        let spec = spectrum::parse(format, &bytes).unwrap();
        bytes = spectrum::write(&spec, next).unwrap();
        format = next;
    }
    assert_eq!(spectrum::parse(format, &bytes).unwrap(), s);
}

#[test]
fn what_a_format_cannot_keep_is_said() {
    let mut s = spec48();
    s.cpu.iff1 = false;
    s.ay = Some(Ay {
        current_register: 3,
        registers: [7; 16],
    });
    s.cpu.halted = true;
    let sna = spectrum::losses(&s, Format::Sna).unwrap();
    assert!(sna.iter().any(|n| n.contains("IFF2")), "{sna:?}");
    assert!(sna.iter().any(|n| n.contains("stack")), "{sna:?}");
    assert!(sna.iter().any(|n| n.contains("AY")));
    assert!(sna.iter().any(|n| n.contains("halted")));
    let szx = spectrum::losses(&s, Format::Szx).unwrap();
    assert!(szx.is_empty(), "{szx:?}");
    let mut locked = spec128();
    locked.p128.as_mut().unwrap().locked = true;
    assert!(
        spectrum::losses(&locked, Format::Sna)
            .unwrap()
            .iter()
            .any(|n| n.contains("lock"))
    );
}

#[test]
fn the_ay_chip_goes_through_szx() {
    let mut s = spec128();
    s.ay = Some(Ay {
        current_register: 9,
        registers: pattern(16, 5).try_into().unwrap(),
    });
    let back = spectrum::parse(Format::Szx, &spectrum::write(&s, Format::Szx).unwrap()).unwrap();
    assert_eq!(back.ay, s.ay);
}

#[test]
fn the_cpu_address_space_follows_the_paging() {
    let s = spec128();
    let space = s.address_space();
    assert_eq!(space.len(), 0xC000);
    let Ram::Banks(b) = &s.ram else {
        unreachable!()
    };
    assert_eq!(&space[..0x4000], &b[5][..]);
    assert_eq!(&space[0x4000..0x8000], &b[2][..]);
    assert_eq!(&space[0x8000..], &b[3][..], "bank 3 is paged in at 0xC000");
    assert_eq!(s.read(0xC010, 4).unwrap(), b[3][0x10..0x14]);
    assert!(s.read(0x1000, 1).unwrap_err().contains("ROM"));
    assert!(s.read(0xFFFF, 2).is_err());
}

#[test]
fn poke_stays_inside_ram() {
    let mut s = Spec::blank48(0x8000);
    s.poke(0x8000, &[1, 2, 3]).unwrap();
    assert_eq!(s.read(0x8000, 3).unwrap(), [1, 2, 3]);
    assert!(s.poke(0x3FFF, &[1]).unwrap_err().contains("ROM"));
    assert!(s.poke(0xFFFF, &[1, 2]).is_err());
    assert!(spec128().poke(0x8000, &[1]).is_err());
}

#[test]
fn the_z80_version_is_read_from_the_header() {
    let s = spec48();
    let v3 = spectrum::write(&s, Format::Z80).unwrap();
    assert_eq!(spectrum::z80_version(&v3), Some(3));
    // A version 1 header has the PC in bytes 6-7.
    let mut v1 = vec![0u8; 30];
    v1[6] = 0x00;
    v1[7] = 0x80;
    v1.resize(32, 0);
    assert_eq!(spectrum::z80_version(&v1), Some(1));
}

#[test]
fn zx81_names_round_trip() {
    for name in ["HELLO WORLD", "TEST 1.0", "A-B+C=D"] {
        let p81 = zx81::write_p81(name, &[1, 2, 3, 4]).unwrap();
        let back = zx81::parse(&p81, true).unwrap();
        assert_eq!(back.name.as_deref(), Some(name));
        assert_eq!(back.data, [1, 2, 3, 4]);
    }
    assert_eq!(zx81::decode_char(0x26), 'A');
    assert_eq!(zx81::decode_char(0x1C), '0');
    assert_eq!(
        zx81::decode_char(0x80 | 0x25),
        '9',
        "inverse video prints as the plain character"
    );
    assert!(zx81::write_p81("no~way", &[]).unwrap_err().contains("~"));
    assert!(zx81::write_p81("", &[]).is_err());
}

#[test]
fn zx81_system_variables_give_the_basic_lines() {
    // VERSN, E_PPC, D_FILE (0x4082), DF_CC, VARS (0x4082), DEST, E_LINE (0x4082), ...; one line: 10 REM
    let mut data = vec![0u8; 0x407D - 0x4009];
    data[3..5].copy_from_slice(&0x4083u16.to_le_bytes());
    data[7..9].copy_from_slice(&0x4083u16.to_le_bytes());
    data[11..13].copy_from_slice(&0x4083u16.to_le_bytes());
    // one line: number 10 (big endian), length 2, "REM" token and newline
    data.extend_from_slice(&[0, 10, 2, 0, 0xEA, 0x76]);
    data[3..5].copy_from_slice(&((0x407D + 6) as u16).to_le_bytes());
    let f = zx81::facts(&data).unwrap();
    assert_eq!(
        (f.lines, f.first_line, f.last_line),
        (1, Some(10), Some(10))
    );
}

#[test]
fn plain_cartridges_go_to_raw_and_back() {
    let rom8 = pattern(0x2000, 9);
    let crt = carts::crt_from_raw("TEST", &rom8).unwrap();
    assert!(
        !crt.exrom_level && crt.game_level,
        "an 8K cartridge pulls EXROM low only"
    );
    let parsed = c64_cartridge::parse(&c64_cartridge::write(&crt)).unwrap();
    assert_eq!(carts::raw_from_crt(&parsed).unwrap(), rom8);
    let rom16 = pattern(0x4000, 2);
    let crt = carts::crt_from_raw("BIG", &rom16).unwrap();
    assert!(
        !crt.exrom_level && !crt.game_level,
        "a 16K cartridge pulls both low"
    );
    assert_eq!(carts::raw_from_crt(&crt).unwrap(), rom16);
    assert!(
        carts::crt_from_raw("X", &[0; 100])
            .unwrap_err()
            .contains("8192 or 16384")
    );
    let mut banked = crt.clone();
    banked.hardware_type = 5;
    assert!(carts::raw_from_crt(&banked).unwrap_err().contains("Ocean"));
    assert_eq!(carts::crt_type_name(32), "EasyFlash (type 32)");
    assert_eq!(carts::crt_type_name(99), "type 99");
}

#[test]
fn cpr_pages_go_to_raw_and_back() {
    let raw = pattern(3 * cpc_cartridge::PAGE_SIZE, 4);
    let (cart, note) = carts::cpr_from_raw(&raw).unwrap();
    assert!(note.is_none());
    let parsed = cpc_cartridge::parse(&cpc_cartridge::write(&cart).unwrap()).unwrap();
    assert_eq!(carts::raw_from_cpr(&parsed).unwrap(), raw);
    let (_, note) = carts::cpr_from_raw(&raw[..raw.len() - 5]).unwrap();
    assert!(note.unwrap().contains("padded"));
    let mut gap = parsed.clone();
    gap.pages.remove(1);
    assert!(
        carts::raw_from_cpr(&gap)
            .unwrap_err()
            .contains("page 1 is missing")
    );
    assert!(carts::cpr_from_raw(&[]).is_err());
    assert!(carts::cpr_from_raw(&vec![0; 33 * cpc_cartridge::PAGE_SIZE]).is_err());
}
