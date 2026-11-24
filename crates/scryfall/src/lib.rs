// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// The search query, the choice of the first playable result and the lookup by
// set and number are ported from Proxyshop src/utils/scryfall.py
// (get_card_search, get_card_unique, is_playable_card) and src/cards.py
// (get_card_data).

//! Card data from Scryfall.
//!
//! Scryfall's rules, which this crate keeps (see <https://scryfall.com/docs/api>):
//! every request names the application in `User-Agent` and sends `Accept`;
//! requests are at least 100 ms apart; responses are cached on disk.
//!
//! The cache is what makes the app work offline: anything fetched once is
//! answered from disk, and when a cached answer has gone stale and the network
//! is down, the stale answer is used.
//!
//! This crate is native only (it uses the network, files and the clock). The
//! card logic that turns Scryfall's JSON into a card lives in `cardsmith-cards`.

mod bulk;
mod hexproof;
mod models;

pub use bulk::{BulkCards, BulkKind};
pub use hexproof::SetInfo;

use cardsmith_cards::file_name::CardInfo;
use cardsmith_cards::layout::{card_data, process_card_data, CardData, CardInput};
use serde_json::Value;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const API: &str = "https://api.scryfall.com";
/// Scryfall asks for 50-100 ms between requests.
const SPACING: Duration = Duration::from_millis(100);
/// Card data changes rarely (new printings, errata, prices we do not use).
const FRESH: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// Scryfall answered that there is no such card.
    NotFound(String),
    /// Scryfall could not be reached and nothing usable is cached.
    Offline(String),
    /// Anything else: a bad response, or a file that could not be written.
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::NotFound(what) => write!(f, "Scryfall has no card for {what}"),
            Error::Offline(why) => write!(f, "Scryfall could not be reached and the card is not cached ({why})"),
            Error::Other(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for Error {}

/// What a transport can report. A missing card is an answer; an unreachable server is not.
#[derive(Debug, Clone, PartialEq)]
pub enum FetchError {
    /// The server answered 404, with this body.
    NotFound(Vec<u8>),
    /// No answer: no network, DNS failure, timeout.
    Unreachable(String),
    Other(String),
}

/// How requests leave the machine. The real one is [`Http`]; tests supply their own.
pub trait Transport {
    fn get(&mut self, url: &str) -> Result<Vec<u8>, FetchError>;
    /// Fetch a large file straight to disk.
    fn download(&mut self, url: &str, to: &Path) -> Result<(), FetchError>;
}

/// HTTPS with Scryfall's required headers and spacing between requests.
pub struct Http {
    agent: ureq::Agent,
    last: Option<Instant>,
}

impl Default for Http {
    fn default() -> Http {
        Http::new()
    }
}

impl Http {
    pub fn new() -> Http {
        let agent = ureq::AgentBuilder::new().try_proxy_from_env(true).timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(60)).user_agent(concat!("Cardsmith/", env!("CARGO_PKG_VERSION"), " (desktop card editor)")).build();
        Http { agent, last: None }
    }

    fn request(&mut self, url: &str) -> Result<ureq::Response, FetchError> {
        if let Some(wait) = self.last.and_then(|last| SPACING.checked_sub(last.elapsed())) {
            std::thread::sleep(wait);
        }
        let result = self.agent.get(url).set("Accept", "application/json;q=0.9,*/*;q=0.8").call();
        self.last = Some(Instant::now());
        match result {
            Ok(response) => Ok(response),
            Err(ureq::Error::Status(404, response)) => {
                let mut body = Vec::new();
                let _ = std::io::Read::read_to_end(&mut response.into_reader(), &mut body);
                Err(FetchError::NotFound(body))
            }
            Err(ureq::Error::Status(code, _)) => Err(FetchError::Other(format!("{url} answered {code}"))),
            Err(ureq::Error::Transport(t)) => Err(FetchError::Unreachable(t.to_string())),
        }
    }
}

