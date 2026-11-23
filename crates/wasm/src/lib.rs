//! The core's WebAssembly face.
//!
//! A plain C ABI, with no binding generator: the build is `cargo build
//! --target wasm32-unknown-unknown` and nothing else. The module holds one
//! session (a document with its history, assets and last rendered frame).
//!
//! Conventions:
//!   - The host passes bytes by allocating with `cs_alloc`, writing into
//!     linear memory, and passing `(ptr, len)`. It frees them with `cs_free`.
//!   - Calls that can fail return 1 on success and 0 on failure; the message
//!     is then in the text buffer (`cs_text_ptr`, `cs_text_len`).
//!   - Calls that return text (JSON) put it in the same text buffer.
//!   - The rendered frame stays in linear memory (`cs_frame_ptr`). The host
//!     draws it with `putImageData` on a view of that memory: pixels are never
//!     copied across a process boundary.

use cardsmith_core::commands::{Edit, History};
use cardsmith_core::doc::Doc;
use cardsmith_core::render::render;
use cardsmith_core::samples::samples;
use cardsmith_core::serialize::{load_cardsmith, parse_doc, AssetBytes};
use std::cell::RefCell;

struct Session {
    history: History,
    assets: AssetBytes,
    frame: Vec<u8>,
    frame_size: (u32, u32),
    warnings: Vec<String>,
    text: Vec<u8>,
}

thread_local! {
    static SESSION: RefCell<Session> = RefCell::new(Session {
        history: History::new(Doc::empty(1, 1, 72.0)),
        assets: AssetBytes::new(),
        frame: Vec::new(),
        frame_size: (0, 0),
        warnings: Vec::new(),
        text: Vec::new(),
    });
}

fn with<T>(f: impl FnOnce(&mut Session) -> T) -> T {
    SESSION.with(|s| f(&mut s.borrow_mut()))
}

/// Run a fallible call: on error, store the message and return 0.
fn attempt(f: impl FnOnce(&mut Session) -> Result<(), String>) -> u32 {
    with(|s| match f(s) {
        Ok(()) => 1,
        Err(message) => {
            s.text = message.into_bytes();
            0
        }
    })
}

/// # Safety
/// `ptr` must point to `len` readable bytes, as allocated by `cs_alloc`.
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

/// # Safety
/// As for `bytes`.
unsafe fn text<'a>(ptr: *const u8, len: usize) -> Result<&'a str, String> {
    std::str::from_utf8(bytes(ptr, len)).map_err(|_| "Text passed to the core was not UTF-8".to_string())
}

fn open(s: &mut Session, doc: Doc, assets: AssetBytes) {
    s.history = History::new(doc);
    s.assets = assets;
    s.frame.clear();
    s.frame_size = (0, 0);
    s.warnings.clear();
}

#[no_mangle]
pub extern "C" fn cs_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// # Safety
/// `ptr` and `len` must come from one `cs_alloc(len)` call, freed once.
#[no_mangle]
pub unsafe extern "C" fn cs_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

#[no_mangle]
pub extern "C" fn cs_text_ptr() -> *const u8 {
    with(|s| s.text.as_ptr())
}

#[no_mangle]
pub extern "C" fn cs_text_len() -> usize {
    with(|s| s.text.len())
}

/// Number of built-in sample documents; their names as a JSON array go to the text buffer.
#[no_mangle]
pub extern "C" fn cs_sample_names() -> u32 {
    let names: Vec<&str> = samples().iter().map(|s| s.name).collect();
    with(|s| s.text = serde_json::to_vec(&names).unwrap_or_default());
    names.len() as u32
}

#[no_mangle]
pub extern "C" fn cs_open_sample(index: u32) -> u32 {
    attempt(|s| {
        let sample = samples().into_iter().nth(index as usize).ok_or("No such sample")?;
        open(s, sample.doc, sample.assets);
        Ok(())
    })
}

/// Open a `.cardsmith` file (zip bytes).
///
/// # Safety
/// `(ptr, len)` must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_open_cardsmith(ptr: *const u8, len: usize) -> u32 {
    let data = bytes(ptr, len);
    attempt(|s| {
        let (doc, assets) = load_cardsmith(data).map_err(|e| e.to_string())?;
        open(s, doc, assets);
        Ok(())
    })
}

