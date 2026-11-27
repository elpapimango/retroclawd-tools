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
; Checked against chibiakumas.com/68000/neogeo.php (2026-11-26): every register, palette and VRAM
; address agrees. The REG_IRQACK bit meanings and BACKUP_RAM are not on that page ([H?]).
PALETTE_REFERENCE equ $400000 ; must hold $8000
PALETTE_BACKDROP equ $401FFE
REG_BRAMLOCK    equ $3A000D   ; MVS backup RAM
REG_BRAMUNLOCK  equ $3A001D
; P1CNT/P2CNT: D C B A right left down up (bit 7..0, 0 = pressed)
; BIOS variables
BIOS_P1CURRENT  equ $10FD96
BIOS_USER_REQUEST equ $10FDAE ; the command for USER ($122)
BIOS_USER_MODE  equ $10FDAF   ; 0 init, 1 title/demo, 2 game
BIOS_START_FLAG equ $10FDB4
BIOS_BACKUP_MODE equ $10FEBF
; BIOS calls (jsr)
SYSTEM_RESET    equ $C00402
SYSTEM_INT1     equ $C00438   ; the vertical blank handler's BIOS part
SYSTEM_RETURN   equ $C00444   ; jmp here at the end of USER
SYSTEM_IO       equ $C0044A   ; read the controls
CREDIT_CHECK    equ $C00450
CREDIT_DOWN     equ $C00456
READ_CALENDAR   equ $C0045C
FIX_CLEAR       equ $C004C2
LSP_1ST         equ $C004C8   ; clear the sprites
MESS_OUT        equ $C004CE
BIOS_SYSTEM_MODE equ $10FD80  ; bit 7: 1 = the game runs (its vertical blank handler does the work)
REG_PALBANK1    equ $3A000F   ; the window at $400000 shows palette bank 1 / 0 (write any value)
REG_PALBANK0    equ $3A001F
