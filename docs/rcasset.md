# rcasset: pictures to machine data

```
rcasset tiles   pic.png -f nes|snes2|snes4|md4|pce4 -o tiles.bin    8x8 tiles, left to right, top to bottom
rcasset palette pic.png -f snes|md -o pal.bin                       the palette as machine words
rcasset spectrum pic.png -o screen.scr                              a 256x192 picture as the 6912-byte Spectrum screen
```

The picture is a PNG: an indexed one keeps its palette indices (the index is the colour number in the tile data); an RGB or grey one gets a palette in order of first appearance. A picture with more colours than the
format has bits for, or a size that is not a multiple of 8, is an error that says so. Formats: `nes` 2 bpp (low plane then high plane per tile), `snes2`, `snes4` (planes 0/1 interleaved per row then planes 2/3), `md4` packed
nibbles, `pce4` background tile planes. For the Spectrum screen each 8x8 cell has two colours (palette indices 0-7, 8-15 bright); paper is the colour most of the cell has. Not done yet: sprites with the machines' size and
shape rules, the C64/CPC/Atari bitmap modes, sound (modules to chip registers), tile maps.
