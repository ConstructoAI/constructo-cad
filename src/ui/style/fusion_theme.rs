//! The Fusion theme pair: black chrome around a white canvas, and its
//! inverse.
//!
//! "Mixed black/white" is about the split, not about draining every colour
//! out of the UI. Ribbon, docks, status bar and command line sit at
//! near-black while model space stays paper-white, so the drawing is the
//! only bright thing on screen. One accent survives that split: selection
//! highlight defaults to the theme primary
//! (`ModelSpaceThemeConfig::selection_highlight_color` = 0), and a white
//! primary would be invisible against a white canvas. The accent is
//! therefore a saturated blue that reads against both halves.
//!
//! These are `iced::Theme::Custom` values, so they are absent from
//! `iced::Theme::ALL`. Anything enumerating themes must go through
//! [`crate::app::config::all_themes`] instead.

use iced::theme::palette::Seed;
use iced::{Color, Theme};

/// Display name of the black-chrome theme. Persisted verbatim in
/// `settings.json` as `theme.name`, so changing it orphans a user's choice.
pub const FUSION_BLACK: &str = "Fusion Black";

/// Display name of the light-chrome theme.
pub const FUSION_WHITE: &str = "Fusion White";

/// Model-space canvas for [`FUSION_BLACK`] — paper white against the
/// near-black chrome. Deliberately not pure #FFFFFF: a fraction below white
/// keeps the paper-space sheet distinguishable from the model canvas.
pub const FUSION_BLACK_CANVAS: [u8; 3] = [250, 250, 250];

/// Model-space canvas for [`FUSION_WHITE`].
pub const FUSION_WHITE_CANVAS: [u8; 3] = [255, 255, 255];

fn rgb(hex: u32) -> Color {
    Color::from_rgb8(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    )
}

/// Near-black chrome, white text, blue accent.
///
/// FORK CONSTRUCTO : l'accent d'amont est CONSERVE, apres l'avoir change et
/// remis. Mesure du 2026-09-19 — sur le fond du theme (#1A1A1A), `#0696D7`
/// donne **5,26:1** ; le `#0078D4` de la charte de l'ERP, **3,84:1**. Sous la
/// barre de 4,5 que `ui/command_line.rs:1100-1105` et `ui/icons.rs:594-599`
/// interrogent, l'accent n'est pas seulement plus terne : il **cesse d'etre
/// dessine**, remplace par son repli.
///
/// Les deux bleus sont de la meme famille et se distinguent mal a l'oeil. Le
/// gain de charte etait donc nul, et le cout reel.
#[must_use]
pub fn fusion_black() -> Theme {
    Theme::custom(
        FUSION_BLACK,
        Seed {
            background: rgb(0x1A_1A1A),
            text: rgb(0xF2_F2F2),
            primary: rgb(0x06_96D7),
            success: rgb(0x4C_AF50),
            warning: rgb(0xFF_B300),
            danger: rgb(0xE5_3935),
        },
    )
}

/// The inverse: light chrome, near-black text. The accent darkens so it
/// keeps its contrast against a light surface.
///
/// FORK CONSTRUCTO — LA PHRASE ANGLAISE CI-DESSUS AVAIT RAISON, ET JE
/// L'AVAIS RETIREE. L'accent d'amont est conserve.
///
/// Ce que j'ai fait, et pourquoi c'etait faux : j'ai remplace cet accent par le
/// bleu de la charte de l'ERP (#0078D4) apres avoir mesure son contraste sur
/// **le canevas** — la seule surface que `fusion_accent_is_visible_on_the_canvas`
/// regarde, avec une barre de 3,0. Le chiffre etait juste (4,53:1) ; la
/// conclusion ne l'etait pas, parce que j'avais mesure la surface la plus
/// flatteuse et aucune de celles qui decident.
///
/// CE QUE LE RETEST A MESURE, sur le vrai generateur de palette d'iced :
///
/// | surface | #0277BD (amont) | #0078D4 (le mien) | barre |
/// |---|---:|---:|---:|
/// | canevas (#FFFFFF) | 4,80 | 4,53 | 3,0 |
/// | **fond du theme (#FAFAFA)** | **4,60** | **4,34** | **4,5** |
///
/// Sous 4,5, l'accent n'est pas « un peu moins contraste » : il **cesse d'etre
/// dessine**, remplace par son repli.
///   - `ui/command_line.rs:1100-1105` — les lignes « Info » de la ligne de
///     commande perdaient leur bleu et prenaient la couleur du texte ordinaire,
///     devenant indistinguables des lignes de commande.
///   - `ui/icons.rs:594-599` — la coche des cellules basculait au quasi-noir.
///
/// UNE MESURE SE PUBLIE AVEC SA SURFACE. Ce que ce fork garde de la charte,
/// c'est le theme CLAIR par defaut — le changement que l'utilisateur voit. Pas
/// une teinte a deux pas de celle d'amont, payee d'un accent qui disparait.
#[must_use]
pub fn fusion_white() -> Theme {
    Theme::custom(
        FUSION_WHITE,
        Seed {
            background: rgb(0xFA_FAFA),
            text: rgb(0x1A_1A1A),
            primary: rgb(0x02_77BD),
            success: rgb(0x2E_7D32),
            warning: rgb(0xE6_5100),
            danger: rgb(0xC6_2828),
        },
    )
}

