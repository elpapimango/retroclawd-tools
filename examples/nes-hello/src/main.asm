.org $8000
reset:  sei
        cld
        ldx #$FF
        txs
forever: jmp forever
nmi:    rti
irq:    rti
