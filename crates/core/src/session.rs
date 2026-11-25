//! An open document: the model, its undo history, its pixels and what is on screen.
//!
//! The session is the one place that mutates a document. Every change, whether
//! a structural edit or a brush stroke, goes through it, is undoable, and
//! leaves the rendered frame up to date by redrawing only what changed.

use crate::text::Fonts;
use crate::commands::{apply, Edit};
use crate::doc::build::pixel;
use crate::doc::{AssetKind, AssetRef, Doc, GroupLayer, Layer};
use crate::effects::effect_extent;
use crate::paint::{crop, paste, Brush, Stroke};
use crate::raster::{Image, Raster, Region};
use crate::render::{can_split_at, encode_png, hash, Renderer, Split};
use crate::serialize::{save_cardsmith, AssetBytes};
use crate::{err, Result};
use crate::proxy::{downscale, scale_doc, TextOutlines};
use std::collections::{BTreeMap, BTreeSet, HashMap};

enum Step {
    Edit { label: String, edit: Edit, inverse: Edit },
    /// A brush stroke: the blocks of one image it touched, each as it was before and after.
    Paint { label: String, asset: String, offset: (i32, i32), patches: Vec<(Region, Vec<u8>, Vec<u8>)>, margin: i32 },
}

impl Step {
    fn label(&self) -> &str {
        match self {
            Step::Edit { label, .. } | Step::Paint { label, .. } => label,
        }
    }
}

struct ActiveStroke {
    stroke: Stroke,
    asset: String,
    /// Document position of the image's top-left pixel.
    offset: (i32, i32),
    /// How far beyond the painted pixels the document can change (layer effects).
    margin: i32,
}

/// The flattened layers beneath the layer being painted, kept per tile so a
/// stroke does not recomposite them on every move.
struct Below {
    layer: String,
    ancestors: Vec<String>,
    tiles: std::collections::HashMap<(i32, i32), Raster>,
}

/// Side of a `Below` tile in pixels.
const BELOW_TILE: i32 = 256;

/// What an edit can change on screen.
enum Reach {
    Everything,
    Layers(Vec<String>),
}

impl Reach {
    fn of(edit: &Edit) -> Reach {
        match edit {
            Edit::SetLayerProps { id, .. } | Edit::RemoveLayer { id } | Edit::MoveLayer { id, .. } => Reach::Layers(vec![id.clone()]),
            Edit::AddLayer { layer, .. } => Reach::Layers(vec![layer.id().to_string()]),
            // The card's data and the template's name are not drawn; the document's size and resolution are.
            Edit::SetDocProps { props } if props.keys().all(|key| key == "card" || key == "templateId") => Reach::Layers(Vec::new()),
            Edit::SetAsset { .. } | Edit::SetDocProps { .. } => Reach::Everything,
            Edit::Batch { edits } => {
                let mut all = Vec::new();
                for edit in edits {
                    match Reach::of(edit) {
                        Reach::Everything => return Reach::Everything,
                        Reach::Layers(ids) => all.extend(ids),
                    }
                }
                Reach::Layers(all)
            }
        }
    }
}

/// The layers that differ between two versions of a document: changed in
/// themselves, added, removed, moved to another group, or reordered. A group
/// is only listed for its own properties; what changed inside it is listed
/// separately. This is what a batch of edits changed in the end, however many
/// layers it touched on the way: rebuilding a card sets every text layer
/// again, and most come out as they were.
fn changed_layers(before: &Doc, after: &Doc) -> Vec<String> {
    type Seen<'a> = BTreeMap<&'a str, (&'a str, u64)>;
    fn note<'a>(group: &'a GroupLayer, layers: &mut Seen<'a>, order: &mut BTreeMap<&'a str, Vec<&'a str>>) {
        for layer in &group.children {
            // A layer's own fields only: a group's children are looked at in their own right.
            let own = match layer {
                Layer::Group(g) => serde_json::to_vec(&(&g.base, g.pass_through)),
                other => serde_json::to_vec(other),
            };
            layers.insert(layer.id(), (group.base.id.as_str(), hash(&own.unwrap_or_default())));
            order.entry(group.base.id.as_str()).or_default().push(layer.id());
            if let Layer::Group(inner) = layer {
                note(inner, layers, order);
            }
        }
    }
    let (mut old, mut new, mut old_order, mut new_order) = (Seen::new(), Seen::new(), BTreeMap::new(), BTreeMap::new());
    note(&before.root, &mut old, &mut old_order);
    note(&after.root, &mut new, &mut new_order);

    let mut changed: BTreeSet<&str> = BTreeSet::new();
    changed.extend(old.iter().filter(|(id, seen)| new.get(*id) != Some(seen)).map(|(id, _)| *id));
    changed.extend(new.keys().filter(|id| !old.contains_key(*id)));
    // Layers that stayed in a group but swapped places among themselves.
    for (group, was) in &old_order {
        let Some(now) = new_order.get(group) else { continue };
        let stayed_was: Vec<&str> = was.iter().copied().filter(|id| now.contains(id)).collect();
        let stayed_now: Vec<&str> = now.iter().copied().filter(|id| was.contains(id)).collect();
        changed.extend(stayed_was.iter().zip(&stayed_now).filter(|(a, b)| a != b).flat_map(|(a, b)| [*a, *b]));
    }
    // A layer added, removed or changed under a clipping layer changes what that one shows too:
    // clipped layers are drawn through the layer below them, so their own reach covers it.
    changed.into_iter().map(str::to_string).collect()
}

fn union(a: Option<Region>, b: Option<Region>) -> Option<Region> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            Some(Region::new(x, y, (a.x + a.w).max(b.x + b.w) - x, (a.y + a.h).max(b.y + b.h) - y))
        }
        (a, b) => a.or(b),
    }
}

/// The part of the document shown on screen: `scale` screen pixels per
/// document pixel, with document point (x, y) at the view's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub w: u32,
    pub h: u32,
}

/// The small copy of the document the screen is drawn from while zoomed out. See [`crate::proxy`].
struct Proxy {
    /// The copy is 1/2^level of the document's size.
    level: u32,
    doc: Doc,
    /// The document changed since the copy was made.
    doc_stale: bool,
    renderer: Renderer,
    frame: Raster,
    /// The host has been handed this copy's frame before, so it only needs what changed since.
    shown: bool,
    /// Areas of the document that changed and have not been drawn in this copy: what was
    /// off screen during a drag, and everything that changed while the copy was put by.
    stale: Vec<Region>,
    /// The version of each image the copy's smaller one was made from.
    made: HashMap<String, u64>,
    texts: TextOutlines,
}

/// Add an area to a list of areas that never overlap. A list grown long is replaced by its bounds.
fn add_area(list: &mut Vec<Region>, area: Region) {
    let mut kept: Vec<Region> = std::mem::take(list).into_iter().flat_map(|old| subtract(old, &area)).collect();
    kept.push(area);
    if kept.len() > 48 {
        let bounds = kept.iter().fold(None, |acc, r| union(acc, Some(*r)));
        kept = bounds.into_iter().collect();
    }
    *list = kept;
}

/// The parts of `a` that are not in `b`.
fn subtract(a: Region, b: &Region) -> Vec<Region> {
    let Some(i) = a.intersect(b) else { return vec![a] };
    let mut parts = Vec::new();
    let mut add = |x: i32, y: i32, w: i32, h: i32| {
        if w > 0 && h > 0 {
            parts.push(Region::new(x, y, w, h));
        }
    };
    add(a.x, a.y, a.w, i.y - a.y);
    add(a.x, i.y + i.h, a.w, a.y + a.h - i.y - i.h);
    add(a.x, i.y, i.x - a.x, i.h);
    add(i.x + i.w, i.y, a.x + a.w - i.x - i.w, i.h);
    parts
}

pub struct Session {
    doc: Doc,
    assets: AssetBytes,
    renderer: Renderer,
    done: Vec<Step>,
    undone: Vec<Step>,
    frame: Raster,
    warnings: Vec<String>,
    view: View,
    view_pixels: Vec<u8>,
    /// View-space area changed by the last call that redrew anything.
    dirty_view: Region,
    stroke: Option<ActiveStroke>,
    /// Present while its layer is the one being painted and nothing beneath it has changed.
    below: Option<Below>,
    /// Image assets whose pixels differ from their encoded bytes.
    painted: BTreeSet<String>,
    /// Assets supplied without redrawing, whose layers are still to be drawn.
    stashed: BTreeSet<String>,
    /// An edit being tried out (a drag in progress): what it is, and how to take it back.
    preview: Option<(Edit, Edit)>,
    /// A drag is in progress, from its first step until it is kept or given up.
    dragging: bool,
    /// A view has been set. Until then everything counts as on screen.
    viewing: bool,
    /// The host draws the screen from the frame itself ([`Session::display`]) and scales it;
    /// the session then keeps no screen-sized picture and does no work when the view moves.
    direct: bool,
    /// With `direct`: the part of the displayed frame, in its own pixels, changed since it was last asked for.
    display_dirty: Option<Region>,
    /// With `direct`: the host has been handed the full-size frame before, and what of it has
    /// been drawn since while a smaller copy was on display.
    full_shown: bool,
    full_unshown: Option<Region>,
    /// After a drag that ended zoomed in: the smaller copy stays on display while the real
    /// pixels are drawn a slice at a time ([`Session::refine`]), instead of all at once.
    holding: bool,
    /// Copies of other sizes, kept for when the view or a drag wants them again.
    spare: Vec<Proxy>,
    /// Present while the view is zoomed out far enough to be drawn from a smaller copy.
    proxy: Option<Proxy>,
    /// Areas of the full-size frame that changed and have not been drawn since:
    /// off screen, or shown from the smaller copy. [`Session::settle`] draws them.
    stale: Vec<Region>,
}

