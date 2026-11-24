; ZX Spectrum hardware kit: addresses and values for rcb projects.
SCREEN      equ 0x4000       ; 256x192 pixels, 6144 bytes (the third-of-screen layout)
ATTRS       equ 0x5800       ; 32x24 colour cells
FRAMES      equ 0x5C78       ; 3-byte frame counter (system variable)
ULA         equ 0xFE         ; out: bits 0-2 border, 3 MIC, 4 beeper; in: keyboard rows (select with B)
ROW_CAPS_V  equ 0xFEFE       ; in a,(c) with bc = this: bits 0-4 = CAPS Z X C V (0 = pressed)
ROW_A_G     equ 0xFDFE
ROW_Q_T     equ 0xFBFE
ROW_1_5     equ 0xF7FE
ROW_0_6     equ 0xEFFE
ROW_P_Y     equ 0xDFFE
ROW_ENTER_H equ 0xBFFE
ROW_SPACE_B equ 0x7FFE
BANK_PORT   equ 0x7FFD       ; 128K: bits 0-2 RAM bank at 0xC000, 3 screen, 4 ROM
AY_REGISTER equ 0xFFFD       ; 128K: out the register number here
AY_DATA     equ 0xBFFD       ; then the value here
; ROM routines
ROM_CLS     equ 0x0D6B
ROM_CHAN_OPEN equ 0x1601     ; A = stream (2 = the screen) before printing with rst 0x10
ROM_KEY_SCAN equ 0x028E
; attribute bits and colours
BLACK       equ 0
BLUE        equ 1
RED         equ 2
MAGENTA     equ 3
GREEN       equ 4
CYAN        equ 5
YELLOW      equ 6
WHITE       equ 7
BRIGHT      equ 0x40
FLASH       equ 0x80