impl Transport for Http {
    fn get(&mut self, url: &str) -> Result<Vec<u8>, FetchError> {
        let mut body = Vec::new();
        std::io::Read::read_to_end(&mut self.request(url)?.into_reader(), &mut body).map_err(|e| FetchError::Unreachable(e.to_string()))?;
        Ok(body)
    }

    fn download(&mut self, url: &str, to: &Path) -> Result<(), FetchError> {
        let response = self.request(url)?;
        // Written beside the target and renamed, so a broken download never looks complete.
        let partial = to.with_extension("partial");
        let mut file = std::fs::File::create(&partial).map_err(|e| FetchError::Other(format!("{}: {e}", partial.display())))?;
        std::io::copy(&mut response.into_reader(), &mut file).map_err(|e| FetchError::Unreachable(e.to_string()))?;
        std::fs::rename(&partial, to).map_err(|e| FetchError::Other(format!("{}: {e}", to.display())))
    }
}

/// How a search by name chooses among a card's printings. The defaults are Proxyshop's.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOptions {
    /// "arts" or "prints".
    pub unique: String,
    /// "released", "set", "rarity", "usd", "eur", "edhrec" or "artist".
    pub order: String,
    pub ascending: bool,
    /// Include tokens, planes, schemes and the like.
    pub extras: bool,
    /// Scryfall language code.
    pub lang: String,
}

impl Default for SearchOptions {
    fn default() -> SearchOptions {
        SearchOptions { unique: "arts".into(), order: "released".into(), ascending: false, extras: false, lang: "en".into() }
    }
}

/// Percent-encode a query value or path segment.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Not a game piece Proxyshop would render: minigame inserts, art cards, reversible cards, theme inserts.
fn is_playable(card: &Value) -> bool {
    let text = |key: &str| card[key].as_str().unwrap_or("");
    !(text("set_type") == "minigame" || matches!(text("layout"), "art_series" | "reversible_card") || text("set_type") == "memorabilia" && text("name").contains("(Theme)"))
}

