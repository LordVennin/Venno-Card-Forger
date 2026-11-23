//! Symbols and the rules text formatter.

use cardsmith_cards::rules_text::*;
use cardsmith_core::doc::{TextAlign, TextRun};

const BLACK: [f64; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f64; 4] = [1.0, 1.0, 1.0, 1.0];

fn rgb(r: u8, g: u8, b: u8) -> [f64; 4] {
    [r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 1.0]
}

fn colors(symbol: &str) -> Vec<[f64; 4]> {
    symbol_colors(symbol, symbol_chars(symbol).unwrap(), &SymbolColors::default()).unwrap()
}

#[test]
fn every_symbol_in_the_map_has_one_color_per_character() {
    let all = [
        "{W/P}", "{U/P}", "{B/P}", "{R/P}", "{G/P}", "{W/U/P}", "{U/B/P}", "{B/R/P}", "{R/G/P}", "{G/W/P}", "{W/B/P}", "{B/G/P}", "{G/U/P}", "{U/R/P}", "{R/W/P}", "{A}", "{E}", "{T}", "{X}", "{Y}", "{Z}", "{∞}",
        "{0}", "{1}", "{2}", "{3}", "{4}", "{5}", "{6}", "{7}", "{8}", "{9}", "{10}", "{11}", "{12}", "{13}", "{14}", "{15}", "{16}", "{17}", "{18}", "{19}", "{20}", "{W}", "{U}", "{B}", "{R}", "{G}", "{C}",
        "{W/U}", "{U/B}", "{B/R}", "{R/G}", "{G/W}", "{W/B}", "{B/G}", "{G/U}", "{U/R}", "{R/W}", "{2/W}", "{2/U}", "{2/B}", "{2/R}", "{2/G}", "{S}", "{Q}", "{CHAOS}",
    ];
    for symbol in all {
        let chars = symbol_chars(symbol).unwrap_or_else(|| panic!("{symbol} is not in the map"));
        assert_eq!(colors(symbol).len(), chars.chars().count(), "{symbol}");
    }
    assert_eq!(symbol_chars("{NOPE}"), None);
}

#[test]
fn symbols_take_proxyshops_colors() {
    let (w, u, b, generic) = (rgb(255, 251, 214), rgb(170, 224, 250), rgb(204, 194, 193), rgb(204, 194, 193));
    let dark_b = rgb(159, 146, 143);
    // A disc in the mana's color under a black glyph.
    assert_eq!(colors("{W}"), [w, BLACK]);
    // Numbers, tap and colorless: a grey disc.
    assert_eq!(colors("{3}"), [generic, BLACK]);
    assert_eq!(colors("{T}"), [generic, BLACK]);
    assert_eq!(colors("{C}"), [generic, BLACK]);
    // Hybrid: the second color's half first, then the first color's, then the two glyphs.
    assert_eq!(colors("{W/U}"), [u, w, BLACK, BLACK]);
    // Generic hybrid and Phyrexian use the darker black.
    assert_eq!(colors("{2/B}"), [dark_b, generic, BLACK, BLACK]);
    assert_eq!(colors("{B/P}"), [dark_b, BLACK]);
    assert_eq!(colors("{W/B/P}"), [b, w, BLACK, BLACK]);
    assert_eq!(colors("{E}"), [BLACK]);
    assert_eq!(colors("{S}"), [generic, BLACK, WHITE]);
    assert_eq!(colors("{Q}"), [BLACK, WHITE]);
}

fn format() -> RulesFormat {
    RulesFormat::for_dpi(1200.0)
}

