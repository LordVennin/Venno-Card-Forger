//! Scryfall's bulk data: every card in one file, for lookups without the network.
//!
//! The download is hundreds of megabytes once unpacked: either one card per
//! line, gzipped (what Scryfall offers now), or a single JSON array (what it
//! offered before). Either is read once, as a stream, and rewritten as one
//! card per line beside an index of names, so a lookup reads the index and
//! then one line.

use crate::{is_playable, Error, FetchError, Transport, API, FRESH};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Which bulk file. See <https://scryfall.com/docs/api/bulk-data>.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkKind {
    /// One printing of every card: small, enough to look cards up by name.
    OracleCards,
    /// Every printing in English: large, needed to pick a particular set.
    DefaultCards,
}

impl BulkKind {
    fn slug(self) -> &'static str {
        match self {
            BulkKind::OracleCards => "oracle-cards",
            BulkKind::DefaultCards => "default-cards",
        }
    }
}

/// Writes cards to the lines file and the index as they are read.
struct Rewrite<'a> {
    lines: &'a mut BufWriter<File>,
    index: &'a mut BufWriter<File>,
    offset: u64,
    count: usize,
}

impl Rewrite<'_> {
    fn card(&mut self, card: &Value) -> std::io::Result<()> {
        let line = serde_json::to_vec(card)?;
        let text = |key: &str| card[key].as_str().unwrap_or("").replace(['\t', '\n'], " ");
        // name, set, collector number, release date, whether it is a game piece, where its line is.
        writeln!(self.index, "{}\t{}\t{}\t{}\t{}\t{}\t{}", text("name"), text("set"), text("collector_number"), text("released_at"), u8::from(is_playable(card)), self.offset, line.len())?;
        self.lines.write_all(&line)?;
        self.lines.write_all(b"\n")?;
        self.offset += line.len() as u64 + 1;
        self.count += 1;
        Ok(())
    }
}

impl<'de> serde::de::Visitor<'de> for &mut Rewrite<'_> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a list of cards")
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        use serde::de::Error as _;
        while let Some(card) = seq.next_element::<Value>()? {
            self.card(&card).map_err(A::Error::custom)?;
        }
        Ok(())
    }
}

/// How a bulk download is laid out.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Format {
    /// One JSON array of cards.
    Array,
    /// One card per line.
    Lines,
}

/// Turn a bulk download into the lines file and its index. Returns how many cards it held.
pub(crate) fn rewrite(source: impl Read, format: Format, lines: &Path, index: &Path) -> Result<usize, Error> {
    let io = |path: &Path| { let p = path.display().to_string(); move |e: std::io::Error| Error::Other(format!("{p}: {e}")) };
    let mut lines_out = BufWriter::new(File::create(lines).map_err(io(lines))?);
    let mut index_out = BufWriter::new(File::create(index).map_err(io(index))?);
    let mut rewrite = Rewrite { lines: &mut lines_out, index: &mut index_out, offset: 0, count: 0 };
    let bad = |e: String| Error::Other(format!("The bulk file is not a list of cards: {e}"));
    match format {
        Format::Array => {
            let mut reader = serde_json::Deserializer::from_reader(BufReader::new(source));
            serde::Deserializer::deserialize_seq(&mut reader, &mut rewrite).map_err(|e| bad(e.to_string()))?;
        }
        Format::Lines => {
            for line in BufReader::new(source).lines() {
                let line = line.map_err(|e| bad(e.to_string()))?;
                if line.trim().is_empty() {
                    continue;
                }
                let card: Value = serde_json::from_str(&line).map_err(|e| bad(e.to_string()))?;
                rewrite.card(&card).map_err(|e| bad(e.to_string()))?;
            }
        }
    }
    let count = rewrite.count;
    lines_out.flush().map_err(io(lines))?;
    index_out.flush().map_err(io(index))?;
    Ok(count)
}

struct Entry {
    /// The name is the card's whole name, not just one of its faces.
    whole: bool,
    set: String,
    number: String,
    released: String,
    playable: bool,
    offset: u64,
    len: usize,
}

/// A bulk file on disk, opened for lookups.
pub struct BulkCards {
    lines: PathBuf,
    /// Lower-cased card name (and each face's name) to its printings.
    names: HashMap<String, Vec<Entry>>,
    count: usize,
}

