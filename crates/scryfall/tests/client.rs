//! The client against a pretend network: what is asked, what is cached, and
//! what happens when the network goes away.

use cardsmith_scryfall::{BulkKind, Client, Error, FetchError, Transport};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Answers from a table of URL -> body, and records every request.
#[derive(Clone, Default)]
struct Fake {
    pages: Rc<RefCell<HashMap<String, Vec<u8>>>>,
    asked: Rc<RefCell<Vec<String>>>,
    offline: Rc<RefCell<bool>>,
}

impl Fake {
    fn page(&self, url: &str, body: Value) {
        self.pages.borrow_mut().insert(url.to_string(), body.to_string().into_bytes());
    }
    fn asked(&self) -> Vec<String> {
        self.asked.borrow().clone()
    }
}

impl Transport for Fake {
    fn get(&mut self, url: &str) -> Result<Vec<u8>, FetchError> {
        self.asked.borrow_mut().push(url.to_string());
        if *self.offline.borrow() {
            return Err(FetchError::Unreachable("no network".into()));
        }
        self.pages.borrow().get(url).cloned().ok_or_else(|| FetchError::NotFound(json!({ "object": "error", "details": "No cards found." }).to_string().into_bytes()))
    }
    fn download(&mut self, url: &str, to: &Path) -> Result<(), FetchError> {
        let body = self.get(url)?;
        std::fs::write(to, body).map_err(|e| FetchError::Other(e.to_string()))
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cardsmith-scryfall-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

const BOLT: &str = "https://api.scryfall.com/cards/search?q=%21%22Lightning%20Bolt%22%20lang%3Aen&unique=arts&order=released&dir=desc&include_extras=false";

fn bolt() -> Value {
    json!({ "object": "list", "data": [
        { "object": "card", "name": "Lightning Bolt", "layout": "art_series", "set": "art" },
        { "object": "card", "name": "Lightning Bolt", "layout": "normal", "set": "clu", "mana_cost": "{R}" },
        { "object": "card", "name": "Lightning Bolt", "layout": "normal", "set": "2x2", "mana_cost": "{R}" }
    ] })
}

#[test]
fn a_card_by_name_is_the_first_playable_search_result_and_then_comes_from_the_cache() {
    let dir = temp("name");
    let net = Fake::default();
    net.page(BOLT, bolt());
    let mut client = Client::with_transport(net.clone(), &dir);
    let card = client.card_by_name("Lightning Bolt", None).unwrap();
    assert_eq!(card["set"], "clu");
    assert_eq!(net.asked(), [BOLT]);

    // Asked again: no request. Offline, with a new client: still answered.
    assert_eq!(client.card_by_name("Lightning Bolt", None).unwrap()["set"], "clu");
    assert_eq!(net.asked().len(), 1);
    *net.offline.borrow_mut() = true;
    let mut later = Client::with_transport(net.clone(), &dir);
    assert_eq!(later.card_by_name("Lightning Bolt", None).unwrap()["mana_cost"], "{R}");
    assert_eq!(net.asked().len(), 1);

    // Something never fetched cannot be answered offline, and says so.
    assert!(matches!(later.card_by_name("Shock", None), Err(Error::Offline(_))));
}

#[test]
fn the_card_itself_is_preferred_over_a_newer_card_that_has_it_as_a_face() {
    let dir = temp("faces");
    let net = Fake::default();
    let url = "https://api.scryfall.com/cards/search?q=%21%22Old%20Spell%22%20lang%3Aen&unique=arts&order=released&dir=desc&include_extras=false";
    net.page(url, json!({ "object": "list", "data": [
        { "object": "card", "name": "New Wizard // Old Spell", "layout": "prepare", "set": "new" },
        { "object": "card", "name": "Old Spell", "layout": "normal", "set": "old" }
    ] }));
    let mut client = Client::with_transport(net.clone(), &dir);
    assert_eq!(client.card_by_name("Old Spell", None).unwrap()["set"], "old");
    // A back face has no card of its own name, so the card that carries it is used.
    let back = "https://api.scryfall.com/cards/search?q=%21%22Back%20Face%22%20lang%3Aen&unique=arts&order=released&dir=desc&include_extras=false";
    net.page(back, json!({ "object": "list", "data": [{ "object": "card", "name": "Front Face // Back Face", "layout": "transform", "set": "dfc" }] }));
    assert_eq!(client.card_by_name("Back Face", None).unwrap()["set"], "dfc");
}

#[test]
fn a_set_narrows_the_search_and_extras_are_tried_when_nothing_is_found() {
    let dir = temp("set");
    let net = Fake::default();
    let with_extras = "https://api.scryfall.com/cards/search?q=%21%22Akoum%22%20lang%3Aen%20set%3Aopca&unique=arts&order=released&dir=desc&include_extras=true";
    net.page(with_extras, json!({ "object": "list", "data": [{ "object": "card", "name": "Akoum", "layout": "planar", "set": "opca" }] }));
    let mut client = Client::with_transport(net.clone(), &dir);
    assert_eq!(client.card_by_name("Akoum", Some("OPCA")).unwrap()["layout"], "planar");
    let asked = net.asked();
    assert_eq!(asked.len(), 2);
    assert!(asked[0].ends_with("set%3Aopca&unique=arts&order=released&dir=desc&include_extras=false"), "{}", asked[0]);
    assert_eq!(asked[1], with_extras);

    match client.card_by_name("No Such Card", None) {
        Err(Error::NotFound(what)) => assert_eq!(what, "\"No Such Card\""),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_card_by_set_and_number_drops_leading_zeros_and_works_offline_afterwards() {
    let dir = temp("number");
    let net = Fake::default();
    net.page("https://api.scryfall.com/cards/mh2/75", json!({ "object": "card", "name": "Damn", "set": "mh2", "collector_number": "75" }));
    let mut client = Client::with_transport(net.clone(), &dir);
    assert_eq!(client.card("ignored", Some("MH2"), Some("075")).unwrap()["name"], "Damn");
    *net.offline.borrow_mut() = true;
    let mut later = Client::with_transport(net.clone(), &dir);
    assert_eq!(later.card_by_number("mh2", "75").unwrap()["name"], "Damn");
    assert!(matches!(later.card_by_number("mh2", "76"), Err(Error::Offline(_))));
    // Without a number the name is used.
    net.page(BOLT, bolt());
    *net.offline.borrow_mut() = false;
    assert_eq!(later.card("Lightning Bolt", Some(""), None).unwrap()["set"], "clu");
}

#[test]
fn bulk_data_is_downloaded_once_and_answers_lookups_without_the_network() {
    let dir = temp("bulk");
    let net = Fake::default();
    net.page("https://api.scryfall.com/bulk-data/default-cards", json!({ "object": "bulk_data", "download_uri": "https://data.scryfall.io/default-cards/today.json" }));
    net.page(
        "https://data.scryfall.io/default-cards/today.json",
        json!([
            { "name": "Lightning Bolt", "set": "lea", "collector_number": "161", "released_at": "1993-08-05", "layout": "normal" },
            { "name": "Lightning Bolt", "set": "clu", "collector_number": "141", "released_at": "2024-02-23", "layout": "normal" },
            { "name": "Lightning Bolt", "set": "art", "collector_number": "1", "released_at": "2025-01-01", "layout": "art_series" },
            { "name": "Delver of Secrets // Insectile Aberration", "set": "isd", "collector_number": "51", "released_at": "2011-09-30", "layout": "transform" }
        ]),
    );
    let mut client = Client::with_transport(net.clone(), &dir);
    let cards = client.bulk(BulkKind::DefaultCards).unwrap();
    assert_eq!(cards.len(), 4);
    // The newest playable printing, or the one in the set asked for.
    assert_eq!(cards.card_by_name("lightning bolt", None).unwrap()["set"], "clu");
    assert_eq!(cards.card_by_name("Lightning Bolt", Some("LEA")).unwrap()["collector_number"], "161");
    assert_eq!(cards.card_by_name("Insectile Aberration", None).unwrap()["set"], "isd");
    assert_eq!(cards.card_by_number("ISD", "051").unwrap()["layout"], "transform");
    assert!(cards.card_by_name("Shock", None).is_none());

    // A second open, offline, makes no request and still answers.
    let before = net.asked().len();
    *net.offline.borrow_mut() = true;
    let again = Client::with_transport(net.clone(), &dir).bulk(BulkKind::DefaultCards).unwrap();
    assert_eq!(net.asked().len(), before);
    assert_eq!(again.card_by_name("Lightning Bolt", None).unwrap()["set"], "clu");
    // A kind never downloaded cannot be opened offline.
    assert!(matches!(Client::with_transport(net.clone(), &dir).bulk(BulkKind::OracleCards), Err(Error::Offline(_))));
}

#[test]
fn bulk_data_is_also_read_as_gzipped_lines_which_is_what_scryfall_offers_now() {
    use std::io::Write;
    let dir = temp("bulk-lines");
    let net = Fake::default();
    net.page("https://api.scryfall.com/bulk-data/oracle-cards", json!({ "object": "bulk_data", "jsonl_download_uri": "https://data.scryfall.io/oracle-cards/today.jsonl.gz" }));
    let lines = [json!({ "name": "Shock", "set": "m21", "collector_number": "159", "released_at": "2020-07-03", "layout": "normal" }), json!({ "name": "Opt", "set": "eld", "collector_number": "59", "released_at": "2019-10-04", "layout": "normal" })];
    let mut packed = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    for line in &lines {
        writeln!(packed, "{line}").unwrap();
    }
    net.pages.borrow_mut().insert("https://data.scryfall.io/oracle-cards/today.jsonl.gz".into(), packed.finish().unwrap());
    let cards = Client::with_transport(net.clone(), &dir).bulk(BulkKind::OracleCards).unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards.card_by_name("opt", None).unwrap()["set"], "eld");
    assert_eq!(cards.card_by_number("m21", "159").unwrap()["name"], "Shock");

    // Offline, a card that was never looked up is still found in the bulk data.
    *net.offline.borrow_mut() = true;
    let mut offline = Client::with_transport(net.clone(), &dir);
    assert_eq!(offline.card("Shock", None, None).unwrap()["set"], "m21");
    assert_eq!(offline.card("", Some("ELD"), Some("059")).unwrap()["name"], "Opt");
    assert!(matches!(offline.card("Counterspell", None, None), Err(Error::Offline(_))));
}

#[test]
fn card_data_comes_back_the_same_from_the_cache_when_offline() {
    use cardsmith_cards::file_name::parse_card_info;
    let dir = temp("data");
    let net = Fake::default();
    let bolt = json!({ "object": "card", "name": "Lightning Bolt", "layout": "normal", "set": "clu", "collector_number": "141", "rarity": "uncommon", "mana_cost": "{R}", "type_line": "Instant", "oracle_text": "Lightning Bolt deals 3 damage to any target.", "color_identity": ["R"], "artist": "Someone" });
    net.page(BOLT, json!({ "object": "list", "data": [bolt.clone()] }));
    net.page("https://api.scryfall.com/cards/clu/141", bolt);
    // A meld pair: the half asked for, its partner, and what they become.
    let part = |name: &str, kind: &str| json!({ "object": "related_card", "component": kind, "name": name, "uri": format!("https://api.scryfall.com/cards/{}", name.to_lowercase()) });
    let parts = json!([part("Half", "meld_part"), part("Partner", "meld_part"), part("Whole", "meld_result")]);
    let half = json!({ "object": "card", "name": "Half", "layout": "meld", "set": "emn", "mana_cost": "{2}{W}", "type_line": "Creature — Angel", "power": "4", "toughness": "3", "all_parts": parts });
    net.page("https://api.scryfall.com/cards/search?q=%21%22Half%22%20lang%3Aen&unique=arts&order=released&dir=desc&include_extras=false", json!({ "object": "list", "data": [half.clone()] }));
    net.page("https://api.scryfall.com/cards/half", half);
    net.page("https://api.scryfall.com/cards/whole", json!({ "object": "card", "name": "Whole", "layout": "meld", "set": "emn", "mana_cost": "", "type_line": "Creature — Eldrazi Angel", "power": "9", "toughness": "10" }));

    let mut client = Client::with_transport(net.clone(), &dir);
    let by_name = client.card_data(&parse_card_info("Lightning Bolt (Painter).png")).unwrap();
    let by_number = client.card_data(&parse_card_info("Anything [CLU] {141}.png")).unwrap();
    let melded = client.card_data(&parse_card_info("Half.jpg")).unwrap();
    assert_eq!((by_name.name.as_str(), by_name.artist.as_str(), by_name.frame.pinlines.as_str(), by_name.rarity.as_str()), ("Lightning Bolt", "Painter", "R", "uncommon"));
    assert_eq!((by_number.name.as_str(), by_number.artist.as_str(), by_number.collector_data().as_str()), ("Lightning Bolt", "Someone", "U 0141"));
    assert_eq!((melded.layout.class_name(), melded.is_front, melded.transform_icon.as_str()), ("TransformLayout", true, "meld"));
    assert_eq!(melded.other_face.as_ref().unwrap().name, "Whole");

    // Offline, a new client gives exactly the same cards and asks for nothing.
    let asked = net.asked().len();
    *net.offline.borrow_mut() = true;
    let mut later = Client::with_transport(net.clone(), &dir);
    assert_eq!(later.card_data(&parse_card_info("Lightning Bolt (Painter).png")).unwrap(), by_name);
    assert_eq!(later.card_data(&parse_card_info("Anything [CLU] {141}.png")).unwrap(), by_number);
    assert_eq!(later.card_data(&parse_card_info("Half.jpg")).unwrap(), melded);
    assert_eq!(net.asked().len(), asked);
    assert!(matches!(later.card_data(&parse_card_info("Shock.png")), Err(Error::Offline(_))));
}

#[test]
fn set_symbols_and_set_sizes_come_from_hexproof_and_then_from_the_cache() {
    use std::io::Write;
    let dir = temp("hexproof");
    let net = Fake::default();
    net.page("https://api.hexproof.io/sets/", json!({
        "neo": { "code_symbol": "neo", "count_cards": 515, "count_printed": 302 },
        "pneo": { "code_symbol": "neo", "count_cards": 40 },
        "odd": { "code_symbol": "default", "count_cards": 0 }
    }));
    // A package with a symbol folder for NEO, one for a set the data calls "default", and the default itself.
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, body) in [("manifest.json", "{}"), ("set/NEO/R.svg", "<svg>neo rare</svg>"), ("set/NEO/M.svg", "<svg>neo mythic</svg>"), ("set/ODD/C.svg", "<svg>odd common</svg>"), ("set/DEFAULT/C.svg", "<svg>default common</svg>"), ("set/DEFAULT/R.svg", "<svg>default rare</svg>")] {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    net.pages.borrow_mut().insert("https://api.hexproof.io/symbols/package/".into(), zip.finish().unwrap().into_inner());

    let mut client = Client::with_transport(net.clone(), &dir);
    assert_eq!(client.symbol_svg("NEO", "R").as_deref(), Some("<svg>neo rare</svg>"));
    // A promo set shares its parent's symbol; an odd rarity falls back to the mythic one.
    assert_eq!(client.symbol_svg("pneo", "T").as_deref(), Some("<svg>neo mythic</svg>"));
    // A set whose data names the default symbol still uses a folder of its own if there is one, then the default.
    assert_eq!(client.symbol_svg("odd", "C").as_deref(), Some("<svg>odd common</svg>"));
    assert_eq!(client.symbol_svg("odd", "R").as_deref(), Some("<svg>default rare</svg>"));
    assert_eq!(client.symbol_svg("nope", "C").as_deref(), Some("<svg>default common</svg>"));
    assert_eq!(client.symbol_svg("nope", "U"), None);
    // Printed size where known, else the card count; none when the data has neither.
    assert_eq!((client.set_info("NEO").size, client.set_info("pneo").size, client.set_info("odd").size, client.set_info("nope").size), (Some(302), Some(40), None, None));

    // The package was fetched once; offline, a new client still has everything.
    let asked = net.asked();
    assert_eq!(asked.iter().filter(|u| u.contains("symbols/package")).count(), 1);
    *net.offline.borrow_mut() = true;
    let mut later = Client::with_transport(net.clone(), &dir);
    assert_eq!(later.symbol_svg("neo", "R").as_deref(), Some("<svg>neo rare</svg>"));
    assert_eq!(later.set_info("neo"), cardsmith_scryfall::SetInfo { symbol: "NEO".into(), size: Some(302) });
    assert_eq!(net.asked().len(), asked.len());
    // With nothing cached and no network there is simply no symbol.
    assert_eq!(Client::with_transport(net.clone(), temp("hexproof-empty")).symbol_svg("neo", "R"), None);
}