const HISTORY_LIMIT: usize = 200;

impl Session {
    pub fn new(doc: Doc, assets: AssetBytes) -> Session {
        Session::with_fonts(doc, assets, Fonts::new())
    }

    /// Open a document with fonts already loaded, so its text is drawn in the first frame.
    pub fn with_fonts(doc: Doc, assets: AssetBytes, fonts: Fonts) -> Session {
        let whole = Region::new(0, 0, doc.width as i32, doc.height as i32);
        let view = View { scale: 1.0, x: 0.0, y: 0.0, w: doc.width, h: doc.height };
        let mut session = Session {
            doc,
            assets,
            renderer: Renderer::new(),
            done: Vec::new(),
            undone: Vec::new(),
            frame: Raster::new(whole),
            warnings: Vec::new(),
            view,
            view_pixels: Vec::new(),
            dirty_view: Region::new(0, 0, 0, 0),
            stroke: None,
            below: None,
            painted: BTreeSet::new(),
            stashed: BTreeSet::new(),
            preview: None,
            dragging: false,
            viewing: false,
            direct: false,
            display_dirty: None,
            full_shown: false,
            full_unshown: None,
            holding: false,
            spare: Vec::new(),
            proxy: None,
            stale: Vec::new(),
        };
        *session.renderer.fonts_mut() = fonts;
        session.renderer.sync(&session.doc);
        // Nothing is drawn yet. What is drawn first depends on the view the host sets: a small
        // copy for a card seen whole, which is a fraction of the work of the full-size picture.
        // The full-size picture is drawn when it is asked for (`frame`, `settle`).
        session.stale = vec![whole];
        session.check();
        session
    }

    /// The fonts loaded into this session.
    pub fn fonts(&self) -> &Fonts {
        self.renderer.fonts()
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    fn whole(&self) -> Region {
        self.frame.region
    }

    // ---- drawing --------------------------------------------------------------

    /// The part of the document on screen, with a little around it. The whole document until a view is set.
    fn visible(&self) -> Region {
        if !self.viewing {
            return self.whole();
        }
        let v = self.view;
        let (x0, y0) = (v.x.floor() as i32 - 64, v.y.floor() as i32 - 64);
        let (x1, y1) = ((v.x + v.w as f32 / v.scale).ceil() as i32 + 64, (v.y + v.h as f32 / v.scale).ceil() as i32 + 64);
        Region::new(x0, y0, x1 - x0, y1 - y0).intersect(&self.whole()).unwrap_or(Region::new(0, 0, 0, 0))
    }

    /// Draw what changed in `region` of the document, as cheaply as the view allows:
    ///
    ///   - zoomed out, into the smaller copy the screen is drawn from;
    ///   - otherwise at full size, but only the part on screen.
    ///
    /// What is not drawn at full size is noted and drawn when it is wanted
    /// ([`Session::settle`]) or scrolled into view. A brush stroke always draws
    /// at full size: its areas are small, and it works on the real pixels.
    fn redraw(&mut self, region: Region) {
        let Some(region) = region.intersect(&self.whole()) else { return };
        // Copies of other sizes that are put by are not drawn now; they note what to draw when wanted again.
        for spare in &mut self.spare {
            add_area(&mut spare.stale, region);
        }
        if self.stroke.is_some() && self.proxy.is_some() {
            // Zoomed out, a stroke is drawn through the smaller copy like any other change: the
            // painted part of the image is shrunk into the copy's image, and that part of the
            // copy is drawn. Nothing is drawn at full size until it is wanted.
            self.note_stale(vec![region]);
            self.shrink_painted(region);
            self.draw_small(region);
        } else if self.stroke.is_some() {
            // Whole blocks of the smaller copy, so it can be refreshed from the frame exactly.
            let k = self.proxy.as_ref().map_or(1, |p| 1 << p.level);
            let (x0, y0) = (region.x.div_euclid(k) * k, region.y.div_euclid(k) * k);
            let (x1, y1) = ((region.x + region.w + k - 1).div_euclid(k) * k, (region.y + region.h + k - 1).div_euclid(k) * k);
            let blocks = Region::new(x0, y0, x1 - x0, y1 - y0).intersect(&self.whole()).unwrap_or(region);
            self.draw_full(blocks);
            self.shrink_into_proxy(blocks);
        } else if self.proxy.is_some() {
            self.note_stale(vec![region]);
            self.draw_small(region);
        } else {
            let visible = self.visible();
            self.note_stale(subtract(region, &visible));
            let shown = region.intersect(&visible);
            if let Some(shown) = shown {
                self.draw_full(shown);
            }
            if self.direct {
                // Only what was drawn is new to the host; the rest is drawn, and told, when it comes on screen.
                if let Some(shown) = shown {
                    self.note_display(shown);
                }
                self.dirty_view = Region::new(0, 0, 0, 0);
                return;
            }
        }
        self.update_view(region);
    }

    /// Note areas of the full-size frame as changed and not yet drawn. The noted areas never overlap.
    fn note_stale(&mut self, areas: Vec<Region>) {
        for area in areas {
            self.stale = std::mem::take(&mut self.stale).into_iter().flat_map(|old| subtract(old, &area)).collect();
            self.stale.push(area);
        }
    }

    /// Draw one region of the document into the full-size frame.
    fn draw_full(&mut self, region: Region) {
        if self.proxy.is_some() {
            // Not on display now: remembered for when it is.
            self.full_unshown = union(self.full_unshown, Some(region));
        }
        let (part, warnings) = match (&mut self.below, &self.stroke) {
            // Mid-stroke: start from the cached layers beneath, then draw the painted layer and what is above it.
            (Some(below), Some(_)) => {
                let mut part = Raster::new(region);
                for ty in region.y.div_euclid(BELOW_TILE)..=(region.y + region.h - 1).div_euclid(BELOW_TILE) {
                    for tx in region.x.div_euclid(BELOW_TILE)..=(region.x + region.w - 1).div_euclid(BELOW_TILE) {
                        let (doc, assets, renderer) = (&self.doc, &self.assets, &mut self.renderer);
                        let tile = below.tiles.entry((tx, ty)).or_insert_with(|| {
                            let mut tile = Raster::new(Region::new(tx * BELOW_TILE, ty * BELOW_TILE, BELOW_TILE, BELOW_TILE));
                            renderer.draw_onto(doc, assets, &mut tile, Some(Split { layer: &below.layer, ancestors: &below.ancestors, below: true }));
                            tile
                        });
                        let Some(area) = region.intersect(&tile.region) else { continue };
                        let width = area.w as usize * 4;
                        for y in area.y..area.y + area.h {
                            let (from, to) = (tile.region.index(area.x, y) * 4, part.region.index(area.x, y) * 4);
                            part.data[to..to + width].copy_from_slice(&tile.data[from..from + width]);
                        }
                    }
                }
                let split = Split { layer: &below.layer, ancestors: &below.ancestors, below: false };
                let warnings = self.renderer.draw_onto(&self.doc, &self.assets, &mut part, Some(split));
                (part, warnings)
            }
            _ => self.renderer.draw_region(&self.doc, &self.assets, region),
        };
        let _ = warnings;
        let width = region.w as usize * 4;
        for y in region.y..region.y + region.h {
            let to = self.frame.region.index(region.x, y) * 4;
            let from = part.region.index(region.x, y) * 4;
            self.frame.data[to..to + width].copy_from_slice(&part.data[from..from + width]);
        }
        self.stale = std::mem::take(&mut self.stale).into_iter().flat_map(|old| subtract(old, &region)).collect();
    }

    /// Draw one region of the document into the smaller copy. During a drag
    /// only the part of it on screen is drawn, and the copy is marked as
    /// partial; the rest is caught up when the drag ends.
    fn draw_small(&mut self, region: Region) {
        let visible = self.visible();
        let dragging = self.dragging;
        let Some(proxy) = self.proxy.as_mut() else { return };
        let region = if dragging {
            // Off screen for now: noted, and drawn when the drag ends.
            for hidden in subtract(region, &visible) {
                add_area(&mut proxy.stale, hidden);
            }
            let Some(shown) = region.intersect(&visible) else { return };
            shown
        } else {
            region
        };
        proxy.stale = std::mem::take(&mut proxy.stale).into_iter().flat_map(|old| subtract(old, &region)).collect();
        if proxy.doc_stale {
            proxy.doc = scale_doc(&self.doc, proxy.level, self.renderer.fonts(), &mut proxy.texts);
            proxy.doc_stale = false;
        }
        // Smaller versions of the images that layers now showing use, made when first needed and again when one is painted on.
        fn showing<'a>(group: &'a GroupLayer, ids: &mut Vec<&'a str>) {
            for layer in group.children.iter().filter(|l| l.base().visible) {
                ids.extend(layer.base().mask.as_ref().map(|m| m.asset.as_str()));
                match layer {
                    Layer::Pixel(l) => ids.push(&l.asset),
                    Layer::Fill(l) => ids.push(&l.asset),
                    Layer::Art(l) => ids.extend(l.asset.as_deref()),
                    Layer::Group(g) => showing(g, ids),
                    _ => {}
                }
            }
        }
        let mut ids = Vec::new();
        showing(&self.doc.root, &mut ids);
        for id in ids {
            let version = self.renderer.image_version(id);
            if proxy.made.get(id) != Some(&version) {
                if let Some(image) = self.renderer.decoded_image(id, &self.assets) {
                    proxy.renderer.set_image(id, downscale(&image, proxy.level));
                    proxy.made.insert(id.to_string(), version);
                }
            }
        }
        proxy.renderer.sync(&proxy.doc);
        let k = 1 << proxy.level;
        let (x0, y0) = (region.x.div_euclid(k) - 1, region.y.div_euclid(k) - 1);
        let (x1, y1) = ((region.x + region.w + k - 1).div_euclid(k) + 1, (region.y + region.h + k - 1).div_euclid(k) + 1);
        let Some(small) = Region::new(x0, y0, x1 - x0, y1 - y0).intersect(&proxy.frame.region) else { return };
        let (part, warnings) = proxy.renderer.draw_region(&proxy.doc, &AssetBytes::new(), small);
        let _ = warnings;
        let width = small.w as usize * 4;
        for y in small.y..small.y + small.h {
            let (to, from) = (proxy.frame.region.index(small.x, y) * 4, part.region.index(small.x, y) * 4);
            proxy.frame.data[to..to + width].copy_from_slice(&part.data[from..from + width]);
        }
    }

