; main program: clears the screen with the library routine and shows a message.
        .import clear, print
        .export start

        .segment "CODE"
start:  jsr clear
        ldx #0
loop:   lda text,x
        beq done
        jsr print
        inx
        bne loop
done:   rts

        .segment "RODATA"
text:   .byte 8, 5, 12, 12, 15, 32, 3, 54, 52, 0     ; "HELLO C64" in screen codes
