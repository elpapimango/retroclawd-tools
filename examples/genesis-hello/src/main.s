        .org $200
reset:  move.w  #$2700,sr
forever: bra.s  forever
vint:   rte
hint:   rte
ext:    rte
