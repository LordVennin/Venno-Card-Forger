//! A small SVG reader, for set symbols and watermarks.
//!
//! It turns the drawing into shape layers of our own document: one per path,
//! in the SVG's order, with a solid color or a gradient and optionally an
//! outline. That keeps a placed symbol an ordinary, editable part of the card.
//!
//! It reads what the symbol artwork uses: paths, circles, ellipses and
//! rectangles; groups and transforms; fills and strokes given as attributes,
//! `style` or CSS classes; linear and radial gradients, including ones that
//! borrow their stops from another. It does not draw `<use>`, clip paths,
//! filters, masks, text or images: each of those found is listed in
//! [`SvgArt::unsupported`] so the caller can say so.

use cardsmith_core::doc::{Color, Layer};
use kurbo::{Affine, BezPath, PathEl, Point, Shape as _};
use serde_json::{json, Value};
use std::collections::HashMap;

type Attrs = HashMap<String, String>;

#[derive(Clone, Debug)]
enum Paint {
    Solid(Color),
    /// Stops along the line from `from` to `to`.
    Linear { stops: Vec<(f64, Color)>, from: Point, to: Point },
    /// Stops from `center` out to `radius`.
    Radial { stops: Vec<(f64, Color)>, center: Point, radius: f64 },
}

#[derive(Clone, Debug)]
struct Shape {
    /// In the drawing's own coordinates, transforms applied.
    path: BezPath,
    fill: Option<Paint>,
    even_odd: bool,
    opacity: f64,
    /// Color and width of an outline.
    stroke: Option<(Color, f64)>,
}

/// A drawing read from SVG text.
#[derive(Clone, Debug)]
pub struct SvgArt {
    pub width: f64,
    pub height: f64,
    shapes: Vec<Shape>,
    /// Kinds of element or feature that were skipped, each named once.
    pub unsupported: Vec<String>,
}

// ---- reading the markup ---------------------------------------------------------

enum Token<'a> {
    Open { name: &'a str, attrs: Attrs, closed: bool },
    Close(&'a str),
    Text(&'a str),
}

/// Split markup into tags and the text between them. Comments, declarations and CDATA wrappers are dropped.
fn tokens(text: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        if !rest[..start].trim().is_empty() {
            out.push(Token::Text(&rest[..start]));
        }
        rest = &rest[start..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let end = after.find("]]>").unwrap_or(after.len());
            out.push(Token::Text(&after[..end]));
            rest = after.get(end + 3..).unwrap_or("");
            continue;
        }
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[1..end];
        rest = &rest[end + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            out.push(Token::Close(name.trim()));
            continue;
        }
        let closed = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
        let mut attrs = Attrs::new();
        let mut body = &tag[name_end..];
        // name="value" pairs, with either kind of quote.
        while let Some(eq) = body.find('=') {
            let key = body[..eq].trim();
            let after = body[eq + 1..].trim_start();
            let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
            let Some(close) = after[1..].find(quote) else { break };
            attrs.insert(key.to_string(), after[1..1 + close].to_string());
            body = &after[close + 2..];
        }
        out.push(Token::Open { name: &tag[..name_end], attrs, closed });
    }
    out
}

fn number(text: &str) -> Option<f64> {
    let text = text.trim();
    let end = text.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(text.len());
    text[..end].parse().ok()
}

/// A length that may be a percentage of `whole`.
fn length(text: &str, whole: f64) -> Option<f64> {
    let value = number(text)?;
    Some(if text.trim().ends_with('%') { value / 100.0 * whole } else { value })
}

fn numbers(text: &str) -> Vec<f64> {
    text.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|n| n.parse().ok()).collect()
}

