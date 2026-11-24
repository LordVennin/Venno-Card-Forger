//! The core's WebAssembly face.
//!
//! A plain C ABI, with no binding generator: the build is `cargo build
//! --target wasm32-unknown-unknown` and nothing else. The module holds one
//! session: a document with its history, pixels and on-screen view.
//!
//! Conventions:
//!   - The host passes bytes by allocating with `cs_alloc`, writing into
//!     linear memory, and passing `(ptr, len)`. It frees them with `cs_free`.
//!   - Calls that can fail return 1 on success and 0 on failure; the message
//!     is then in the output buffer (`cs_out_ptr`, `cs_out_len`).
//!   - Calls that return text (JSON) or a file put it in the same output buffer.
//!   - Pixels stay in linear memory. The host draws the view (`cs_view_ptr`)
//!     with `putImageData` on a view of that memory, limited to the dirty
//!     rectangle the last call reported. Pixels never cross a process boundary.

use cardsmith_core::commands::Edit;
use cardsmith_core::doc::Doc;
use cardsmith_core::paint::Brush;
use cardsmith_core::raster::Region;
use cardsmith_core::samples;
use cardsmith_core::serialize::{load_cardsmith, parse_doc, AssetBytes};
use cardsmith_core::session::{Session, View};
use cardsmith_engine::workbench::{BuildRequest, Workbench};
use cardsmith_engine::Template;
use std::cell::RefCell;

struct State {
    session: Session,
    /// Present while the open document is a template a card is being built on.
    bench: Option<Workbench>,
    /// Assets supplied after `cs_open_json`, until the session is (re)built with them.
    out: Vec<u8>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State { session: Session::new(Doc::empty(1, 1, 72.0), AssetBytes::new()), bench: None, out: Vec::new() });
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

