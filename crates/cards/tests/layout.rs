//! The fields read from Scryfall data, checked on a few saved cards.

use cardsmith_cards::layout::{card_data, normalize_name, process_card_data, CardData, CardInput, LayoutKind};
use serde_json::{json, Value};

fn saved(key: &str) -> Value {
    let cards: Value = serde_json::from_str(include_str!("data/frame_cards.json")).unwrap();
    cards["cards"][key].clone()
}

fn card(key: &str) -> CardData {
    let mut data = saved(key);
    let parts = data["meld_parts"].clone();
    process_card_data(&mut data, key, &mut |part| Some(parts[part["name"].as_str()?].clone()).filter(Value::is_object));
    card_data(&data, &CardInput { name: key.into(), ..CardInput::default() })
}

#[test]
fn reads_the_plain_fields_of_a_creature() {
    let c = card("Thassa, God of the Sea");
    assert_eq!(c.layout, LayoutKind::Normal);
    assert_eq!((c.supertypes.as_slice(), c.types.as_slice(), c.subtypes.as_slice()), (&["Legendary".to_string()][..], &["Enchantment".to_string(), "Creature".to_string()][..], &["God".to_string()][..]));
    assert!(c.is_creature && c.is_legendary && c.is_nyx && !c.is_land && !c.is_transform && c.is_front);
    assert_eq!((c.transform_icon.as_str(), c.other_face.is_none()), ("", true));
    assert_eq!(c.lang, "EN");
}

#[test]
fn a_back_face_knows_it_is_one_and_what_is_on_the_other_side() {
    let back = card("Insectile Aberration");
    assert_eq!((back.layout, back.is_front, back.is_transform), (LayoutKind::Transform, false, true));
    assert_eq!(back.name, "Insectile Aberration");
    assert!(!back.transform_icon.is_empty());
    let other = back.other_face.unwrap();
    assert_eq!((other.name.as_str(), other.mana_cost.as_str(), other.twins.as_str()), ("Delver of Secrets", "{U}", "U"));
}

#[test]
fn a_modal_land_face_hints_at_the_other_faces_type_and_mana() {
    let front = card("Barkchannel Pathway");
    assert_eq!((front.layout, front.is_mdfc, front.is_front, front.is_land), (LayoutKind::ModalDoubleFaced, true, true, true));
    let other = front.other_face.unwrap();
    assert_eq!((other.name.as_str(), other.hint_left.as_str(), other.hint_right.as_str()), ("Tidechannel Pathway", "Land", "{T}: Add {U}."));
    // A spell on the other side shows its cost instead.
    let blex = card("Blex, Vexing Pest").other_face.unwrap();
    assert_eq!((blex.hint_left.as_str(), blex.hint_right.as_str()), ("Sorcery", "{2}{B}{B}"));
}

#[test]
fn a_melded_card_is_the_back_of_its_first_half_and_planeswalkers_get_their_own_layouts() {
    let brisela = card("Brisela, Voice of Nightmares");
    assert_eq!((brisela.layout, brisela.is_front, brisela.transform_icon.as_str()), (LayoutKind::Transform, false, "meld"));
    assert_eq!(card("Urza, Planeswalker").layout, LayoutKind::PlaneswalkerTransform);
    let ajani = card("Ajani, Sleeper Agent");
    assert_eq!(ajani.layout, LayoutKind::Planeswalker);
    assert!(!ajani.loyalty.is_empty() && !ajani.oracle_text.contains('\u{2212}'));
}

#[test]
fn rarity_collector_line_artist_and_promo_come_from_the_printing() {
    let base = json!({ "object": "card", "layout": "normal", "name": "Test", "type_line": "Basic Snow Land — Forest", "set": "tst", "collector_number": "★12a", "artist": "Jack Smith & Jill Smith", "rarity": "special", "frame": "1997", "promo_types": ["stamped"] });
    let read = |data: &Value, input: CardInput| card_data(data, &input);
    let c = read(&base, CardInput { name: "Test".into(), ..CardInput::default() });
    assert_eq!((c.rarity.as_str(), c.rarity_letter().as_str()), ("timeshifted", "T"));
    assert_eq!((c.collector_number_raw.as_str(), c.collector_number, c.collector_data().as_str()), ("★12a", 12, "T 0012"));
    assert_eq!(c.artist, "Jack & Jill Smith");
    assert!(c.is_promo && c.is_basic_land && c.is_snow && c.is_land && !c.is_creature);
    assert_eq!((c.set.as_str(), c.supertypes.len(), c.subtypes.as_slice()), ("TST", 2, &["Forest".to_string()][..]));
    // The file name's artist and creator win; other special rarities are drawn as common or mythic.
    let mut other = base.clone();
    other["frame"] = json!("2015");
    let c = read(&other, CardInput { name: "Test".into(), artist: "Painter".into(), creator: "Me".into() });
    assert_eq!((c.rarity.as_str(), c.artist.as_str(), c.creator.as_str()), ("common", "Painter", "Me"));
    other["rarity"] = json!("bonus");
    assert_eq!(read(&other, CardInput::default()).rarity, "mythic");
}

#[test]
fn mutate_cards_and_unknown_layouts() {
    let mut data = json!({ "object": "card", "layout": "normal", "name": "Gemrazer", "type_line": "Creature — Beast", "keywords": ["Mutate", "Reach"] });
    process_card_data(&mut data, "Gemrazer", &mut |_| None);
    assert_eq!(card_data(&data, &CardInput { name: "Gemrazer".into(), ..CardInput::default() }).layout, LayoutKind::Mutate);
    // A layout Proxyshop has never heard of is drawn as a normal card.
    assert_eq!(LayoutKind::from_scryfall("something_new"), LayoutKind::Normal);
    assert_eq!(normalize_name("Lim-Dûl's Vault"), "limdulsvault");
}
