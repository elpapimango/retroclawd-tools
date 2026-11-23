//! Hard disk geometry: the cylinders, heads and sectors a CHD records for a disk image (`GDDD` metadata), guessed from the size as
//! chdman guesses them, or given by the user or by a template of a known drive.

use crate::{Result, fail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chs {
    pub cylinders: u32,
    pub heads: u32,
    pub sectors: u32,
}

impl Chs {
    pub fn total_sectors(self) -> u64 {
        u64::from(self.cylinders) * u64::from(self.heads) * u64::from(self.sectors)
    }

    /// The `GDDD` metadata text.
    pub fn text(self, bytes_per_sector: u32) -> String {
        format!(
            "CYLS:{},HEADS:{},SECS:{},BPS:{}",
            self.cylinders, self.heads, self.sectors, bytes_per_sector
        )
    }

    /// Reads a `GDDD` entry (or the older `CHHD`-style text with the same keywords): the geometry and the sector size.
    pub fn parse(data: &[u8]) -> Result<(Chs, u32)> {
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        let text = String::from_utf8_lossy(&data[..end]).into_owned();
        let field = |name: &str| -> Result<u32> {
            text.split(',')
                .find_map(|p| p.trim().strip_prefix(name)?.strip_prefix(':'))
                .and_then(|v| v.trim().parse().ok())
                .ok_or_else(|| {
                    crate::Error(format!("the hard disk description lacks {name}: {text}"))
                })
        };
        Ok((
            Chs {
                cylinders: field("CYLS")?,
                heads: field("HEADS")?,
                sectors: field("SECS")?,
            },
            field("BPS")?,
        ))
    }

    /// `cylinders,heads,sectors` as given on a command line.
    pub fn from_arg(text: &str) -> Result<Chs> {
        let parts: Vec<&str> = text.split(',').collect();
        let num = |s: &str| {
            s.trim()
                .parse::<u32>()
                .ok()
                .filter(|&v| v > 0)
                .ok_or_else(|| crate::Error(format!("{text:?} is not cylinders,heads,sectors")))
        };
        match parts.as_slice() {
            [c, h, s] => Ok(Chs {
                cylinders: num(c)?,
                heads: num(h)?,
                sectors: num(s)?,
            }),
            _ => fail(format!("{text:?} is not cylinders,heads,sectors")),
        }
    }
}

/// A geometry for `total` sectors: the most sectors a track can hold (up to 63) and then the most heads (up to 16) that divide the
/// disk evenly. A size that no geometry divides evenly is rounded up to the next that one does (the image keeps its own size).
pub fn guess(total: u64) -> Chs {
    let mut n = total.max(1);
    loop {
        for sectors in (2..=63u64).rev() {
            if !n.is_multiple_of(sectors) {
                continue;
            }
            for heads in (2..=16u64).rev() {
                if (n / sectors).is_multiple_of(heads) {
                    return Chs {
                        cylinders: (n / sectors / heads) as u32,
                        heads: heads as u32,
                        sectors: sectors as u32,
                    };
                }
            }
        }
        n += 1;
    }
}

/// A drive whose geometry can be asked for by number (`createhd --template`).
pub struct Template {
    pub manufacturer: &'static str,
    pub model: &'static str,
    pub chs: Chs,
    pub sector_size: u32,
}

const fn t(manufacturer: &'static str, model: &'static str, c: u32, h: u32, s: u32) -> Template {
    Template {
        manufacturer,
        model,
        chs: Chs {
            cylinders: c,
            heads: h,
            sectors: s,
        },
        sector_size: 512,
    }
}

pub const TEMPLATES: [Template; 17] = [
    t("Conner", "CFA170A", 332, 16, 63),
    t("Rodime", "R0201", 321, 2, 16),
    t("Rodime", "R0202", 321, 4, 16),
    t("Rodime", "R0203", 321, 6, 16),
    t("Rodime", "R0204", 321, 8, 16),
    t("Seagate", "ST-213", 615, 2, 17),
    t("Seagate", "ST-225", 615, 4, 17),
    t("Seagate", "ST-251", 820, 6, 17),
    t("Seagate", "ST-3600N", 1877, 7, 76),
    t("Maxtor", "LXT-213S", 1314, 7, 53),
    t("Maxtor", "LXT-340S", 1574, 7, 70),
    t("Maxtor", "MXT-540SL", 2466, 7, 87),
    t("Micropolis", "1528", 2094, 15, 83),
    t("Quantum", "Fireball CR 4.3 AT", 14848, 9, 63),
    t("Quantum", "Fireball CR 6.4 AT", 13328, 15, 63),
    t("Quantum", "Fireball CR 8.4 AT", 16383, 16, 63),
    t("Quantum", "Fireball CR 13.0 AT", 25228, 16, 63),
];

/// The table `listtemplates` prints.
pub fn template_table() -> String {
    let mut s = String::from(
        "ID  Manufacturer  Model               Cylinders  Heads  Sectors  Sector Size  Total Size\n",
    );
    s.push_str(&"-".repeat(88));
    s.push('\n');
    for (i, t) in TEMPLATES.iter().enumerate() {
        let mb = t.chs.total_sectors() * u64::from(t.sector_size) / (1024 * 1024);
        s.push_str(&format!(
            "{:>2}  {:<12}  {:<18}  {:>9}  {:>5}  {:>7}  {:>11}  {:>7} MB\n",
            i,
            t.manufacturer,
            t.model,
            t.chs.cylinders,
            t.chs.heads,
            t.chs.sectors,
            t.sector_size,
            mb
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guess_is_chdmans_for_sizes_it_was_asked_about() {
        // the (cylinders, heads, sectors) chdman 0.289 chose for images of this many 512-byte sectors
        for (n, c, h, s) in [
            (16384, 32, 16, 32),
            (6144, 8, 16, 48),
            (100_000, 125, 16, 50),
            (17, 1, 2, 9),
            (1009, 101, 2, 5),
            (4000, 5, 16, 50),
            (63, 1, 3, 21),
            (126, 1, 2, 63),
            (20480, 32, 16, 40),
            (10080, 10, 16, 63),
            (12345, 823, 3, 5),
            (262_144, 512, 16, 32),
            (1_000_000, 1250, 16, 50),
            (3969, 7, 9, 63),
        ] {
            assert_eq!(
                guess(n),
                Chs {
                    cylinders: c,
                    heads: h,
                    sectors: s
                },
                "{n} sectors"
            );
        }
    }

    #[test]
    fn descriptions_and_arguments_round_trip() {
        let chs = Chs {
            cylinders: 100,
            heads: 4,
            sectors: 17,
        };
        assert_eq!(chs.text(512), "CYLS:100,HEADS:4,SECS:17,BPS:512");
        assert_eq!(
            Chs::parse(b"CYLS:100,HEADS:4,SECS:17,BPS:512\0").unwrap(),
            (chs, 512)
        );
        assert_eq!(Chs::from_arg("100,4,17").unwrap(), chs);
        assert!(Chs::from_arg("100,4").is_err() && Chs::from_arg("0,1,1").is_err());
        assert!(Chs::parse(b"CYLS:1").is_err());
        assert!(template_table().contains("Fireball CR 13.0 AT"));
    }
}