/// Run a fallible call: on error, store the message and return 0.
fn attempt(f: impl FnOnce(&mut State) -> Result<(), String>) -> u32 {
    with(|s| match f(s) {
        Ok(()) => 1,
        Err(message) => {
            s.out = message.into_bytes();
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

/// Replace the session, keeping the loaded fonts, and the current view size so the screen stays in step.
fn open(s: &mut State, doc: Doc, assets: AssetBytes) {
    let view = s.session.view();
    // Fonts belong to the app, not to a document: they carry over.
    let fonts = s.session.fonts().clone();
    s.session = Session::with_fonts(doc, assets, fonts);
    s.bench = None;
    s.session.set_view(View { scale: 1.0, x: 0.0, y: 0.0, ..view });
}

// ---- memory and output ---------------------------------------------------------

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
pub extern "C" fn cs_out_ptr() -> *const u8 {
    with(|s| s.out.as_ptr())
}

#[no_mangle]
pub extern "C" fn cs_out_len() -> usize {
    with(|s| s.out.len())
}

// ---- opening and saving ----------------------------------------------------------

/// Names of the built-in sample documents, as a JSON array in the output buffer.
#[no_mangle]
pub extern "C" fn cs_sample_names() {
    let mut names: Vec<&str> = samples::samples().iter().map(|s| s.name).collect();
    names.push("card");
    with(|s| s.out = serde_json::to_vec(&names).unwrap_or_default());
}

#[no_mangle]
pub extern "C" fn cs_open_sample(index: u32) -> u32 {
    attempt(|s| {
        let mut all = samples::samples();
        let sample = if index as usize == all.len() { samples::card() } else if (index as usize) < all.len() { all.swap_remove(index as usize) } else { return Err("No such sample".into()) };
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

/// Open a document from JSON together with its assets, packed as repeated
/// records of `[id length: u32 LE][id][data length: u32 LE][data]`.
///
/// # Safety
/// Both `(ptr, len)` pairs must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_open_json(json_ptr: *const u8, json_len: usize, assets_ptr: *const u8, assets_len: usize) -> u32 {
    let json = text(json_ptr, json_len);
    let packed = bytes(assets_ptr, assets_len);
    attempt(|s| {
        let doc = parse_doc(json?).map_err(|e| e.to_string())?;
        let mut assets = AssetBytes::new();
        let mut rest = packed;
        let mut take = |n: usize| -> Result<&[u8], String> {
            if rest.len() < n {
                return Err("The packed assets are cut short".into());
            }
            let (part, tail) = rest.split_at(n);
            rest = tail;
            Ok(part)
        };
        let mut read = packed.len();
        while read > 0 {
            let id_len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
            let id = String::from_utf8(take(id_len)?.to_vec()).map_err(|_| "An asset id was not UTF-8")?;
            let data_len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
            assets.insert(id, take(data_len)?.to_vec());
            read = read.saturating_sub(8 + id_len + data_len);
        }
        open(s, doc, assets);
        Ok(())
    })
}

/// The document as `.cardsmith` bytes, in the output buffer.
#[no_mangle]
pub extern "C" fn cs_save() -> u32 {
    attempt(|s| {
        s.out = s.session.save().map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ---- cards -----------------------------------------------------------------------

/// Open a template package to build a card on: its `doc.json` and
/// `manifest.json`. Its images are not needed yet; see `cs_missing_assets`.
///
/// # Safety
/// Both `(ptr, len)` pairs must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_open_template(doc_ptr: *const u8, doc_len: usize, manifest_ptr: *const u8, manifest_len: usize) -> u32 {
    let doc = text(doc_ptr, doc_len);
    let manifest = text(manifest_ptr, manifest_len);
    attempt(|s| {
        let bench = Workbench::new(Template::from_json(doc?, manifest?)?);
        open(s, bench.doc(), AssetBytes::new());
        s.bench = Some(bench);
        Ok(())
    })
}

/// The assets that showing layers need and that have not been supplied, as
/// JSON in the output buffer: `[{ "id", "path" }]`.
#[no_mangle]
pub extern "C" fn cs_missing_assets() {
    with(|s| {
        let missing: Vec<_> = s.session.missing_assets().into_iter().map(|(id, path)| serde_json::json!({ "id": id, "path": path })).collect();
        s.out = serde_json::to_vec(&missing).unwrap_or_default();
    });
}

/// Supply an asset's bytes. With `redraw` 0 nothing is drawn, for supplying
/// several before one redraw (the last call passes 1).
///
/// # Safety
/// Both `(ptr, len)` pairs must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_add_asset(id_ptr: *const u8, id_len: usize, ptr: *const u8, len: usize, redraw: u32) -> u32 {
    let id = text(id_ptr, id_len);
    let data = bytes(ptr, len).to_vec();
    attempt(|s| {
        if redraw == 0 {
            s.session.stash_asset(id?, data);
            Ok(())
        } else {
            s.session.add_asset(id?, data).map_err(|e| e.to_string())
        }
    })
}

/// Use a picture (PNG, JPEG or WebP bytes) as the card's art from the next
/// build on. Its size is in the output buffer: `{ "width", "height" }`.
///
/// # Safety
/// `(ptr, len)` must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_set_art(ptr: *const u8, len: usize) -> u32 {
    let data = bytes(ptr, len).to_vec();
    attempt(|s| {
        let bench = s.bench.as_mut().ok_or("Open a template first")?;
        let (width, height) = bench.set_art(&mut s.session, data)?;
        s.out = serde_json::to_vec(&serde_json::json!({ "width": width, "height": height })).unwrap_or_default();
        Ok(())
    })
}

/// Build the card (a `BuildRequest` as JSON) into the document as one
/// undoable step. The report is in the output buffer: `{ "build", "warnings", "fill" }`.
///
/// # Safety
/// `(ptr, len)` must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_build_card(ptr: *const u8, len: usize) -> u32 {
    let request = text(ptr, len);
    attempt(|s| {
        let request: BuildRequest = serde_json::from_str(request?).map_err(|e| format!("Invalid card: {e}"))?;
        let bench = s.bench.as_mut().ok_or("Open a template first")?;
        let report = bench.build(&mut s.session, &request)?;
        s.out = serde_json::to_vec(&report).map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// Add a finished fill (PNG bytes) made for build number `build`. The output
/// buffer says `true`, or `false` when that card is no longer showing and
/// nothing was added.
///
/// # Safety
/// All three `(ptr, len)` pairs must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_add_fill(build: u32, ptr: *const u8, len: usize, source_ptr: *const u8, source_len: usize, params_ptr: *const u8, params_len: usize) -> u32 {
    let png = bytes(ptr, len).to_vec();
    let source = text(source_ptr, source_len);
    let params = text(params_ptr, params_len);
    attempt(|s| {
        let params = serde_json::from_str(params?).map_err(|e| format!("Invalid fill parameters: {e}"))?;
        let bench = s.bench.as_mut().ok_or("Open a template first")?;
        let added = bench.add_fill(&mut s.session, build as u64, png, source?, params)?;
        s.out = added.to_string().into_bytes();
        Ok(())
    })
}

// ---- state -----------------------------------------------------------------------

/// The document as JSON, in the output buffer.
#[no_mangle]
pub extern "C" fn cs_doc_json() -> u32 {
    attempt(|s| {
        s.out = serde_json::to_vec(s.session.doc()).map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// History state and render warnings as JSON, in the output buffer:
/// `{ "canUndo", "canRedo", "labels", "warnings" }`.
#[no_mangle]
pub extern "C" fn cs_state_json() {
    with(|s| {
        let state = serde_json::json!({
            "canUndo": s.session.can_undo(),
            "canRedo": s.session.can_redo(),
            "labels": s.session.labels(),
            "warnings": s.session.warnings(),
        });
        s.out = serde_json::to_vec(&state).unwrap_or_default();
    });
}

// ---- edits -----------------------------------------------------------------------

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
        s.session.execute(edit, label?).map_err(|e| e.to_string())
    })
}

#[no_mangle]
pub extern "C" fn cs_undo() -> u32 {
    with(|s| s.session.undo() as u32)
}

#[no_mangle]
pub extern "C" fn cs_redo() -> u32 {
    with(|s| s.session.redo() as u32)
}

/// Load a font file (TTF or OTF bytes) for text layers to use. The PostScript
/// names it provides go to the output buffer as a JSON array.
///
/// # Safety
/// `(ptr, len)` must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_add_font(ptr: *const u8, len: usize) -> u32 {
    let data = bytes(ptr, len).to_vec();
    attempt(|s| {
        let names = s.session.add_font(data).map_err(|e| e.to_string())?;
        s.out = serde_json::to_vec(&names).unwrap_or_default();
        Ok(())
    })
}

// ---- painting --------------------------------------------------------------------

/// Add an empty, document-sized pixel layer on top. Its id goes to the output buffer.
///
/// # Safety
/// `(ptr, len)` must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_add_paint_layer(name_ptr: *const u8, name_len: usize) -> u32 {
    let name = text(name_ptr, name_len);
    attempt(|s| {
        s.out = s.session.add_paint_layer(name?).map_err(|e| e.to_string())?.into_bytes();
        Ok(())
    })
}

/// Start a stroke on a pixel layer. The brush is JSON (see `cardsmith_core::paint::Brush`).
///
/// # Safety
/// Both `(ptr, len)` pairs must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_stroke_begin(layer_ptr: *const u8, layer_len: usize, brush_ptr: *const u8, brush_len: usize) -> u32 {
    let layer = text(layer_ptr, layer_len);
    let brush = text(brush_ptr, brush_len);
    attempt(|s| {
        let brush: Brush = serde_json::from_str(brush?).map_err(|e| format!("Invalid brush: {e}"))?;
        s.session.stroke_begin(layer?, brush).map_err(|e| e.to_string())
    })
}

/// Extend the stroke to a document position. The view is updated; read the dirty rectangle afterwards.
#[no_mangle]
pub extern "C" fn cs_stroke_to(x: f32, y: f32, pressure: f32) {
    with(|s| s.session.stroke_to(x, y, pressure));
}

/// Finish the stroke and record it for undo.
///
/// # Safety
/// `(ptr, len)` must be valid as described in the module docs.
#[no_mangle]
pub unsafe extern "C" fn cs_stroke_end(label_ptr: *const u8, label_len: usize) {
    let label = text(label_ptr, label_len).unwrap_or("Brush");
    with(|s| s.session.stroke_end(label));
}

// ---- pixels ----------------------------------------------------------------------

/// Set what part of the document is on screen: `scale` screen pixels per
/// document pixel, document point (x, y) at the top-left, `w × h` screen pixels.
#[no_mangle]
pub extern "C" fn cs_set_view(scale: f32, x: f32, y: f32, w: u32, h: u32) {
    with(|s| s.session.set_view(View { scale, x, y, w, h }));
}

/// The view: straight-alpha RGBA, `w × h × 4` bytes as given to `cs_set_view`.
/// Valid until the next call that changes the document or the view.
#[no_mangle]
pub extern "C" fn cs_view_ptr() -> *const u8 {
    with(|s| s.session.view_pixels().as_ptr())
}

fn dirty() -> Region {
    with(|s| s.session.dirty_view())
}

/// The part of the view the last call redrew, in view pixels. Width 0 means nothing changed.
#[no_mangle]
pub extern "C" fn cs_dirty_x() -> i32 {
    dirty().x
}
#[no_mangle]
pub extern "C" fn cs_dirty_y() -> i32 {
    dirty().y
}
#[no_mangle]
pub extern "C" fn cs_dirty_w() -> i32 {
    dirty().w
}
#[no_mangle]
pub extern "C" fn cs_dirty_h() -> i32 {
    dirty().h
}

/// The full-resolution frame: straight-alpha RGBA, document size. For export and checks.
#[no_mangle]
pub extern "C" fn cs_frame_ptr() -> *const u8 {
    with(|s| s.session.frame().as_ptr())
}

/// FNV-1a hash of the full-resolution frame, as two 32-bit halves. Lets native
/// and WASM renders be compared without moving the pixels anywhere.
#[no_mangle]
pub extern "C" fn cs_frame_hash_hi() -> u32 {
    with(|s| (s.session.frame_hash() >> 32) as u32)
}

#[no_mangle]
pub extern "C" fn cs_frame_hash_lo() -> u32 {
    with(|s| s.session.frame_hash() as u32)
}
