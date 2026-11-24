; Mega Drive / Genesis hardware kit.
VDP_DATA        equ $C00000
VDP_CTRL        equ $C00004   ; word write: $8000 | register << 8 | value ; long write: a memory command
VDP_HV          equ $C00008
PSG             equ $C00011   ; byte writes
YM2612          equ $A04000   ; with the Z80 bus requested
Z80_RAM         equ $A00000
Z80_BUSREQ      equ $A11100   ; write $0100 to request, wait until bit 0 reads 0
Z80_RESET       equ $A11200
VERSION         equ $A10001
PAD1_DATA       equ $A10003
PAD2_DATA       equ $A10005
PAD1_CTRL       equ $A10009   ; write $40 to make the TH line an output
PAD2_CTRL       equ $A1000B
TMSS            equ $A14000   ; write 'SEGA' on a model 1 with a TMSS
WORK_RAM        equ $FF0000
; VDP memory commands (long write to VDP_CTRL): command | ((addr & $3FFF) << 16) | ((addr >> 14) & 3)
VDP_VRAM_WRITE  equ $40000000
VDP_CRAM_WRITE  equ $C0000000
VDP_VSRAM_WRITE equ $40000010
VDP_VRAM_READ   equ $00000000
; the pad after setting TH: bit 0 up, 1 down, 2 left, 3 right, 4 B, 5 C (0 = pressed)
PAD_UP          equ $01
PAD_DOWN        equ $02
PAD_LEFT        equ $04
PAD_RIGHT       equ $08
PAD_B           equ $10
PAD_C           equ $20
