; Neo Geo hardware kit.
WORK_RAM        equ $100000
REG_P1CNT       equ $300000   ; player 1 (active low)
REG_DIPSW       equ $300001   ; read: DIP switches; write: kick the watchdog
REG_SOUND       equ $320000   ; to the sound CPU
REG_STATUS_A    equ $320001
REG_P2CNT       equ $340000
REG_STATUS_B    equ $380000   ; start / select buttons
REG_POUTPUT     equ $380001
REG_SWPBIOS     equ $3A0003   ; use the BIOS vectors
REG_SWPROM      equ $3A0013   ; use the cartridge vectors
REG_VRAMADDR    equ $3C0000
REG_VRAMRW      equ $3C0002
REG_VRAMMOD     equ $3C0004   ; added to the address after each access
REG_LSPCMODE    equ $3C0006
REG_TIMERHIGH   equ $3C0008
REG_TIMERLOW    equ $3C000A
REG_IRQACK      equ $3C000C   ; write to acknowledge: bit 2 vblank, 1 timer, 0 reset
REG_TIMERSTOP   equ $3C000E
PALETTE_RAM     equ $400000   ; 16-bit colours, 16 per palette
BACKUP_RAM      equ $D00000
; video RAM word addresses
VRAM_SCB1       equ $0000     ; sprite tiles
VRAM_FIX        equ $7000     ; the fix layer
VRAM_SCB2       equ $8000     ; sprite shrink
VRAM_SCB3       equ $8200     ; sprite Y and height
VRAM_SCB4       equ $8400     ; sprite X
