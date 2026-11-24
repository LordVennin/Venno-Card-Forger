// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/utils/hexapi.py (get_sets, process_data_sets,
// update_hexproof_cache) and src/layouts.py (NormalLayout.symbol_code,
// symbol_svg, card_count).

//! Set symbols and set data from hexproof.io, the service Proxyshop uses.
//!
//! Two things come from it. The **symbols package** is a zip of SVG drawings:
//! `set/<CODE>/<R>.svg`, one per set and rarity, already in the rarity's
//! colors, plus watermarks. The **set data** says which symbol a set uses
//! (many sets share one) and how many cards it has.
//!
//! Both are kept in the cache folder and refreshed weekly when the network is
//! there; without it, whatever is on disk is used. Nothing from the package is
//! ever part of the repository or of a saved document's template.

use crate::{Client, Error, FetchError, Transport, FRESH};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const API: &str = "https://api.hexproof.io";

/// What hexproof.io knows about a set that a card needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetInfo {
    /// The folder of the symbols package this set's symbol is in, upper case. "DEFAULT" when it has none of its own.
    pub symbol: String,
    /// The number printed after the slash on a collector line, when known.
    pub size: Option<u32>,
}

fn fresh(path: &Path) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|at| SystemTime::now().duration_since(at).ok()).is_some_and(|age| age < FRESH)
}

impl<T: Transport> Client<T> {
    fn symbols_dir(&self) -> PathBuf {
        self.cache.join("symbols")
    }

    /// Symbol folder and size of a set, by its code. Unknown sets use the default symbol.
    pub fn set_info(&mut self, set: &str) -> SetInfo {
        let unknown = SetInfo { symbol: "DEFAULT".into(), size: None };
        let Ok(sets) = self.get_json(&format!("{API}/sets/")) else { return unknown };
        let data = &sets[set.to_lowercase().as_str()];
        if !data.is_object() {
            return unknown;
        }
        let count = |key: &str| data[key].as_u64().map(|n| n as u32);
        SetInfo { symbol: data["code_symbol"].as_str().unwrap_or("DEFAULT").to_uppercase(), size: count("count_printed").or(count("count_cards")).filter(|n| *n > 0) }
    }

    /// Make sure the symbols package is unpacked in the cache, downloading it
    /// if it is missing or more than a week old. Returns the folder it is in.
    pub fn symbols(&mut self) -> Result<PathBuf, Error> {
        let dir = self.symbols_dir();
        let stamp = dir.join("manifest.json");
        if fresh(&stamp) {
            return Ok(dir);
        }
        let have = stamp.exists();
        std::fs::create_dir_all(&dir).map_err(|e| Error::Other(format!("{}: {e}", dir.display())))?;
        let archive = dir.join("package.zip");
        match self.transport.download(&format!("{API}/symbols/package/"), &archive) {
            Ok(()) => {
                unpack(&archive, &dir)?;
                let _ = std::fs::remove_file(&archive);
                // The manifest's own date is old; note when we fetched it.
                let _ = std::fs::File::open(&stamp).and_then(|f| f.set_modified(SystemTime::now()));
                Ok(dir)
            }
            Err(_) if have => Ok(dir),
            Err(FetchError::Unreachable(why)) => Err(Error::Offline(why)),
            Err(FetchError::NotFound(_)) => Err(Error::Other("hexproof.io has no symbols package at the usual address".into())),
            Err(FetchError::Other(why)) => Err(Error::Other(why)),
        }
    }

    /// The SVG of a set's symbol in a rarity ("C", "U", "R", "M", "T"...), as
    /// text. Looks, as Proxyshop does, in the set's own folder, then the folder
    /// its set data names, then that folder's mythic symbol for odd rarities,
    /// then the default symbol.
    pub fn symbol_svg(&mut self, set: &str, rarity_letter: &str) -> Option<String> {
        let dir = self.symbols().ok()?.join("set");
        let info = self.set_info(set);
        let file = |code: &str, rarity: &str| std::fs::read_to_string(dir.join(code).join(format!("{rarity}.svg"))).ok();
        let odd = !matches!(rarity_letter, "C" | "U" | "R" | "M");
        let own = if info.symbol == "DEFAULT" { file(&set.to_uppercase(), rarity_letter) } else { None };
        own.or_else(|| file(&info.symbol, rarity_letter)).or_else(|| if odd { file(&info.symbol, "M") } else { None }).or_else(|| file("DEFAULT", rarity_letter))
    }
}

/// Unpack a zip into a folder. Entries that would land outside the folder are skipped.
fn unpack(archive: &Path, into: &Path) -> Result<(), Error> {
    let bad = |e: String| Error::Other(format!("The symbols package could not be unpacked: {e}"));
    let file = std::fs::File::open(archive).map_err(|e| bad(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| bad(e.to_string()))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| bad(e.to_string()))?;
        let Some(inside) = entry.enclosed_name() else { continue };
        let target = into.join(inside);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| bad(e.to_string()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| bad(e.to_string()))?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| bad(e.to_string()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| bad(e.to_string()))?;
    }
    Ok(())
}

