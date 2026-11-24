; Amstrad CPC hardware kit: addresses and values for rcb projects.
SCREEN      equ 0xC000       ; 16 KB, 80 bytes a line, eight 2 KB blocks per character row
GA_PORT     equ 0x7F00       ; gate array: ld bc,GA_PORT | value / out (c),c  (B = 0x7F)
GA_PEN      equ 0x00         ; function bits: select pen (bit 4: border)
GA_INK      equ 0x40         ; set the colour (0-31) of the selected pen
GA_MODE     equ 0x80         ; bits 0-1 screen mode, 2 lower ROM off, 3 upper ROM off
GA_RAM      equ 0xC0
CRTC_SELECT equ 0xBC00       ; out (c),register then
CRTC_DATA   equ 0xBD00       ; out (c),value
PPI_A       equ 0xF400       ; sound chip data / keyboard
PPI_B       equ 0xF500       ; vsync in bit 0
PPI_C       equ 0xF600       ; keyboard row, PSG control
PPI_CTRL    equ 0xF700
; firmware jump block (ROMs on)
KM_WAIT_CHAR equ 0xBB06      ; A = the key pressed
KM_READ_CHAR equ 0xBB09      ; carry set and A = key when one is waiting
TXT_CLEAR_WINDOW equ 0xBB6C
TXT_SET_CURSOR equ 0xBB75    ; H = column, L = row (from 1)
TXT_OUTPUT  equ 0xBB5A       ; print the character in A
TXT_SET_PEN equ 0xBB90
TXT_SET_PAPER equ 0xBB96
SCR_SET_MODE equ 0xBC0E      ; A = 0, 1 or 2
SCR_SET_INK equ 0xBC32       ; A = pen, B and C = colours
SCR_SET_BORDER equ 0xBC38
MC_WAIT_FLYBACK equ 0xBD19
