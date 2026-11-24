; PC Engine / TurboGrafx-16 hardware kit: addresses and values for rcb projects.
; The hardware page ($FF) is mapped at $0000 at reset. Reach the video chip with st0 #reg / st1 #low / st2 #high
; (the VDC registers below); do not write them with sta $0000: an address below $0100 is zero page ($2000).
VCE_CTRL    = $0400
VCE_INDEX   = $0402          ; colour index 0-511 (low, high)
VCE_DATA    = $0404          ; low byte GGRRRBBB, high byte bit 0 = G's low bit
PSG_CHANNEL = $0800          ; 0-5
PSG_BALANCE = $0801
PSG_FREQ_LO = $0802
PSG_FREQ_HI = $0803
PSG_CONTROL = $0804          ; bit 7 on, 6 reset write index, 0-4 volume
PSG_PAN     = $0805
PSG_WAVE    = $0806          ; write 32 five-bit samples
TIMER_COUNT = $0C00
TIMER_CTRL  = $0C01
JOYPAD      = $1000          ; write 1 then 0 selecting directions / buttons; read the low nibble
IRQ_DISABLE = $1402
IRQ_STATUS  = $1403
; VDC registers (select with st0, then data with st1 and st2)
VDC_MAWR    = $00            ; write address
VDC_MARR    = $01
VDC_VWR     = $02            ; data
VDC_CR      = $05            ; control: bit 7 background on, 6 sprites on, interrupt enables
VDC_RCR     = $06
VDC_BXR     = $07
VDC_BYR     = $08
VDC_MWR     = $09            ; map size
VDC_HSR     = $0A
VDC_HDR     = $0B
VDC_VPR     = $0C
VDC_VDW     = $0D
VDC_VCR     = $0E
VDC_DCR     = $0F
VDC_SOUR    = $10
VDC_DESR    = $11
VDC_LENR    = $12
VDC_SATB    = $13            ; sprite attribute table source
JOY_I       = $01            ; buttons, with the select line 0: I, II, select, run
JOY_II      = $02
JOY_SELECT  = $04
JOY_RUN     = $08
