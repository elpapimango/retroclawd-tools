; a small library: clear the screen and print a character through a cursor in zero page.
        .export clear, print

        .segment "CODE"
clear:  lda #32
        ldx #0
c1:     sta $0400,x
        sta $0500,x
        sta $0600,x
        sta $0700,x
        inx
        bne c1
        lda #0
        sta pos
        rts

print:  ldy pos
        sta $0400,y
        inc pos
        rts

        .segment "BSS"
pos:    .res 1