fn color(text: &str) -> Option<Color> {
    let text = text.trim().to_lowercase();
    let rgb = |r: u8, g: u8, b: u8| Some([r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 1.0]);
    if let Some(hex) = text.strip_prefix('#') {
        let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
        let pair = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        return match hex.len() {
            3 | 4 => rgb(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17),
            6 | 8 => rgb(pair(0)?, pair(2)?, pair(4)?),
            _ => None,
        };
    }
    if let Some(inner) = text.strip_prefix("rgb(").and_then(|t| t.strip_suffix(')')) {
        let parts: Vec<f64> = inner.split(',').filter_map(|p| length(p, 255.0)).collect();
        return match parts[..] {
            [r, g, b] => rgb(r.clamp(0.0, 255.0) as u8, g.clamp(0.0, 255.0) as u8, b.clamp(0.0, 255.0) as u8),
            _ => None,
        };
    }
    match text.as_str() {
        "black" | "currentcolor" => rgb(0, 0, 0),
        "white" => rgb(255, 255, 255),
        "gray" | "grey" => rgb(128, 128, 128),
        "silver" => rgb(192, 192, 192),
        "red" => rgb(255, 0, 0),
        "green" => rgb(0, 128, 0),
        "blue" => rgb(0, 0, 255),
        "yellow" => rgb(255, 255, 0),
        _ => None,
    }
}

