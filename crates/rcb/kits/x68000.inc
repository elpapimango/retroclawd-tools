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
MFP             equ $E88001   ; GPIP (bit 4: vertical display)
PPI_JOY         equ $E9A001
SPRITE_REGS     equ $EB0000
; Checked against chibiakumas.com/68000/x68000.php (2026-11-26): DOS calls, CRTC, VRAM, palettes,
; PPI and sprite addresses agree. The named IOCS calls above are not on that page ([H?]).
DOS_SUPER       equ $FF20     ; push 0.l: supervisor mode
IOCS_MS_INIT    equ $70
IOCS_MS_CURON   equ $71
IOCS_MS_CUROF   equ $72
IOCS_MS_GETDT   equ $74       ; d0 = $XXYYLLRR
CRTC_R20        equ $E80028   ; mode control
VC_R1           equ $E82500   ; layer priority
VC_R2           equ $E82600   ; layers on/off
SPRITE_PALETTE  equ $E82220
PPI_JOY2        equ $E9A003
PPI_CTRL        equ $E9A005
BG_CTRL         equ $EB0808
PCG             equ $EB8000   ; sprite patterns, 128 bytes each
; joystick (PPI_JOY): bit 6 button 2, 5 button 1, 3 right, 2 left, 1 down, 0 up
; palettes: GGGGG RRRRR BBBBB plus the transparent bit
