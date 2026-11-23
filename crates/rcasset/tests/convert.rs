use rcasset::{Error, Image};

/// An indexed PNG of `w` x `h` with the given palette and pixel indices.
fn indexed_png(w: u32, h: u32, palette: &[[u8; 3]], pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(palette.iter().flatten().copied().collect::<Vec<u8>>());
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(pixels).unwrap();
    }
    out
}

fn rgb_png(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(rgb).unwrap();
    }
    out
}

const PAL: [[u8; 3]; 4] = [[0, 0, 0], [255, 0, 0], [0, 255, 0], [0, 0, 255]];

/// One tile whose row y is `y % 4` repeated, except the first pixel which is 3.
fn tile_pixels() -> Vec<u8> {
    let mut p = Vec::new();
    for y in 0..8u8 {
        for x in 0..8u8 {
            p.push(if x == 0 { 3 } else { y % 4 });
        }
    }
    p
}

#[test]
fn nes_and_snes_planes() {
    let img = Image::from_png(&indexed_png(8, 8, &PAL, &tile_pixels())).unwrap();
    let nes = img.tiles("nes").unwrap();
    assert_eq!(nes.len(), 16);
    // row 1 has colour 1 (bit 0 set) in x 1..8, plus x 0 = 3: plane 0 = 11111111
    assert_eq!(nes[1], 0xFF);
    // row 2 has colour 2 (bit 0 clear) and x 0 = 3 (bit 0 set): plane 0 = 10000000, plane 1 = 11111111
    assert_eq!(nes[2], 0x80);
    assert_eq!(nes[8 + 2], 0xFF);
    let snes = img.tiles("snes2").unwrap();
    assert_eq!(snes[2 * 2], 0x80); // row 2 plane 0
    assert_eq!(snes[2 * 2 + 1], 0xFF); // row 2 plane 1
}

#[test]
fn four_bit_formats() {
    let pal16: Vec<[u8; 3]> = (0..16u8).map(|i| [i * 16, 0, 0]).collect();
    let mut p = Vec::new();
    for y in 0..8u8 {
        for x in 0..8u8 {
            p.push((x + y) & 15);
        }
    }
    let img = Image::from_png(&indexed_png(8, 8, &pal16, &p)).unwrap();
    let md = img.tiles("md4").unwrap();
    assert_eq!(md.len(), 32);
    assert_eq!(md[0], 0x01); // row 0: pixels 0 and 1
    assert_eq!(md[4], 0x12); // row 1: pixels 1 and 2
    let snes = img.tiles("snes4").unwrap();
    assert_eq!(snes.len(), 32);
    // row 0 holds the values 0-7: plane 2 is set for 4-7, plane 3 for none
    assert_eq!(snes[16], 0x0F);
    assert_eq!(snes[17], 0);
    // row 1, plane 3: values 1..8: only 8 (x = 7) has bit 3
    assert_eq!(snes[16 + 2 + 1], 0x01);
}

#[test]
fn rgb_pictures_get_a_palette_in_order_of_appearance() {
    let mut rgb = Vec::new();
    for i in 0..64u8 {
        rgb.extend_from_slice(if i % 2 == 0 {
            &[10, 20, 30]
        } else {
            &[200, 100, 0]
        });
    }
    let img = Image::from_png(&rgb_png(8, 8, &rgb)).unwrap();
    assert_eq!(img.palette, vec![[10, 20, 30], [200, 100, 0]]);
    assert_eq!(img.pixels[0], 0);
    assert_eq!(img.pixels[1], 1);
}

#[test]
fn palettes_in_machine_words() {
    let img = Image::from_png(&indexed_png(8, 8, &PAL, &tile_pixels())).unwrap();
    let snes = img.palette_words("snes").unwrap();
    assert_eq!(&snes[2..4], &0x001Fu16.to_le_bytes()); // red
    assert_eq!(&snes[4..6], &0x03E0u16.to_le_bytes()); // green
    let md = img.palette_words("md").unwrap();
    assert_eq!(&md[2..4], &0x000Eu16.to_be_bytes());
    assert_eq!(&md[6..8], &0x0E00u16.to_be_bytes());
    assert!(img.palette_words("zx").is_err());
}

#[test]
fn the_spectrum_screen_is_interleaved_with_attributes() {
    let pal: Vec<[u8; 3]> = (0..16u8).map(|i| [i, i, i]).collect();
    let mut p = vec![0u8; 256 * 192];
    // a vertical bar of ink 2 (red) on paper 7 in cell (0, 0), x = 0 only; the rest paper 7
    for y in 0..192 {
        for x in 0..256 {
            p[y * 256 + x] = 7;
        }
    }
    for y in 0..8 {
        p[y * 256] = 2;
    }
    let img = Image::from_png(&indexed_png(256, 192, &pal, &p)).unwrap();
    let scr = img.spectrum_screen().unwrap();
    assert_eq!(scr.len(), 6912);
    assert_eq!(scr[0], 0x80);
    assert_eq!(scr[256], 0x80); // row 1 of cell row 0 is 256 bytes on
    assert_eq!(scr[6144], 7 << 3 | 2);
    assert_eq!(scr[6144 + 1], 7 << 3 | 7);
    // the second third of the screen starts at 2048
    let mut p2 = p.clone();
    p2[64 * 256] = 2;
    let scr = Image::from_png(&indexed_png(256, 192, &pal, &p2))
        .unwrap()
        .spectrum_screen()
        .unwrap();
    assert_eq!(scr[2048], 0x80);
}

#[test]
fn mistakes_are_named() {
    let img = Image::from_png(&indexed_png(8, 8, &PAL, &tile_pixels())).unwrap();
    assert!(matches!(img.tiles("bogus"), Err(Error::Format(_))));
    let wide = Image::from_png(&indexed_png(10, 8, &PAL, &[0; 80])).unwrap();
    assert!(matches!(wide.tiles("nes"), Err(Error::Size(_))));
    let mut p = tile_pixels();
    p[5] = 9;
    let pal: Vec<[u8; 3]> = (0..16u8).map(|i| [i, 0, 0]).collect();
    let big = Image::from_png(&indexed_png(8, 8, &pal, &p)).unwrap();
    assert!(matches!(
        big.tiles("nes"),
        Err(Error::TooManyColours { .. })
    ));
    assert!(matches!(Image::from_png(b"not a png"), Err(Error::Png(_))));
    assert!(matches!(img.spectrum_screen(), Err(Error::Size(_))));
}