    /// Make sure the smaller copy's version of an image is up to date, making it afresh if not.
    fn sync_proxy_image(&mut self, asset: &str) {
        let Some(proxy) = self.proxy.as_mut() else { return };
        let version = self.renderer.image_version(asset);
        if proxy.made.get(asset) != Some(&version) {
            if let Some(image) = self.renderer.image(asset) {
                proxy.renderer.set_image(asset, downscale(image, proxy.level));
                proxy.made.insert(asset.to_string(), version);
            }
        }
    }

    /// During a stroke: bring the smaller copy's version of the painted image up
    /// to date over `region` of the document, by shrinking just that part.
    fn shrink_painted(&mut self, region: Region) {
        let (Some(active), Some(proxy)) = (self.stroke.as_ref(), self.proxy.as_mut()) else { return };
        let Some(full) = self.renderer.image(&active.asset) else { return };
        if proxy.renderer.image(&active.asset).is_none() {
            return;
        }
        // Whatever of the image this stroke has changed is shrunk here, so the copy's image stays current.
        proxy.made.insert(active.asset.clone(), self.renderer.image_version(&active.asset));
        let k = 1 << proxy.level;
        // The part of the image, in its own pixels, as whole blocks of the smaller one.
        let (ox, oy) = active.offset;
        let (x0, y0) = (((region.x - ox).div_euclid(k) * k).max(0), ((region.y - oy).div_euclid(k) * k).max(0));
        let (x1, y1) = (((region.x + region.w - ox + k - 1).div_euclid(k) * k).min(full.w), ((region.y + region.h - oy + k - 1).div_euclid(k) * k).min(full.h));
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let part = Region::new(x0, y0, x1 - x0, y1 - y0);
        let small = downscale(&Image { w: part.w, h: part.h, data: crop(&full.data, full.w, &part) }, proxy.level);
        let Some(target) = proxy.renderer.image_mut(&active.asset, &AssetBytes::new()) else { return };
        let at = Region::new(x0 / k, y0 / k, small.w.min(target.w - x0 / k), small.h.min(target.h - y0 / k));
        if at.w <= 0 || at.h <= 0 {
            return;
        }
        let rows: Vec<u8> = (0..at.h).flat_map(|y| small.data[(y * small.w) as usize * 4..(y * small.w + at.w) as usize * 4].iter().copied()).collect();
        paste(target, &at, &rows);
    }

    /// Refresh the smaller copy from the full-size frame over `region`, which is whole blocks of it.
    fn shrink_into_proxy(&mut self, region: Region) {
        let Some(proxy) = self.proxy.as_mut() else { return };
        let k = 1 << proxy.level;
        let part = Image { w: region.w, h: region.h, data: (region.y..region.y + region.h).flat_map(|y| {
            let at = self.frame.region.index(region.x, y) * 4;
            self.frame.data[at..at + region.w as usize * 4].iter().copied()
        }).collect() };
        let small = downscale(&part, proxy.level);
        let (sx, sy) = (region.x / k, region.y / k);
        for y in 0..small.h {
            if sy + y >= proxy.frame.region.h {
                break;
            }
            let width = small.w.min(proxy.frame.region.w - sx).max(0) as usize * 4;
            let (to, from) = (proxy.frame.region.index(sx, sy + y) * 4, (y * small.w) as usize * 4);
            proxy.frame.data[to..to + width].copy_from_slice(&small.data[from..from + width]);
        }
    }

    /// Bring the full-size frame up to date: draw every area that changed while
    /// it was off screen or shown from the smaller copy.
    pub fn settle(&mut self) {
        // Drawing one area takes it out of the others, so nothing is drawn twice.
        while let Some(region) = self.stale.pop() {
            self.draw_full(region);
        }
    }

    /// Look the document over for what is wrong with it (fonts not loaded, images missing).
    /// Done after anything that can change the answer; it draws nothing.
    fn check(&mut self) {
        self.warnings = self.renderer.check(&self.doc, &self.assets);
    }

    /// The full-resolution frame: straight RGBA, document size.
    pub fn frame(&mut self) -> &[u8] {
        self.settle();
        &self.frame.data
    }

