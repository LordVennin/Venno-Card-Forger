//! Proxyshop's frame logic cases (tests/data/frame_data.toml), run against
//! Scryfall game data saved in tests/data/frame_cards.json, plus its art file
//! name cases (tests/data/text_file_name.toml).

use cardsmith_cards::file_name::{parse_card_info, CardInfo};
use cardsmith_cards::layout::{card_data, process_card_data, CardInput};
use serde_json::Value;

fn flag(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

#[test]
fn every_proxyshop_frame_case_passes() {
    let cases: toml::Table = include_str!("data/frame_data.toml").parse().unwrap();
    let cards: Value = serde_json::from_str(include_str!("data/frame_cards.json")).unwrap();
    let (mut total, mut failures) = (0, Vec::new());
    for (group, list) in &cases {
        for (key, expected) in list.as_table().unwrap() {
            total += 1;
            let expected: Vec<&str> = expected.as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
            // "Name [SET]" asks for a printing; the saved data is already that printing.
            let name = key.split('[').next().unwrap().trim();
            let mut data = cards["cards"][key.as_str()].clone();
            assert!(data.is_object(), "{key} is missing from frame_cards.json; run tools/fetch-frame-cards.py");
            let parts = data["meld_parts"].clone();
            process_card_data(&mut data, name, &mut |part| Some(parts[part["name"].as_str()?].clone()).filter(Value::is_object));
            let card = card_data(&data, &CardInput { name: name.to_string(), ..CardInput::default() });
            let actual = [card.layout.class_name(), &card.frame.background, &card.frame.pinlines, &card.frame.twins, flag(card.is_nyx), flag(card.frame.is_colorless)];
            if actual[..] != expected[..] {
                failures.push(format!("{group} / {key}:\n  expected {expected:?}\n  found    {actual:?}"));
            }
        }
    }
    assert!(total >= 200, "only {total} cases were read");
    assert!(failures.is_empty(), "{} of {total} cases failed:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn every_proxyshop_file_name_case_passes() {
    let cases: toml::Table = include_str!("data/text_file_name.toml").parse().unwrap();
    assert_eq!(cases.len(), 9);
    for (scenario, case) in &cases {
        let want = |key: &str| case["result"][key].as_str().unwrap().to_string();
        let mut expected = CardInfo { name: want("name"), set: want("set"), number: want("number"), artist: want("artist"), creator: want("creator") };
        // Proxyshop's data has a slip here: the name says there is a set, and the text has one, but the result leaves it empty.
        if scenario == "Name/Artist/Set/Num" {
            expected.set = "SET".into();
        }
        assert_eq!(parse_card_info(case["text"].as_str().unwrap()), expected, "{scenario}");
    }
}
