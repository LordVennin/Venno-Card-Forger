// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/cards.py (generate_italics, strip_reminder_text,
// locate_symbols, locate_italics), src/utils/mtg.py (get_symbol_colors),
// src/enums/mtg.py (mana_symbol_map, non_italics_abilities),
// src/schema/colors.py (SymbolColorMap) and src/text_layers.py
// (FormattedTextField.text_details and format_text).

//! Rules text: which parts are italic, how symbols are drawn, and how the
//! paragraphs of a text box are spaced.
//!
//! Proxyshop makes these decisions and then drives Photoshop with character
//! indexes. Here the same decisions produce a [`RichText`] the core lays out.
//!
//! Where this differs from Proxyshop, on purpose:
//! - Italics are found by position. Proxyshop collects italic strings and
//!   italicizes every place each one occurs, so an ability word repeated later
//!   in a sentence would turn italic too.
//! - Stripping reminder text also removes the space before it, and lines left
//!   empty. Proxyshop deletes any run of two or more spaces outright, which
//!   joins words when a reminder sits mid-sentence.
//! - Lines may be separated by "\n" or "\r" (Proxyshop converts to "\r" first).

use cardsmith_core::doc::{Color, ParagraphStyle, RichText, TextAlign, TextRun};
use std::ops::Range;

/// Words that fit the "Word — text" pattern but are not italic ability words.
const NON_ITALIC_ABILITIES: [&str; 7] = [
    "Boast",    // Kaldheim
    "Forecast", // Dissension
    "Gotcha",   // Unhinged
    "Visit",    // Unfinity
    "Whack", "Doodle", "Buzz", // Unstable
];

fn is_break(c: char) -> bool {
    c == '\r' || c == '\n'
}

/// The parts of a card's rules text that are italic: reminder text (anything
/// in parentheses) and ability words. Byte ranges into `text`, reminder text first.
pub fn italic_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();

    // Each reminder text block.
    let mut end = 0;
    while let Some(start) = text[end..].find('(').map(|i| i + end) {
        let Some(close) = text[start + 1..].find(')') else { break };
        end = start + 1 + close + 1;
        ranges.push(start..end);
    }

    // Ability words: "Word — text" at the start of a line or bullet.
    if !text.contains(" — ") {
        return ranges;
    }
    let lower = text.to_lowercase();
    let mut offset = 0;
    for line in text.split(is_break) {
        let line_start = offset;
        offset += line.len() + 1;
        let mut rest = line;
        while let Some(stripped) = rest.strip_prefix("• ") {
            rest = stripped;
        }
        // Everything up to the last dash on the line.
        let Some(dash) = rest.rfind(" — ").filter(|&i| i > 0) else { continue };
        let word = &rest[..dash];
        // "Villainous choice" is rules text.
        if word.contains("villainous") {
            continue;
        }
        // Mirrodin Besieged: named bullets that are not modes of a "choose one".
        if text.contains(&format!("• {word}")) && !lower.contains("choose one") {
            continue;
        }
        if NON_ITALIC_ABILITIES.contains(&word) {
            continue;
        }
        // Celebr-8000: a one or two digit die result.
        if word.chars().count() < 3 && word.chars().all(char::is_numeric) {
            continue;
        }
        let start = line_start + (line.len() - rest.len());
        ranges.push(start..start + dash);
    }
    ranges
}

/// The italic strings of a card's rules text. See [`italic_ranges`].
pub fn generate_italics(text: &str) -> Vec<String> {
    italic_ranges(text).into_iter().map(|r| text[r].to_string()).collect()
}