    pub fn frame_hash(&mut self) -> u64 {
        self.settle();
        hash(&self.frame.data)
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    // ---- the view --------------------------------------------------------------

    pub fn set_view(&mut self, view: View) {
        self.view = View { scale: view.scale.max(1e-3), ..view };
        self.viewing = true;
        if !self.direct {
            let size = view.w as usize * view.h as usize * 4;
            if self.view_pixels.len() != size {
                self.view_pixels = vec![0; size];
            }
        }
        let switched = self.stroke.is_none() && self.use_level(self.wanted_level());
        if self.stroke.is_none() && !switched && self.dragging {
            // Scrolled or zoomed in the middle of a drag: what has come on screen is drawn.
            self.catch_up();
        }
        if self.proxy.is_none() && self.stroke.is_none() {
            // What was left undrawn off screen and has come into view.
            let visible = self.visible();
            let due: Vec<Region> = self.stale.iter().filter_map(|area| area.intersect(&visible)).collect();
            for area in due {
                self.draw_full(area);
                if self.direct {
                    self.note_display(area);
                }
            }
        }
        if !self.direct {
            self.update_view(self.whole());
            self.dirty_view = Region::new(0, 0, view.w as i32, view.h as i32);
        }
    }

    /// The size of copy the screen should be drawn from now: the smallest that
    /// still has a pixel or more for every screen pixel, and one size smaller
    /// than that while something is being dragged, when every step redraws a
    /// large part of the card and speed matters more than sharpness.
    fn wanted_level(&self) -> u32 {
        let level = ((1.0 / self.view.scale).log2().floor() as i32).clamp(0, 3) as u32;
        if self.holding && level == 0 && !self.dragging {
            // The copy the drag used stays up until the real pixels are ready.
            return self.proxy.as_ref().map_or(0, |p| p.level);
        }
        if !self.dragging || !self.viewing {
            return level;
        }
        // Dragging: every step redraws what moved with everything over and under it, so the
        // copy is the largest that keeps the part on screen under about a third of a million
        // pixels, however far in the view is zoomed. Changing copies costs only what the other
        // one missed while it was put by, and the picture is sharp again when the drag ends.
        let visible = self.visible();
        let mut drag = level;
        while drag < 3 && (visible.area() >> (2 * drag)) > 350_000 {
            drag += 1;
        }
        drag
    }

    /// Draw the screen from the copy of this level (0: the real pixels) from now on. Returns true if that is a change.
    fn use_level(&mut self, level: u32) -> bool {
        if self.proxy.as_ref().map_or(0, |p| p.level) == level {
            return false;
        }
        // The copy in use is put by with a note of what changes meanwhile: going back to it
        // later costs drawing those areas, not the whole of it.
        self.spare.extend(self.proxy.take());
        // What the host needs of the frame now coming on display: all of it the first time,
        // and afterwards only what is drawn into it from here on.
        self.display_dirty = None;
        if level > 0 {
            let proxy = match self.spare.iter().position(|p| p.level == level) {
                Some(at) => self.spare.swap_remove(at),
                None => {
                    let k = 1 << level;
                    let size = Region::new(0, 0, (self.doc.width as i32 + k - 1) / k, (self.doc.height as i32 + k - 1) / k);
                    Proxy { level, doc: Doc::empty(1, 1, 72.0), doc_stale: true, shown: false, stale: vec![self.whole()], renderer: Renderer::new(), frame: Raster::new(size), made: HashMap::new(), texts: TextOutlines::new() }
                }
            };
            let seen = proxy.shown;
            let frame = proxy.frame.region;
            self.proxy = Some(proxy);
            self.catch_up();
            if self.direct && !seen {
                self.display_dirty = Some(frame);
            }
        } else {
            // The real pixels on screen again: draw what changed while they were not shown.
            let visible = self.visible();
            let due: Vec<Region> = self.stale.iter().filter_map(|area| area.intersect(&visible)).collect();
            for area in due {
                self.draw_full(area);
            }
            if self.direct {
                self.display_dirty = if self.full_shown { self.full_unshown.take() } else { Some(self.whole()) };
                self.full_unshown = None;
            }
        }
        if !self.direct {
            self.update_view(self.whole());
        }
        true
    }

    /// Draw what the copy in use has noted as changed and not drawn (during a drag, the part of it on screen).
    fn catch_up(&mut self) {
        let Some(proxy) = self.proxy.as_mut() else { return };
        for area in std::mem::take(&mut proxy.stale) {
            // Drawing takes each area off the list again, or puts back the part that is off screen during a drag.
            self.draw_small(area);
            if self.direct {
                self.note_display(area);
            }
        }
    }

    /// With `direct`: note that this area of the document changed in the frame on display.
    fn note_display(&mut self, region: Region) {
        let (frame, k) = match &self.proxy {
            Some(proxy) => (proxy.frame.region, 1 << proxy.level),
            None => (self.whole(), 1),
        };
        let (x0, y0) = (region.x.div_euclid(k) - 1, region.y.div_euclid(k) - 1);
        let (x1, y1) = ((region.x + region.w + k - 1).div_euclid(k) + 1, (region.y + region.h + k - 1).div_euclid(k) + 1);
        if let Some(area) = Region::new(x0, y0, x1 - x0, y1 - y0).intersect(&frame) {
            self.display_dirty = union(self.display_dirty, Some(area));
        }
    }

    /// Have the host draw the screen from the frame itself, scaled by the host
    /// (a canvas does this almost for free), instead of from a screen-sized
    /// picture resampled here on every change of view. See [`Session::display`].
    pub fn set_direct(&mut self, direct: bool) {
        self.direct = direct;
        if direct {
            self.view_pixels = Vec::new();
            self.display_dirty = Some(self.proxy.as_ref().map_or(self.whole(), |p| p.frame.region));
        }
    }

    /// The frame the screen is drawn from, for a host that scales it itself:
    /// its pixels (straight RGBA), width, height, and how many document pixels
    /// one of its pixels covers along a side (1 for the real frame; 2, 4 or 8
    /// for a smaller copy). At full size only the part on screen is kept up to
    /// date; the rest is whatever it last was.
    pub fn display(&self) -> (&[u8], u32, u32, u32) {
        match &self.proxy {
            Some(proxy) => (&proxy.frame.data, proxy.frame.region.w as u32, proxy.frame.region.h as u32, 1 << proxy.level),
            None => (&self.frame.data, self.frame.region.w as u32, self.frame.region.h as u32, 1),
        }
    }

    /// The part of [`Session::display`]'s frame, in its own pixels, that has
    /// changed since this was last called. `None` if nothing has.
    pub fn take_display_dirty(&mut self) -> Option<Region> {
        match &mut self.proxy {
            Some(proxy) => proxy.shown = true,
            None => self.full_shown = true,
        }
        self.display_dirty.take()
    }

    /// After a drag that ended zoomed in, the smaller copy is still on display.
    /// Draw up to about `budget` pixels of the real picture where it is on
    /// screen and out of date, and when none is left, put the real picture on
    /// display. Call once a frame until it returns true. Returns true at once
    /// when there is nothing to do.
    pub fn refine(&mut self, budget: usize) -> bool {
        if !self.holding {
            return true;
        }
        if self.stroke.is_some() || self.dragging {
            return false;
        }
        let visible = self.visible();
        let mut left = budget.max(1) as i64;
        while left > 0 {
            let Some(area) = self.stale.iter().find_map(|area| area.intersect(&visible)) else { break };
            // A strip of it, as tall as the budget allows.
            let rows = ((left / area.w.max(1) as i64).min(i32::MAX as i64) as i32).max(16).min(area.h);
            let strip = Region::new(area.x, area.y, area.w, rows);
            self.draw_full(strip);
            left -= strip.area() as i64;
        }
        if self.stale.iter().any(|area| area.intersect(&visible).is_some()) {
            return false;
        }
        self.holding = false;
        self.level_changed();
        true
    }

    /// Make ready, ahead of time, the smaller copy a drag at this zoom would
    /// use, so the first drag does not wait for it. Costs what drawing that
    /// copy once costs; call when nothing else is going on.
    pub fn warm(&mut self) {
        if !self.viewing || self.stroke.is_some() || self.dragging || self.holding {
            return;
        }
        self.dragging = true;
        let level = self.wanted_level();
        self.dragging = false;
        let have = self.proxy.as_ref().is_some_and(|p| p.level == level) || self.spare.iter().any(|p| p.level == level);
        if level == 0 || have {
            return;
        }
        // Brought up as if for a drag, drawn, and put by again. The host is told nothing: it
        // has not seen this copy, and will be handed all of it when a drag does bring it up.
        // As in a drag, only the part on screen is drawn; the rest waits until it is needed.
        let (dirty, back) = (self.display_dirty, self.proxy.as_ref().map_or(0, |p| p.level));
        self.dragging = true;
        self.use_level(level);
        self.dragging = false;
        self.use_level(back);
        self.display_dirty = dirty;
    }

    /// The view's pixels: straight RGBA, `view.w × view.h`.
    pub fn view_pixels(&self) -> &[u8] {
        &self.view_pixels
    }

    pub fn view(&self) -> View {
        self.view
    }

    /// The part of the view that the last change redrew.
    pub fn dirty_view(&self) -> Region {
        self.dirty_view
    }

    /// Refresh the view pixels that show `region` of the document.
    fn update_view(&mut self, region: Region) {
        let v = self.view;
        if self.direct {
            // The host scales the frame itself: it only needs to know which part of it changed.
            self.note_display(region);
            self.dirty_view = Region::new(0, 0, 0, 0);
            return;
        }
        if self.view_pixels.is_empty() {
            self.dirty_view = Region::new(0, 0, 0, 0);
            return;
        }
        let x0 = (((region.x as f32 - v.x) * v.scale).floor() as i32 - 1).max(0);
        let y0 = (((region.y as f32 - v.y) * v.scale).floor() as i32 - 1).max(0);
        let x1 = ((((region.x + region.w) as f32 - v.x) * v.scale).ceil() as i32 + 1).min(v.w as i32);
        let y1 = ((((region.y + region.h) as f32 - v.y) * v.scale).ceil() as i32 + 1).min(v.h as i32);
        self.dirty_view = Region::new(x0, y0, x1 - x0, y1 - y0);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        // From the smaller copy when there is one: `f` of its pixels to a document pixel.
        let (doc, frame, f) = match &self.proxy {
            Some(proxy) => (proxy.frame.region, &proxy.frame.data, 1.0 / (1 << proxy.level) as f32),
            None => (self.whole(), &self.frame.data, 1.0),
        };
        let step = 1.0 / v.scale;
        // Zoomed out, one screen pixel covers several document pixels: average a small grid of them.
        let taps = ((step * f).ceil() as i32).clamp(1, 4);
        for vy in y0..y1 {
            for vx in x0..x1 {
                let mut acc = [0f32; 4];
                for j in 0..taps {
                    for i in 0..taps {
                        let dx = ((v.x + (vx as f32 + (i as f32 + 0.5) / taps as f32) * step) * f).floor() as i32;
                        let dy = ((v.y + (vy as f32 + (j as f32 + 0.5) / taps as f32) * step) * f).floor() as i32;
                        if dx < 0 || dy < 0 || dx >= doc.w || dy >= doc.h {
                            continue;
                        }
                        let p = (dy as usize * doc.w as usize + dx as usize) * 4;
                        let a = frame[p + 3] as f32;
                        acc[0] += frame[p] as f32 * a;
                        acc[1] += frame[p + 1] as f32 * a;
                        acc[2] += frame[p + 2] as f32 * a;
                        acc[3] += a;
                    }
                }
                let o = (vy as usize * v.w as usize + vx as usize) * 4;
                let out = &mut self.view_pixels[o..o + 4];
                if acc[3] > 0.0 {
                    out[0] = (acc[0] / acc[3] + 0.5) as u8;
                    out[1] = (acc[1] / acc[3] + 0.5) as u8;
                    out[2] = (acc[2] / acc[3] + 0.5) as u8;
                    out[3] = (acc[3] / (taps * taps) as f32 + 0.5) as u8;
                } else {
                    out.fill(0);
                }
            }
        }
    }

    // ---- edits and history -------------------------------------------------------

    fn push(&mut self, step: Step) {
        self.done.push(step);
        if self.done.len() > HISTORY_LIMIT {
            self.done.remove(0);
        }
        self.undone.clear();
    }

    /// The areas the layers named by `reach` can affect right now.
    fn areas(&mut self, reach: &Reach) -> Vec<Region> {
        match reach {
            Reach::Everything => vec![self.whole()],
            Reach::Layers(ids) => {
                let root = self.doc.root.base.id.clone();
                if ids.contains(&root) {
                    return vec![self.whole()];
                }
                ids.iter().filter_map(|id| self.renderer.layer_reach(&self.doc, &self.assets, id)).collect()
            }
        }
    }

    /// Redraw several areas, and leave the union of what changed on screen as the dirty view.
    fn redraw_all(&mut self, areas: Vec<Region>) {
        // Areas that touch are drawn as one, so nothing is drawn twice; areas apart stay
        // apart, so a change at the top and one at the bottom do not redraw the middle.
        let mut merged: Vec<Region> = Vec::new();
        for mut area in areas {
            while let Some(at) = merged.iter().position(|other| other.intersect(&area).is_some()) {
                area = union(Some(area), Some(merged.swap_remove(at))).unwrap_or(area);
            }
            merged.push(area);
        }
        let mut dirty_view = None;
        for area in merged {
            self.redraw(area);
            dirty_view = union(dirty_view, Some(self.dirty_view).filter(|d| d.area() > 0));
        }
        self.dirty_view = dirty_view.unwrap_or(Region::new(0, 0, 0, 0));
    }

    /// Apply an edit and redraw what it could have changed: where the affected
    /// layers were before, and where they are after. Returns the inverse edit.
    fn apply_and_redraw(&mut self, edit: &Edit) -> Result<Edit> {
        self.apply_and_redraw_in(edit, None)
    }

    /// As [`Session::apply_and_redraw`], for an edit whose reach the caller
    /// knows: a step of a drag changes the dragged layers and nothing else,
    /// which saves trying the edit on a copy of the document to find out.
    fn apply_and_redraw_in(&mut self, edit: &Edit, known: Option<Reach>) -> Result<Edit> {
        self.below = None;
        let reach = match edit {
            _ if known.is_some() => known.expect("just checked"),
            // A batch is judged by what it changes in the end, found by trying it on a copy.
            Edit::Batch { .. } => {
                let mut trial = self.doc.clone();
                apply(&mut trial, edit)?;
                if (trial.width, trial.height, trial.dpi) != (self.doc.width, self.doc.height, self.doc.dpi) {
                    Reach::Everything
                } else {
                    Reach::Layers(changed_layers(&self.doc, &trial))
                }
            }
            _ => Reach::of(edit),
        };
        let mut areas = self.areas(&reach);
        let inverse = apply(&mut self.doc, edit)?;
        self.renderer.sync(&self.doc);
        for proxy in self.proxy.iter_mut().chain(self.spare.iter_mut()) {
            proxy.doc_stale = true;
        }
        let size = (self.doc.width as i32, self.doc.height as i32);
        if size != (self.frame.region.w, self.frame.region.h) {
            // The document was resized: start the frame again.
            self.frame = Raster::new(Region::new(0, 0, size.0, size.1));
            self.proxy = None;
            self.spare.clear();
            self.stale.clear();
            self.draw_full(self.whole());
            self.set_view(self.view);
            return Ok(inverse);
        }
        areas.extend(self.areas(&reach));
        self.redraw_all(areas);
        Ok(inverse)
    }

    // ---- trying an edit out ------------------------------------------------------

    /// Show an edit without making it a step of history yet: the state of a
    /// drag. Each call replaces the one before, always starting from the
    /// document as it was when the first was made. [`Session::commit_preview`]
    /// keeps it as one step; [`Session::cancel_preview`] takes it back.
    pub fn preview(&mut self, edit: Edit) -> Result<()> {
        if self.stroke.is_some() {
            return Err(err!("Finish the brush stroke first"));
        }
        let Some((_, back)) = self.preview.take() else {
            // The smaller copy takes over first, so even the first step is drawn small.
            self.dragging = true;
            let switched = self.viewing && self.use_level(self.wanted_level());
            let back = match self.apply_and_redraw_in(&edit, Some(Reach::of(&edit))) {
                Ok(back) => back,
                Err(e) => {
                    self.dragging = false;
                    self.level_changed();
                    return Err(e);
                }
            };
            self.preview = Some((edit, back));
            if switched {
                self.dirty_view = Region::new(0, 0, self.view.w as i32, self.view.h as i32);
            }
            return Ok(());
        };
        // Back to the start and on to the new state as one change, so only what differs between
        // the last state and this one is drawn.
        // Both halves touch the same layers: the ones being dragged.
        let reach = Reach::of(&edit);
        match self.apply_and_redraw_in(&Edit::Batch { edits: vec![back.clone(), edit.clone()] }, Some(reach)) {
            Ok(Edit::Batch { mut edits }) if edits.len() == 2 => {
                // The inverse of the pair is "undo the new edit, then redo the old": the first half takes this state back.
                self.preview = Some((edit, edits.swap_remove(0)));
                Ok(())
            }
            Ok(_) => unreachable!("a batch of two has an inverse of two"),
            Err(e) => {
                // Refused, so nothing changed: the earlier preview is still what shows. Take it back.
                self.apply_and_redraw(&back).expect("a recorded inverse always applies");
                self.dragging = false;
                self.level_changed();
                Err(e)
            }
        }
    }

    /// A drag began or ended: change to the size of copy that suits, and if that
    /// is a change, the whole screen has been redrawn.
    fn level_changed(&mut self) {
        if !self.viewing {
            return;
        }
        let switched = self.use_level(self.wanted_level());
        // The same copy carries on after a drag: bring the part that was off screen up to date.
        // (Not while the copy is only being held until the real pixels are ready: it is about to go.)
        let behind = !switched && !self.dragging && !self.holding && self.proxy.as_ref().is_some_and(|p| !p.stale.is_empty());
        if behind {
            self.catch_up();
        }
        if (switched || behind) && !self.direct {
            self.update_view(self.whole());
            self.dirty_view = Region::new(0, 0, self.view.w as i32, self.view.h as i32);
        }
    }

    /// Keep what [`Session::preview`] is showing as one undoable step. False if nothing was being tried.
    pub fn commit_preview(&mut self, label: &str) -> bool {
        let Some((edit, inverse)) = self.preview.take() else { return false };
        self.push(Step::Edit { label: label.to_string(), edit, inverse });
        self.dirty_view = Region::new(0, 0, 0, 0);
        self.dragging = false;
        self.holding = self.direct && self.proxy.is_some() && self.view.scale > 0.5;
        self.level_changed();
        true
    }

    /// Take back what [`Session::preview`] is showing.
    pub fn cancel_preview(&mut self) {
        if let Some((_, back)) = self.preview.take() {
            self.apply_and_redraw(&back).expect("a recorded inverse always applies");
            self.dragging = false;
            self.holding = self.direct && self.proxy.is_some() && self.view.scale > 0.5;
            self.level_changed();
        }
    }

    /// The topmost showing layer with ink at a document point.
    pub fn hit(&mut self, x: f64, y: f64) -> Option<String> {
        self.renderer.hit(&self.doc, &self.assets, x.floor() as i32, y.floor() as i32)
    }

    /// A layer's bounds (left, top, width, height) without the reach of its effects.
    pub fn layer_box(&mut self, id: &str) -> Option<Region> {
        self.renderer.layer_box(&self.doc, &self.assets, id)
    }

    /// Make a copy of a layer (and everything in it, if it is a group) just
    /// above it, as one step of history. The copy shares the original's
    /// pictures; a shared mask is given its own the first time either is erased.
    /// Returns the copy's id.
    pub fn duplicate(&mut self, layer_id: &str) -> Result<String> {
        let path = crate::tree::locate(&self.doc, layer_id).ok_or_else(|| err!("No layer with id \"{layer_id}\""))?;
        let mut copy = crate::tree::layer_at(&self.doc, &path).clone();
        let parent = if path.len() == 1 { self.doc.root.base.id.clone() } else { crate::tree::layer_at(&self.doc, &path[..path.len() - 1]).id().to_string() };
        let taken: BTreeSet<String> = crate::tree::all_ids(&self.doc).into_iter().map(str::to_string).collect();
        fn rename(layer: &mut Layer, taken: &BTreeSet<String>, made: &mut BTreeSet<String>) {
            let base = layer.id().split("-copy").next().unwrap_or_default().to_string();
            let mut n = 1;
            let mut id = format!("{base}-copy");
            while taken.contains(&id) || made.contains(&id) {
                n += 1;
                id = format!("{base}-copy{n}");
            }
            made.insert(id.clone());
            layer.base_mut().id = id;
            // A copy is the user's own layer, not one the card engine looks after.
            layer.base_mut().role = None;
            if let Layer::Group(group) = layer {
                for child in &mut group.children {
                    rename(child, taken, made);
                }
            }
        }
        rename(&mut copy, &taken, &mut BTreeSet::new());
        let name = format!("{} copy", copy.base().name);
        copy.base_mut().name = name;
        let id = copy.id().to_string();
        let label = format!("Duplicate {}", crate::tree::layer_at(&self.doc, &path).base().name);
        self.execute(Edit::AddLayer { parent, layer: copy, index: Some(path[path.len() - 1] + 1) }, &label)?;
        Ok(id)
    }

    /// Apply a structural edit. A refused edit changes nothing.
    pub fn execute(&mut self, edit: Edit, label: &str) -> Result<()> {
        if self.stroke.is_some() {
            return Err(err!("Finish the brush stroke first"));
        }
        // A drag left unfinished is taken back before anything else changes the document.
        self.cancel_preview();
        let inverse = self.apply_and_redraw(&edit)?;
        self.push(Step::Edit { label: label.to_string(), edit, inverse });
        self.check();
        Ok(())
    }

    fn replay(&mut self, step: &Step, forward: bool) {
        match step {
            Step::Edit { edit, inverse, .. } => {
                self.apply_and_redraw(if forward { edit } else { inverse }).expect("a recorded edit always applies");
            }
            Step::Paint { asset, offset, patches, margin, .. } => {
                self.below = None;
                if let Some(image) = self.renderer.image_mut(asset, &self.assets) {
                    for (region, before, after) in patches {
                        paste(image, region, if forward { after } else { before });
                    }
                }
                // The pixels no longer match the encoded bytes, whichever way this went.
                self.painted.insert(asset.clone());
                self.renderer.sync(&self.doc);
                let mut dirty_view = None;
                for (region, _, _) in patches {
                    self.redraw(Region::new(region.x + offset.0 - margin, region.y + offset.1 - margin, region.w + margin * 2, region.h + margin * 2));
                    dirty_view = union(dirty_view, Some(self.dirty_view).filter(|d| d.area() > 0));
                }
                self.dirty_view = dirty_view.unwrap_or(Region::new(0, 0, 0, 0));
            }
        }
    }

    /// Returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        if self.stroke.is_some() {
            return false;
        }
        self.cancel_preview();
        let Some(step) = self.done.pop() else { return false };
        self.replay(&step, false);
        self.undone.push(step);
        self.check();
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.stroke.is_some() {
            return false;
        }
        self.cancel_preview();
        let Some(step) = self.undone.pop() else { return false };
        self.replay(&step, true);
        self.done.push(step);
        self.check();
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Labels of the undo stack, oldest first.
    pub fn labels(&self) -> Vec<&str> {
        self.done.iter().map(Step::label).collect()
    }