/// Which search result to use: the first game piece that is the card itself,
/// or failing that the first that merely has a face by that name.
///
/// Proxyshop takes the first playable result. That breaks when a newer card
/// carries an old card as one of its faces: asking for the old card by name
/// would return the new one.
fn pick<'a>(cards: &'a [Value], name: &str) -> Option<&'a Value> {
    let mut playable = cards.iter().filter(|c| is_playable(c));
    let whole = playable.clone().find(|c| c["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)));
    whole.or_else(|| playable.next())
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// One printing of a card, for choosing between them.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Printing {
    pub set: String,
    pub set_name: String,
    pub number: String,
    pub year: String,
    pub rarity: String,
    pub artist: String,
    /// Address of a small picture of it on Scryfall's image server.
    pub thumbnail: Option<String>,
}

/// Card lookups with an on-disk cache.
pub struct Client<T: Transport = Http> {
    transport: T,
    cache: PathBuf,
    pub search: SearchOptions,
}

impl Client<Http> {
    /// A client that talks to Scryfall and keeps its cache in `cache_dir`.
    pub fn new(cache_dir: impl Into<PathBuf>) -> Client<Http> {
        Client::with_transport(Http::new(), cache_dir)
    }
}

impl<T: Transport> Client<T> {
    pub fn with_transport(transport: T, cache_dir: impl Into<PathBuf>) -> Client<T> {
        Client { transport, cache: cache_dir.into(), search: SearchOptions::default() }
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    /// The cache file for a URL: a readable hint plus a hash of the whole URL.
    fn cache_file(&self, url: &str) -> PathBuf {
        let hint: String = url.trim_start_matches(API).chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let hint: String = hint.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-").chars().take(60).collect();
        self.cache.join("api").join(format!("{hint}-{:016x}.json", fnv(url)))
    }

    /// GET a JSON document through the cache.
    pub fn get_json(&mut self, url: &str) -> Result<Value, Error> {
        let file = self.cache_file(url);
        let cached = std::fs::read(&file).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let age = std::fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|at| SystemTime::now().duration_since(at).ok());
        let answer = |value: Value| if value["object"] == "error" { Err(Error::NotFound(value["details"].as_str().unwrap_or(url).to_string())) } else { Ok(value) };
        if let (Some(value), Some(age)) = (&cached, age) {
            if age < FRESH {
                return answer(value.clone());
            }
        }
        let fetched = match self.transport.get(url) {
            Ok(body) => body,
            // Not cached: a card that does not exist today may be printed tomorrow.
            Err(FetchError::NotFound(body)) => {
                let details = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v["details"].as_str().map(str::to_string));
                return Err(Error::NotFound(details.unwrap_or_else(|| url.to_string())));
            }
            // Stale is better than nothing when the network is down.
            Err(FetchError::Unreachable(why)) => return cached.map_or(Err(Error::Offline(why)), answer),
            Err(FetchError::Other(why)) => return cached.map_or(Err(Error::Other(why)), answer),
        };
        let value: Value = serde_json::from_slice(&fetched).map_err(|e| Error::Other(format!("{url}: not JSON ({e})")))?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::Other(format!("{}: {e}", dir.display())))?;
        }
        std::fs::write(&file, &fetched).map_err(|e| Error::Other(format!("{}: {e}", file.display())))?;
        answer(value)
    }

    /// The printing of a card that a search by exact name (and optionally set)
    /// puts first, skipping things that are not game pieces and preferring the
    /// card itself over a card that has it as a face (see [`pick`]). If nothing
    /// is found, the search is repeated with extras (planes, tokens) included.
    pub fn card_by_name(&mut self, name: &str, set: Option<&str>) -> Result<Value, Error> {
        let what = || format!("\"{name}\"{}", set.map(|s| format!(" in set {}", s.to_uppercase())).unwrap_or_default());
        let mut extras = self.search.extras;
        loop {
            let query = format!("!\"{name}\" lang:{}{}", self.search.lang, set.map(|s| format!(" set:{}", s.to_lowercase())).unwrap_or_default());
            let url = format!(
                "{API}/cards/search?q={}&unique={}&order={}&dir={}&include_extras={}",
                encode(&query),
                encode(&self.search.unique),
                encode(&self.search.order),
                if self.search.ascending { "asc" } else { "desc" },
                extras
            );
            let found = match self.get_json(&url) {
                Ok(list) => list["data"].as_array().and_then(|cards| pick(cards, name).cloned()),
                Err(Error::NotFound(_)) => None,
                Err(other) => return Err(other),
            };
            match found {
                Some(card) => return Ok(card),
                None if !extras => extras = true,
                None => return Err(Error::NotFound(what())),
            }
        }
    }

    /// One exact printing, by set code and collector number.
    pub fn card_by_number(&mut self, set: &str, number: &str) -> Result<Value, Error> {
        // Collector numbers are written without leading zeros, except "0" itself.
        let trimmed = number.trim().trim_start_matches('0');
        let number = if trimmed.is_empty() && !number.trim().is_empty() { "0" } else { trimmed };
        let lang = if self.search.lang == "en" { String::new() } else { format!("/{}", encode(&self.search.lang)) };
        let url = format!("{API}/cards/{}/{}{lang}", encode(&set.to_lowercase()), encode(number));
        self.get_json(&url).map_err(|e| match e {
            Error::NotFound(_) => Error::NotFound(format!("{} #{number}", set.to_uppercase())),
            other => other,
        })
    }

    /// Card names that start with, or contain, what has been typed. Scryfall
    /// answers from three letters on (at most 20 names).
    pub fn autocomplete(&mut self, typed: &str) -> Result<Vec<String>, Error> {
        let typed = typed.trim();
        if typed.chars().count() < 3 {
            return Ok(Vec::new());
        }
        let found = self.get_json(&format!("{API}/cards/autocomplete?q={}", encode(typed)))?;
        Ok(found["data"].as_array().map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default())
    }

    /// Every paper printing of a card, newest first (the first page: up to 175).
    pub fn printings(&mut self, name: &str) -> Result<Vec<Printing>, Error> {
        let query = format!("!\"{name}\" game:paper");
        let found = self.get_json(&format!("{API}/cards/search?q={}&unique=prints&order=released&dir=desc", encode(&query)))?;
        let text = |card: &Value, key: &str| card[key].as_str().unwrap_or_default().to_string();
        Ok(found["data"]
            .as_array()
            .map(|cards| {
                cards
                    .iter()
                    .map(|card| Printing {
                        set: text(card, "set"),
                        set_name: text(card, "set_name"),
                        number: text(card, "collector_number"),
                        year: text(card, "released_at").chars().take(4).collect(),
                        rarity: text(card, "rarity"),
                        artist: text(card, "artist"),
                        // A two-faced card keeps its pictures on its faces.
                        thumbnail: card["image_uris"]["small"].as_str().or(card["card_faces"][0]["image_uris"]["small"].as_str()).map(str::to_string),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// A card as the art file name describes it: by set and number when both
    /// are given, otherwise by name (and set).
    ///
    /// When Scryfall cannot be reached and the card was never fetched, bulk
    /// data already on disk is searched: every printing first, then the
    /// one-per-card file.
    pub fn card(&mut self, name: &str, set: Option<&str>, number: Option<&str>) -> Result<Value, Error> {
        let (set, number) = (set.filter(|s| !s.is_empty()), number.filter(|n| !n.is_empty()));
        let online = match (set, number) {
            (Some(set), Some(number)) => self.card_by_number(set, number),
            (set, _) => self.card_by_name(name, set),
        };
        let Err(Error::Offline(why)) = online else { return online };
        [BulkKind::DefaultCards, BulkKind::OracleCards]
            .into_iter()
            .filter_map(|kind| bulk::on_disk(&self.cache, kind))
            .find_map(|cards| match (set, number) {
                (Some(set), Some(number)) => cards.card_by_number(set, number),
                (set, _) => cards.card_by_name(name, set),
            })
            .ok_or(Error::Offline(why))
    }

    /// The card an art file's name asks for, ready for the card engine: looked
    /// up, its layout settled (fetching the other cards of a meld pair when
    /// needed) and its fields read. Everything fetched is cached, so the same
    /// call works offline afterwards.
    pub fn card_data(&mut self, info: &CardInfo) -> Result<CardData, Error> {
        let (data, name) = self.card_settled(info)?;
        Ok(card_data(&data, &CardInput { name, artist: info.artist.clone(), creator: info.creator.clone() }))
    }

    /// The same lookup, stopping before the fields are read: Scryfall's JSON
    /// with its layout settled, and the name of the face that is wanted. An
    /// editor keeps this, changes fields in it, and reads the card from it
    /// again with [`cardsmith_cards::layout::card_data`].
    pub fn card_settled(&mut self, info: &CardInfo) -> Result<(Value, String), Error> {
        let mut data = self.card(&info.name, Some(&info.set), Some(&info.number))?;
        // Looked up by number, the name may be missing: the card's own name says which face is meant.
        let name = if info.name.is_empty() { data["name"].as_str().unwrap_or("").split(" // ").next().unwrap_or("").to_string() } else { info.name.clone() };
        let mut failure = None;
        process_card_data(&mut data, &name, &mut |part| match part["uri"].as_str().map(|uri| self.get_json(uri)) {
            Some(Ok(face)) => Some(face),
            Some(Err(e)) => {
                failure = Some(e);
                None
            }
            None => None,
        });
        if let Some(e) = failure {
            return Err(e);
        }
        Ok((data, name))
    }

    /// Download one of Scryfall's bulk files (or reuse the copy on disk if it
    /// is less than a week old) and open it for lookups by name.
    pub fn bulk(&mut self, kind: BulkKind) -> Result<BulkCards, Error> {
        bulk::fetch(&mut self.transport, &self.cache, kind)
    }
}
