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
}

pub const ALL: &[Template] = &[
    Template {
        machine: "c64",
        source_name: "main.asm",
        source: "* = $080D\nstart:  inc $d020          ; the border flashes\n        jmp start\n",
        layout: "machine = \"c64\"\nentry = \"start\"\n\n[options]\nbasic_stub = true     # `10 SYS 2061`: RUN starts the program\n",
    },
    Template {
        machine: "spectrum",
        source_name: "main.asm",
        source: ".org 0x8000\nstart:  ld a,2\n        out (254),a        ; red border\nhere:   jr here\n",
        layout: "machine = \"spectrum\"\nentry = \"start\"\n",
    },
    Template {
        machine: "cpc",
        source_name: "main.asm",
        source: ".org 0x4000\nstart:  jp start\n",
        layout: "machine = \"cpc\"\nentry = \"start\"\n",
    },
    Template {
        machine: "atari",
        source_name: "main.asm",
        source: ".org $2000\nstart:  inc $d01a          ; the background colour changes\n        jmp start\n",
        layout: "machine = \"atari\"\nentry = \"start\"\n",
    },
    Template {
        machine: "atari-st",
        source_name: "main.s",
        source: "        .text\nstart:  clr.w   -(sp)         ; Pterm0\n        trap    #1\n",
        layout: "machine = \"atari-st\"\n",
    },
    Template {
        machine: "pcengine",
        source_name: "main.asm",
        source: ".bank 0\n.org $E000\nreset:  sei\n        csh                ; the fast clock\n        cld\n        ldx #$FF\n        txs\nforever: bra forever\nirq2:   rti\nirq1:   rti\ntimer:  rti\nnmi:    rti\n",
        layout: "machine = \"pcengine\"\nentry = \"reset\"\n",
    },
    Template {
        machine: "nes",
        source_name: "main.asm",
        source: ".org $8000\nreset:  sei\n        cld\n        ldx #$FF\n        txs\nforever: jmp forever\nnmi:    rti\nirq:    rti\n",
        layout: "machine = \"nes\"\nentry = \"reset\"\n\n[options]\nmapper = 0\nmirroring = \"vertical\"\n",
    },
    Template {
        machine: "snes",
        source_name: "main.asm",
        source: ".org $808000\nreset:  sei\n        clc\n        xce                ; native mode\n        rep #$30           ; 16-bit registers\nforever: bra forever\nnmi:    rti\nirq:    rti\n",
        layout: "machine = \"snes\"\nentry = \"reset\"\n\n[options]\nmapping = \"lorom\"\n",
    },
    Template {
        machine: "genesis",
        source_name: "main.s",
        source: "        .org $200\nreset:  move.w  #$2700,sr\nforever: bra.s  forever\nvint:   rte\nhint:   rte\next:    rte\n",
        layout: "machine = \"genesis\"\nentry = \"reset\"\n",
    },
    Template {
        machine: "neogeo",
        source_name: "main.s",
        source: "        .org 0\n        dc.l    $10F300, start\n        ds.b    $F8\n        dc.b    \"NEO-GEO\"       ; the header: see the Neo Geo developer wiki for the rest\n        ds.b    $F9\nstart:  bra.s   start\n",
        layout: "machine = \"neogeo\"\n\n[options]\nngh = 999\n",
    },
    Template {
        machine: "amiga",
        source_name: "main.s",
        source: "        .text\nstart:  moveq   #0,d0         ; return code 0\n        rts\n",
        layout: "machine = \"amiga\"\n",
    },
    Template {
        machine: "x68000",
        source_name: "main.s",
        source: "        .text\nstart:  rts\n",
        layout: "machine = \"x68000\"\nentry = \"start\"\n",
    },
];

pub fn find(machine: &str) -> Option<&'static Template> {
    ALL.iter().find(|t| t.machine == machine)
}

/// The files of a new project: (path, text).
pub fn project(name: &str, t: &Template) -> Vec<(String, String)> {
    let toml = format!(
        "# rcb project: `rcb build`, `rcb run`, `rcb debug`
{}
[project]
name = \"{name}\"
sources = [\"src/{}\"]
",
        t.layout, t.source_name
    );
    vec![
        ("rcb.toml".to_string(), toml),
        (format!("src/{}", t.source_name), t.source.to_string()),
        (".gitignore".to_string(), "build/\n".to_string()),
    ]
}
