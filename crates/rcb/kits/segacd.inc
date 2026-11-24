; Mega-CD hardware kit (the Mega Drive's own hardware is in `rcb kit genesis`).
; main CPU
MCD_BIOS        equ $000000   ; the Mega-CD BIOS ROM, 128 KB
MCD_PRG_WINDOW  equ $020000   ; 128 KB window onto the sub CPU's program RAM (bank: MCD_MEMMODE bits 7-6)
MCD_WORD_RAM    equ $200000   ; 256 KB word RAM (2M mode)
MCD_RESET       equ $A12000   ; bit 0 SRES (sub reset), bit 1 SBRQ (sub bus request); high byte bit 0: IFL2 interrupt to the sub
MCD_MEMMODE     equ $A12002   ; write protect and bank in the high byte; low byte: bit 2 mode (1 = 1M), 1 DMNA, 0 RET
MCD_CDC_MODE    equ $A12004
MCD_HINT_VECTOR equ $A12006
MCD_CDC_HOST    equ $A12008
MCD_STOPWATCH   equ $A1200C
MCD_COMM_FLAGS  equ $A1200E   ; main writes the high byte, the sub CPU the low byte
MCD_COMM_CMD    equ $A12010   ; 16 bytes the main CPU writes
MCD_COMM_STATUS equ $A12020   ; 16 bytes the sub CPU writes
; sub CPU
SUB_PRG_RAM     equ $000000   ; 512 KB; the BIOS owns the first $6000 bytes, the sub program is loaded at $6000
SUB_WORD_RAM_2M equ $080000
SUB_WORD_RAM_1M equ $0C0000
SUB_BACKUP_RAM  equ $FE0000   ; 8 KB on odd addresses
SUB_PCM         equ $FF0000   ; RF5C164, odd addresses
SUB_REGS        equ $FF8000   ; the gate array as the sub CPU sees it