    /// Labels of the steps that were undone and can be done again, the next one first.
    pub fn redo_labels(&self) -> Vec<&str> {
        self.undone.iter().rev().map(Step::label).collect()
    }

    // ---- painting ----------------------------------------------------------------

    /// Add an empty, document-sized pixel layer on top and return its id.
    pub fn add_paint_layer(&mut self, name: &str) -> Result<String> {
        let mut n = 1;
        while self.doc.assets.contains_key(&format!("paint-{n}")) || crate::tree::find(&self.doc, &format!("paint-{n}")).is_some() {
            n += 1;
        }
        let id = format!("paint-{n}");
        let (w, h) = (self.doc.width, self.doc.height);
        self.renderer.set_image(&id, Image { w: w as i32, h: h as i32, data: vec![0; w as usize * h as usize * 4] });
        self.painted.insert(id.clone());
        let mut layer = pixel(&id, &id, crate::doc::IDENTITY);
        layer.base_mut().name = name.to_string();
        let asset = AssetRef { kind: AssetKind::Image, path: format!("assets/{id}.png"), width: Some(w), height: Some(h) };
        let root = self.doc.root.base.id.clone();
        self.execute(Edit::Batch { edits: vec![Edit::SetAsset { id: id.clone(), asset: Some(asset) }, Edit::AddLayer { parent: root, layer, index: None }] }, "New paint layer")?;
        Ok(id)
    }