/// Rules text without its reminder text (the parts in parentheses). Text that
/// is nothing but reminder text is returned unchanged.
pub fn strip_reminder_text(text: &str) -> String {
    if !text.contains('(') {
        return text.to_string();
    }
    // Remove innermost parenthesized spans, and the spaces just before each.
    let mut stripped = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let after = &rest[open + 1..];
        match after.find([')', '(']) {
            Some(i) if after.as_bytes()[i] == b')' => {
                stripped.push_str(rest[..open].trim_end_matches(' '));
                rest = &after[i + 1..];
            }
            // An opening parenthesis with no partner of its own stays.
            Some(i) => {
                stripped.push_str(&rest[..open + 1 + i]);
                rest = &after[i..];
            }
            None => break,
        }
    }
    stripped.push_str(rest);

    let lines: Vec<&str> = stripped.split(is_break).map(str::trim).filter(|line| !line.is_empty()).collect();
    if lines.is_empty() {
        return text.to_string();
    }
    lines.join("\n")
}

// ---- symbols --------------------------------------------------------------------

/// The characters of the Proxyglyph font that draw a symbol, back to front:
/// for "{W}" that is "o" (the disc) then "w" (the sun), each drawn over the last.
pub fn symbol_chars(symbol: &str) -> Option<&'static str> {
    Some(match symbol {
        "{W/P}" | "{U/P}" | "{B/P}" | "{R/P}" | "{G/P}" => "Qp",
        "{W/U/P}" | "{U/B/P}" | "{B/R/P}" | "{R/G/P}" | "{G/W/P}" | "{W/B/P}" | "{B/G/P}" | "{G/U/P}" | "{U/R/P}" | "{R/W/P}" => "Qqyz",
        "{A}" => "oi",
        "{E}" => "e",
        "{T}" => "ot",
        "{X}" => "ox",
        "{Y}" => "oY",
        "{Z}" => "oZ",
        "{∞}" => "o∞",
        "{0}" => "o0",
        "{1}" => "o1",
        "{2}" => "o2",
        "{3}" => "o3",
        "{4}" => "o4",
        "{5}" => "o5",
        "{6}" => "o6",
        "{7}" => "o7",
        "{8}" => "o8",
        "{9}" => "o9",
        "{10}" => "oA",
        "{11}" => "oB",
        "{12}" => "oC",
        "{13}" => "oD",
        "{14}" => "oE",
        "{15}" => "oF",
        "{16}" => "oG",
        "{17}" => "oÅ",
        "{18}" => "oÆ",
        "{19}" => "oÃ",
        "{20}" => "oK",
        "{W}" => "ow",
        "{U}" => "ou",
        "{B}" => "ob",
        "{R}" => "or",
        "{G}" => "og",
        "{C}" => "oc",
        "{W/U}" => "QqLS",
        "{U/B}" => "QqMT",
        "{B/R}" => "QqNU",
        "{R/G}" => "QqOV",
        "{G/W}" => "QqPR",
        "{W/B}" => "QqLT",
        "{B/G}" => "QqNV",
        "{G/U}" => "QqPS",
        "{U/R}" => "QqMU",
        "{R/W}" => "QqOR",
        "{2/W}" => "QqWR",
        "{2/U}" => "QqWS",
        "{2/B}" => "QqWT",
        "{2/R}" => "QqWU",
        "{2/G}" => "QqWV",
        "{S}" => "omn",
        "{Q}" => "ol",
        "{CHAOS}" => "?",
        _ => return None,
    })
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    [r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 1.0]
}

/// One color per color of mana, plus the one generic ("2") mana uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ManaColors {
    pub w: Color,
    pub u: Color,
    pub b: Color,
    pub r: Color,
    pub g: Color,
    pub generic: Color,
}

impl ManaColors {
    const fn all(color: Color) -> ManaColors {
        ManaColors { w: color, u: color, b: color, r: color, g: color, generic: color }
    }

    fn get(&self, key: &str) -> Option<Color> {
        Some(match key {
            "W" => self.w,
            "U" => self.u,
            "B" => self.b,
            "R" => self.r,
            "G" => self.g,
            "2" => self.generic,
            _ => return None,
        })
    }
}