/// Both Fusion themes, in the order the theme picker should show them.
#[must_use]
pub fn fusion_themes() -> Vec<Theme> {
    vec![fusion_black(), fusion_white()]
}

/// The canvas colour a Fusion theme asks for, or `None` for any other theme.
/// Keyed by name because `Theme::Custom` carries no discriminant to match on.
#[must_use]
pub fn fusion_canvas(theme: &Theme) -> Option<[u8; 3]> {
    match theme.to_string().as_str() {
        FUSION_BLACK => Some(FUSION_BLACK_CANVAS),
        FUSION_WHITE => Some(FUSION_WHITE_CANVAS),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::common::wcag_contrast;

    /// The same bar `theme_accessibility_tests` holds the built-in themes to.
    /// These themes are hand-written rather than generated, so nothing else
    /// would catch a palette typo that drops text below AA.
    #[test]
    fn fusion_themes_meet_wcag_aa_on_every_surface() {
        for theme in fusion_themes() {
            let p = theme.palette();
            for (name, pair) in [
                ("base", p.background.base),
                ("weak", p.background.weak),
                ("strong", p.background.strong),
                ("weakest", p.background.weakest),
            ] {
                let contrast = wcag_contrast(pair.text, pair.color);
                assert!(
                    contrast >= 4.5,
                    "{theme} {name} surface text is {contrast:.2}:1, below AA"
                );
            }
        }
    }

    /// FORK CONSTRUCTO — LE BANC QUI MANQUAIT, ET QUI AURAIT ATTRAPE LA
    /// REGRESSION DU 2026-09-19.
    ///
    /// `fusion_accent_is_visible_on_the_canvas`, juste en dessous, ne regarde
    /// que le CANEVAS, avec une barre de 3,0. C'est la seule surface qu'il
    /// mesure, et c'est sur elle que #0078D4 a ete valide (4,53:1) — puis pose,
    /// alors qu'il tombe a **4,34** contre le fond du theme.
    ///
    /// Or deux surfaces de production interrogent le seuil de **4,5** :
    ///   - `ui/command_line.rs:1100-1105`, pour les lignes « Info » ;
    ///   - `ui/icons.rs:594-599`, pour la coche des cellules.
    /// Sous ce seuil elles ne prennent pas un bleu plus terne : elles basculent
    /// sur un REPLI, donc l'accent cesse d'etre dessine. Les lignes « Info »
    /// devenaient indistinguables des lignes de commande.
    ///
    /// Une mesure se publie avec sa surface. Celle-ci tient la surface que le
    /// produit interroge vraiment, pas celle qui donnait le chiffre le plus
    /// flatteur.
    #[test]
    fn fusion_accent_survives_the_threshold_production_asks_for() {
        for theme in fusion_themes() {
            let p = theme.palette();
            let contrast = wcag_contrast(p.primary.base.color, p.background.base.color);
            assert!(
                contrast >= 4.5,
                "{theme} accent is {contrast:.2}:1 on the theme background, below \
                 the 4.5 that command_line.rs and icons.rs require — the accent \
                 would be silently replaced by its fallback"
            );
        }
    }

    /// The accent exists so selection stays visible on the white canvas. If
    /// it ever drifts toward white this catches it.
    #[test]
    fn fusion_accent_is_visible_on_the_canvas() {
        for (theme, canvas) in [
            (fusion_black(), FUSION_BLACK_CANVAS),
            (fusion_white(), FUSION_WHITE_CANVAS),
        ] {
            let canvas = Color::from_rgb8(canvas[0], canvas[1], canvas[2]);
            let contrast = wcag_contrast(theme.palette().primary.base.color, canvas);
            assert!(
                contrast >= 3.0,
                "{theme} accent is {contrast:.2}:1 on its canvas, too faint \
                 to show a selection"
            );
        }
    }

    #[test]
    fn fusion_canvas_is_keyed_by_name() {
        assert_eq!(fusion_canvas(&fusion_black()), Some(FUSION_BLACK_CANVAS));
        assert_eq!(fusion_canvas(&fusion_white()), Some(FUSION_WHITE_CANVAS));
        assert_eq!(fusion_canvas(&Theme::Dark), None);
    }
}