    /// How far beyond a layer's own pixels the document can change when they do: the largest effect on it or on a group around it.
    fn effect_margin(&self, layer_id: &str) -> i32 {
        let extent = |layer: &Layer| layer.base().active_effects().iter().map(|e| effect_extent(e)).max().unwrap_or(0);
        let Some(path) = crate::tree::locate(&self.doc, layer_id) else { return 0 };
        let mut layer = &self.doc.root.children[path[0]];
        let mut margin = extent(layer);
        for &i in &path[1..] {
            layer = &layer.children()[i];
            margin = margin.max(extent(layer));
        }
        margin
    }

    /// Make sure a layer has a mask that can be painted on, and say where it is:
    /// its image, that image's place in the document, and the effect margin.
    ///
    /// A paintable mask is one made here: the size of the document, unscaled,
    /// revealing everything outside itself. A layer with no mask gets a white
    /// one. A layer with another kind of mask (a template's, or one that was
    /// scaled along with its layer) gets that mask as it acts now, redrawn as
    /// a plain one, so nothing changes on screen. Adding the mask is a step of
    /// history of its own.
    fn paint_mask(&mut self, layer_id: &str) -> Result<(String, (i32, i32), i32)> {
        let layer = crate::tree::find(&self.doc, layer_id).ok_or_else(|| err!("No layer with id \"{layer_id}\""))?;
        let name = layer.base().name.clone();
        let margin = self.effect_margin(layer_id);
        let (w, h) = (self.doc.width as i32, self.doc.height as i32);
        if let Some(mask) = &layer.base().mask {
            let plain = mask.asset.starts_with("mask-") && mask.scale.is_none_or(|s| s == 1.0) && mask.offset.x.fract() == 0.0 && mask.offset.y.fract() == 0.0 && mask.enabled != Some(false) && mask.inverted != Some(true) && mask.density.is_none_or(|d| d == 1.0) && mask.feather.is_none_or(|f| f == 0.0);
            let whole = self.renderer.decoded_image(&mask.asset, &self.assets).is_some_and(|image| mask.offset.x <= 0.0 && mask.offset.y <= 0.0 && mask.offset.x as i32 + image.w >= w && mask.offset.y as i32 + image.h >= h);
            let mut users = 0;
            crate::tree::walk(&self.doc.root, &mut |other| users += usize::from(other.base().mask.as_ref().is_some_and(|m| m.asset == mask.asset)));
            if plain && whole && users == 1 {
                return Ok((mask.asset.clone(), (mask.offset.x as i32, mask.offset.y as i32), margin));
            }
        }
        let hides_effects = layer.base().mask.as_ref().and_then(|m| m.hides_effects);
        let cover = self.renderer.pixel_mask(&self.doc, &self.assets, layer_id, self.whole());
        let mut n = 1;
        while self.doc.assets.contains_key(&format!("mask-{n}")) {
            n += 1;
        }
        let asset = format!("mask-{n}");
        self.renderer.set_image(&asset, Image { w, h, data: cover.iter().flat_map(|&c| [c, c, c, 255]).collect() });
        self.painted.insert(asset.clone());
        let image = AssetRef { kind: AssetKind::Image, path: format!("assets/{asset}.png"), width: Some(w as u32), height: Some(h as u32) };
        let mut mask = serde_json::json!({ "asset": asset, "offset": { "x": 0, "y": 0 }, "outside": "reveal" });
        if let Some(hides) = hides_effects {
            mask["hidesEffects"] = serde_json::json!(hides);
        }
        let props = serde_json::json!({ "mask": mask }).as_object().cloned().unwrap_or_default();
        self.execute(Edit::Batch { edits: vec![Edit::SetAsset { id: asset.clone(), asset: Some(image) }, Edit::SetLayerProps { id: layer_id.to_string(), props }] }, &format!("Mask {name}"))?;
        Ok((asset, (0, 0), margin))
    }

    /// A mask image was changed in place, inside `area` of the document: draw
    /// what changed and record it for undo, as the tiles that differ from
    /// `before`, which is the mask's pixels over `area` as they were.
    fn mask_changed(&mut self, asset: String, offset: (i32, i32), margin: i32, area: Region, before: &[u8], label: &str) {
        let Some(image) = self.renderer.image(&asset) else { return };
        let (ox, oy) = offset;
        // The area in the mask image's own pixels.
        let inside = Region::new(area.x - ox, area.y - oy, area.w, area.h);
        let mut patches = Vec::new();
        let mut changed: Option<Region> = None;
        for ty in (inside.y.div_euclid(BELOW_TILE)..=(inside.y + inside.h - 1).div_euclid(BELOW_TILE)).map(|t| t * BELOW_TILE) {
            for tx in (inside.x.div_euclid(BELOW_TILE)..=(inside.x + inside.w - 1).div_euclid(BELOW_TILE)).map(|t| t * BELOW_TILE) {
                let Some(tile) = Region::new(tx, ty, BELOW_TILE, BELOW_TILE).intersect(&inside) else { continue };
                let was = crop(before, inside.w, &Region::new(tile.x - inside.x, tile.y - inside.y, tile.w, tile.h));
                let now = crop(&image.data, image.w, &tile);
                if was != now {
                    changed = union(changed, Some(Region::new(tile.x + ox, tile.y + oy, tile.w, tile.h)));
                    patches.push((tile, was, now));
                }
            }
        }
        let Some(changed) = changed else { return };
        self.painted.insert(asset.clone());
        self.below = None;
        self.renderer.sync(&self.doc);
        self.redraw(Region::new(changed.x - margin, changed.y - margin, changed.w + margin * 2, changed.h + margin * 2));
        self.push(Step::Paint { label: label.to_string(), asset, offset, patches, margin });
    }

    /// The part of the document a mask image covers, cut to `area`; and the mask's pixels there now.
    fn mask_before(&mut self, asset: &str, offset: (i32, i32), area: Region) -> Option<(Region, Vec<u8>)> {
        let image = self.renderer.image_mut(asset, &self.assets)?;
        let area = area.intersect(&Region::new(offset.0, offset.1, image.w, image.h))?;
        let before = crop(&image.data, image.w, &Region::new(area.x - offset.0, area.y - offset.1, area.w, area.h));
        Some((area, before))
    }

    /// Erase the part of a layer inside a closed path (an SVG path in document
    /// pixels: a rectangle or a lasso), or with `outside` everything of it but
    /// that part, which is how a character is cut out: draw around it, erase
    /// outside. Through the layer's mask, as one step of history.
    pub fn erase_area(&mut self, layer_id: &str, d: &str, outside: bool, label: &str) -> Result<()> {
        self.cancel_preview();
        if self.stroke.is_some() {
            return Err(err!("Finish the brush stroke first"));
        }
        let whole = self.whole();
        // Only where the path is, or with `outside`, only where the layer is: not the whole card.
        let bounds = kurbo::BezPath::from_svg(d).ok().map(|path| kurbo::Shape::bounding_box(&path)).ok_or_else(|| err!("The selection is empty"))?;
        let around = Region::new(bounds.x0.floor() as i32 - 1, bounds.y0.floor() as i32 - 1, (bounds.x1.ceil() - bounds.x0.floor()) as i32 + 2, (bounds.y1.ceil() - bounds.y0.floor()) as i32 + 2);
        let wanted = if outside { self.layer_box(layer_id).unwrap_or(whole) } else { around };
        let Some(wanted) = wanted.intersect(&whole) else { return Err(err!("The selection is off the card")) };
        let cover = self.renderer.path_cover(&self.doc, &self.assets, d, wanted).ok_or_else(|| err!("The selection is empty"))?;
        let (asset, offset, margin) = self.paint_mask(layer_id)?;
        let Some((area, before)) = self.mask_before(&asset, offset, wanted) else { return Ok(()) };
        let image = self.renderer.image_mut(&asset, &self.assets).ok_or_else(|| err!("The layer's mask is unavailable"))?;
        for y in area.y..area.y + area.h {
            for x in area.x..area.x + area.w {
                let inside = cover[wanted.index(x, y)];
                let take = if outside { 255 - inside } else { inside } as u32;
                if take == 0 {
                    continue;
                }
                let m = ((y - offset.1) as usize * image.w as usize + (x - offset.0) as usize) * 4;
                let now = (image.data[m] as u32 * (255 - take) / 255) as u8;
                image.data[m..m + 3].fill(now);
            }
        }
        self.mask_changed(asset, offset, margin, area, &before, label);
        Ok(())
    }