/// The colors symbols are drawn in. The defaults are Proxyshop's.
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolColors {
    pub primary: Color,
    pub secondary: Color,
    pub colorless: Color,
    /// The disc of a symbol of each color.
    pub colors: ManaColors,
    /// Discs of hybrid-style symbols: black's is darker.
    pub hybrid: ManaColors,
    /// The glyph on top of the disc.
    pub colors_inner: ManaColors,
    pub hybrid_inner: ManaColors,
}

impl Default for SymbolColors {
    fn default() -> SymbolColors {
        let colors = ManaColors { w: rgb(255, 251, 214), u: rgb(170, 224, 250), b: rgb(204, 194, 193), r: rgb(249, 169, 143), g: rgb(154, 211, 175), generic: rgb(204, 194, 193) };
        SymbolColors {
            primary: rgb(0, 0, 0),
            secondary: rgb(255, 255, 255),
            colorless: rgb(204, 194, 193),
            colors,
            hybrid: ManaColors { b: rgb(159, 146, 143), ..colors },
            colors_inner: ManaColors::all(rgb(0, 0, 0)),
            hybrid_inner: ManaColors::all(rgb(0, 0, 0)),
        }
    }
}

/// The color of each character of a symbol, in the order of [`symbol_chars`].
/// None when there is no rule for the symbol.
pub fn symbol_colors(symbol: &str, chars: &str, map: &SymbolColors) -> Option<Vec<Color>> {
    match symbol {
        "{E}" | "{CHAOS}" => return Some(vec![map.primary]),
        "{S}" => return Some(vec![map.colorless, map.primary, map.secondary]),
        "{Q}" => return Some(vec![map.primary, map.secondary]),
        _ => {}
    }
    let inner = symbol.strip_prefix('{').and_then(|s| s.strip_suffix('}')).unwrap_or(symbol);
    let parts: Vec<&str> = inner.split('/').collect();
    let is_color = |p: &str| matches!(p, "W" | "U" | "B" | "R" | "G");
    match parts.as_slice() {
        // Normal mana.
        [c] if is_color(c) => return Some(vec![map.colors.get(c)?, map.colors_inner.get(c)?]),
        // Hybrid. Generic/color hybrids use the hybrid discs, where black is darker.
        [a, b] if (is_color(a) || *a == "2") && is_color(b) => {
            let discs = if *a == "2" { &map.hybrid } else { &map.colors };
            return Some(vec![discs.get(b)?, discs.get(a)?, map.colors_inner.get(a)?, map.colors_inner.get(b)?]);
        }
        // Phyrexian.
        [c, "P"] if is_color(c) => return Some(vec![map.hybrid.get(c)?, map.hybrid_inner.get(c)?]),
        // Phyrexian hybrid.
        [a, b, "P"] if is_color(a) && is_color(b) => {
            return Some(vec![map.colors.get(b)?, map.colors.get(a)?, map.colors_inner.get(a)?, map.colors_inner.get(b)?]);
        }
        _ => {}
    }
    // Anything else drawn as a disc and a glyph: numbers, X, tap, colorless.
    (chars.chars().count() == 2).then(|| vec![map.colorless, map.primary])
}

// ---- formatting -----------------------------------------------------------------

/// How a rules text box is formatted. Lengths are in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct RulesFormat {
    /// PostScript names of the fonts for italic text, bold rules text and symbols.
    pub font_italic: String,
    pub font_bold: String,
    pub font_symbol: String,
    pub colors: SymbolColors,
    /// A color for flavor text, if not the layer's.
    pub flavor_color: Option<Color>,
    /// Center every line (and put no extra space between paragraphs).
    pub centered: bool,
    /// Set the rules text (not reminder text or flavor) in the bold font.
    pub bold_rules: bool,
    /// Right-align the attribution line of a quote in the flavor text.
    pub right_align_quote: bool,
    /// The flavor divider will be shown, so flavor text gets more room above it.
    pub divider: bool,
    /// Space above each paragraph of rules text.
    pub line_break_lead: f64,
    /// Space above each bullet of a modal list.
    pub bullet_lead: f64,
    /// Space above the flavor text, without and with the divider.
    pub flavor_lead: f64,
    pub flavor_lead_divider: f64,
    /// How far the lines of a bullet are indented; the bullet itself hangs back by the same amount.
    pub modal_indent: f64,
}

