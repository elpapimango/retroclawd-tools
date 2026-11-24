//! `rcb new`: a starting project per machine. Each builds as it is and does nothing visible: a loop (or a return)
//! with the vectors a machine needs, so there is a correct skeleton to grow.

pub struct Template {
    /// The machine name as `rcb new --machine` takes it.
    pub machine: &'static str,
    /// The source file name under `src/`.
    pub source_name: &'static str,
    pub source: &'static str,
    /// The part of `rcb.toml` after `[project]`.
    pub layout: &'static str,
    /// More source files of the skeleton (name under `src/`, text), for machines with two programs.
    pub extra_sources: &'static [(&'static str, &'static str)],
}

pub const ALL: &[Template] = &[
    Template {
        machine: "c64",
        source_name: "main.asm",
        source: "* = $080D\nstart:  inc $d020          ; the border flashes\n        jmp start\n",
        layout: "machine = \"c64\"\nentry = \"start\"\n\n[options]\nbasic_stub = true     # `10 SYS 2061`: RUN starts the program\n",
        extra_sources: &[],
    },
    Template {
        machine: "spectrum",
        source_name: "main.asm",
        source: ".org 0x8000\nstart:  ld a,2\n        out (254),a        ; red border\nhere:   jr here\n",
        layout: "machine = \"spectrum\"\nentry = \"start\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "cpc",
        source_name: "main.asm",
        source: ".org 0x4000\nstart:  jp start\n",
        layout: "machine = \"cpc\"\nentry = \"start\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "atari",
        source_name: "main.asm",
        source: ".org $2000\nstart:  inc $d01a          ; the background colour changes\n        jmp start\n",
        layout: "machine = \"atari\"\nentry = \"start\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "atari-st",
        source_name: "main.s",
        source: "        .text\nstart:  clr.w   -(sp)         ; Pterm0\n        trap    #1\n",
        layout: "machine = \"atari-st\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "pcengine",
        source_name: "main.asm",
        source: ".bank 0\n.org $E000\nreset:  sei\n        csh                ; the fast clock\n        cld\n        ldx #$FF\n        txs\nforever: bra forever\nirq2:   rti\nirq1:   rti\ntimer:  rti\nnmi:    rti\n",
        layout: "machine = \"pcengine\"\nentry = \"reset\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "nes",
        source_name: "main.asm",
        source: ".org $8000\nreset:  sei\n        cld\n        ldx #$FF\n        txs\nforever: jmp forever\nnmi:    rti\nirq:    rti\n",
        layout: "machine = \"nes\"\nentry = \"reset\"\n\n[options]\nmapper = 0\nmirroring = \"vertical\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "snes",
        source_name: "main.asm",
        source: ".org $808000\nreset:  sei\n        clc\n        xce                ; native mode\n        rep #$30           ; 16-bit registers\nforever: bra forever\nnmi:    rti\nirq:    rti\n",
        layout: "machine = \"snes\"\nentry = \"reset\"\n\n[options]\nmapping = \"lorom\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "genesis",
        source_name: "main.s",
        source: "        .org $200\nreset:  move.w  #$2700,sr\nforever: bra.s  forever\nvint:   rte\nhint:   rte\next:    rte\n",
        layout: "machine = \"genesis\"\nentry = \"reset\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "neogeo",
        source_name: "main.s",
        source: "        .org 0\n        dc.l    $10F300, start\n        ds.b    $F8\n        dc.b    \"NEO-GEO\"       ; the header: see the Neo Geo developer wiki for the rest\n        ds.b    $F9\nstart:  bra.s   start\n",
        layout: "machine = \"neogeo\"\n\n[options]\nngh = 999\n",
        extra_sources: &[],
    },
    Template {
        machine: "amiga",
        source_name: "main.s",
        source: "        .text\nstart:  moveq   #0,d0         ; return code 0\n        rts\n",
        layout: "machine = \"amiga\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "x68000",
        source_name: "main.s",
        source: "        .text\nstart:  rts\n",
        layout: "machine = \"x68000\"\nentry = \"start\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "segacd",
        source_name: "ip.s",
        source: "        .org $FF0584       ; the main CPU starts here after the security code\nstart:  bra.s   start\n",
        layout: "machine = \"segacd\"\nname = \"MYGAME\"\n\n[options]\nregion = \"U\"\n# security = \"security-us.bin\"   # Sega's region code, from a disc of your own: the BIOS checks it\n",
        extra_sources: &[(
            "sp.s",
            "        .org $6000         ; the sub CPU's module: header, then the entry table\n        dc.b    \"MAIN-RCB\",0,0,0,0 ; 11 characters and a zero\n        dc.b    0,1,0,0\n        dc.l    0,0,$20,0\nentries: dc.w   init-entries, main-entries, level2-entries, user-entries, 0\ninit:   rts\nmain:   bra.s   main\nlevel2: rts\nuser:   rts\n",
        )],
    },
];

pub fn find(machine: &str) -> Option<&'static Template> {
    ALL.iter().find(|t| t.machine == machine)
}

/// The files of a new project: (path, text).
pub fn project(name: &str, t: &Template) -> Vec<(String, String)> {
    let sources = std::iter::once(t.source_name)
        .chain(t.extra_sources.iter().map(|(n, _)| *n))
        .map(|n| format!("\"src/{n}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!(
        "# rcb project: `rcb build`, `rcb run`, `rcb debug`
{}
[project]
name = \"{name}\"
sources = [{}]
",
        t.layout, sources
    );
    let kit = crate::kits::find(t.machine).map_or("", |k| k.text);
    vec![
        ("rcb.toml".to_string(), toml),
        (
            format!("src/{}", t.source_name),
            format!(
                "; the machine's addresses and values: src/kit.inc (`rcb kit {}`)
.include \"src/kit.inc\"
{}",
                t.machine, t.source
            ),
        ),
        ("src/kit.inc".to_string(), kit.to_string()),
        (".gitignore".to_string(), "build/\n".to_string()),
    ]
    .into_iter()
    .chain(
        t.extra_sources
            .iter()
            .map(|(n, text)| (format!("src/{n}"), text.to_string())),
    )
    .collect()
}