/// Runs as (text, font) pairs, with "-" for the layer's font.
fn fonts(runs: &[TextRun]) -> Vec<(String, String)> {
    runs.iter().map(|r| (r.text.clone(), r.font.clone().unwrap_or_else(|| "-".into()))).collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

#[test]
fn symbols_become_colored_characters_of_the_symbol_font() {
    let out = format_rules_text("{1}{W}, {T}: Draw a card.", "", &format());
    assert_eq!(
        fonts(&out.content.runs),
        pairs(&[("o", "Proxyglyph"), ("1", "Proxyglyph"), ("o", "Proxyglyph"), ("w", "Proxyglyph"), (", ", "-"), ("o", "Proxyglyph"), ("t", "Proxyglyph"), (": Draw a card.", "-")])
    );
    let w = &out.content.runs[2..4];
    assert_eq!((w[0].color, w[1].color), (Some(rgb(255, 251, 214)), Some(BLACK)));
    assert_eq!(w[0].symbol.as_deref(), Some("{W}"));
    assert!(out.unknown_symbols.is_empty() && out.flavor_paragraph.is_none());
}

#[test]
fn unknown_symbols_are_written_out_and_reported() {
    let out = format_rules_text("{TK}{TK}: Do a thing.", "", &format());
    assert_eq!(fonts(&out.content.runs), pairs(&[("TKTK: Do a thing.", "-")]));
    assert_eq!(out.unknown_symbols, ["{TK}"]);
}

#[test]
fn reminder_text_and_ability_words_are_italic_where_they_stand() {
    let out = format_rules_text("Flying (It can fly.)\nLandfall — Whenever a land enters, you gain 1 life. Landfall is fun.", "", &format());
    assert_eq!(
        fonts(&out.content.runs),
        // The line break joins the run before it; the second "Landfall" is not an ability word.
        pairs(&[("Flying ", "-"), ("(It can fly.)\nLandfall", "PlantinMTPro-Italic"), (" — Whenever a land enters, you gain 1 life. Landfall is fun.", "-")])
    );
    assert_eq!(out.content.runs[1].italic, Some(true));
    // Symbols inside reminder text keep the symbol font.
    let mana = format_rules_text("({T}: Add {G}.)", "", &format());
    assert_eq!(fonts(&mana.content.runs)[..3], pairs(&[("(", "PlantinMTPro-Italic"), ("o", "Proxyglyph"), ("t", "Proxyglyph")]));
}

#[test]
fn bold_rules_text_leaves_italics_and_flavor_alone() {
    let out = format_rules_text("Flying (It can fly.)", "Up high.", &RulesFormat { bold_rules: true, ..format() });
    assert_eq!(fonts(&out.content.runs), pairs(&[("Flying ", "PlantinMTPro-Bold"), ("(It can fly.)\nUp high.", "PlantinMTPro-Italic")]));
}

#[test]
fn paragraphs_get_proxyshops_spacing() {
    // 1200 dpi: a point is 16.67 px.
    let out = format_rules_text("Flying\nVigilance", "", &format());
    let before: Vec<f64> = out.content.paragraphs.iter().map(|p| p.space_before.unwrap().round()).collect();
    assert_eq!(before, [40.0, 40.0]);
    assert!(out.content.paragraphs.iter().all(|p| p.indent.is_none() && p.align.is_none()));

    let centered = format_rules_text("Flying\nVigilance", "", &RulesFormat { centered: true, ..format() });
    assert!(centered.content.paragraphs.iter().all(|p| p.space_before == Some(0.0) && p.align == Some(TextAlign::Center)));
}

#[test]
fn modal_bullets_hang_in_the_margin() {
    let out = format_rules_text("Choose one —\n• Draw a card.\n• Gain 3 life.\nEntwine {2}", "", &format());
    let p = &out.content.paragraphs;
    assert_eq!(p.len(), 4);
    assert_eq!((p[0].indent, p[3].indent), (None, None));
    for bullet in &p[1..3] {
        assert_eq!(bullet.indent.unwrap().round(), 95.0);
        assert_eq!(bullet.first_line_indent.unwrap().round(), -95.0);
        assert_eq!(bullet.space_before.unwrap().round(), 17.0);
    }
}

#[test]
fn flavor_text_is_italic_spaced_apart_and_marks_where_the_divider_goes() {
    let out = format_rules_text("Flying", "\"Look up.\"\n—Somebody", &format());
    assert_eq!(fonts(&out.content.runs), pairs(&[("Flying\n", "-"), ("\"Look up.\"\n—Somebody", "PlantinMTPro-Italic")]));
    assert_eq!(out.flavor_paragraph, Some(1));
    let before: Vec<f64> = out.content.paragraphs.iter().map(|p| p.space_before.unwrap().round()).collect();
    // 4.4 pt above the flavor text; the attribution sits tight under the quote.
    assert_eq!(before, [40.0, 73.0, 0.0]);

    let divided = format_rules_text("Flying", "Up high.", &RulesFormat { divider: true, ..format() });
    assert_eq!(divided.content.paragraphs[1].space_before.unwrap().round(), 117.0);

    let right = format_rules_text("Flying", "\"Look up.\"\n—Somebody", &RulesFormat { right_align_quote: true, ..format() });
    let aligns: Vec<Option<TextAlign>> = right.content.paragraphs.iter().map(|p| p.align).collect();
    assert_eq!(aligns, [None, None, Some(TextAlign::Right)]);

    // Flavor text alone: no divider position, and no rules paragraph before it.
    let only = format_rules_text("", "Up high.", &format());
    assert_eq!((only.flavor_paragraph, only.content.paragraphs.len()), (None, 1));
    assert_eq!(fonts(&only.content.runs), pairs(&[("Up high.", "PlantinMTPro-Italic")]));
}

#[test]
fn asterisks_in_flavor_text_switch_italics_off_and_are_removed() {
    let out = format_rules_text("", "She read *The Art of War* twice.", &format());
    assert_eq!(fonts(&out.content.runs), pairs(&[("She read ", "PlantinMTPro-Italic"), ("The Art of War", "-"), (" twice.", "PlantinMTPro-Italic")]));
    // A single asterisk is just an asterisk.
    let one = format_rules_text("", "Rated 5*", &format());
    assert_eq!(fonts(&one.content.runs), pairs(&[("Rated 5*", "PlantinMTPro-Italic")]));
    let colored = format_rules_text("Flying", "Up high.", &RulesFormat { flavor_color: Some(WHITE), ..format() });
    assert_eq!(colored.content.runs.last().unwrap().color, Some(WHITE));
}

#[test]
fn empty_text_gives_empty_rich_text() {
    let out = format_rules_text("", "", &format());
    assert!(out.content.runs.is_empty() && out.content.paragraphs.is_empty());
}
