//! The hardware kits: an include file per machine with the addresses and values of its hardware (registers, system
//! calls, colours, joypad bits). `rcb new` puts the machine's kit in `src/kit.inc`; `rcb kit MACHINE` prints it.

pub struct Kit {
    pub machine: &'static str,
    pub about: &'static str,
    pub text: &'static str,
}

pub const ALL: &[Kit] = &[
    Kit {
        machine: "c64",
        about: "VIC-II, SID, CIAs, KERNAL calls, colours, joystick bits",
        text: include_str!("../kits/c64.inc"),
    },
    Kit {
        machine: "spectrum",
        about: "screen, attributes, ULA ports, keyboard rows, AY, ROM routines",
        text: include_str!("../kits/spectrum.inc"),
    },
    Kit {
        machine: "cpc",
        about: "gate array, CRTC, PPI, firmware jump block",
        text: include_str!("../kits/cpc.inc"),
    },
    Kit {
        machine: "atari",
        about: "GTIA, POKEY, ANTIC, PIA, OS shadow registers",
        text: include_str!("../kits/atari.inc"),
    },
    Kit {
        machine: "atari-st",
        about: "GEMDOS / BIOS / XBIOS numbers, shifter, YM2149, MFP, system variables",
        text: include_str!("../kits/atari-st.inc"),
    },
    Kit {
        machine: "pcengine",
        about: "VCE, VDC register numbers, PSG, timer, joypad",
        text: include_str!("../kits/pcengine.inc"),
    },
    Kit {
        machine: "nes",
        about: "PPU, APU, joypads, button bits",
        text: include_str!("../kits/nes.inc"),
    },
    Kit {
        machine: "snes",
        about: "PPU, DMA, joypad registers and bits",
        text: include_str!("../kits/snes.inc"),
    },
    Kit {
        machine: "genesis",
        about: "VDP, Z80 bus, pads, TMSS, VDP command words",
        text: include_str!("../kits/genesis.inc"),
    },
    Kit {
        machine: "segacd",
        about: "gate array (main and sub side), word RAM, program RAM window",
        text: include_str!("../kits/segacd.inc"),
    },
    Kit {
        machine: "neogeo",
        about: "the REG_ registers, palette RAM, video RAM map",
        text: include_str!("../kits/neogeo.inc"),
    },
    Kit {
        machine: "amiga",
        about: "exec / dos library vectors, custom chip registers, CIA",
        text: include_str!("../kits/amiga.inc"),
    },
    Kit {
        machine: "x68000",
        about: "Human68k DOS calls, IOCS calls, video and I/O addresses",
        text: include_str!("../kits/x68000.inc"),
    },
];

pub fn find(machine: &str) -> Option<&'static Kit> {
    ALL.iter().find(|k| k.machine == machine)
}
