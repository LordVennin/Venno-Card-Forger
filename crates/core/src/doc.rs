//! The document model (SPEC §2).
//!
//! Plain data that serializes to the same JSON the TypeScript model used, so
//! existing documents load unchanged. Binary data lives in assets and is
//! referenced by id.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

pub type LayerId = String;
pub type AssetId = String;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// 2D affine transform, row-major: `[a, c, tx, b, d, ty]` maps layer-local
/// pixels to document pixels as `x' = a·x + c·y + tx`, `y' = b·x + d·y + ty`.
pub type Mat3 = [f64; 6];
pub const IDENTITY: Mat3 = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// Straight (non-premultiplied) RGBA, each 0..1, in sRGB.
pub type Color = [f64; 4];

/// Vector outline as an SVG path string in document pixels.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Path {
    pub d: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_rule: Option<FillRule>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FillRule {
    Nonzero,
    Evenodd,
}

/// Only the modes found by the survey (docs/SURVEY.md): four on layers, plus
/// multiply, screen and luminosity used inside effects.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    SoftLight,
    LinearLight,
    Luminosity,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GradientStop {
    pub offset: f64,
    pub color: Color,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Gradient {
    pub stops: Vec<GradientStop>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GradientStyle {
    Linear,
    Radial,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StrokePosition {
    Outside,
    Inside,
    Center,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Contour {
    Linear,
    Gaussian,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BevelStyle {
    Inner,
    Outer,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BevelTechnique {
    /// The only technique the survey found.
    ChiselHard,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Up,
    Down,
}

/// Color, blend mode and opacity of a bevel's highlight or shadow.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tone {
    pub color: Color,
    pub blend: BlendMode,
    pub opacity: f64,
}

/// Layer effects. Only the seven types found by the survey. Sizes and
/// distances are in document pixels (the importer applies Photoshop's effects
/// scale). Angles are in degrees, counter-clockwise from +x, pointing toward
/// the light.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Effect {
    ColorOverlay {
        enabled: bool,
        blend: BlendMode,
        opacity: f64,
        color: Color,
    },
    GradientOverlay {
        enabled: bool,
        blend: BlendMode,
        opacity: f64,
        gradient: Gradient,
        style: GradientStyle,
        angle: f64,
        /// 1 = the gradient spans the layer's bounds exactly.
        scale: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reverse: Option<bool>,
        /// Shift of the gradient's center, as a fraction of the layer's width and height.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<Vec2>,
    },
    Stroke {
        enabled: bool,
        blend: BlendMode,
        opacity: f64,
        color: Color,
        size: f64,
        position: StrokePosition,
    },
    DropShadow {
        enabled: bool,
        blend: BlendMode,
        opacity: f64,
        color: Color,
        angle: f64,
        distance: f64,
        /// 0..1: the share of `size` that is a hard expansion instead of blur.
        spread: f64,
        size: f64,
        /// Falloff curve. Default linear.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contour: Option<Contour>,
    },
    InnerShadow {
        enabled: bool,
        blend: BlendMode,
        opacity: f64,
        color: Color,
        angle: f64,
        distance: f64,
        /// 0..1: the share of `size` that is a hard contraction instead of blur.
        choke: f64,
        size: f64,
    },
    OuterGlow {
        enabled: bool,
        blend: BlendMode,
        opacity: f64,
        color: Color,
        spread: f64,
        size: f64,
    },
    Bevel {
        enabled: bool,
        style: BevelStyle,
        technique: BevelTechnique,
        direction: Direction,
        /// 1 = 100%.
        depth: f64,
        size: f64,
        soften: f64,
        angle: f64,
        altitude: f64,
        highlight: Tone,
        shadow: Tone,
        /// The bevel's cross-section, as points (position across the bevel,
        /// height), both 0..1, joined by a smooth curve. Omit for a straight
        /// ramp. Photoshop's "Contour" sub-option.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<Vec<[f64; 2]>>,
    },
}

impl Effect {
    pub fn enabled(&self) -> bool {
        match self {
            Effect::ColorOverlay { enabled, .. }
            | Effect::GradientOverlay { enabled, .. }
            | Effect::Stroke { enabled, .. }
            | Effect::DropShadow { enabled, .. }
            | Effect::InnerShadow { enabled, .. }
            | Effect::OuterGlow { enabled, .. }
            | Effect::Bevel { enabled, .. } => *enabled,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outside {
    Hide,
    Reveal,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RasterMask {
    /// Grayscale image asset: white reveals, black hides.
    pub asset: AssetId,
    /// Document position of the mask image's top-left pixel.
    pub offset: Vec2,
    /// What the mask does outside its image (Photoshop's "default color"). Default hide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside: Option<Outside>,
    /// Masks can be switched off without being removed. Default true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverted: Option<bool>,
    /// 0..1. At 0 the mask has no effect. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
    /// Blur radius in pixels. Default 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feather: Option<f64>,
}

impl RasterMask {
    pub fn active(&self) -> bool {
        self.enabled != Some(false) && self.density.unwrap_or(1.0) > 0.0
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VectorMask {
    pub path: Path,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverted: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Locked {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<bool>,
}

/// Properties every layer has.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LayerBase {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    /// 0..1. Scales the layer and all of its effects.
    pub opacity: f64,
    /// 0..1. Scales the layer's own pixels but not its effects. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_opacity: Option<f64>,
    pub blend: BlendMode,
    /// Clipped to the nearest layer below that is not itself clipped.
    pub clip: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<RasterMask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_mask: Option<VectorMask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<Effect>>,
    /// Master switch for the whole layer style. Default true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<Locked>,
    /// Semantic tag used by the card engine, e.g. "pinlines.W".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

impl LayerBase {
    pub fn new(id: &str) -> Self {
        LayerBase {
            id: id.to_string(),
            name: id.to_string(),
            visible: true,
            opacity: 1.0,
            fill_opacity: None,
            blend: BlendMode::Normal,
            clip: false,
            mask: None,
            vector_mask: None,
            effects: None,
            effects_enabled: None,
            locked: None,
            role: None,
        }
    }

    /// Effects that will actually be drawn.
    pub fn active_effects(&self) -> Vec<&Effect> {
        if self.effects_enabled == Some(false) {
            return Vec::new();
        }
        self.effects.iter().flatten().filter(|e| e.enabled()).collect()
    }

    pub fn has_active_mask(&self) -> bool {
        self.mask.as_ref().is_some_and(RasterMask::active)
            || self.vector_mask.as_ref().is_some_and(|v| v.enabled != Some(false))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupLayer {
    #[serde(flatten)]
    pub base: LayerBase,
    /// Paint order: `children[0]` is the bottom layer.
    pub children: Vec<Layer>,
    /// True: children blend straight into what is below the group. False: the group is isolated.
    pub pass_through: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PixelLayer {
    #[serde(flatten)]
    pub base: LayerBase,
    pub asset: AssetId,
    pub transform: Mat3,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Fill {
    /// The only fill the survey found.
    Solid { color: Color },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StrokeAlign {
    Center,
    Inside,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShapeStroke {
    pub color: Color,
    pub width: f64,
    pub align: StrokeAlign,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShapeLayer {
    #[serde(flatten)]
    pub base: LayerBase,
    pub path: Path,
    pub fill: Fill,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<ShapeStroke>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TextRun {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// PostScript name of the font for this run, if not the layer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font size in pixels for this run, if not the layer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    /// Color for this run, if not the layer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// Tracking for this run, if not the layer's, in thousandths of the font size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<f64>,
    /// Set on runs that draw (part of) a card symbol, e.g. "{W}". The run's
    /// text is the symbol font's characters; this records what they stand for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

/// Settings for one paragraph that differ from the layer's style. Lengths are
/// in pixels and do not shrink when the text is fitted to its box.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphStyle {
    /// Space above this paragraph, instead of the style's `paragraphSpacing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_before: Option<f64>,
    /// Left indent of every line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent: Option<f64>,
    /// Added to the indent of the first line. Negative for a hanging indent, as bullet lists use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_line_indent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<TextAlign>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct RichText {
    pub runs: Vec<TextRun>,
    /// Per-paragraph settings, in order. Paragraphs are separated by "\n" in
    /// the runs' text; ones past the end of this list use the layer's style.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paragraphs: Vec<ParagraphStyle>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    pub font: String,
    pub size: f64,
    pub color: Color,
    /// Line height as a multiple of size. Omit for the font's natural leading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leading: Option<f64>,
    /// Extra space after every character, in thousandths of the font size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking: Option<f64>,
    /// Horizontal stretch of the glyphs. Default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_x: Option<f64>,
    /// Extra space above each paragraph after the first, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paragraph_spacing: Option<f64>,
    pub align: TextAlign,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FitMode {
    None,
    ShrinkHeight,
    ShrinkWidth,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VerticalAlign {
    Top,
    Center,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextLayer {
    #[serde(flatten)]
    pub base: LayerBase,
    /// For box text, the box the text wraps and is fitted in. For point text,
    /// the area it may occupy when a fit mode is set.
    #[serde(rename = "box")]
    pub bounds: Rect,
    /// Present for point text: where the first line's baseline starts (or its
    /// center or end, by alignment). Point text does not wrap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Vec2>,
    pub content: RichText,
    pub style: TextStyle,
    pub fit: FitMode,
    /// How the size is searched when shrinking: down from the layer's size in
    /// steps of this many pixels until the text fits, then half a step back up
    /// if that still fits (Proxyshop's search, which is what card renders are
    /// compared against). Without it, the largest size that fits is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit_step: Option<f64>,
    /// Where box text sits in its box when it is shorter than the box. Default top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<VerticalAlign>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtFit {
    Cover,
    Contain,
    Manual,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtFrameLayer {
    #[serde(flatten)]
    pub base: LayerBase,
    pub frame_ref: Rect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetId>,
    pub transform: Mat3,
    pub fit: ArtFit,
}

/// Generated content (rule 4): always its own layer, never the user's art.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FillLayer {
    #[serde(flatten)]
    pub base: LayerBase,
    pub source: String,
    pub params: serde_json::Map<String, serde_json::Value>,
    pub asset: AssetId,
    pub transform: Mat3,
}

// No adjustment layers: the survey found none.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Layer {
    Group(GroupLayer),
    Pixel(PixelLayer),
    Shape(ShapeLayer),
    Text(TextLayer),
    Art(ArtFrameLayer),
    Fill(FillLayer),
}

impl Layer {
    pub fn base(&self) -> &LayerBase {
        match self {
            Layer::Group(l) => &l.base,
            Layer::Pixel(l) => &l.base,
            Layer::Shape(l) => &l.base,
            Layer::Text(l) => &l.base,
            Layer::Art(l) => &l.base,
            Layer::Fill(l) => &l.base,
        }
    }

    pub fn base_mut(&mut self) -> &mut LayerBase {
        match self {
            Layer::Group(l) => &mut l.base,
            Layer::Pixel(l) => &mut l.base,
            Layer::Shape(l) => &mut l.base,
            Layer::Text(l) => &mut l.base,
            Layer::Art(l) => &mut l.base,
            Layer::Fill(l) => &mut l.base,
        }
    }

    pub fn id(&self) -> &str {
        &self.base().id
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Layer::Group(_) => "group",
            Layer::Pixel(_) => "pixel",
            Layer::Shape(_) => "shape",
            Layer::Text(_) => "text",
            Layer::Art(_) => "art",
            Layer::Fill(_) => "fill",
        }
    }

    pub fn children(&self) -> &[Layer] {
        match self {
            Layer::Group(g) => &g.children,
            _ => &[],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Image,
    Svg,
    Font,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AssetRef {
    pub kind: AssetKind,
    /// Path inside the save file or template package, e.g. "assets/frame.png".
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub dpi: f64,
    /// The root group. In JSON it carries `"kind": "group"` like any other layer.
    #[serde(serialize_with = "root_out", deserialize_with = "root_in")]
    pub root: GroupLayer,
    pub assets: BTreeMap<AssetId, AssetRef>,
    /// Scryfall-shaped card JSON, present when produced by the card engine. Typed in Phase 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
}

fn root_out<S: Serializer>(root: &GroupLayer, s: S) -> std::result::Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "lowercase")]
    enum Tagged<'a> {
        Group(&'a GroupLayer),
    }
    Tagged::Group(root).serialize(s)
}

fn root_in<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<GroupLayer, D::Error> {
    match Layer::deserialize(d)? {
        Layer::Group(g) => Ok(g),
        other => Err(serde::de::Error::custom(format!("root must be a group, not a {} layer", other.kind()))),
    }
}

impl Doc {
    pub fn empty(width: u32, height: u32, dpi: f64) -> Doc {
        let mut base = LayerBase::new("root");
        base.name = "Root".to_string();
        Doc {
            version: 1,
            width,
            height,
            dpi,
            root: GroupLayer { base, children: Vec::new(), pass_through: true },
            assets: BTreeMap::new(),
            card: None,
            template_id: None,
        }
    }
}

/// Small constructors so hand-written documents and tests stay readable.
pub mod build {
    use super::*;

    pub fn group(id: &str, pass_through: bool, children: Vec<Layer>) -> Layer {
        Layer::Group(GroupLayer { base: LayerBase::new(id), children, pass_through })
    }

    pub fn pixel(id: &str, asset: &str, transform: Mat3) -> Layer {
        Layer::Pixel(PixelLayer { base: LayerBase::new(id), asset: asset.to_string(), transform })
    }

    pub fn shape(id: &str, d: &str, color: Color) -> Layer {
        Layer::Shape(ShapeLayer {
            base: LayerBase::new(id),
            path: Path { d: d.to_string(), fill_rule: None },
            fill: Fill::Solid { color },
            stroke: None,
        })
    }

    /// SVG path for an axis-aligned rectangle.
    pub fn rect_path(x: f64, y: f64, w: f64, h: f64) -> String {
        format!("M{x} {y}H{}V{}H{x}Z", x + w, y + h)
    }

    /// Apply changes to a layer's shared properties: `with(shape(..), |b| b.opacity = 0.5)`.
    pub fn with(mut layer: Layer, edit: impl FnOnce(&mut LayerBase)) -> Layer {
        edit(layer.base_mut());
        layer
    }
}