    /// Erase everything of a layer that is the color found at a document
    /// point, to within `tolerance` (0 to 255 on each of red, green, blue and
    /// opacity): the part touching the point if `contiguous`, or every part.
    /// The edge is softened over as much again. It is erased through the
    /// layer's mask, like the eraser, and is one step of history. Returns how
    /// many pixels went.
    pub fn magic_erase(&mut self, layer_id: &str, x: f64, y: f64, tolerance: f32, contiguous: bool, label: &str) -> Result<usize> {
        self.cancel_preview();
        if self.stroke.is_some() {
            return Err(err!("Finish the brush stroke first"));
        }
        let whole = self.whole();
        let (px, py) = (x.floor() as i32, y.floor() as i32);
        // Only where the layer is: a picture a quarter the size of the card is a quarter of the work.
        let Some(area) = self.layer_box(layer_id).and_then(|b| b.intersect(&whole)) else {
            return Err(err!("There is nothing of this layer at that point"));
        };
        if px < area.x || py < area.y || px >= area.x + area.w || py >= area.y + area.h {
            return Err(err!("There is nothing of this layer at that point"));
        }
        let pixels = self.renderer.layer_pixels(&self.doc, &self.assets, layer_id, area);
        let (w, h) = (area.w as usize, area.h as usize);
        let data = &pixels.data;
        let seed = (py - area.y) as usize * w + (px - area.x) as usize;
        let wanted = [data[seed * 4] as i32, data[seed * 4 + 1] as i32, data[seed * 4 + 2] as i32, data[seed * 4 + 3] as i32];
        if wanted[3] < 8 {
            return Err(err!("There is nothing of this layer at that point"));
        }
        // How far a pixel is from the color clicked: the largest difference of any channel
        // (0..255), or more than any tolerance where there is nothing of the layer.
        let distance = |i: usize| -> i32 {
            let p = &data[i * 4..i * 4 + 4];
            if p[3] < 8 {
                return i32::MAX;
            }
            (p[0] as i32 - wanted[0]).abs().max((p[1] as i32 - wanted[1]).abs()).max((p[2] as i32 - wanted[2]).abs()).max((p[3] as i32 - wanted[3]).abs())
        };
        let tolerance = tolerance.clamp(0.0, 255.0) as i32;
        let mut picked = vec![false; w * h];
        // The pixels just outside what is picked, to soften the cut edge afterwards.
        let mut edge: Vec<usize> = Vec::new();
        if contiguous {
            let mut queue = vec![seed];
            picked[seed] = true;
            while let Some(i) = queue.pop() {
                let (ix, iy) = (i % w, i / w);
                let mut visit = |j: usize, queue: &mut Vec<usize>| {
                    if picked[j] {
                        return;
                    }
                    if distance(j) <= tolerance {
                        picked[j] = true;
                        queue.push(j);
                    } else {
                        edge.push(j);
                    }
                };
                if ix > 0 { visit(i - 1, &mut queue); }
                if ix + 1 < w { visit(i + 1, &mut queue); }
                if iy > 0 { visit(i - w, &mut queue); }
                if iy + 1 < h { visit(i + w, &mut queue); }
            }
        } else {
            for i in 0..w * h {
                picked[i] = distance(i) <= tolerance;
            }
            for i in 0..w * h {
                if picked[i] {
                    continue;
                }
                let (ix, iy) = (i % w, i / w);
                if (ix > 0 && picked[i - 1]) || (ix + 1 < w && picked[i + 1]) || (iy > 0 && picked[i - w]) || (iy + 1 < h && picked[i + w]) {
                    edge.push(i);
                }
            }
        }
        let count = picked.iter().filter(|p| **p).count();

        let (asset, offset, margin) = self.paint_mask(layer_id)?;
        let Some((kept, before)) = self.mask_before(&asset, offset, area) else { return Ok(0) };
        let image = self.renderer.image_mut(&asset, &self.assets).ok_or_else(|| err!("The layer's mask is unavailable"))?;
        let (iw, ox, oy) = (image.w as usize, offset.0, offset.1);
        let mut darken = |i: usize, to: u8| {
            let (mx, my) = (area.x + (i % w) as i32 - ox, area.y + (i / w) as i32 - oy);
            if mx < 0 || my < 0 || mx >= image.w || my >= image.h {
                return;
            }
            let m = (my as usize * iw + mx as usize) * 4;
            let now = image.data[m].min(to);
            image.data[m..m + 3].fill(now);
        };
        for (i, _) in picked.iter().enumerate().filter(|(_, p)| **p) {
            darken(i, 0);
        }
        // Next to what was picked: partly erased, the more so the closer its color, so the cut edge is not jagged.
        if tolerance > 0 {
            for i in edge {
                let d = distance(i);
                if d <= tolerance * 2 {
                    darken(i, (255 * (d - tolerance) / tolerance) as u8);
                }
            }
        }
        self.mask_changed(asset, offset, margin, kept, &before, label);
        Ok(count)
    }

    /// Make ready what a stroke on this layer will need: the layers beneath it,
    /// flattened, for the part of the card on screen. A stroke makes these as
    /// it reaches each part, which is a pause the first time the brush enters
    /// a new area; made ahead of time, there is none. Draws up to `budget`
    /// tiles a call and returns true when there is nothing left to make.
    /// Only matters when the real pixels are on display (zoomed in): zoomed
    /// out, strokes are drawn through the smaller copy.
    pub fn prepare_stroke(&mut self, layer_id: &str, budget: usize) -> bool {
        if self.stroke.is_some() || self.dragging || self.holding || self.proxy.is_some() || !self.viewing {
            return true;
        }
        if self.below.as_ref().is_none_or(|b| b.layer != layer_id) {
            self.below = can_split_at(&self.doc, layer_id).map(|ancestors| Below { layer: layer_id.to_string(), ancestors, tiles: Default::default() });
        }
        let visible = self.visible();
        let (doc, assets, renderer) = (&self.doc, &self.assets, &mut self.renderer);
        let Some(below) = self.below.as_mut() else { return true };
        let mut made = 0;
        for ty in visible.y.div_euclid(BELOW_TILE)..=(visible.y + visible.h - 1).div_euclid(BELOW_TILE) {
            for tx in visible.x.div_euclid(BELOW_TILE)..=(visible.x + visible.w - 1).div_euclid(BELOW_TILE) {
                if below.tiles.contains_key(&(tx, ty)) {
                    continue;
                }
                if made == budget {
                    return false;
                }
                let mut tile = Raster::new(Region::new(tx * BELOW_TILE, ty * BELOW_TILE, BELOW_TILE, BELOW_TILE));
                renderer.draw_onto(doc, assets, &mut tile, Some(Split { layer: &below.layer, ancestors: &below.ancestors, below: true }));
                below.tiles.insert((tx, ty), tile);
                made += 1;
            }
        }
        true
    }

    /// Start a brush stroke on a pixel layer.
    pub fn stroke_begin(&mut self, layer_id: &str, brush: Brush) -> Result<()> {
        self.cancel_preview();
        // Painting is done on the sharpest picture the zoom calls for, not on one a drag left up.
        while !self.refine(usize::MAX) {}
        if self.stroke.is_some() {
            return Err(err!("A brush stroke is already in progress"));
        }
        let layer = crate::tree::find(&self.doc, layer_id).ok_or_else(|| err!("No layer with id \"{layer_id}\""))?;
        // A paint layer's own pixels are painted and erased. Anything else (the art, a
        // template's layers, text, a shape, a group) is erased through a mask, so that
        // nothing of it is lost and it can be brought back.
        let own_pixels = layer.id().starts_with("paint-") && matches!(layer, Layer::Pixel(_));
        let mut brush = brush;
        let (asset, offset, margin) = if own_pixels || !brush.erase {
            // Color goes onto a pixel layer's own image (the editor gives the brush a paint layer for this).
            match paint_target(&self.doc.root, layer_id, 0).ok_or_else(|| err!("No layer with id \"{layer_id}\""))? {
                Ok(target) => target,
                Err(_) if !brush.erase && !matches!(layer, Layer::Pixel(_)) => {
                    let (name, kind) = (layer.base().name.clone(), layer.kind());
                    return Err(err!("\"{name}\" is a {kind} layer: it can be erased, but color goes on a paint layer above it."));
                }
                Err(e) => return Err(e),
            }
        } else if brush.erase {
            let target = self.paint_mask(layer_id)?;
            // On a mask, black hides and white shows.
            brush.color = if brush.restore { [255, 255, 255] } else { [0, 0, 0] };
            brush.erase = false;
            target
        } else {
            unreachable!("a brush either erases or paints")
        };
        // Painting changes the image itself, which would silently change every other layer showing it.
        let mut users = 0;
        crate::tree::walk(&self.doc.root, &mut |layer| {
            let own = match layer {
                Layer::Pixel(l) => Some(&l.asset),
                Layer::Fill(l) => Some(&l.asset),
                Layer::Art(l) => l.asset.as_ref(),
                _ => None,
            };
            users += usize::from(own == Some(&asset)) + usize::from(layer.base().mask.as_ref().is_some_and(|m| m.asset == asset));
        });
        if users > 1 {
            return Err(err!("This layer shares its image with another layer. Painting on shared images is not supported yet."));
        }
        let image = self.renderer.image_mut(&asset, &self.assets).ok_or_else(|| err!("The layer's image is unavailable"))?;
        self.stroke = Some(ActiveStroke { stroke: Stroke::new(image, brush), asset: asset.clone(), offset, margin });
        self.sync_proxy_image(&asset);
        // While the stroke runs, effects on this layer and on the groups around it change with
        // every dab, so they are computed fresh instead of read from the cache.
        let mut live = Vec::new();
        if let Some(path) = crate::tree::locate(&self.doc, layer_id) {
            let mut layer = &self.doc.root.children[path[0]];
            for &i in &path[1..] {
                live.push(layer.id().to_string());
                layer = &layer.children()[i];
            }
            live.push(layer.id().to_string());
        }
        self.renderer.set_live(&live);
        // Keep the cached layers beneath from the previous stroke on this layer, if nothing else has changed.
        if self.below.as_ref().is_none_or(|b| b.layer != layer_id) {
            self.below = can_split_at(&self.doc, layer_id).map(|ancestors| Below { layer: layer_id.to_string(), ancestors, tiles: Default::default() });
        }
        Ok(())
    }