impl RulesFormat {
    /// Proxyshop's defaults, which are in points, for a document of the given resolution.
    pub fn for_dpi(dpi: f64) -> RulesFormat {
        let pt = dpi / 72.0;
        RulesFormat {
            font_italic: "PlantinMTPro-Italic".into(),
            font_bold: "PlantinMTPro-Bold".into(),
            font_symbol: "Proxyglyph".into(),
            colors: SymbolColors::default(),
            flavor_color: None,
            centered: false,
            bold_rules: false,
            right_align_quote: false,
            divider: false,
            line_break_lead: 2.4 * pt,
            bullet_lead: 1.0 * pt,
            flavor_lead: 4.4 * pt,
            flavor_lead_divider: 7.0 * pt,
            modal_indent: 5.7 * pt,
        }
    }
}

/// Rules and flavor text ready to put on a text layer.
#[derive(Clone, Debug, PartialEq)]
pub struct FormattedText {
    pub content: RichText,
    /// Symbols in the text that have no glyph. They are written out without their braces.
    pub unknown_symbols: Vec<String>,
    /// Which paragraph the flavor text starts at, when there is flavor text
    /// below rules text. The divider goes between it and the one before.
    pub flavor_paragraph: Option<usize>,
}

#[derive(Clone, PartialEq, Default)]
struct Attr {
    italic: bool,
    bold: bool,
    color: Option<Color>,
    /// The symbol this character is part of, and which of its characters this is.
    symbol: Option<(String, usize)>,
}

