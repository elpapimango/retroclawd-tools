; Atari ST hardware and system call kit.
; GEMDOS: trap #1, function number on the stack (word) first
GEMDOS_PTERM0   equ 0
GEMDOS_CCONIN   equ 1
GEMDOS_CCONOUT  equ 2
GEMDOS_CCONWS   equ 9         ; push the address of a zero-terminated string
GEMDOS_CCONIS   equ $0B
GEMDOS_SUPER    equ $20       ; supervisor mode (push 0 to switch on)
; BIOS: trap #13
BIOS_BCONSTAT   equ 1
BIOS_BCONIN     equ 2
BIOS_BCONOUT    equ 3
; XBIOS: trap #14
XBIOS_PHYSBASE  equ 2
XBIOS_GETREZ    equ 4
XBIOS_SETSCREEN equ 5
XBIOS_SETPALETTE equ 6
XBIOS_SETCOLOR  equ 7
XBIOS_RANDOM    equ 17
XBIOS_VSYNC     equ 37
XBIOS_SUPEXEC   equ 38
; hardware (supervisor mode only)
SHIFTER_BASE_HI equ $FF8201   ; screen base, high / middle bytes
SHIFTER_BASE_MID equ $FF8203
SHIFTER_PALETTE equ $FF8240   ; 16 words 0000 0RRR 0GGG 0BBB
SHIFTER_RES     equ $FF8260   ; 0 low (320x200, 16 colours), 1 medium (640x200, 4), 2 high (640x400, mono)
YM_SELECT       equ $FF8800   ; write the register number; read the register
YM_WRITE        equ $FF8802
MFP             equ $FFFA01
KBD_CTRL        equ $FFFC00
KBD_DATA        equ $FFFC02
; system variables
V_BAS_AD        equ $044E     ; screen base address
FRCLOCK         equ $0466     ; vertical blank counter (long)
HZ_200          equ $04BA     ; 200 Hz counter (long)
