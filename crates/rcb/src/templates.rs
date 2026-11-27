//! `rcb new`: a starting project per machine. Each builds as it is and shows that it runs in the smallest way the
//! machine allows (a border or background colour, a line of text) with the vectors and header the machine needs, so
//! there is a correct skeleton to grow. The 68000 ones follow the Chibi Akumas lessons (chibiakumas.com).

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
        source: "        .text\nstart:  pea     message\n        move.w  #GEMDOS_CCONWS,-(sp)\n        trap    #1\n        addq.l  #6,sp\n        move.w  #GEMDOS_CCONIN,-(sp) ; wait for a key\n        trap    #1\n        addq.l  #2,sp\n        clr.w   -(sp)                ; Pterm0\n        trap    #1\n        .data\nmessage: dc.b   \"Hello from rcb!\",13,10,0\n",
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
        source: "        .org $200\nreset:  move.w  #$2700,sr\n        move.b  VERSION,d0      ; a console with a TMSS wants \"SEGA\" written first\n        andi.b  #$0F,d0\n        beq.s   notmss\n        move.l  #$53454741,TMSS\nnotmss: move.w  VDP_CTRL,d0     ; reading the status resets the control port\n        move.w  #$8004,VDP_CTRL ; register 0\n        move.w  #$8144,VDP_CTRL ; register 1: display on, mode 5\n        move.w  #$8700,VDP_CTRL ; register 7: the backdrop is palette 0 colour 0\n        move.l  #VDP_CRAM_WRITE,VDP_CTRL\n        move.w  #$000E,VDP_DATA ; colour 0 = red (----BBB-GGG-RRR-): the screen turns red\nforever: bra.s  forever\nvint:   rte\nhint:   rte\next:    rte\n",
        layout: "machine = \"genesis\"\nentry = \"reset\"\n",
        extra_sources: &[],
    },
    Template {
        machine: "neogeo",
        source_name: "main.s",
        source: "        .org 0\n        dc.l    $10F300, $C00402 ; stack; reset: the BIOS starts and calls USER\n        dc.l    $C00408, $C0040E, $C00414 ; bus error, address error, illegal instruction: the BIOS's handlers\n        dc.l    none, none, none ; divide by zero, chk, trapv\n        dc.l    $C0041A, $C00420 ; privilege violation, trace\n        dc.l    none, none      ; line A, line F\n        dc.l    $C00426, $C00426, $C00426 ; reserved\n        dc.l    $C0042C         ; uninitialised interrupt\n        dc.l    $C00426, $C00426, $C00426, $C00426, $C00426, $C00426, $C00426, $C00426 ; reserved\n        dc.l    $C00432         ; spurious interrupt\n        dc.l    vblank          ; level 1: vertical blank\n        dc.l    timer           ; level 2: the timer\n        dc.l    reset_irq       ; level 3: the power-on interrupt\n        ds.b    $100-$70\n        dc.b    \"NEO-GEO\",0     ; $100: a cartridge, system version 0\n        dc.w    $0999           ; $108: the game number in BCD (options.ngh)\n        dc.l    $100000         ; $10A: the program ROM size\n        dc.l    $100000         ; $10E: the backup RAM block\n        dc.w    0               ; $112: its size\n        dc.b    2,0             ; $114: no eye catcher\n        dc.l    dips,dips,dips  ; $116: software DIPs (Japan, USA, Europe)\n        dc.w    $4EF9           ; $122: jmp USER\n        dc.l    user\n        dc.w    $4EF9           ; $128: jmp PLAYER_START\n        dc.l    player_start\n        dc.w    $4EF9           ; $12E: jmp DEMO_END\n        dc.l    demo_end\n        dc.w    $4EF9           ; $134: jmp COIN_SOUND\n        dc.l    coin_sound\n        ds.b    $182-$13A\n        dc.l    security        ; $182: SNK's security code, which the BIOS compares\ndips:   dc.b    \"RCB PROJECT     \" ; the game name, 16 characters\n        dc.b    $FF,$FF,$FF,$FF,$FF,$FF\n        dc.b    0,0,0,0,0,0,0,0,0,0\nsecurity:                       ; SNK's security code goes here: options.security in rcb.toml\n        ds.b    122\nuser:   move.b  d0,REG_DIPSW    ; kick the watchdog\n        move.b  BIOS_USER_REQUEST,d0\n        cmpi.b  #2,d0\n        bcc.s   game            ; 2 demo, 3 title: the program runs\n        jmp     SYSTEM_RETURN   ; 0 initialise, 1 eye catcher: nothing to do\ngame:   move.w  #$2000,sr\n        jsr     FIX_CLEAR       ; the BIOS's own screen goes\n        jsr     LSP_1ST\n        move.b  d0,REG_PALBANK0 ; show palette bank 0\n        move.w  #$4F00,PALETTE_BACKDROP ; the backdrop turns red\nforever: move.b d0,REG_DIPSW\n        bra.s   forever\nvblank: move.w  #4,REG_IRQACK   ; acknowledge first: the interrupt stays raised until then\n        move.b  d0,REG_DIPSW    ; kick the watchdog\n        btst    #7,BIOS_SYSTEM_MODE\n        bne.s   ours\n        jmp     SYSTEM_INT1     ; the BIOS's own frames\nours:   rte\ntimer:  move.w  #2,REG_IRQACK\n        rte\nreset_irq:\n        move.w  #1,REG_IRQACK\nnone:   rte\nplayer_start:\ndemo_end:\ncoin_sound:\n        rts\n",
        layout: "machine = \"neogeo\"\n\n[options]\nngh = 999\n# security = \"../roms/game-p1.p1\"  # SNK's security code, which the BIOS checks: the program file of a cartridge of your own\n",
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
        source: "        .org $FF0584       ; the main CPU starts here after the security code\nstart:  move.w  #$2700,sr\n        move.w  VDP_CTRL,d0     ; reading the status resets the control port\n        move.w  #$8144,VDP_CTRL ; register 1: display on, mode 5\n        move.w  #$8700,VDP_CTRL ; register 7: the backdrop is palette 0 colour 0\n        move.l  #VDP_CRAM_WRITE,VDP_CTRL\n        move.w  #$000E,VDP_DATA ; colour 0 = red: the backdrop behind the BIOS's licence screen turns red\nforever: bra.s  forever\n",
        layout: "machine = \"segacd\"\nname = \"MYGAME\"\n\n[options]\nregion = \"U\"\n# security = \"../discs/My Game (USA).bin\"   # Sega's region code: a disc of your own of the same region (.iso or the\n#                                            # track 1 .bin), or the 1412 bytes cut from one. The BIOS checks it.\n",
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
