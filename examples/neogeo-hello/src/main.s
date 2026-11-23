        .org 0
        dc.l    $10F300, start
        ds.b    $F8
        dc.b    "NEO-GEO"       ; the header: see the Neo Geo developer wiki for the rest
        ds.b    $F9
start:  bra.s   start
