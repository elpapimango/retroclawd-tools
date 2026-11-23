.org $808000
reset:  sei
        clc
        xce                ; native mode
        rep #$30           ; 16-bit registers
forever: bra forever
nmi:    rti
irq:    rti
