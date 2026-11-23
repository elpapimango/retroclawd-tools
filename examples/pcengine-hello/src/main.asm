.bank 0
.org $E000
reset:  sei
        csh                ; the fast clock
        cld
        ldx #$FF
        txs
forever: bra forever
irq2:   rti
irq1:   rti
timer:  rti
nmi:    rti
