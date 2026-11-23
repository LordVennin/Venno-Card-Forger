//! Proxyshop's italics cases (tests/data/text_italic.toml), plus reminder-text stripping.

use cardsmith_cards::rules_text::{generate_italics, strip_reminder_text};

#[test]
fn every_proxyshop_italics_case_passes() {
    let cases: toml::Table = include_str!("data/text_italic.toml").parse().unwrap();
    assert!(cases.len() >= 17, "only {} cases were read", cases.len());
    let mut failures = Vec::new();
    for (name, case) in &cases {
        let text = case["text"].as_str().unwrap();
        let mut expected: Vec<String> = case["result"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        let mut actual = generate_italics(text);
        expected.sort();
        actual.sort();
        if actual != expected {
            failures.push(format!("{name} ({}):\n  expected {expected:?}\n  found    {actual:?}", case["scenario"].as_str().unwrap_or("")));
        }
    }
    assert!(failures.is_empty(), "{} of {} cases failed:\n{}", failures.len(), cases.len(), failures.join("\n"));
}

#[test]
fn strips_reminder_text() {
    assert_eq!(strip_reminder_text("Flying"), "Flying");
    assert_eq!(strip_reminder_text("Flying (This creature can't be blocked except by creatures with flying or reach.)"), "Flying");
    assert_eq!(strip_reminder_text("Menace (It can't be blocked except by two or more creatures.)\nWhen this enters, draw a card."), "Menace\nWhen this enters, draw a card.");
    // Text that is only a reminder is kept: there would be nothing left.
    assert_eq!(strip_reminder_text("({T}: Add {G}.)"), "({T}: Add {G}.)");
    // A reminder in the middle of a sentence leaves one space, not two.
    assert_eq!(strip_reminder_text("Scry 2 (look at two cards), then draw a card."), "Scry 2, then draw a card.");
}