impl BulkCards {
    /// Open a rewritten bulk file by its two paths.
    pub fn open(lines: &Path, index: &Path) -> Result<BulkCards, Error> {
        let file = File::open(index).map_err(|e| Error::Other(format!("{}: {e}", index.display())))?;
        let mut names: HashMap<String, Vec<Entry>> = HashMap::new();
        let mut count = 0;
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|e| Error::Other(format!("{}: {e}", index.display())))?;
            let parts: Vec<&str> = line.split('\t').collect();
            let [name, set, number, released, playable, offset, len] = parts[..] else { continue };
            let (Ok(offset), Ok(len)) = (offset.parse(), len.parse()) else { continue };
            count += 1;
            // "Front // Back" is found by its whole name and by either face.
            let mut keys = vec![name.to_lowercase()];
            if name.contains(" // ") {
                keys.extend(name.split(" // ").map(str::to_lowercase));
            }
            for (i, key) in keys.into_iter().enumerate() {
                names.entry(key).or_default().push(Entry { whole: i == 0, set: set.into(), number: number.into(), released: released.into(), playable: playable == "1", offset, len });
            }
        }
        Ok(BulkCards { lines: lines.to_path_buf(), names, count })
    }

    /// How many cards the file holds.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn read(&self, entry: &Entry) -> Option<Value> {
        let mut file = File::open(&self.lines).ok()?;
        file.seek(SeekFrom::Start(entry.offset)).ok()?;
        let mut line = vec![0; entry.len];
        file.read_exact(&mut line).ok()?;
        serde_json::from_slice(&line).ok()
    }

    /// The newest playable printing of a card by exact name (case does not
    /// matter), in the given set if one is named.
    pub fn card_by_name(&self, name: &str, set: Option<&str>) -> Option<Value> {
        let set = set.map(str::to_lowercase);
        let entries = self.names.get(&name.to_lowercase())?;
        // The card itself before a card that has it as a face; then the newest.
        let best = entries.iter().filter(|e| e.playable && set.as_ref().is_none_or(|s| &e.set == s)).max_by(|a, b| (a.whole, &a.released).cmp(&(b.whole, &b.released)))?;
        self.read(best)
    }

    /// One exact printing, by set code and collector number.
    pub fn card_by_number(&self, set: &str, number: &str) -> Option<Value> {
        let set = set.to_lowercase();
        let trimmed = number.trim().trim_start_matches('0');
        let number = if trimmed.is_empty() { "0" } else { trimmed };
        let entry = self.names.values().flatten().find(|e| e.set == set && e.number == number)?;
        self.read(entry)
    }
}

/// Open a bulk file that is already on disk, however old, without touching the network.
pub(crate) fn on_disk(cache: &Path, kind: BulkKind) -> Option<BulkCards> {
    let dir = cache.join("bulk");
    BulkCards::open(&dir.join(format!("{}.jsonl", kind.slug())), &dir.join(format!("{}.index", kind.slug()))).ok()
}

/// Make sure a recent copy of the bulk file is on disk, and open it. With no
/// network, whatever copy is there is used.
pub(crate) fn fetch(transport: &mut impl Transport, cache: &Path, kind: BulkKind) -> Result<BulkCards, Error> {
    let dir = cache.join("bulk");
    std::fs::create_dir_all(&dir).map_err(|e| Error::Other(format!("{}: {e}", dir.display())))?;
    let lines = dir.join(format!("{}.jsonl", kind.slug()));
    let index = dir.join(format!("{}.index", kind.slug()));
    let have = lines.exists() && index.exists();
    let fresh = have && std::fs::metadata(&index).and_then(|m| m.modified()).ok().and_then(|at| SystemTime::now().duration_since(at).ok()).is_some_and(|age| age < FRESH);
    if fresh {
        return BulkCards::open(&lines, &index);
    }

    // Ask where today's file is, download it, and rewrite it for lookups.
    let download = dir.join(format!("{}.download", kind.slug()));
    let fetched = (|| {
        let info = transport.get(&format!("{API}/bulk-data/{}", kind.slug()))?;
        let info: Value = serde_json::from_slice(&info).map_err(|e| FetchError::Other(format!("bulk data listing is not JSON: {e}")))?;
        // One card per line where offered; the single array otherwise.
        let uri = info["jsonl_download_uri"].as_str().or(info["download_uri"].as_str()).ok_or_else(|| FetchError::Other("bulk data listing has no download address".into()))?;
        transport.download(uri, &download)?;
        Ok(uri.to_string())
    })();
    match fetched {
        Ok(uri) => {
            let file = File::open(&download).map_err(|e| Error::Other(format!("{}: {e}", download.display())))?;
            let name = uri.split('?').next().unwrap_or(&uri);
            let (packed, name) = match name.strip_suffix(".gz") {
                Some(inner) => (true, inner),
                None => (false, name),
            };
            let format = if name.ends_with(".jsonl") { Format::Lines } else { Format::Array };
            let source: Box<dyn Read> = if packed { Box::new(flate2::read::MultiGzDecoder::new(file)) } else { Box::new(file) };
            // Rewritten under temporary names, so a failure leaves the old copy usable.
            let (new_lines, new_index) = (lines.with_extension("jsonl.new"), index.with_extension("index.new"));
            rewrite(source, format, &new_lines, &new_index)?;
            let io = |e: std::io::Error| Error::Other(format!("{}: {e}", dir.display()));
            std::fs::rename(&new_lines, &lines).map_err(io)?;
            std::fs::rename(&new_index, &index).map_err(io)?;
            let _ = std::fs::remove_file(&download);
            BulkCards::open(&lines, &index)
        }
        Err(_) if have => BulkCards::open(&lines, &index),
        Err(FetchError::Unreachable(why)) => Err(Error::Offline(why)),
        Err(FetchError::NotFound(_)) => Err(Error::Other("Scryfall has no such bulk file".into())),
        Err(FetchError::Other(why)) => Err(Error::Other(why)),
    }
}
