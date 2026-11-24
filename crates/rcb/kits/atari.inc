; Atari 8-bit (800 / XL / XE) hardware kit: addresses and values for rcb projects.
COLPF0      = $D016          ; GTIA colour registers (value = hue * 16 + luminance)
COLPF1      = $D017
COLPF2      = $D018
COLPF3      = $D019
COLBK       = $D01A
CONSOL      = $D01F          ; bit 0 START, 1 SELECT, 2 OPTION (0 = pressed)
TRIG0       = $D010          ; joystick 0 fire (0 = pressed)
AUDF1       = $D200          ; POKEY: frequency, then control (volume 0-15, distortion in bits 5-7)
AUDC1       = $D201
AUDCTL      = $D208
RANDOM      = $D20A
SKCTL       = $D20F
PORTA       = $D300          ; joysticks 0 (low nibble) and 1: bit 0 up, 1 down, 2 left, 3 right
DMACTL      = $D400          ; ANTIC: DMA control
DLISTL      = $D402          ; display list address
DLISTH      = $D403
WSYNC       = $D40A          ; write: wait for the end of the line
VCOUNT      = $D40B          ; scan line / 2
NMIEN       = $D40E
; operating system shadows and entry points
RTCLOK      = $0012          ; 3-byte frame counter
SDMCTL      = $022F
SDLSTL      = $0230
CH          = $02FC          ; last key (255 = none)
COLOR0      = $02C4          ; shadows of COLPF0..COLPF3, COLBK
COLOR4      = $02C8
CIOV        = $E456