/// Turn a card's rules text and flavor text into rich text: symbols become
/// colored characters of the symbol font, reminder text and ability words
/// become italic, and each paragraph gets its spacing and indent.
pub fn format_rules_text(rules: &str, flavor: &str, format: &RulesFormat) -> FormattedText {
    let rules = rules.replace("\r\n", "\n").replace('\r', "\n");
    let mut flavor = flavor.replace("\r\n", "\n").replace('\r', "\n");
    let mut cells: Vec<(char, Attr)> = Vec::new();
    let mut unknown_symbols = Vec::new();

    // Rules text: symbols and italics.
    let italics = italic_ranges(&rules);
    let is_italic = |at: usize| italics.iter().any(|r| r.contains(&at));
    let plain = |at: usize| Attr { italic: is_italic(at), bold: format.bold_rules && !is_italic(at), ..Attr::default() };
    let mut at = 0;
    while at < rules.len() {
        let rest = &rules[at..];
        if let Some(close) = rest.starts_with('{').then(|| rest.find('}')).flatten() {
            let symbol = &rest[..=close];
            let glyphs = symbol_chars(symbol).and_then(|chars| Some((chars, symbol_colors(symbol, chars, &format.colors)?)));
            match glyphs {
                Some((chars, colors)) => {
                    for (i, (ch, color)) in chars.chars().zip(colors).enumerate() {
                        cells.push((ch, Attr { color: Some(color), symbol: Some((symbol.to_string(), i)), ..Attr::default() }));
                    }
                }
                None => {
                    if !unknown_symbols.iter().any(|s| s == symbol) {
                        unknown_symbols.push(symbol.to_string());
                    }
                    cells.extend(symbol[1..close].chars().map(|ch| (ch, plain(at))));
                }
            }
            at += close + 1;
        } else {
            let ch = rest.chars().next().expect("at is inside the text");
            cells.push((ch, plain(at)));
            at += ch.len_utf8();
        }
    }

    // Flavor text: italic, except between pairs of asterisks.
    let rules_paragraphs = if rules.is_empty() { 0 } else { rules.split('\n').count() };
    if !flavor.is_empty() {
        if !rules.is_empty() {
            cells.push(('\n', Attr::default()));
        }
        let toggles = flavor.matches('*').count() >= 2;
        let mut italic = true;
        for ch in flavor.chars() {
            if ch == '*' && toggles {
                italic = !italic;
                continue;
            }
            cells.push((ch, Attr { italic, color: format.flavor_color, ..Attr::default() }));
        }
        if toggles {
            flavor = flavor.replace('*', "");
        }
    }

    // Runs: neighbours that are styled alike. Each character of a symbol is its own run.
    let mut runs: Vec<TextRun> = Vec::new();
    let mut last: Option<&Attr> = None;
    for (ch, attr) in &cells {
        // A line break carries no style of its own: it joins whatever is before it.
        let same = *ch == '\n' && last.is_some_and(|a| a.symbol.is_none()) || last == Some(attr) && attr.symbol.is_none();
        if same {
            runs.last_mut().expect("last is set with a run").text.push(*ch);
            continue;
        }
        let font = if attr.symbol.is_some() {
            Some(format.font_symbol.clone())
        } else if attr.italic {
            Some(format.font_italic.clone())
        } else if attr.bold {
            Some(format.font_bold.clone())
        } else {
            None
        };
        runs.push(TextRun {
            text: ch.to_string(),
            italic: attr.italic.then_some(true),
            bold: attr.bold.then_some(true),
            font,
            color: attr.color,
            symbol: attr.symbol.as_ref().map(|(s, _)| s.clone()),
            ..TextRun::default()
        });
        last = Some(attr);
    }

    // Paragraphs.
    let text: String = cells.iter().map(|(ch, _)| *ch).collect();
    let lines: Vec<&str> = if text.is_empty() { Vec::new() } else { text.split('\n').collect() };
    let lead = if format.centered { 0.0 } else { format.line_break_lead };
    let align = format.centered.then_some(TextAlign::Center);
    let mut paragraphs: Vec<ParagraphStyle> = lines.iter().map(|_| ParagraphStyle { space_before: Some(lead), align, ..ParagraphStyle::default() }).collect();

    // A modal list: from the first bullet to the last, bullets hang in the margin.
    let bullets: Vec<usize> = lines.iter().enumerate().filter(|(_, line)| line.contains('•')).map(|(i, _)| i).collect();
    if let (Some(&first), Some(&last)) = (bullets.first(), bullets.last()) {
        for paragraph in &mut paragraphs[first..=last] {
            paragraph.space_before = Some(format.bullet_lead);
            paragraph.indent = Some(format.modal_indent);
            paragraph.first_line_indent = Some(-format.modal_indent);
        }
    }

    let mut flavor_paragraph = None;
    if !flavor.is_empty() {
        let first = rules_paragraphs;
        // More room between rules and flavor text, and no indent.
        if let Some(paragraph) = paragraphs.get_mut(first) {
            paragraph.space_before = Some(if format.divider && !rules.is_empty() { format.flavor_lead_divider } else { format.flavor_lead });
            paragraph.indent = None;
            paragraph.first_line_indent = None;
        }
        // Later lines of the flavor text (a quote and who said it) sit tight together.
        for paragraph in paragraphs.iter_mut().skip(first + 1) {
            paragraph.space_before = Some(0.0);
            paragraph.indent = None;
            paragraph.first_line_indent = None;
        }
        if format.right_align_quote {
            let flavor_lines: Vec<&str> = flavor.split('\n').collect();
            let credit = (1..flavor_lines.len()).find(|&i| flavor_lines[i - 1].ends_with('"') && flavor_lines[i].starts_with('—'));
            if let Some(credit) = credit {
                for paragraph in paragraphs.iter_mut().skip(first + credit) {
                    paragraph.align = Some(TextAlign::Right);
                }
            }
        }
        if !rules.is_empty() {
            flavor_paragraph = Some(first);
        }
    }

    FormattedText { content: RichText { runs, paragraphs }, unknown_symbols, flavor_paragraph }
}