/// An SVG `transform` list as one matrix.
fn transform(text: &str) -> Affine {
    let mut out = Affine::IDENTITY;
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else { break };
        let n = numbers(&rest[open + 1..open + close]);
        rest = &rest[open + close + 1..];
        let step = match (name, n.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine::new([*a, *b, *c, *d, *e, *f]),
            ("translate", [x]) => Affine::translate((*x, 0.0)),
            ("translate", [x, y]) => Affine::translate((*x, *y)),
            ("scale", [s]) => Affine::scale(*s),
            ("scale", [x, y]) => Affine::scale_non_uniform(*x, *y),
            ("rotate", [a]) => Affine::rotate(a.to_radians()),
            ("rotate", [a, x, y]) => Affine::rotate_about(a.to_radians(), Point::new(*x, *y)),
            ("skewX", [a]) => Affine::new([1.0, 0.0, a.to_radians().tan(), 1.0, 0.0, 0.0]),
            ("skewY", [a]) => Affine::new([1.0, a.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => Affine::IDENTITY,
        };
        out *= step;
    }
    out
}

/// `a:b;c:d` declarations.
fn declarations(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.split(';').filter_map(|pair| pair.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
}

// ---- gradients and styles -------------------------------------------------------

#[derive(Default, Clone)]
struct RawGradient {
    radial: bool,
    attrs: Attrs,
    stops: Vec<(f64, Color)>,
}

/// What an element is drawn with, after inheritance.
#[derive(Clone)]
struct Style {
    props: Attrs,
    /// Opacity of every group above, multiplied together.
    opacity: f64,
    matrix: Affine,
}

/// Properties a child takes from its parent when it does not set them.
const INHERITED: [&str; 8] = ["fill", "fill-rule", "fill-opacity", "stroke", "stroke-width", "stroke-opacity", "display", "visibility"];
const OWN: [&str; 9] = ["fill", "fill-rule", "fill-opacity", "stroke", "stroke-width", "stroke-opacity", "display", "visibility", "opacity"];

struct Reader {
    gradients: HashMap<String, RawGradient>,
    classes: HashMap<String, Attrs>,
    unsupported: Vec<String>,
}

impl Reader {
    fn skipped(&mut self, what: &str) {
        if !self.unsupported.iter().any(|u| u == what) {
            self.unsupported.push(what.to_string());
        }
    }

    /// An element's style: its parent's, then its attributes, its classes, and its `style`.
    fn style(&self, parent: &Style, attrs: &Attrs) -> Style {
        let mut props: Attrs = parent.props.iter().filter(|(k, _)| INHERITED.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        for key in OWN {
            if let Some(value) = attrs.get(key) {
                props.insert(key.to_string(), value.clone());
            }
        }
        for class in attrs.get("class").map(|c| c.split_whitespace().collect::<Vec<_>>()).unwrap_or_default() {
            if let Some(rules) = self.classes.get(class) {
                props.extend(rules.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        if let Some(style) = attrs.get("style") {
            props.extend(declarations(style));
        }
        let own = props.remove("opacity").and_then(|o| number(&o)).unwrap_or(1.0);
        Style { props, opacity: parent.opacity * own, matrix: parent.matrix * attrs.get("transform").map_or(Affine::IDENTITY, |t| transform(t)) }
    }

    /// A gradient by id, with whatever it leaves unset taken from the one it refers to.
    fn gradient(&self, id: &str, depth: usize) -> Option<RawGradient> {
        let mut own = self.gradients.get(id)?.clone();
        let parent = own.attrs.get("xlink:href").or(own.attrs.get("href")).and_then(|h| h.strip_prefix('#')).filter(|_| depth < 8).and_then(|h| self.gradient(h, depth + 1));
        if let Some(parent) = parent {
            if own.stops.is_empty() {
                own.stops = parent.stops;
            }
            for (key, value) in parent.attrs {
                own.attrs.entry(key).or_insert(value);
            }
        }
        Some(own)
    }

    fn paint(&mut self, value: &str, opacity: f64, path: &BezPath, matrix: Affine) -> Option<Paint> {
        let value = value.trim();
        if value == "none" || value == "transparent" {
            return None;
        }
        let fade = |c: Color| [c[0], c[1], c[2], c[3] * opacity];
        let Some(id) = value.strip_prefix("url(").and_then(|v| v.split(')').next()).map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').trim_start_matches('#')) else {
            return color(value).map(|c| Paint::Solid(fade(c)));
        };
        let Some(gradient) = self.gradient(id, 0).filter(|g| !g.stops.is_empty()) else {
            self.skipped("a fill that refers to something missing");
            return None;
        };
        let stops: Vec<(f64, Color)> = gradient.stops.iter().map(|(at, c)| (*at, fade(*c))).collect();
        // Coordinates are in the drawing's space, or fractions of the shape's own box.
        let boxed = gradient.attrs.get("gradientUnits").is_none_or(|u| u != "userSpaceOnUse");
        let b = path.bounding_box();
        let space = if boxed { Affine::new([b.width(), 0.0, 0.0, b.height(), b.x0, b.y0]) } else { Affine::IDENTITY };
        let total = matrix * space * gradient.attrs.get("gradientTransform").map_or(Affine::IDENTITY, |t| transform(t));
        let get = |key: &str, default: f64| gradient.attrs.get(key).and_then(|v| length(v, 1.0)).unwrap_or(default);
        Some(if gradient.radial {
            let center = total * Point::new(get("cx", 0.5), get("cy", 0.5));
            // A stretched circle is drawn as a round one of the same area.
            Paint::Radial { stops, center, radius: get("r", 0.5) * total.determinant().abs().sqrt() }
        } else {
            Paint::Linear { stops, from: total * Point::new(get("x1", 0.0), get("y1", 0.0)), to: total * Point::new(get("x2", 1.0), get("y2", 0.0)) }
        })
    }

    fn shape(&mut self, name: &str, attrs: &Attrs, parent: &Style) -> Option<Shape> {
        let style = self.style(parent, attrs);
        if style.props.get("display").is_some_and(|d| d == "none") || style.props.get("visibility").is_some_and(|v| v == "hidden") {
            return None;
        }
        let get = |key: &str| attrs.get(key).and_then(|v| number(v)).unwrap_or(0.0);
        let own = match name {
            "path" => BezPath::from_svg(attrs.get("d")?).ok()?,
            "circle" => kurbo::Circle::new((get("cx"), get("cy")), get("r")).to_path(0.01),
            "ellipse" => kurbo::Ellipse::new((get("cx"), get("cy")), (get("rx"), get("ry")), 0.0).to_path(0.01),
            "rect" => kurbo::RoundedRect::new(get("x"), get("y"), get("x") + get("width"), get("y") + get("height"), get("rx").max(get("ry"))).to_path(0.01),
            _ => return None,
        };
        if own.elements().is_empty() {
            return None;
        }
        let prop = |key: &str| style.props.get(key).cloned();
        let opacity = |key: &str| prop(key).and_then(|o| number(&o)).unwrap_or(1.0);
        let fill = self.paint(&prop("fill").unwrap_or_else(|| "black".into()), opacity("fill-opacity"), &own, style.matrix);
        let stroke = prop("stroke").filter(|s| s != "none").and_then(|s| {
            let width = prop("stroke-width").and_then(|w| number(&w)).unwrap_or(1.0) * style.matrix.determinant().abs().sqrt();
            // Gradient strokes are drawn in their first color.
            let color = match self.paint(&s, opacity("stroke-opacity"), &own, style.matrix)? {
                Paint::Solid(c) => c,
                Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => stops.first()?.1,
            };
            (width > 0.0).then_some((color, width))
        });
        if fill.is_none() && stroke.is_none() {
            return None;
        }
        let mut path = own;
        path.apply_affine(style.matrix);
        Some(Shape { path, fill, even_odd: prop("fill-rule").is_some_and(|r| r == "evenodd"), opacity: style.opacity, stroke })
    }
}

/// Read a drawing from SVG text.
pub fn parse_svg(text: &str) -> Result<SvgArt, String> {
    let tokens = tokens(text);
    let mut reader = Reader { gradients: HashMap::new(), classes: HashMap::new(), unsupported: Vec::new() };

    // First the things that can be referred to before they appear: gradients and CSS classes.
    let mut current: Option<String> = None;
    let mut in_style = false;
    for token in &tokens {
        match token {
            Token::Open { name, attrs, closed } if matches!(*name, "linearGradient" | "radialGradient") => {
                let id = attrs.get("id").cloned().unwrap_or_default();
                reader.gradients.insert(id.clone(), RawGradient { radial: *name == "radialGradient", attrs: attrs.clone(), stops: Vec::new() });
                current = (!closed).then_some(id);
            }
            Token::Open { name: "stop", attrs, .. } => {
                let mut props = attrs.clone();
                props.extend(attrs.get("style").map(|s| declarations(s).collect::<Vec<_>>()).unwrap_or_default());
                let at = props.get("offset").and_then(|o| length(o, 1.0)).unwrap_or(0.0).clamp(0.0, 1.0);
                let mut c = props.get("stop-color").and_then(|c| color(c)).unwrap_or([0.0, 0.0, 0.0, 1.0]);
                c[3] = props.get("stop-opacity").and_then(|o| number(o)).unwrap_or(1.0).clamp(0.0, 1.0);
                if let Some(gradient) = current.as_ref().and_then(|id| reader.gradients.get_mut(id)) {
                    gradient.stops.push((at, c));
                }
            }
            Token::Close(name) if matches!(*name, "linearGradient" | "radialGradient") => current = None,
            Token::Open { name: "style", closed: false, .. } => in_style = true,
            Token::Close("style") => in_style = false,
            // `.a,.b{fill:#fff}` rules; anything fancier than class selectors is ignored.
            Token::Text(css) if in_style => {
                for rule in css.split('}') {
                    let Some((selectors, body)) = rule.split_once('{') else { continue };
                    for selector in selectors.split(',') {
                        if let Some(class) = selector.trim().strip_prefix('.') {
                            reader.classes.entry(class.to_string()).or_default().extend(declarations(body));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Then the drawing itself.
    let mut size = (0.0, 0.0);
    let mut shapes = Vec::new();
    let mut stack: Vec<(Style, &str)> = Vec::new();
    // Inside one of these nothing is drawn directly.
    let mut hidden: Vec<&str> = Vec::new();
    let mut seen_root = false;
    for token in &tokens {
        match token {
            Token::Open { name, attrs, closed } => {
                let name = *name;
                if matches!(name, "defs" | "clipPath" | "mask" | "filter" | "symbol" | "pattern" | "marker" | "foreignObject" | "linearGradient" | "radialGradient" | "style" | "title" | "desc" | "metadata") {
                    if matches!(name, "clipPath" | "mask" | "filter" | "pattern") {
                        reader.skipped(name);
                    }
                    if !closed {
                        hidden.push(name);
                    }
                    continue;
                }
                if !hidden.is_empty() {
                    continue;
                }
                let root = Style { props: Attrs::new(), opacity: 1.0, matrix: Affine::IDENTITY };
                let parent = stack.last().map_or(&root, |(style, _)| style);
                match name {
                    "svg" if !seen_root => {
                        seen_root = true;
                        let view = attrs.get("viewBox").map(|v| numbers(v)).filter(|v| v.len() == 4);
                        let (w, h) = (attrs.get("width").and_then(|v| number(v)), attrs.get("height").and_then(|v| number(v)));
                        let mut style = reader.style(parent, attrs);
                        // The drawing's coordinates are the view box's, moved to start at zero.
                        if let Some(view) = &view {
                            size = (view[2], view[3]);
                            style.matrix = Affine::translate((-view[0], -view[1]));
                        } else {
                            size = (w.unwrap_or(0.0), h.unwrap_or(0.0));
                        }
                        if !closed {
                            stack.push((style, name));
                        }
                    }
                    "g" | "svg" | "switch" | "a" => {
                        if !closed {
                            stack.push((reader.style(parent, attrs), name));
                        }
                    }
                    "path" | "circle" | "ellipse" | "rect" => {
                        let parent = parent.clone();
                        shapes.extend(reader.shape(name, attrs, &parent));
                    }
                    other => reader.skipped(other),
                }
            }
            Token::Close(name) => {
                if hidden.last() == Some(name) {
                    hidden.pop();
                } else if stack.last().is_some_and(|(_, open)| open == name) {
                    stack.pop();
                }
            }
            Token::Text(_) => {}
        }
    }
    if !seen_root {
        return Err("This is not an SVG drawing.".into());
    }
    if size.0 <= 0.0 || size.1 <= 0.0 {
        // No stated size: the extent of what was drawn.
        let all = shapes.iter().map(|s: &Shape| s.path.bounding_box()).reduce(|a, b| a.union(b)).ok_or("The SVG drawing is empty.")?;
        size = (all.x1, all.y1);
    }
    if shapes.is_empty() {
        return Err("The SVG drawing has nothing in it that can be drawn.".into());
    }
    Ok(SvgArt { width: size.0, height: size.1, shapes, unsupported: reader.unsupported })
}

// ---- into layers ------------------------------------------------------------------

/// A path as absolute SVG commands.
pub(crate) fn path_data(path: &BezPath) -> String {
    let p = |point: Point| format!("{} {}", (point.x * 1000.0).round() / 1000.0, (point.y * 1000.0).round() / 1000.0);
    path.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(a) => format!("M{}", p(a)),
            PathEl::LineTo(a) => format!("L{}", p(a)),
            PathEl::QuadTo(a, b) => format!("Q{} {}", p(a), p(b)),
            PathEl::CurveTo(a, b, c) => format!("C{} {} {}", p(a), p(b), p(c)),
            PathEl::ClosePath => "Z".to_string(),
        })
        .collect()
}

/// The box the core measures a shape's gradient against: its points, control points included, on whole pixels.
fn layer_box(path: &BezPath, grow: f64) -> (f64, f64, f64, f64) {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut add = |point: Point| b = [b[0].min(point.x), b[1].min(point.y), b[2].max(point.x), b[3].max(point.y)];
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(a) | PathEl::LineTo(a) => add(a),
            PathEl::QuadTo(a, c) => [a, c].into_iter().for_each(&mut add),
            PathEl::CurveTo(a, c, d) => [a, c, d].into_iter().for_each(&mut add),
            PathEl::ClosePath => {}
        }
    }
    let (x0, y0) = ((b[0] - grow).floor(), (b[1] - grow).floor());
    (x0, y0, (b[2] + grow).ceil() - x0, (b[3] + grow).ceil() - y0)
}

impl SvgArt {
    /// The drawing as shape layers, bottom first, placed in a document by `place`
    /// (a matrix `[a, b, c, d, e, f]` taking the drawing's coordinates to the
    /// document's). Layer ids start with `prefix`.
    pub fn layers(&self, place: [f64; 6], prefix: &str) -> Vec<Layer> {
        let place = Affine::new(place);
        let size = place.determinant().abs().sqrt();
        let mut layers = Vec::new();
        for (n, shape) in self.shapes.iter().enumerate() {
            let mut path = shape.path.clone();
            path.apply_affine(place);
            let stroke = shape.stroke.map(|(color, width)| (color, width * size));
            let mut layer = json!({
                "kind": "shape", "id": format!("{prefix}-{}", n + 1), "name": format!("Shape {}", n + 1), "visible": true, "opacity": shape.opacity, "blend": "normal", "clip": false,
                "path": { "d": path_data(&path) }, "fill": { "type": "solid", "color": [0, 0, 0, 0] }
            });
            if shape.even_odd {
                layer["path"]["fillRule"] = json!("evenodd");
            }
            if let Some((color, width)) = stroke {
                layer["stroke"] = json!({ "color": color, "width": width, "align": "center" });
            }
            let (bx, by, bw, bh) = layer_box(&path, stroke.map_or(0.0, |(_, w)| w / 2.0));
            let stops = |stops: &[(f64, Color)]| -> Value { stops.iter().map(|(at, c)| json!({ "offset": at, "color": c })).collect() };
            let offset = |at: Point| json!({ "x": (at.x - (bx + bw / 2.0)) / bw.max(1.0), "y": (at.y - (by + bh / 2.0)) / bh.max(1.0) });
            let overlay = |style: &str, list: &[(f64, Color)], angle: f64, scale: f64, at: Point| json!([{ "type": "gradientOverlay", "enabled": true, "blend": "normal", "opacity": 1, "gradient": { "stops": stops(list) }, "style": style, "angle": angle, "scale": scale, "offset": offset(at) }]);
            match &shape.fill {
                None => {}
                Some(Paint::Solid(color)) => layer["fill"]["color"] = json!(color),
                Some(Paint::Linear { stops: list, from, to }) => {
                    let (from, to) = (place * *from, place * *to);
                    let (dx, dy) = (to.x - from.x, to.y - from.y);
                    let len = dx.hypot(dy);
                    layer["fill"]["color"] = json!(list[0].1);
                    if len > 1e-9 {
                        // The core runs a gradient through the layer's box at an angle, edge to edge; `scale` stretches that to this line's length.
                        let reach = |extent: f64, along: f64| if along.abs() > 1e-9 { extent / along.abs() } else { f64::INFINITY };
                        let chord = reach(bw, dx / len).min(reach(bh, dy / len));
                        layer["effects"] = overlay("linear", list, (-dy).atan2(dx).to_degrees(), len / chord, from.midpoint(to));
                    }
                }
                Some(Paint::Radial { stops: list, center, radius }) => {
                    layer["fill"]["color"] = json!(list[0].1);
                    layer["effects"] = overlay("radial", list, 0.0, 2.0 * radius * size / bw.max(1.0), place * *center);
                }
            }
            if let Ok(layer) = serde_json::from_value(layer) {
                layers.push(layer);
            }
        }
        layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cardsmith_core::doc::Doc;
    use cardsmith_core::render::render;
    use cardsmith_core::serialize::AssetBytes;

    fn draw(svg: &str, place: [f64; 6], w: u32, h: u32) -> impl Fn(u32, u32) -> [u8; 4] {
        let art = parse_svg(svg).unwrap();
        let mut doc = Doc::empty(w, h, 72.0);
        doc.root.children = art.layers(place, "s");
        let r = render(&doc, &AssetBytes::new());
        move |x, y| {
            let i = ((y * r.width + x) * 4) as usize;
            [r.pixels[i], r.pixels[i + 1], r.pixels[i + 2], r.pixels[i + 3]]
        }
    }

    const ID: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

    #[test]
    fn paths_keep_their_order_colors_and_transforms() {
        let svg = r##"<?xml version="1.0"?><!-- a comment --><svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
            <path d="M0 0h40v20h-40z" fill="#ff0000"/>
            <g transform="translate(20 0)" style="fill:#00f"><path d="m0 0 10 0 0 10-10 0z"/><rect x="10" y="10" width="10" height="10" style="fill:lime;fill:#0f0"/></g>
            <circle cx="5" cy="5" r="4" fill="white" opacity=".5"/><path d="M0 0h1v1z" fill="none"/></svg>"##;
        let art = parse_svg(svg).unwrap();
        assert_eq!((art.width, art.height, art.shapes.len()), (40.0, 20.0, 4));
        let px = draw(svg, ID, 40, 20);
        assert_eq!(px(10, 15), [255, 0, 0, 255]);
        assert_eq!(px(25, 5), [0, 0, 255, 255]);
        assert_eq!(px(35, 15), [0, 255, 0, 255]);
        // Half-opaque white over red.
        assert_eq!(px(5, 5), [255, 128, 128, 255]);
        // Placed at twice the size, 4 px in.
        let px = draw(svg, [2.0, 0.0, 0.0, 2.0, 4.0, 0.0], 90, 40);
        assert_eq!((px(2, 10)[3], px(54, 10), px(74, 30)), (0, [0, 0, 255, 255], [0, 255, 0, 255]));
    }

    #[test]
    fn a_view_box_sets_the_size_and_classes_and_even_odd_fills_are_read() {
        let svg = r##"<svg viewBox="100 100 20 20" width="200" height="200"><defs><style>.a,.b{fill:#00ff00}.c{fill-rule:evenodd}</style></defs>
            <path class="a c" d="M100 100h20v20h-20zM105 105h10v10h-10z"/></svg>"##;
        let art = parse_svg(svg).unwrap();
        assert_eq!((art.width, art.height), (20.0, 20.0));
        let px = draw(svg, ID, 20, 20);
        assert_eq!((px(2, 2), px(10, 10)[3]), ([0, 255, 0, 255], 0));
    }

    #[test]
    fn linear_and_radial_gradients_land_where_the_drawing_puts_them() {
        // Left to right across x 10..30 of a 40-wide bar: red, then blue. One gradient borrows the other's stops.
        let svg = r##"<svg width="40" height="40"><defs>
            <linearGradient id="a"><stop offset="0" style="stop-color:#ff0000"/><stop offset="1" stop-color="#0000ff"/></linearGradient>
            <linearGradient xlink:href="#a" id="b" x1="10" x2="30" y1="0" y2="0" gradientUnits="userSpaceOnUse"/>
            <radialGradient id="r" cx="20" cy="30" r="5" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></radialGradient></defs>
            <path d="M0 0h40v20H0z" style="fill:url(#b)"/><path d="M0 20h40v20H0z" fill="url(#r)"/></svg>"##;
        let px = draw(svg, ID, 40, 40);
        assert_eq!((px(4, 10), px(36, 10)), ([255, 0, 0, 255], [0, 0, 255, 255]));
        let mid = px(20, 10);
        assert!((mid[0] as i32 - 128).abs() <= 10 && (mid[2] as i32 - 128).abs() <= 10, "{mid:?}");
        // White at the radial center, black from 5 px out.
        assert!(px(20, 30)[0] > 200 && px(27, 30)[0] < 8 && px(20, 37)[0] < 8, "{:?} {:?}", px(20, 30), px(27, 30));
        // Placed elsewhere and larger, the gradient moves and grows with the shape.
        let px = draw(svg, [2.0, 0.0, 0.0, 2.0, 10.0, 0.0], 100, 80);
        assert_eq!((px(18, 20), px(82, 20)), ([255, 0, 0, 255], [0, 0, 255, 255]));
        assert!(px(50, 60)[0] > 200 && px(64, 60)[0] < 8);
    }

    #[test]
    fn gradients_given_as_fractions_follow_the_shapes_own_box() {
        let svg = r##"<svg width="40" height="10"><linearGradient id="g"><stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff"/></linearGradient>
            <path d="M20 0h20v10h-20z" fill="url(#g)"/></svg>"##;
        let px = draw(svg, ID, 40, 10);
        assert!(px(21, 5)[0] < 20 && px(38, 5)[0] > 235, "{:?} {:?}", px(21, 5), px(38, 5));
    }

    #[test]
    fn strokes_are_outlines_and_what_cannot_be_drawn_is_named() {
        let svg = r##"<svg width="30" height="30"><path d="M5 5h20v20H5z" fill="none" stroke="#ff0000" stroke-width="4"/>
            <use xlink:href="#x"/><clipPath id="c"><path d="M0 0h5v5z"/></clipPath><text>hi</text></svg>"##;
        let art = parse_svg(svg).unwrap();
        assert_eq!(art.unsupported, ["use", "clipPath", "text"]);
        // The clip path's own path is not drawn.
        assert_eq!(art.shapes.len(), 1);
        let px = draw(svg, ID, 30, 30);
        assert_eq!((px(5, 15), px(15, 15)[3]), ([255, 0, 0, 255], 0));
        assert!(parse_svg("not svg at all").is_err());
        assert!(parse_svg("<svg width='5' height='5'></svg>").is_err());
    }
}