/// Open a bare document from JSON. Its assets are supplied afterwards with `cs_set_asset_data`.
///
/// # Safety
/// `(ptr, len)` must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_open_json(ptr: *const u8, len: usize) -> u32 {
    let json = text(ptr, len);
    attempt(|s| {
        let doc = parse_doc(json?).map_err(|e| e.to_string())?;
        open(s, doc, AssetBytes::new());
        Ok(())
    })
}

/// Supply the bytes for one asset id.
///
/// # Safety
/// Both `(ptr, len)` pairs must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_set_asset_data(id_ptr: *const u8, id_len: usize, ptr: *const u8, len: usize) -> u32 {
    let id = text(id_ptr, id_len);
    let data = bytes(ptr, len).to_vec();
    attempt(|s| {
        s.assets.insert(id?.to_string(), data);
        Ok(())
    })
}

/// Apply an edit given as JSON (see `cardsmith_core::commands::Edit`).
///
/// # Safety
/// Both `(ptr, len)` pairs must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_execute(edit_ptr: *const u8, edit_len: usize, label_ptr: *const u8, label_len: usize) -> u32 {
    let edit = text(edit_ptr, edit_len);
    let label = text(label_ptr, label_len);
    attempt(|s| {
        let edit: Edit = serde_json::from_str(edit?).map_err(|e| format!("Invalid edit: {e}"))?;
        s.history.execute(edit, label?).map_err(|e| e.to_string())
    })
}

#[no_mangle]
pub extern "C" fn cs_undo() -> u32 {
    with(|s| s.history.undo() as u32)
}

#[no_mangle]
pub extern "C" fn cs_redo() -> u32 {
    with(|s| s.history.redo() as u32)
}

/// The document as JSON, into the text buffer.
#[no_mangle]
pub extern "C" fn cs_doc_json() -> u32 {
    attempt(|s| {
        s.text = serde_json::to_vec(s.history.doc()).map_err(|e| e.to_string())?;
        Ok(())
    })
    // On success the text buffer holds the document, which is what the caller reads.
}

/// History state and render warnings as JSON, into the text buffer:
/// `{ "canUndo", "canRedo", "labels", "warnings" }`.
#[no_mangle]
pub extern "C" fn cs_state_json() {
    with(|s| {
        let state = serde_json::json!({
            "canUndo": s.history.can_undo(),
            "canRedo": s.history.can_redo(),
            "labels": s.history.labels(),
            "warnings": s.warnings,
        });
        s.text = serde_json::to_vec(&state).unwrap_or_default();
    });
}

/// Render the whole document into the frame buffer.
#[no_mangle]
pub extern "C" fn cs_render() {
    with(|s| {
        let result = render(s.history.doc(), &s.assets);
        s.frame = result.pixels;
        s.frame_size = (result.width, result.height);
        s.warnings = result.warnings;
    });
}

/// Straight-alpha RGBA, `cs_frame_width() × cs_frame_height() × 4` bytes.
/// Valid until the next call that renders or opens a document.
#[no_mangle]
pub extern "C" fn cs_frame_ptr() -> *const u8 {
    with(|s| s.frame.as_ptr())
}

#[no_mangle]
pub extern "C" fn cs_frame_width() -> u32 {
    with(|s| s.frame_size.0)
}

#[no_mangle]
pub extern "C" fn cs_frame_height() -> u32 {
    with(|s| s.frame_size.1)
}

/// FNV-1a hash of the frame, as two 32-bit halves. Lets native and WASM
/// renders be compared without moving the pixels anywhere.
#[no_mangle]
pub extern "C" fn cs_frame_hash_hi() -> u32 {
    with(|s| (cardsmith_core::render::hash(&s.frame) >> 32) as u32)
}

#[no_mangle]
pub extern "C" fn cs_frame_hash_lo() -> u32 {
    with(|s| cardsmith_core::render::hash(&s.frame) as u32)
}