    /// Extend the stroke to a document position. Redraws what changed.
    pub fn stroke_to(&mut self, x: f32, y: f32, pressure: f32) {
        self.dirty_view = Region::new(0, 0, 0, 0);
        let Some(active) = self.stroke.as_mut() else { return };
        let Some(image) = self.renderer.image_mut(&active.asset, &self.assets) else { return };
        let (ox, oy) = active.offset;
        let Some(changed) = active.stroke.line_to(image, x - ox as f32, y - oy as f32, pressure) else { return };
        let m = active.margin;
        self.redraw(Region::new(changed.x + ox - m, changed.y + oy - m, changed.w + m * 2, changed.h + m * 2));
    }

    /// Extend the stroke through several positions (x, y, pressure) and redraw
    /// once for all of them: what a fast pointer sends between two frames.
    pub fn stroke_through(&mut self, points: &[(f32, f32, f32)]) {
        self.dirty_view = Region::new(0, 0, 0, 0);
        let Some(active) = self.stroke.as_mut() else { return };
        let Some(image) = self.renderer.image_mut(&active.asset, &self.assets) else { return };
        let (ox, oy) = active.offset;
        let mut changed: Option<Region> = None;
        for &(x, y, pressure) in points {
            changed = union(changed, active.stroke.line_to(image, x - ox as f32, y - oy as f32, pressure));
        }
        let m = active.margin;
        if let Some(changed) = changed {
            self.redraw(Region::new(changed.x + ox - m, changed.y + oy - m, changed.w + m * 2, changed.h + m * 2));
        }
    }

    /// Finish the stroke and record it for undo.
    pub fn stroke_end(&mut self, label: &str) {
        let Some(active) = self.stroke.take() else { return };
        self.renderer.set_live(&[]);
        self.renderer.sync(&self.doc);
        let Some(image) = self.renderer.image(&active.asset) else { return };
        let patches: Vec<(Region, Vec<u8>, Vec<u8>)> = active.stroke.before().into_iter().map(|(region, before)| {
            let after = crop(&image.data, image.w, &region);
            (region, before, after)
        }).collect();
        if patches.is_empty() {
            return;
        }
        self.painted.insert(active.asset.clone());
        self.push(Step::Paint { label: label.to_string(), asset: active.asset, offset: active.offset, patches, margin: active.margin });
    }

    // ---- assets -----------------------------------------------------------------

    /// Supply (or replace) the bytes of an asset after the document was
    /// opened, and redraw. A template's images can arrive this way as they
    /// are needed, instead of all being loaded before anything is shown.
    pub fn add_asset(&mut self, id: &str, bytes: Vec<u8>) -> Result<()> {
        if self.stroke.is_some() {
            return Err(err!("Finish the brush stroke first"));
        }
        self.assets.insert(id.to_string(), bytes);
        self.painted.remove(id);
        self.renderer.forget_image(id);
        self.below = None;
        self.renderer.sync(&self.doc);
        // Only the layers that show this image, or one supplied quietly before it, need drawing again.
        let mut fresh = std::mem::take(&mut self.stashed);
        fresh.insert(id.to_string());
        let mut users = Vec::new();
        crate::tree::walk(&self.doc.root, &mut |layer| {
            let own = match layer {
                Layer::Pixel(l) => fresh.contains(&l.asset),
                Layer::Art(l) => l.asset.as_ref().is_some_and(|asset| fresh.contains(asset)),
                Layer::Fill(l) => fresh.contains(&l.asset),
                _ => false,
            };
            if own || layer.base().mask.as_ref().is_some_and(|mask| fresh.contains(&mask.asset)) {
                users.push(layer.id().to_string());
            }
        });
        let areas = self.areas(&Reach::Layers(users));
        self.redraw_all(areas);
        self.check();
        Ok(())
    }

    /// An image asset as file bytes, as it is now: a painted layer's picture is encoded afresh.
    pub fn asset_bytes(&mut self, id: &str) -> Result<Vec<u8>> {
        if self.painted.contains(id) {
            if let Some(image) = self.renderer.image(id) {
                return encode_png(image.w as u32, image.h as u32, &image.data);
            }
        }
        self.assets.get(id).cloned().ok_or_else(|| err!("The picture \"{id}\" has not been loaded"))
    }

    /// Supply an asset's bytes without redrawing: one that nothing shows yet,
    /// or one of several, drawn together when the last arrives through [`Session::add_asset`].
    pub fn stash_asset(&mut self, id: &str, bytes: Vec<u8>) {
        self.assets.insert(id.to_string(), bytes);
        self.renderer.forget_image(id);
        self.stashed.insert(id.to_string());
    }

    /// The assets that layers which are showing need and whose bytes have not
    /// been supplied, as (id, path), in document order without repeats.
    pub fn missing_assets(&self) -> Vec<(String, String)> {
        fn visit<'a>(group: &'a GroupLayer, wanted: &mut Vec<&'a str>) {
            for layer in &group.children {
                let base = layer.base();
                if !base.visible {
                    continue;
                }
                if let Some(mask) = &base.mask {
                    wanted.push(&mask.asset);
                }
                match layer {
                    Layer::Pixel(l) => wanted.push(&l.asset),
                    Layer::Art(l) => wanted.extend(l.asset.as_deref()),
                    Layer::Fill(l) => wanted.push(&l.asset),
                    Layer::Group(g) => visit(g, wanted),
                    _ => {}
                }
            }
        }
        let mut wanted = Vec::new();
        visit(&self.doc.root, &mut wanted);
        let mut seen = BTreeSet::new();
        wanted
            .into_iter()
            .filter(|id| !self.assets.contains_key(*id) && seen.insert(*id))
            .filter_map(|id| self.doc.assets.get(id).map(|asset| (id.to_string(), asset.path.clone())))
            .collect()
    }

    // ---- fonts ------------------------------------------------------------------

    /// Load a font file for text layers to use, and redraw. Returns the
    /// PostScript names it provides.
    pub fn add_font(&mut self, bytes: Vec<u8>) -> Result<Vec<String>> {
        let names = self.renderer.fonts_mut().add(bytes)?;
        for proxy in self.proxy.iter_mut().chain(self.spare.iter_mut()) {
            proxy.doc_stale = true;
        }
        self.below = None;
        self.renderer.sync(&self.doc);
        self.redraw(self.whole());
        self.check();
        Ok(names)
    }

    // ---- saving -------------------------------------------------------------------

    /// As [`Session::save`], leaving out images that were never loaded (a
    /// template's layers that the card does not show).
    pub fn save_available(&mut self) -> Result<Vec<u8>> {
        self.cancel_preview();
        for id in std::mem::take(&mut self.painted) {
            if let Some(image) = self.renderer.image(&id) {
                self.assets.insert(id, encode_png(image.w as u32, image.h as u32, &image.data)?);
            }
        }
        crate::serialize::save_cardsmith_available(&self.doc, &self.assets)
    }

    /// The document as `.cardsmith` bytes, with painted layers encoded.
    pub fn save(&mut self) -> Result<Vec<u8>> {
        for id in std::mem::take(&mut self.painted) {
            if let Some(image) = self.renderer.image(&id) {
                self.assets.insert(id, encode_png(image.w as u32, image.h as u32, &image.data)?);
            }
        }
        save_cardsmith(&self.doc, &self.assets)
    }
}

/// Find a layer for painting. Returns its image asset, the image's document
/// offset, and the largest effect extent on the layer or any group around it.
/// The outer `None` means the layer is not under `group`.
fn paint_target(group: &GroupLayer, id: &str, margin: i32) -> Option<Result<(String, (i32, i32), i32)>> {
    let extent = |layer: &Layer| layer.base().active_effects().iter().map(|e| effect_extent(e)).max().unwrap_or(0);
    for layer in &group.children {
        let margin = margin.max(extent(layer));
        if layer.id() == id {
            let Layer::Pixel(p) = layer else {
                return Some(Err(err!("Only pixel layers can be painted on. \"{}\" is a {} layer.", layer.base().name, layer.kind())));
            };
            let [a, c, tx, b, d, ty] = p.transform;
            if a != 1.0 || d != 1.0 || b != 0.0 || c != 0.0 || tx.fract() != 0.0 || ty.fract() != 0.0 {
                return Some(Err(err!("\"{}\" is scaled or rotated. Painting on transformed layers is not supported yet.", p.base.name)));
            }
            return Some(Ok((p.asset.clone(), (tx as i32, ty as i32), margin)));
        }
        if let Layer::Group(g) = layer {
            if let Some(found) = paint_target(g, id, margin) {
                return Some(found);
            }
        }
    }
    None
}
