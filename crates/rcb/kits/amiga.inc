; Amiga hardware and library kit.
EXECBASE        equ 4         ; the pointer to exec.library
; exec.library vectors (a6 = exec base)
LVO_DISABLE     equ -120
LVO_ENABLE      equ -126
LVO_FORBID      equ -132
LVO_PERMIT      equ -138
LVO_ALLOCMEM    equ -198
LVO_FREEMEM     equ -210
LVO_CLOSELIBRARY equ -414
LVO_OPENLIBRARY equ -552
; dos.library vectors (a6 = dos base)
LVO_OPEN        equ -30
LVO_CLOSE       equ -36
LVO_READ        equ -42
LVO_WRITE       equ -48
LVO_OUTPUT      equ -60
LVO_DELAY       equ -198
; custom chips: add to CUSTOM (a base register is the usual way)
CUSTOM          equ $DFF000
DMACONR         equ $002
VPOSR           equ $004
VHPOSR          equ $006
JOY0DAT         equ $00A
JOY1DAT         equ $00C
POTGOR          equ $016
INTENAR         equ $01C
INTREQR         equ $01E
BPL1PTH         equ $0E0
COP1LC          equ $080
COPJMP1         equ $088
DIWSTRT         equ $08E
DIWSTOP         equ $090
DDFSTRT         equ $092
DDFSTOP         equ $094
DMACON          equ $096      ; bit 15 set/clear, 9 master, 8 bitplanes, 7 copper, 6 blitter
INTENA          equ $09A
INTREQ          equ $09C
BPLCON0         equ $100
COLOR00         equ $180
CIAA_PRA        equ $BFE001   ; bit 6 / 7: fire buttons of ports 1 / 2 (0 = pressed)
