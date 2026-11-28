// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Ported from Proxyshop src/data/manifest.yml (which template file draws
// which layouts, under which class) and the class list of src/templates.

//! Which of Proxyshop's template classes draws a card on a template.
//!
//! A Proxyshop template file can carry several looks ("Normal", "Fullart" and
//! "Stargazing" all live in `normal.psd`) and one look can draw several
//! layouts with different classes. The card engine follows the class: it
//! decides which layers are looked up and what is done with them.

use cardsmith_cards::layout::LayoutKind;

/// Proxyshop's template classes, as far as the card engine draws them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// `M15Template`: the normal frame, with Nyx and companion crowns.
    M15,
    Fullart,
    Stargazing,
    Extended,
    /// `InventionTemplate` (Kaladesh masterpieces).
    Invention,
    /// `ExpeditionTemplate` (Zendikar Rising expeditions).
    Expedition,
    Snow,
    Miracle,
    Classic,
    Etched,
    ClassicRemastered,
    ClassicModern,
    UniversesBeyond,
    Lotr,
    BorderlessVector,
    Transform,
    Mdfc,
    Ixalan,
    Mutate,
    AdventureVector,
    Leveler,
    SagaVector,
    ClassVector,
    UniversesBeyondSaga,
    UniversesBeyondClass,
    Split,
    Battle,
    UniversesBeyondBattle,
    Prototype,
    Planeswalker,
    PlaneswalkerBorderless,
    PlaneswalkerTransform,
    PlaneswalkerTransformBorderless,
    PlaneswalkerMdfc,
    PlaneswalkerMdfcBorderless,
    Token,
    Planar,
}

/// Where a layout's face sits: the front or back of a two-faced card, or either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Any,
    Front,
    Back,
}

/// One look a template file offers, and what it draws with which class.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub name: &'static str,
    pub draws: &'static [(LayoutKind, Face, Class)],
}

use Class::*;
use Face::*;
use LayoutKind as L;

/// The looks of a template file, by the file's name without `.psd`.
pub fn styles(file: &str) -> &'static [Style] {
    match file {
        "normal" => &[Style { name: "Normal", draws: &[(L::Normal, Any, M15)] }, Style { name: "Fullart", draws: &[(L::Normal, Any, Fullart)] }, Style { name: "Stargazing", draws: &[(L::Normal, Any, Stargazing)] }],
        "normal-classic" => &[Style { name: "Classic", draws: &[(L::Normal, Any, Classic)] }],
        "classic-remastered" => &[Style { name: "Classic Remastered", draws: &[(L::Normal, Any, ClassicRemastered), (L::Transform, Any, ClassicRemastered)] }],
        "classic-modern" => &[Style { name: "Classic Modern", draws: &[(L::Normal, Any, ClassicModern), (L::Transform, Any, ClassicModern), (L::ModalDoubleFaced, Any, ClassicModern)] }],
        "normal-extended" => &[Style { name: "Extended", draws: &[(L::Normal, Any, Extended)] }],
        "etched" => &[Style { name: "Etched", draws: &[(L::Normal, Any, Etched)] }],
        "masterpiece" => &[Style { name: "Invention", draws: &[(L::Normal, Any, Invention)] }],
        "znrexp" => &[Style { name: "Expedition (ZNR)", draws: &[(L::Normal, Any, Expedition)] }],
        "borderless-vector" => &[Style { name: "Borderless", draws: &[(L::Normal, Any, BorderlessVector), (L::Transform, Any, BorderlessVector), (L::ModalDoubleFaced, Any, BorderlessVector)] }],
        "miracle" => &[Style { name: "Miracle", draws: &[(L::Normal, Any, Miracle)] }],
        "snow" => &[Style { name: "Snow", draws: &[(L::Normal, Any, Snow)] }],
        "universes-beyond" => &[Style { name: "Universes Beyond", draws: &[(L::Normal, Any, UniversesBeyond), (L::Transform, Any, UniversesBeyond)] }],
        "lord-of-the-rings" => &[Style { name: "Lord of the Rings", draws: &[(L::Normal, Any, Lotr)] }],
        "tf-front" => &[Style { name: "Normal", draws: &[(L::Transform, Front, Transform)] }],
        "tf-back" => &[Style { name: "Normal", draws: &[(L::Transform, Back, Transform)] }],
        "mdfc-front" => &[Style { name: "Normal", draws: &[(L::ModalDoubleFaced, Front, Mdfc)] }],
        "mdfc-back" => &[Style { name: "Normal", draws: &[(L::ModalDoubleFaced, Back, Mdfc)] }],
        "ixalan" => &[Style { name: "Ixalan", draws: &[(L::Transform, Any, Ixalan)] }],
        "mutate" => &[Style { name: "Normal", draws: &[(L::Mutate, Any, Mutate)] }],
        "normal-vector" => &[Style { name: "Normal", draws: &[(L::Adventure, Any, AdventureVector)] }],
        "leveler" => &[Style { name: "Normal", draws: &[(L::Leveler, Any, Leveler)] }],
        "vertical-mode" => &[
            Style { name: "Normal", draws: &[(L::Saga, Any, SagaVector), (L::Class, Any, ClassVector)] },
            Style { name: "Universes Beyond", draws: &[(L::Saga, Any, UniversesBeyondSaga), (L::Class, Any, UniversesBeyondClass)] },
        ],
        "split" => &[Style { name: "Normal", draws: &[(L::Split, Any, Split)] }],
        "battle" => &[Style { name: "Normal", draws: &[(L::Battle, Any, Battle)] }, Style { name: "Universes Beyond", draws: &[(L::Battle, Any, UniversesBeyondBattle)] }],
        "prototype" => &[Style { name: "Normal", draws: &[(L::Prototype, Any, Prototype)] }],
        "pw" => &[Style { name: "Normal", draws: &[(L::Planeswalker, Any, Planeswalker)] }],
        "pw-extended" => &[Style { name: "Borderless", draws: &[(L::Planeswalker, Any, PlaneswalkerBorderless)] }],
        "pw-tf-front" => &[Style { name: "Normal", draws: &[(L::PlaneswalkerTransform, Front, PlaneswalkerTransform)] }],
        "pw-tf-front-extended" => &[Style { name: "Borderless", draws: &[(L::PlaneswalkerTransform, Front, PlaneswalkerTransformBorderless)] }],
        "pw-tf-back" => &[Style { name: "Normal", draws: &[(L::PlaneswalkerTransform, Back, PlaneswalkerTransform)] }],
        "pw-tf-back-extended" => &[Style { name: "Borderless", draws: &[(L::PlaneswalkerTransform, Back, PlaneswalkerTransformBorderless)] }],
        "pw-mdfc-front" => &[Style { name: "Normal", draws: &[(L::PlaneswalkerMdfc, Front, PlaneswalkerMdfc)] }],
        "pw-mdfc-front-extended" => &[Style { name: "Borderless", draws: &[(L::PlaneswalkerMdfc, Front, PlaneswalkerMdfcBorderless)] }],
        "pw-mdfc-back" => &[Style { name: "Normal", draws: &[(L::PlaneswalkerMdfc, Back, PlaneswalkerMdfc)] }],
        "pw-mdfc-back-extended" => &[Style { name: "Borderless", draws: &[(L::PlaneswalkerMdfc, Back, PlaneswalkerMdfcBorderless)] }],
        "token" => &[Style { name: "Chilli Token", draws: &[(L::Token, Any, Token)] }],
        "planar" => &[Style { name: "Normal", draws: &[(L::Planar, Any, Planar)] }],
        _ => &[],
    }
}

