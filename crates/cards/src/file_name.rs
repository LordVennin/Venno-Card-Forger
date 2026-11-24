// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/cards.py (parse_card_info) and src/enums/mtg.py
// (CardTextPatterns.PATH_*).

//! The tags in an art file's name: `Name (Artist) [SET] {num} $creator`.
//!
//! Kept for compatibility with art folders made for Proxyshop. Differences:
//! the number has its leading zeros dropped here (Proxyshop does it later,
//! when it queries Scryfall), and a file extension is only removed when it is
//! one, so a name with a full stop in it survives.

/// What an art file's name says about the card it is for.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CardInfo {
    pub name: String,
    /// Set code, as written.
    pub set: String,
    /// Collector number. Only used together with a set.
    pub number: String,
    pub artist: String,
    /// Who made the proxy, after a `$`.
    pub creator: String,
}

/// The text between the first `open` and the last `close` after it.
fn between(text: &str, open: char, close: char) -> Option<&str> {
    let start = text.find(open)? + open.len_utf8();
    let end = text[start..].rfind(close)? + start;
    Some(&text[start..end])
}

/// Read the tags from an art file name, with or without its extension.
pub fn parse_card_info(file_name: &str) -> CardInfo {
    // Drop a real extension: a short run of letters and digits after the last full stop.
    let stem = match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) => stem,
        _ => file_name,
    };
    let tags = ['[', '(', '{', '$'];
    let name = stem.split(tags).next().unwrap_or("").trim().to_string();
    let set = between(stem, '[', ']').unwrap_or("").to_string();
    // The artist is the first parenthesized part: "(A) (B)" gives A.
    let artist = stem.find('(').and_then(|start| stem[start..].find(')').map(|end| stem[start..start + end].trim_start_matches('(').to_string())).unwrap_or_default();
    let number = match between(stem, '{', '}') {
        Some(number) if !set.is_empty() => {
            let trimmed = number.trim().trim_start_matches('0');
            if trimmed.is_empty() && !number.trim().is_empty() { "0".to_string() } else { trimmed.to_string() }
        }
        _ => String::new(),
    };
    let creator = if stem.contains('$') { stem.rsplit(tags).next().unwrap_or("").to_string() } else { String::new() };
    CardInfo { name, set, number, artist, creator }
}
