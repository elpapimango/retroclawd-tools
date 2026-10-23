//! Raw ATR operations for the existing disk tool.
//! No Atari DOS directory or free-space interpretation is claimed.

use atari_media::Atr;

/// Facts suitable for `rcdisk info`; sector count is not filesystem free space.
pub fn info(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    let atr = Atr::parse(bytes)?;
    Ok(vec![
        ("format".into(), "atr".into()),
        ("sectors".into(), atr.sector_count().to_string()),
        ("sector_size".into(), atr.sector_size().to_string()),
        ("boot_sector_size".into(), "128".into()),
        ("write_protected".into(), atr.write_protected().to_string()),
    ])
}

/// Extracts one raw, one-based sector, without stripping any bytes.
pub fn extract_sector(bytes: &[u8], number: u16) -> Result<Vec<u8>, String> {
    Ok(Atr::parse(bytes)?.sector(number)?.to_vec())
}

/// Extracts consecutive raw sectors. Boot-sector lengths are kept exactly.
pub fn extract_sectors(bytes: &[u8], number: u16, count: u16) -> Result<Vec<u8>, String> {
    let atr = Atr::parse(bytes)?;
    let end = sector_end(&atr, number, count)?;
    let mut output = Vec::new();
    for sector in number..=end {
        output.extend_from_slice(atr.sector(sector)?);
    }
    Ok(output)
}

/// Replaces whole consecutive sectors; errors never produce a partially changed image.
pub fn insert_sectors(bytes: &[u8], number: u16, data: &[u8]) -> Result<Vec<u8>, String> {
    let mut atr = Atr::parse(bytes)?;
    if data.is_empty() {
        return Err("raw sector input is empty".into());
    }
    let mut left = data;
    let mut sector = number;
    loop {
        let target = atr.sector_mut(sector)?;
        let (part, rest) = left
            .split_at_checked(target.len())
            .ok_or("raw input ends with a partial ATR sector")?;
        target.copy_from_slice(part);
        left = rest;
        if left.is_empty() {
            break;
        }
        sector = sector.checked_add(1).ok_or("ATR sector range overflow")?;
    }
    Ok(atr.to_bytes())
}

fn sector_end(atr: &Atr, first: u16, count: u16) -> Result<u16, String> {
    if count == 0 {
        return Err("sector count must be positive".into());
    }
    let end = first
        .checked_add(count - 1)
        .ok_or("ATR sector range overflow")?;
    atr.sector(first)?;
    atr.sector(end)?;
    Ok(end)
}

/// Creates an unformatted ATR, not a disk containing an invented filesystem.
pub fn create(sectors: u16, sector_size: u16) -> Result<Vec<u8>, String> {
    Ok(Atr::blank(sectors, sector_size)?.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_atr_operations_respect_boot_sizes_and_bounds() {
        let mut atr = Atr::blank(720, 256).unwrap();
        atr.sector_mut(4).unwrap().fill(0xa5);
        let bytes = atr.to_bytes();
        assert_eq!(extract_sector(&bytes, 3).unwrap(), vec![0; 128]);
        assert_eq!(extract_sector(&bytes, 4).unwrap(), vec![0xa5; 256]);
        assert!(extract_sector(&bytes, 0).is_err());
        assert!(extract_sector(&bytes, 721).is_err());
        assert!(
            info(&bytes)
                .unwrap()
                .contains(&("sectors".into(), "720".into()))
        );
        assert_eq!(create(720, 256).unwrap().len(), 183952);
        assert!(create(720, 512).is_err());
        assert!(info(&bytes[..bytes.len() - 1]).is_err());
    }
}
