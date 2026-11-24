//! Linking 68000 objects: the sections `TEXT`, `DATA` and `BSS` of the objects `rcasm --cpu 68000 -f object` makes, placed
//! by the linker either at fixed addresses (the cartridge machines) or one after another from 0 as a program a loader
//! relocates (the Atari ST, the Amiga and the X68000).

use crate::layout::Layout;
use crate::link::{LinkOptions, Linked, Rule};
use crate::{Item, link_inputs};
use rcasm::m68k::{M68kAssembly, M68kSymbol};
use std::path::Path;

/// The address space of the 68000's bus.
const SPACE: u64 = 0x100_0000;

/// A flat program: `TEXT` at `text_at`, `DATA` after it in the ROM, `BSS` at `bss_at` (the work RAM), unless the layout's
/// `[[segment]]` rules say otherwise.
pub(crate) fn link_flat(
    layout: &Layout,
    base: &Path,
    items: &[Item],
    defines: &[(String, i64)],
    text_at: u32,
    bss_at: u32,
) -> Result<Linked, String> {
    let defaults = [
        Rule {
            name: "TEXT".to_string(),
            start: Some(text_at),
            ..Rule::default()
        },
        Rule {
            name: "BSS".to_string(),
            start: Some(bss_at),
            ..Rule::default()
        },
    ];
    link_inputs(
        layout,
        base,
        items,
        defines,
        LinkOptions {
            limit: SPACE,
            track_addresses: false,
        },
        &defaults,
    )
}

/// A program for a loader that relocates it: `TEXT`, `DATA`, `BSS` from address 0 in that order, the four-byte fields that
/// hold an address listed for the loader. An address anywhere else (a word, a byte) is an error.
pub(crate) fn link_relocatable(
    layout: &Layout,
    base: &Path,
    items: &[Item],
    defines: &[(String, i64)],
    who: &str,
) -> Result<M68kAssembly, String> {
    let defaults = [Rule {
        name: "TEXT".to_string(),
        start: Some(0),
        ..Rule::default()
    }];
    let linked = link_inputs(
        layout,
        base,
        items,
        defines,
        LinkOptions {
            limit: SPACE,
            track_addresses: true,
        },
        &defaults,
    )?;
    let mut text_end = 0u32;
    let mut data_end = 0u32;
    let mut bss = 0u32;
    for s in &linked.sections {
        match s.name.as_str() {
            "TEXT" => text_end = text_end.max(s.addr + s.size),
            "DATA" => data_end = data_end.max(s.addr + s.size),
            "BSS" => bss += s.size,
            other => {
                return Err(format!(
                    "section \"{other}\": the {who} loader takes the sections TEXT, DATA and BSS"
                ));
            }
        }
    }
    data_end = data_end.max(text_end);
    let mut image = vec![0u8; data_end as usize];
    for w in &linked.wide {
        let at = w.start as usize;
        image
            .get_mut(at..at + w.bytes.len())
            .ok_or_else(|| format!("{}: lies outside the program", w.name))?
            .copy_from_slice(&w.bytes);
    }
    if text_end == 0 {
        return Err("the program has no text".to_string());
    }
    let data = image.split_off(text_end as usize);
    Ok(M68kAssembly {
        text: image,
        data,
        bss: bss + (bss & 1),
        origin: 0,
        relocs: linked.address_fields.clone(),
        symbols: linked
            .symbols
            .iter()
            .map(|(name, value)| M68kSymbol {
                name: name.clone(),
                value: *value,
                label: true,
            })
            .collect(),
        listing: Vec::new(),
        object: None,
    })
}
