//! Picture converters: a PNG becomes the tile or bitmap data of a machine, and its colours the machine's palette.
//!
//! A picture is read as indexed colour: an indexed PNG keeps its palette indices; an RGB(A) or grey PNG gets a palette
//! of the colours in the order they first appear (an error if there are more than the format can hold). Pixels are then
//! cut into 8x8 tiles, left to right, top to bottom (the width and height are multiples of 8).
//!
//! | format | bits per pixel | layout of a tile |
//! |---|---|---|
//! | `nes` | 2 | 16 bytes: the low bit plane (8 rows), then the high plane; the leftmost pixel is bit 7 |
//! | `snes2` | 2 | 16 bytes: per row, plane 0 then plane 1 |
//! | `snes4` | 4 | 32 bytes: planes 0 and 1 interleaved per row (16 bytes), then planes 2 and 3 |
//! | `md4` | 4 | 32 bytes: per row 4 bytes of packed nibbles, the left pixel in the high nibble |
//! | `pce4` | 4 | 32 bytes as the HuC6270's background tile: planes 0 and 1 per row, then planes 2 and 3 |
//! | `spectrum` | 1+attr | the 6912-byte screen: 6144 bitmap bytes in the Spectrum's interleaved order, 768 attributes |
//!
//! Palettes: `snes` words `0BBBBBGGGGGRRRRR` (little endian), `md` words `0000BBB0GGG0RRR0` (big endian).

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// One palette index per pixel.
    pub pixels: Vec<u8>,
    pub palette: Vec<[u8; 3]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Png(String),
    TooManyColours { found: usize, allowed: usize },
    Size(String),
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Png(e) => write!(f, "cannot read the picture: {e}"),
            Error::TooManyColours { found, allowed } => {
                write!(
                    f,
                    "the picture uses {found} colours, the format has {allowed}"
                )
            }
            Error::Size(s) | Error::Format(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

pub const MAX_PIXELS: usize = 1 << 24;

impl Image {
    pub fn from_png(data: &[u8]) -> Result<Image, Error> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
        decoder.set_transformations(png::Transformations::IDENTITY);
        let mut reader = decoder.read_info().map_err(|e| Error::Png(e.to_string()))?;
        let info = reader.info().clone();
        let (w, h) = (info.width as usize, info.height as usize);
        if w == 0 || h == 0 || w.saturating_mul(h) > MAX_PIXELS {
            return Err(Error::Size(format!("a picture of {w} x {h} pixels")));
        }
        let mut buf = vec![0; reader.output_buffer_size()];
        let frame = reader
            .next_frame(&mut buf)
            .map_err(|e| Error::Png(e.to_string()))?;
        let bytes = &buf[..frame.buffer_size()];
        let depth = frame.bit_depth as usize;
        let mut pixels = Vec::with_capacity(w * h);
        let mut palette: Vec<[u8; 3]> = Vec::new();
        let sample = |row: &[u8], x: usize| -> u8 {
            match depth {
                8 => row[x],
                16 => row[x * 2],
                d => {
                    let per = 8 / d;
                    let byte = row[x / per];
                    let shift = 8 - d * (x % per + 1);
                    (byte >> shift) & ((1 << d) - 1)
                }
            }
        };
        let row_len = frame.line_size;
        match frame.color_type {
            png::ColorType::Indexed => {
                let pal = info
                    .palette
                    .as_ref()
                    .ok_or_else(|| Error::Png("no palette".to_string()))?;
                palette = pal.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
                for y in 0..h {
                    let row = &bytes[y * row_len..(y + 1) * row_len];
                    for x in 0..w {
                        pixels.push(sample(row, x));
                    }
                }
            }
            ct => {
                let channels = match ct {
                    png::ColorType::Grayscale => 1,
                    png::ColorType::GrayscaleAlpha => 2,
                    png::ColorType::Rgb => 3,
                    _ => 4,
                };
                let step = channels * depth.div_ceil(8);
                if depth < 8 && channels == 1 {
                    // grey with fewer than 8 bits: the sample is the grey level
                    for y in 0..h {
                        let row = &bytes[y * row_len..(y + 1) * row_len];
                        for x in 0..w {
                            let g = sample(row, x) as u16 * 255 / ((1 << depth) - 1);
                            let c = [g as u8; 3];
                            let i = palette.iter().position(|p| *p == c).unwrap_or_else(|| {
                                palette.push(c);
                                palette.len() - 1
                            });
                            pixels.push(i as u8);
                        }
                    }
                } else {
                    let bpc = depth.div_ceil(8);
                    for y in 0..h {
                        let row = &bytes[y * row_len..(y + 1) * row_len];
                        for x in 0..w {
                            let p = &row[x * step..(x + 1) * step];
                            let c = if channels <= 2 {
                                [p[0]; 3]
                            } else {
                                [p[0], p[bpc], p[2 * bpc]]
                            };
                            let i = match palette.iter().position(|q| *q == c) {
                                Some(i) => i,
                                None => {
                                    if palette.len() >= 256 {
                                        return Err(Error::TooManyColours {
                                            found: 257,
                                            allowed: 256,
                                        });
                                    }
                                    palette.push(c);
                                    palette.len() - 1
                                }
                            };
                            pixels.push(i as u8);
                        }
                    }
                }
            }
        }
        Ok(Image {
            width: w,
            height: h,
            pixels,
            palette,
        })
    }

    fn pixel(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * self.width + x]
    }

    /// The tiles of a picture as the format `name` lays them out.
    pub fn tiles(&self, name: &str) -> Result<Vec<u8>, Error> {
        let (bpp, tile_bytes): (u32, usize) = match name {
            "nes" | "snes2" => (2, 16),
            "snes4" | "md4" | "pce4" => (4, 32),
            _ => {
                return Err(Error::Format(format!(
                    "unknown tile format \"{name}\" (nes, snes2, snes4, md4, pce4)"
                )));
            }
        };
        if !self.width.is_multiple_of(8) || !self.height.is_multiple_of(8) {
            return Err(Error::Size(format!(
                "{} x {}: the width and height are multiples of 8",
                self.width, self.height
            )));
        }
        if let Some(&m) = self.pixels.iter().max()
            && (m as u32) >= 1 << bpp {
                return Err(Error::TooManyColours {
                    found: m as usize + 1,
                    allowed: 1 << bpp,
                });
            }
        let mut out = Vec::with_capacity(self.width * self.height / 64 * tile_bytes);
        for ty in 0..self.height / 8 {
            for tx in 0..self.width / 8 {
                let px = |x: usize, y: usize| self.pixel(tx * 8 + x, ty * 8 + y);
                let plane = |y: usize, p: u32| -> u8 {
                    (0..8).fold(0u8, |a, x| a | ((px(x, y) >> p) & 1) << (7 - x))
                };
                match name {
                    "nes" => {
                        for y in 0..8 {
                            out.push(plane(y, 0));
                        }
                        for y in 0..8 {
                            out.push(plane(y, 1));
                        }
                    }
                    "snes2" => {
                        for y in 0..8 {
                            out.push(plane(y, 0));
                            out.push(plane(y, 1));
                        }
                    }
                    "snes4" | "pce4" => {
                        for y in 0..8 {
                            out.push(plane(y, 0));
                            out.push(plane(y, 1));
                        }
                        for y in 0..8 {
                            out.push(plane(y, 2));
                            out.push(plane(y, 3));
                        }
                    }
                    _ => {
                        for y in 0..8 {
                            for x in (0..8).step_by(2) {
                                out.push(px(x, y) << 4 | px(x + 1, y));
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// The palette as `snes` or `md` words.
    pub fn palette_words(&self, name: &str) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        for c in &self.palette {
            match name {
                "snes" => {
                    let w = (c[0] as u16 >> 3) | (c[1] as u16 >> 3) << 5 | (c[2] as u16 >> 3) << 10;
                    out.extend_from_slice(&w.to_le_bytes());
                }
                "md" => {
                    let w =
                        (c[0] as u16 >> 5) << 1 | (c[1] as u16 >> 5) << 5 | (c[2] as u16 >> 5) << 9;
                    out.extend_from_slice(&w.to_be_bytes());
                }
                _ => {
                    return Err(Error::Format(format!(
                        "unknown palette format \"{name}\" (snes, md)"
                    )));
                }
            }
        }
        Ok(out)
    }

    /// A 256 x 192 picture as the Spectrum's screen: two colours per 8x8 cell (ink and paper from the 8 base colours
    /// by palette index 0-7, indices 8-15 bright), 6144 bitmap bytes in the machine's order then 768 attributes.
    pub fn spectrum_screen(&self) -> Result<Vec<u8>, Error> {
        if self.width != 256 || self.height != 192 {
            return Err(Error::Size(format!(
                "{} x {}: a Spectrum screen is 256 x 192",
                self.width, self.height
            )));
        }
        let mut bitmap = vec![0u8; 6144];
        let mut attrs = vec![0u8; 768];
        for cy in 0..24 {
            for cx in 0..32 {
                let mut colours: Vec<u8> = Vec::new();
                for y in 0..8 {
                    for x in 0..8 {
                        let c = self.pixel(cx * 8 + x, cy * 8 + y);
                        if !colours.contains(&c) {
                            colours.push(c);
                        }
                    }
                }
                if colours.len() > 2 {
                    return Err(Error::TooManyColours {
                        found: colours.len(),
                        allowed: 2,
                    });
                }
                if colours.iter().any(|&c| c > 15) {
                    return Err(Error::Format(
                        "Spectrum colours are palette indices 0-15".to_string(),
                    ));
                }
                // paper is the colour most of the cell has (the first seen on a tie), ink the other
                let count = |c: u8| {
                    (0..8)
                        .flat_map(|y| (0..8).map(move |x| (x, y)))
                        .filter(|&(x, y)| self.pixel(cx * 8 + x, cy * 8 + y) == c)
                        .count()
                };
                if colours.len() == 2 && count(colours[1]) > count(colours[0]) {
                    colours.swap(0, 1);
                }
                let paper = colours[0];
                let ink = *colours.get(1).unwrap_or(&paper);
                let bright = (paper > 7 || ink > 7) as u8;
                attrs[cy * 32 + cx] = bright << 6 | (paper & 7) << 3 | (ink & 7);
                for y in 0..8 {
                    let mut byte = 0u8;
                    for x in 0..8 {
                        if self.pixel(cx * 8 + x, cy * 8 + y) == ink && ink != paper {
                            byte |= 0x80 >> x;
                        }
                    }
                    let row = cy * 8 + y;
                    let addr = (row & 0xC0) << 5 | (row & 7) << 8 | (row & 0x38) << 2 | cx;
                    bitmap[addr] = byte;
                }
            }
        }
        bitmap.extend(attrs);
        Ok(bitmap)
    }
}