/// The class that draws a card of `layout` (front or back) on a template file,
/// in the named look or the first look that can.
/// `vector` says the importer found a vector frame, for a template file there is no record of.
pub fn class_for(file: &str, style: Option<&str>, layout: LayoutKind, front: bool, vector: bool) -> Result<Class, String> {
    let styles = styles(file);
    if styles.is_empty() {
        // A template we have no record of: treat it as the normal or borderless frame it most likely is.
        return Ok(if vector { BorderlessVector } else { M15 });
    }
    let fits = |s: &Style| s.draws.iter().find(|(l, face, _)| *l == layout && (*face == Any || (*face == Front) == front)).map(|(_, _, class)| *class);
    let wanted: Vec<&Style> = match style {
        Some(name) => styles.iter().filter(|s| s.name.eq_ignore_ascii_case(name)).collect(),
        None => styles.iter().collect(),
    };
    if let Some(class) = wanted.iter().find_map(|s| fits(s)) {
        return Ok(class);
    }
    // Normal cards drawn on a two-faced template's front: Proxyshop does not, so neither do we.
    let can: Vec<String> = styles.iter().flat_map(|s| s.draws.iter().map(|(l, face, _)| format!("{}{}", layout_name(*l), match face { Front => " (front)", Back => " (back)", Any => "" }))).collect();
    Err(format!("The \"{file}\" template does not draw {}{} cards; it draws {}.", if front || !(layout.is_transform() || layout.is_mdfc()) { "" } else { "the back of " }, layout_name(layout), can.join(", ")))
}

/// A layout as people call it.
pub fn layout_name(layout: LayoutKind) -> &'static str {
    match layout {
        L::Normal => "normal",
        L::Split => "split",
        L::Transform => "transform",
        L::ModalDoubleFaced => "modal double-faced",
        L::Leveler => "leveler",
        L::Class => "class",
        L::Saga => "saga",
        L::Adventure => "adventure",
        L::Mutate => "mutate",
        L::Prototype => "prototype",
        L::Battle => "battle",
        L::Planar => "planar",
        L::Token => "token",
        L::Planeswalker => "planeswalker",
        L::PlaneswalkerMdfc => "modal double-faced planeswalker",
        L::PlaneswalkerTransform => "transforming planeswalker",
    }
}

impl Class {
    /// Templates drawn with shapes recolored by the engine, rather than pre-drawn color layers.
    pub fn is_vector(self) -> bool {
        matches!(
            self,
            Etched | ClassicRemastered | ClassicModern | UniversesBeyond | Lotr | BorderlessVector | AdventureVector | SagaVector | ClassVector | UniversesBeyondSaga | UniversesBeyondClass | UniversesBeyondBattle
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_class_follows_the_file_the_look_and_the_face() {
        assert_eq!(class_for("normal", None, L::Normal, true, false), Ok(M15));
        assert_eq!(class_for("normal", Some("stargazing"), L::Normal, true, false), Ok(Stargazing));
        assert_eq!(class_for("tf-back", None, L::Transform, false, false), Ok(Transform));
        assert!(class_for("tf-back", None, L::Transform, true, false).is_err());
        assert_eq!(class_for("vertical-mode", Some("Universes Beyond"), L::Class, true, false), Ok(UniversesBeyondClass));
        assert_eq!(class_for("vertical-mode", None, L::Saga, true, false), Ok(SagaVector));
        assert!(class_for("pw", None, L::Normal, true, false).unwrap_err().contains("draws planeswalker"));
        // A template we know nothing about is taken for a normal one.
        assert_eq!(class_for("my-own-frame", None, L::Normal, true, false), Ok(M15));
        assert_eq!(class_for("my-own-frame", None, L::Normal, true, true), Ok(BorderlessVector));
    }
}
