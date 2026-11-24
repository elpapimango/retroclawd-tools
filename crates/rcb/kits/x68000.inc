; Sharp X68000 hardware and system call kit.
; Human68k DOS calls: dc.w the number (words $FFxx), arguments on the stack
DOS_EXIT        equ $FF00
DOS_GETCHAR     equ $FF01
DOS_PUTCHAR     equ $FF02
DOS_PRINT       equ $FF09     ; push the address of a zero-terminated string
DOS_EXIT2       equ $FF4C
; IOCS calls: trap #15 with the number in d0 [H?: from memory]
IOCS_B_KEYINP   equ $00
IOCS_B_KEYSNS   equ $01
IOCS_CRTMOD     equ $10
IOCS_B_PUTC     equ $20
IOCS_B_PRINT    equ $21       ; a1 = string
IOCS_JOYGET     equ $3B
IOCS_G_CLR_ON   equ $90
; hardware
CRTC            equ $E80000
GVRAM           equ $C00000   ; graphics VRAM
TVRAM           equ $E00000   ; text VRAM
GPALETTE        equ $E82000   ; graphic palette
TPALETTE        equ $E82200   ; text palette
VIDEO_CTRL      equ $E82400
MFP             equ $E88001
PPI_JOY         equ $E9A001
SPRITE_REGS     equ $EB0000
