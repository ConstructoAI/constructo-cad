//! Consolidated user configuration. Native builds use one grouped JSON file
//! (`<config>/OpenCADStudio/settings.json`); web builds keep the same JSON in
//! `localStorage`. It holds every app preference except the command aliases,
//! which use native `ocad.pgp` or a separate web storage key. Serialized via
//! serde so the data is structured and grouped, replacing the former scattered
//! flat stores (`settings.txt` / `recent.txt` / `recent_limit.txt` /
//! `statusbar.txt` / `ribbon.txt` / `plot.txt`).

use serde::{Deserialize, Serialize};

use super::settings::UserSettings;
use crate::ui::ribbon::CollapseMode;
use crate::ui::statusbar::statusbar_config::StatusBarConfig;
use crate::ui::window::plot::PlotDialogState;

/// The whole persisted config, grouped into top-level sections.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Input modes, backup, plugin lists, viewport background colours, …
    pub settings: UserSettings,
    /// Iced theme selection and the six base colours used by a custom theme.
    pub theme: UiThemeConfig,
    /// Recent-files list + retained count.
    pub recent: RecentConfig,
    /// Last selected section on the tabbed Start page.
    pub start: StartConfig,
    /// Which status-bar pills the user has hidden.
    pub statusbar: StatusBarConfig,
    /// General edge-stack dock layout (which panels are docked, side, order,
    /// width and auto-collapse) for the Properties panel and block palette.
    pub dock: crate::ui::dock::DockState,
    /// Add a newly selected annotation scale to existing annotative objects.
    pub annotation_auto_scale: i8,
    /// Ribbon collapse density.
    pub ribbon: RibbonConfig,
    /// Print dialog preferences (only the persisted fields; runtime state is
    /// skipped by `PlotDialogState`'s serde attributes).
    pub plot: PlotDialogState,
    /// Complete editable keyboard shortcut table.
    pub shortcuts: ShortcutConfig,
    /// Model space background, grid, and selection appearance.
    pub model_space: ModelSpaceThemeConfig,
    /// FORK CONSTRUCTO : numero de la derniere migration appliquee a CETTE
    /// config. Absent d'une config ecrite avant le fork, il vaut 0 — c'est
    /// precisement ce qui distingue une config ancienne d'une config neuve, et
    /// c'est tout son role.
    ///
    /// L'ATTRIBUT SUR LE CHAMP N'EST PAS REDONDANT AVEC CELUI DE LA STRUCT,
    /// IL L'ANNULE — et sans lui tout ce mecanisme est mort.
    ///
    /// `#[serde(default)]` pose sur le CONTENEUR (ligne 18) ne met pas le defaut
    /// du TYPE dans un champ absent : il construit `AppConfig::default()` et y
    /// PUISE le champ manquant. Ce defaut-la vaut `CONSTRUCTO_MIGRATION`, donc
    /// **1** — et `1 < 1` etant faux, la migration ne serait partie pour
    /// personne, exactement sur la population qu'elle vise.
    ///
    /// Mesure du 2026-09-19, quatre formes essayees sur une config d'avant le
    /// fork : conteneur seul -> 1 (mort) ; `Option<u32>` -> `Some(1)` (mort
    /// aussi) ; `Default` de la struct a 0 -> migre AUSSI les configs neuves ;
    /// attribut sur le CHAMP -> 0. Seule la derniere passe les trois epreuves.
    ///
    /// Le temoin qui tranche vit dans ce fichier : `ShortcutConfig` a le meme
    /// attribut de conteneur et un `Default` NON VIDE. Si serde mettait le
    /// defaut du type, toute config sans cle `shortcuts` demarrerait sans aucun
    /// raccourci clavier.
    #[serde(default)]
    pub constructo_migration: u32,
}

/// Derniere migration connue. `AppConfig::default()` la pose d'emblee : une
/// config neuve n'a rien a migrer.
pub const CONSTRUCTO_MIGRATION: u32 = 1;

/// Le theme par defaut d'AMONT — celui qu'une config d'avant le fork porte
/// quand personne n'a jamais choisi. C'est la seule valeur qu'il soit legitime
/// de remplacer : voir `migrer_constructo`.
const THEME_PAR_DEFAUT_AMONT: &str = "Oxocarbon";

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            settings: UserSettings::default(),
            theme: UiThemeConfig::default(),
            recent: RecentConfig::default(),
            start: StartConfig::default(),
            statusbar: StatusBarConfig::default(),
            dock: crate::ui::dock::DockState::default(),
            annotation_auto_scale: -4,
            ribbon: RibbonConfig::default(),
            plot: PlotDialogState::default(),
            shortcuts: ShortcutConfig::default(),
            model_space: ModelSpaceThemeConfig::default(),
            constructo_migration: CONSTRUCTO_MIGRATION,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutConfig {
    pub bindings: std::collections::BTreeMap<String, String>,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            bindings: super::shortcuts::default_bindings(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockSide {
    Left,
    Right,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiThemeConfig {
    pub name: String,
    pub palette: UiThemePalette,
}

/// FORK CONSTRUCTO : le module est servi DANS l'ERP, qui est clair. Le defaut
/// amont (`Oxocarbon`, sombre) posait une dalle sombre au milieu d'une page
/// claire. `Fusion White` existait deja en amont — fond #FAFAFA, texte #1A1A1A.
///
/// C'est le THEME par defaut qui change ici, pas ses couleurs : l'accent est
/// reste celui d'amont, apres qu'une tentative de le passer au bleu de la
/// charte l'a fait tomber sous la barre de contraste que deux surfaces de
/// production interrogent. Voir `ui/style/fusion_theme.rs`.
///
/// Changer ce defaut ne suffit PAS a atteindre quelqu'un qui a deja ouvert
/// le module : sur le web, la config vit dans `localStorage`
/// (`opencadstudio.settings`) et une valeur enregistree l'emporte, `serde`
/// n'appelant `Default` que sur un champ ABSENT. La bascule des installations
/// existantes se fait par la migration de `AppConfig::load`.
impl Default for UiThemeConfig {
    fn default() -> Self {
        let theme = crate::ui::style::fusion_theme::fusion_white();
        Self {
            name: theme.to_string(),
            palette: UiThemePalette::from_iced(theme.seed()),
        }
    }
}

impl UiThemeConfig {
    pub fn to_iced(&self) -> iced::Theme {
        if self.name == "Custom" {
            iced::Theme::custom("Custom", self.palette.to_iced())
        } else {
            // Le repli suit le defaut : un nom de theme devenu inconnu (theme
            // retire en amont, config bricolee a la main) ne doit pas rendre la
            // dalle sombre que ce fork existe pour eviter.
            builtin_theme(&self.name)
                .unwrap_or_else(crate::ui::style::fusion_theme::fusion_white)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiThemePalette {
    pub background: [u8; 3],
    pub text: [u8; 3],
    pub primary: [u8; 3],
    pub success: [u8; 3],
    pub warning: [u8; 3],
    pub danger: [u8; 3],
}

impl Default for UiThemePalette {
    /// FORK CONSTRUCTO : le JUMEAU de `UiThemeConfig::default`, et il etait reste
    /// sur le sombre d'amont.
    ///
    /// Il ne sert pas qu'a l'editeur de couleurs. `#[serde(default)]` est aussi
    /// sur CE conteneur : une palette PARTIELLE dans le JSON — par exemple
    /// `{"palette":{"primary":[2,119,189]}}` — voit ses champs absents remplis
    /// depuis ici. Laisse sur Oxocarbon, un theme nomme « Fusion White » pouvait
    /// donc naitre avec un fond sombre.
    fn default() -> Self {
        Self::from_iced(crate::ui::style::fusion_theme::fusion_white().seed())
    }
}

impl UiThemePalette {
    pub fn from_iced(palette: iced::theme::palette::Seed) -> Self {
        Self {
            background: color_to_rgb(palette.background),
            text: color_to_rgb(palette.text),
            primary: color_to_rgb(palette.primary),
            success: color_to_rgb(palette.success),
            warning: color_to_rgb(palette.warning),
            danger: color_to_rgb(palette.danger),
        }
    }

    pub fn to_iced(self) -> iced::theme::palette::Seed {
        iced::theme::palette::Seed {
            background: rgb_to_color(self.background),
            text: rgb_to_color(self.text),
            primary: rgb_to_color(self.primary),
            success: rgb_to_color(self.success),
            warning: rgb_to_color(self.warning),
            danger: rgb_to_color(self.danger),
        }
    }

    pub fn hex_values(self) -> [String; 6] {
        [
            rgb_to_hex(self.background),
            rgb_to_hex(self.text),
            rgb_to_hex(self.primary),
            rgb_to_hex(self.success),
            rgb_to_hex(self.warning),
            rgb_to_hex(self.danger),
        ]
    }

    pub fn set_hex(&mut self, index: usize, value: &str) -> bool {
        let Some(rgb) = parse_hex(value) else {
            return false;
        };
        match index {
            0 => self.background = rgb,
            1 => self.text = rgb,
            2 => self.primary = rgb,
            3 => self.success = rgb,
            4 => self.warning = rgb,
            5 => self.danger = rgb,
            _ => return false,
        }
        true
    }
}

/// Every theme the user may pick: iced's built-ins followed by the Fusion
/// pair. The Fusion themes are `Theme::Custom`, so they are not in
/// `iced::Theme::ALL`; anything enumerating themes for display or for test
/// coverage must use this instead, or they silently vanish from the list.
pub fn all_themes() -> Vec<iced::Theme> {
    iced::Theme::ALL
        .iter()
        .cloned()
        .chain(crate::ui::style::fusion_theme::fusion_themes())
        .collect()
}

pub fn builtin_theme(name: &str) -> Option<iced::Theme> {
    all_themes().into_iter().find(|theme| theme.to_string() == name)
}

fn color_to_rgb(color: iced::Color) -> [u8; 3] {
    [
        (color.r * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.b * 255.0).round() as u8,
    ]
}

fn rgb_to_color(rgb: [u8; 3]) -> iced::Color {
    iced::Color::from_rgb8(rgb[0], rgb[1], rgb[2])
}

pub(crate) fn rgb_to_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

pub(crate) fn parse_hex(value: &str) -> Option<[u8; 3]> {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if value.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ])
}

/// Canvas mode for Model space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ModelSpaceMode {
    /// Canvas background dynamically follows the active UI theme.
    #[default]
    MatchTheme,
    /// Classic CAD dark charcoal canvas (#212830) regardless of UI theme.
    ClassicDark,
    /// Custom background colors specified by the user.
    Custom,
}

impl ModelSpaceMode {
    pub const ALL: [Self; 3] = [Self::MatchTheme, Self::ClassicDark, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::MatchTheme => "Match Theme",
            Self::ClassicDark => "Classic CAD Dark",
            Self::Custom => "Custom",
        }
    }
}

/// Standard classic CAD dark charcoal background [33, 40, 48].
pub const CLASSIC_CAD_DARK_BG: [u8; 3] = [33, 40, 48];

/// Standard paper space sheet background [255, 255, 255].
pub const DEFAULT_PAPER_BG: [u8; 3] = [255, 255, 255];

/// Standard paper space desk surround [138, 138, 138].
pub const DEFAULT_DESK_BG: [u8; 3] = [138, 138, 138];

/// Persistent configuration for Model Space appearance and selection visual effects.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSpaceThemeConfig {
    /// How the canvas background is determined.
    pub mode: ModelSpaceMode,
    /// Custom model space canvas background [R, G, B] (0–255).
    pub custom_bg: Option<[u8; 3]>,
    /// Custom paper space sheet background [R, G, B] (0–255).
    pub custom_paper_bg: Option<[u8; 3]>,
    /// Custom paper space desk/surround background [R, G, B] (0–255).
    pub custom_desk_bg: Option<[u8; 3]>,
    /// Grid opacity percentage (5–100, default 18%).
    pub grid_opacity: u8,
    /// SELECTIONAREA: whether selection marquees have a translucent shaded fill.
    pub selection_area: bool,
    /// SELECTIONAREAOPACITY: transparency percentage of selection areas (0–100, default 12%).
    pub selection_opacity: u8,
    /// WINDOWSAREACOLOR: ACI index (0 = Theme Primary, 1..=255 = ACI).
    pub selection_window_color: u8,
    /// CROSSINGAREACOLOR: ACI index (0 = Theme Success, 1..=255 = ACI).
    pub selection_crossing_color: u8,
    /// SELECTIONEFFECTCOLOR: ACI index for selected entities highlight (0 = Theme Primary, 1..=255 = ACI).
    pub selection_highlight_color: u8,
    /// SELECTIONEFFECT: whether selected objects glow with solid highlight (true) or dash (false).
    pub selection_effect: bool,
    /// SELECTIONPREVIEW bitmask: 1 = idle, 2 = during a command, 3 = both.
    pub selection_preview: u8,
    /// GRIPSIZE: grip marker half-size in pixels (1–25, default 5).
    pub grip_size: u8,
    /// GRIPCOLOR: unselected grip ACI color (0 = Theme Primary outline, 1..=255 = ACI).
    pub grip_color: u8,
    /// GRIPHOT: selected/hot grip ACI color (0 = Theme Danger, 1..=255 = ACI).
    pub grip_hot: u8,
    /// GRIPHOVER: hovered/warm grip ACI color (0 = Theme Primary Strong, 1..=255 = ACI).
    pub grip_hover: u8,
}

impl Default for ModelSpaceThemeConfig {
    fn default() -> Self {
        Self {
            mode: ModelSpaceMode::MatchTheme,
            custom_bg: None,
            custom_paper_bg: None,
            custom_desk_bg: None,
            grid_opacity: 18,
            selection_area: true,
            selection_opacity: 12,
            selection_window_color: 0,
            selection_crossing_color: 0,
            selection_highlight_color: 0,
            selection_effect: true,
            selection_preview: 3,
            grip_size: 5,
            grip_color: 0,
            grip_hot: 0,
            grip_hover: 0,
        }
    }
}

impl ModelSpaceThemeConfig {
    /// Resolve active model space canvas background as [f32; 4] based on current theme.
    pub fn resolve_model_bg(&self, theme: &iced::Theme) -> [f32; 4] {
        let rgb = match self.mode {
            ModelSpaceMode::MatchTheme => theme_canvas_background(theme),
            ModelSpaceMode::ClassicDark => CLASSIC_CAD_DARK_BG,
            ModelSpaceMode::Custom => self.custom_bg.unwrap_or(CLASSIC_CAD_DARK_BG),
        };
        [
            rgb[0] as f32 / 255.0,
            rgb[1] as f32 / 255.0,
            rgb[2] as f32 / 255.0,
            1.0,
        ]
    }

    /// Resolve active paper space sheet background as [f32; 4].
    pub fn resolve_paper_bg(&self) -> [f32; 4] {
        let rgb = self.custom_paper_bg.unwrap_or(DEFAULT_PAPER_BG);
        [
            rgb[0] as f32 / 255.0,
            rgb[1] as f32 / 255.0,
            rgb[2] as f32 / 255.0,
            1.0,
        ]
    }

    /// Resolve active paper space desk surround background as [f32; 4].
    pub fn resolve_desk_bg(&self) -> [f32; 4] {
        let rgb = self.custom_desk_bg.unwrap_or(DEFAULT_DESK_BG);
        [
            rgb[0] as f32 / 255.0,
            rgb[1] as f32 / 255.0,
            rgb[2] as f32 / 255.0,
            1.0,
        ]
    }

    /// Resolve active selection highlight overlay color as [f32; 4].
    pub fn resolve_selection_color(&self) -> [f32; 4] {
        if self.selection_highlight_color == 0 {
            crate::scene::model::wire_model::WireModel::SELECTED
        } else if let Some((r, g, b)) = acadrust::types::aci_table::aci_to_rgb(self.selection_highlight_color) {
            [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
        } else {
            crate::scene::model::wire_model::WireModel::SELECTED
        }
    }
}

/// Default canvas background for a given theme variant.
pub fn theme_canvas_background(theme: &iced::Theme) -> [u8; 3] {
    match theme {
        iced::Theme::Light => [255, 255, 255],
        iced::Theme::SolarizedLight => [253, 246, 227],
        iced::Theme::GruvboxLight => [251, 241, 199],
        iced::Theme::TokyoNightLight => [225, 226, 231],
        iced::Theme::KanagawaLotus => [242, 236, 222],
        iced::Theme::Dark => [32, 34, 37],
        iced::Theme::Dracula => [40, 42, 54],
        iced::Theme::Nord => [46, 52, 64],
        iced::Theme::SolarizedDark => [0, 43, 54],
        iced::Theme::GruvboxDark => [40, 40, 40],
        iced::Theme::TokyoNight => [26, 27, 38],
        iced::Theme::TokyoNightStorm => [36, 40, 59],
        iced::Theme::KanagawaWave => [31, 31, 40],
        iced::Theme::KanagawaDragon => [24, 26, 28],
        iced::Theme::Moonfly => [8, 18, 24],
        iced::Theme::Nightfly => [1, 22, 39],
        iced::Theme::Oxocarbon => [22, 22, 22],
        iced::Theme::Ferra => [43, 41, 46],
        // The Fusion pair deliberately breaks the "canvas follows chrome"
        // rule: black chrome is paired with a white canvas, so falling
        // through to the palette below would paint model space black and
        // undo the whole point of the theme.
        other => match crate::ui::style::fusion_theme::fusion_canvas(other) {
            Some(rgb) => rgb,
            None => color_to_rgb(other.palette().background.base.color),
        },
    }
}

/// Parse a flexible theme name string into an `iced::Theme` variant.
/// Normalizes by stripping spaces, underscores, and dashes, case-insensitive.
pub fn parse_theme_name(s: &str) -> Option<iced::Theme> {
    let clean: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(|c| c.to_uppercase())
        .collect();
    match clean.as_str() {
        "DARK" => Some(iced::Theme::Dark),
        "LIGHT" => Some(iced::Theme::Light),
        "DRACULA" => Some(iced::Theme::Dracula),
        "NORD" => Some(iced::Theme::Nord),
        "SOLARIZEDLIGHT" => Some(iced::Theme::SolarizedLight),
        "SOLARIZEDDARK" => Some(iced::Theme::SolarizedDark),
        "GRUVBOXLIGHT" => Some(iced::Theme::GruvboxLight),
        "GRUVBOXDARK" => Some(iced::Theme::GruvboxDark),
        "TOKYONIGHT" => Some(iced::Theme::TokyoNight),
        "TOKYONIGHTSTORM" => Some(iced::Theme::TokyoNightStorm),
        "TOKYONIGHTLIGHT" => Some(iced::Theme::TokyoNightLight),
        "KANAGAWAWAVE" => Some(iced::Theme::KanagawaWave),
        "KANAGAWADRAGON" => Some(iced::Theme::KanagawaDragon),
        "KANAGAWALOTUS" => Some(iced::Theme::KanagawaLotus),
        "MOONFLY" => Some(iced::Theme::Moonfly),
        "NIGHTFLY" => Some(iced::Theme::Nightfly),
        "OXOCARBON" => Some(iced::Theme::Oxocarbon),
        "FERRA" => Some(iced::Theme::Ferra),
        "FUSIONBLACK" => Some(crate::ui::style::fusion_theme::fusion_black()),
        "FUSIONWHITE" => Some(crate::ui::style::fusion_theme::fusion_white()),
        _ => None,
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentConfig {
    /// Recently opened file paths, newest first.
    pub files: Vec<String>,
    /// How many recent files to keep.
    pub limit: usize,
}

impl Default for RecentConfig {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            limit: super::recent::RECENT_DEFAULT,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StartConfig {
    pub section: super::StartSection,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RibbonConfig {
    pub collapse: CollapseMode,
}

impl AppConfig {
    /// Read the saved config, or all-defaults when the file is missing or
    /// unreadable. Unknown or missing fields fall back to their section defaults
    /// via `#[serde(default)]`.
    pub fn load() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let body = config_path().and_then(|p| std::fs::read_to_string(p).ok());

        #[cfg(target_arch = "wasm32")]
        let body = web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .and_then(|storage| storage.get_item(WEB_CONFIG_KEY).ok().flatten());

        Self::depuis_json(body.as_deref())
    }

    /// FORK CONSTRUCTO : la CHAINE — lire le JSON, puis migrer.
    ///
    /// Elle existe parce que `load()` est MAL testable : elle lit le disque ou
    /// `localStorage`.
    ///
    /// ATTENTION -- CE N'EST PAS « impossible », et le dire serait se mentir.
    /// En natif, `config_path()` passe par `config_dir()`, donc par `APPDATA` :
    /// un test peut deposer une config et appeler `load()`. Ce qui l'en empeche
    /// est autre chose, et ca se dit : la variable est un etat de PROCESSUS, et
    /// `OpenCADStudio::new_for_test()` appelle `load()` lui aussi. Un tel test
    /// servirait sa fausse config a tous les tests qui tournent en meme temps.
    /// Il faudrait `--test-threads=1`, que la CI ne pose pas.
    ///
    /// LE TROU QUE CA LAISSE, mesure : remplacer l'appel a `depuis_json`
    /// ci-dessous par l'ancien corps laisse les tests VERTS, et la migration ne
    /// part alors jamais en production. C'est le jumeau du defaut ferme un cran
    /// plus bas. La parade n'est pas un test : c'est que cette ligne reste la
    /// SEULE chose que `load()` fasse, pour qu'un contournement soit une
    /// reecriture visible et non une retouche. Un banc qui appelle `migrer_constructo` a la main sur
    /// une struct batie en Rust mesure la FONCTION, jamais la chaine — et laisse
    /// donc passer la mutation qui RETIRE l'appel. Mesure du 2026-09-19 :
    /// supprimer `config.migrer_constructo()` laissait les cinq tests verts.
    ///
    /// C'est aussi la seule voie qui traverse serde, donc la seule ou le piege
    /// du `#[serde(default)]` de conteneur (voir le champ `constructo_migration`)
    /// puisse etre constate au lieu d'etre suppose.
    fn depuis_json(body: Option<&str>) -> Self {
        let mut config: Self = body
            .and_then(|body| serde_json::from_str(body).ok())
            .unwrap_or_default();
        config.migrer_constructo();
        config
    }

    /// FORK CONSTRUCTO : amene une config ecrite avant le fork a l'etat que ce
    /// fork attend.
    ///
    /// DEUX CONDITIONS, ET CHACUNE FERME UN DEFAUT DIFFERENT.
    ///
    /// 1. **Le compteur** distingue « jamais migre » de « a choisi ». Sans lui,
    ///    forcer le theme a chaque chargement donnerait un reglage qu'on ne PEUT
    ///    PAS changer : l'utilisateur choisit, la config enregistre, le
    ///    rechargement annule son choix.
    ///
    /// 2. **Le nom du theme** borne les degats a ceux qu'on vient reparer.
    ///    Une premiere version ne portait que la condition 1 et remplacait
    ///    `self.theme` EN ENTIER. Mesure du retest :
    ///
    ///    ```text
    ///    avant : name=Custom  palette=[18,24,38]/[220,228,240]/[255,140,0]
    ///    apres : name=Fusion White  palette=[250,250,250]/[26,26,26]/[0,32,80]
    ///    ```
    ///
    ///    Un utilisateur d'avant le fork ayant regle ses six couleurs a la main
    ///    les perdait a la premiere ouverture, **sans retour possible** —
    ///    `saved_custom_palette` (`app/mod.rs:1123`) est un champ de session,
    ///    jamais serialise. Et un theme CLAIR choisi deliberement
    ///    (`SolarizedLight`, `GruvboxLight`) etait ecrase alors qu'il n'a jamais
    ///    pose la dalle sombre que cette migration existe pour supprimer.
    ///
    /// On ne migre donc QUE depuis le defaut d'amont. Celui qui avait choisi
    ///    Oxocarbon deliberement le voit changer une fois ; tous les autres
    ///    gardent ce qu'ils avaient. C'est la seule frontiere que la config
    ///    permette de tracer, et elle laisse passer le moins de monde.
    fn migrer_constructo(&mut self) {
        if self.constructo_migration < 1 && self.theme.name == THEME_PAR_DEFAUT_AMONT {
            // 1 — le module est encadre par un ERP clair ; le defaut amont etait
            // sombre.
            self.theme = UiThemeConfig::default();
        }
        // Hors du `if`, ET C'EST OBSERVABLE -- depuis la clause de nom seulement.
        //
        // Ce commentaire a dit deux choses fausses de suite, et la seconde etait
        // la mienne. D'abord « sans quoi la migration se rejouerait a chaque
        // chargement » : faux, mesure. Puis, apres correction, « sans effet
        // observable a l'interieur » : vrai a l'instant ou je l'ai ecrit, et
        // perime UNE HEURE plus tard quand la clause `&& self.theme.name == ...`
        // est arrivee sur la meme ligne. Avec elle, un utilisateur reste sur
        // `Dracula` ne passe plus dans le `if` -- et son compteur doit quand meme
        // etre releve, sinon la migration se represente a chaque chargement.
        //
        // Le test `le_compteur_est_pose_meme_quand_il_n_y_a_rien_a_migrer` tient
        // cette position. C'est lui qui fait foi, pas ce paragraphe : un
        // commentaire vieillit, un test rougit.
        self.constructo_migration = self.constructo_migration.max(CONSTRUCTO_MIGRATION);
    }

    /// Persist the config as JSON. Best-effort; silent on unavailable or
    /// read-only storage.
    ///
    /// Does nothing under `cfg(test)`. The path is the developer's own
    /// settings file, and the suite builds whole applications and changes
    /// preferences on them — without this, running `cargo test` rewrites the
    /// settings of whoever ran it.
    pub fn save(&self) {
        if cfg!(test) {
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) = config_path() else { return };
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(path, json);
            }
        }

        #[cfg(target_arch = "wasm32")]
        if let (Some(storage), Ok(json)) = (
            web_sys::window().and_then(|window| window.local_storage().ok().flatten()),
            serde_json::to_string(self),
        ) {
            let _ = storage.set_item(WEB_CONFIG_KEY, &json);
        }
    }
}

#[cfg(target_arch = "wasm32")]
const WEB_CONFIG_KEY: &str = "opencadstudio.settings";

#[cfg(not(target_arch = "wasm32"))]
fn config_path() -> Option<std::path::PathBuf> {
    Some(crate::config::config_dir()?.join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_space_defaults() {
        let cfg = ModelSpaceThemeConfig::default();
        assert_eq!(cfg.mode, ModelSpaceMode::MatchTheme);
        assert_eq!(cfg.custom_bg, None);
        assert_eq!(cfg.custom_paper_bg, None);
        assert_eq!(cfg.grid_opacity, 18);
        assert!(cfg.selection_area);
        assert_eq!(cfg.selection_opacity, 12);
        assert_eq!(cfg.selection_window_color, 0);
        assert_eq!(cfg.selection_crossing_color, 0);
        assert_eq!(cfg.selection_highlight_color, 0);
        assert!(cfg.selection_effect);
        assert_eq!(cfg.selection_preview, 3);
    }

    #[test]
    fn test_resolve_model_bg_match_theme() {
        let cfg = ModelSpaceThemeConfig {
            mode: ModelSpaceMode::MatchTheme,
            ..Default::default()
        };
        // Light theme should produce pure white background [1.0, 1.0, 1.0, 1.0]
        let light_bg = cfg.resolve_model_bg(&iced::Theme::Light);
        assert_eq!(light_bg, [1.0, 1.0, 1.0, 1.0]);

        // Oxocarbon theme should produce dark background
        let oxo_bg = cfg.resolve_model_bg(&iced::Theme::Oxocarbon);
        assert!((oxo_bg[0] - 22.0 / 255.0).abs() < 1e-4);
        assert!((oxo_bg[1] - 22.0 / 255.0).abs() < 1e-4);
        assert!((oxo_bg[2] - 22.0 / 255.0).abs() < 1e-4);
        assert_eq!(oxo_bg[3], 1.0);
    }

    #[test]
    fn test_resolve_model_bg_classic_dark() {
        let cfg = ModelSpaceThemeConfig {
            mode: ModelSpaceMode::ClassicDark,
            ..Default::default()
        };
        // Even with Light theme active, ClassicDark must stay locked to [33, 40, 48]
        let bg = cfg.resolve_model_bg(&iced::Theme::Light);
        assert!((bg[0] - 33.0 / 255.0).abs() < 1e-4);
        assert!((bg[1] - 40.0 / 255.0).abs() < 1e-4);
        assert!((bg[2] - 48.0 / 255.0).abs() < 1e-4);
        assert_eq!(bg[3], 1.0);
    }

    #[test]
    fn test_resolve_model_bg_custom() {
        let cfg = ModelSpaceThemeConfig {
            mode: ModelSpaceMode::Custom,
            custom_bg: Some([10, 20, 30]),
            ..Default::default()
        };
        let bg = cfg.resolve_model_bg(&iced::Theme::Light);
        assert!((bg[0] - 10.0 / 255.0).abs() < 1e-4);
        assert!((bg[1] - 20.0 / 255.0).abs() < 1e-4);
        assert!((bg[2] - 30.0 / 255.0).abs() < 1e-4);
    }

    #[test]
    fn test_model_space_config_serde_roundtrip() {
        let mut original = AppConfig::default();
        original.model_space.mode = ModelSpaceMode::Custom;
        original.model_space.custom_bg = Some([12, 34, 56]);
        original.model_space.grid_opacity = 45;
        original.model_space.selection_opacity = 35;
        original.model_space.selection_window_color = 5;

        let serialized = serde_json::to_string(&original).expect("serialize config");
        let deserialized: AppConfig = serde_json::from_str(&serialized).expect("deserialize config");

        assert_eq!(deserialized.model_space.mode, ModelSpaceMode::Custom);
        assert_eq!(deserialized.model_space.custom_bg, Some([12, 34, 56]));
        assert_eq!(deserialized.model_space.grid_opacity, 45);
        assert_eq!(deserialized.model_space.selection_opacity, 35);
        assert_eq!(deserialized.model_space.selection_window_color, 5);
    }

    #[test]
    fn test_parse_theme_name() {
        assert_eq!(parse_theme_name("light"), Some(iced::Theme::Light));
        assert_eq!(parse_theme_name("DARK"), Some(iced::Theme::Dark));
        assert_eq!(parse_theme_name("solarized light"), Some(iced::Theme::SolarizedLight));
        assert_eq!(parse_theme_name("tokyo-night-storm"), Some(iced::Theme::TokyoNightStorm));
        assert_eq!(parse_theme_name("Kanagawa_Lotus"), Some(iced::Theme::KanagawaLotus));
        assert_eq!(parse_theme_name("1"), None);
        assert_eq!(parse_theme_name("0"), None);
        assert_eq!(parse_theme_name("nonexistent_theme"), None);
    }

    // ── FORK CONSTRUCTO ──────────────────────────────────────────────────

    #[test]
    fn le_theme_par_defaut_est_clair_comme_l_erp_qui_l_encadre() {
        let config = AppConfig::default();
        assert_eq!(
            config.theme.name,
            crate::ui::style::fusion_theme::FUSION_WHITE,
            "le module est servi dans un ERP clair ; un defaut sombre y pose une dalle"
        );
        assert_eq!(config.constructo_migration, CONSTRUCTO_MIGRATION);
    }

    #[test]
    fn un_nom_de_theme_inconnu_ne_retombe_pas_sur_du_sombre() {
        let inconnu = UiThemeConfig {
            name: "ThemeQuiNExistePlus".to_string(),
            palette: UiThemePalette::from_iced(iced::Theme::Dark.seed()),
        };
        assert_eq!(
            inconnu.to_iced().to_string(),
            crate::ui::style::fusion_theme::FUSION_WHITE
        );
    }

    /// Le JSON d'une config telle qu'elle existait AVANT le fork : le theme
    /// sombre d'amont, et **aucune** cle `constructo_migration`.
    ///
    /// Fabrique en retirant la cle d'une config serialisee plutot qu'en ecrivant
    /// un JSON a la main : le litteral vieillirait a chaque champ ajoute, et un
    /// banc qui ne se deserialise plus se saute au lieu d'echouer.
    fn json_d_avant_le_fork() -> String {
        json_avec_theme_sombre(true)
    }

    /// Une config serialisee portant le theme sombre d'amont ; `sans_compteur`
    /// retire la cle `constructo_migration`, ce qui est la signature d'une
    /// config ecrite avant le fork.
    fn json_avec_theme_sombre(sans_compteur: bool) -> String {
        let mut valeur = serde_json::to_value(AppConfig::default()).unwrap();
        let objet = valeur.as_object_mut().unwrap();
        if sans_compteur {
            objet.remove("constructo_migration");
        }
        objet.insert(
            "theme".to_string(),
            serde_json::to_value(UiThemeConfig {
                name: "Oxocarbon".to_string(),
                palette: UiThemePalette::from_iced(iced::Theme::Oxocarbon.seed()),
            })
            .unwrap(),
        );
        valeur.to_string()
    }

    #[test]
    fn un_compteur_absent_se_lit_a_zero_et_non_au_defaut_de_la_struct() {
        // LE TEST QUI MANQUAIT, ET SANS LEQUEL TOUT LE RESTE ETAIT MORT.
        //
        // `#[serde(default)]` sur le CONTENEUR puise un champ absent dans
        // `AppConfig::default()` — donc `CONSTRUCTO_MIGRATION`, donc 1. Il a
        // fallu l'attribut sur le CHAMP pour obtenir 0. Aucun des tests
        // precedents ne traversait serde : ils ecrivaient `0` a la main dans un
        // litteral, puis constataient que la fonction faisait ce qu'on lui avait
        // dit. Ils jugeaient la forme fabriquee, pas le comportement reel.
        let brut: AppConfig = serde_json::from_str(&json_d_avant_le_fork()).unwrap();
        assert_eq!(
            brut.constructo_migration, 0,
            "un champ absent doit valoir 0, sinon la migration ne part jamais"
        );
    }

    #[test]
    fn la_chaine_complete_bascule_une_config_d_avant_le_fork() {
        // Par `depuis_json`, la chaine REELLE de `load()`. Un test qui
        // appellerait `migrer_constructo` a la main laisserait passer la
        // mutation qui RETIRE l'appel.
        let migree = AppConfig::depuis_json(Some(&json_d_avant_le_fork()));
        assert_eq!(
            migree.theme.name,
            crate::ui::style::fusion_theme::FUSION_WHITE
        );
        // La PALETTE aussi, pas seulement le nom : une migration qui pose le bon
        // nom en gardant les couleurs sombres reste invisible a un test de nom,
        // et `apply_config` alimente l'editeur de couleurs depuis cette palette.
        //
        // `assert!` et non `assert_eq!` : aucune de ces structs ne derive
        // `Debug` (voir les derivations de `UiThemePalette`), et `assert_eq!`
        // l'exige pour imprimer l'ecart.
        assert!(
            migree.theme.palette
                == UiThemePalette::from_iced(
                    crate::ui::style::fusion_theme::fusion_white().seed()
                ),
            "le nom sans la palette laisse un theme clair aux couleurs sombres"
        );
        assert_eq!(migree.constructo_migration, CONSTRUCTO_MIGRATION);
    }

    #[test]
    fn la_chaine_migre_aussi_une_config_absente() {
        let neuve = AppConfig::depuis_json(None);
        assert_eq!(
            neuve.theme.name,
            crate::ui::style::fusion_theme::FUSION_WHITE
        );
        assert_eq!(neuve.constructo_migration, CONSTRUCTO_MIGRATION);
    }

    #[test]
    fn une_config_illisible_ne_fait_pas_tomber_le_demarrage() {
        let casse = AppConfig::depuis_json(Some("{ ceci n'est pas du json"));
        assert_eq!(
            casse.theme.name,
            crate::ui::style::fusion_theme::FUSION_WHITE
        );
    }

    #[test]
    fn la_migration_ne_repasse_pas_sur_un_choix_deja_fait() {
        // LE PIEGE PRINCIPAL. Une migration qui se rejoue donne un reglage
        // IMPOSSIBLE A CHANGER : on choisit, la config enregistre, le
        // rechargement annule le choix. Le compteur separe « jamais migre » de
        // « a choisi » ; une garde sur la VALEUR du theme ne le saurait pas.
        // Le compteur est PRESENT et a jour : la config a deja ete migree, et
        // c'est apres coup que l'utilisateur a choisi le sombre.
        let choisi = AppConfig::depuis_json(Some(&json_avec_theme_sombre(false)));
        assert_eq!(
            choisi.theme.name, "Oxocarbon",
            "un theme choisi apres la migration doit survivre au rechargement"
        );
    }

    #[test]
    fn le_compteur_est_pose_meme_quand_il_n_y_a_rien_a_migrer() {
        // Il doit etre HORS du `if` : une config neuve qui n'a rien a migrer
        // doit tout de meme porter le numero courant, sinon la migration se
        // rejoue a chaque chargement. Rien dans les tests precedents ne fixait
        // cette position.
        let mut neuve = AppConfig {
            constructo_migration: 0,
            ..AppConfig::default()
        };
        neuve.migrer_constructo();
        assert_eq!(neuve.constructo_migration, CONSTRUCTO_MIGRATION);
    }

    #[test]
    fn un_compteur_venu_du_futur_n_est_pas_rabattu() {
        // Scenario reel : `/cad` est servi depuis une release remplacee a chaque
        // build. Un onglet qui tient encore l'ancien bundle met un module d'hier
        // devant une config ecrite aujourd'hui. Rabattre le compteur ferait
        // rejouer la migration au retour — le defaut meme qu'il empeche.
        let mut futur = AppConfig {
            constructo_migration: CONSTRUCTO_MIGRATION + 7,
            ..AppConfig::default()
        };
        futur.migrer_constructo();
        assert_eq!(futur.constructo_migration, CONSTRUCTO_MIGRATION + 7);
    }

    #[test]
    fn la_migration_est_idempotente() {
        // Elle part d'un theme SOMBRE, et non de `..AppConfig::default()`. Une
        // campagne de mutation a montre qu'en partant du theme deja cible, ce
        // test n'attrapait qu'une seule mutation sur trente-cinq : la premiere
        // migration n'y deplacait rien d'observable. C'etait un sous-ensemble de
        // ses voisins, pas une garde.
        let mut config = AppConfig {
            constructo_migration: 0,
            theme: UiThemeConfig {
                name: THEME_PAR_DEFAUT_AMONT.to_string(),
                palette: UiThemePalette::from_iced(iced::Theme::Oxocarbon.seed()),
            },
            ..AppConfig::default()
        };
        config.migrer_constructo();
        let apres_une = config.clone();
        config.migrer_constructo();
        assert!(apres_une == config, "rejouer la migration doit etre sans effet");
    }

    // ── Les six gardes ajoutees apres la campagne de mutation du 2026-09-19 ──
    //
    // Elle a rendu 26 prises sur 35, et chacune de ces six-la ferme un survivant
    // dont l'effet utilisateur avait ete mesure. Elles ne sont pas des tests de
    // confort : sans elles, les mutations correspondantes laissaient les onze
    // tests precedents VERTS.

    #[test]
    fn une_palette_partielle_se_complete_en_clair() {
        // DEUX defauts d'un coup, tous deux mesures.
        //
        // 1. `#[serde(default)]` sur le CONTENEUR `UiThemePalette` : sans lui, un
        //    champ absent devient une ERREUR de deserialisation, que
        //    `depuis_json` avale en `unwrap_or_default()`. Une palette partielle
        //    fait donc jeter TOUTE la config en silence -- fichiers recents,
        //    raccourcis clavier, dock, reglages d'impression. L'utilisateur
        //    rouvre le module et tout est revenu par defaut, sans un message.
        // 2. `UiThemePalette::default()` : le JUMEAU de `UiThemeConfig::default`.
        //    Laisse sur Oxocarbon, les champs absents naissent SOMBRES sous un
        //    theme nomme « Fusion White ».
        let json = r#"{"constructo_migration":1,"theme":{"name":"Fusion White","palette":{"primary":[255,0,0]}}}"#;
        let config = AppConfig::depuis_json(Some(json));
        assert_eq!(
            config.theme.palette.primary,
            [255, 0, 0],
            "la config a ete jetee : le defaut de conteneur ne s'applique plus"
        );
        assert_eq!(
            config.theme.palette.background,
            [250, 250, 250],
            "les champs absents naissent sombres"
        );
    }

    #[test]
    fn un_theme_sans_palette_ne_jette_pas_la_config() {
        let config =
            AppConfig::depuis_json(Some(r#"{"constructo_migration":1,"theme":{"name":"Nord"}}"#));
        assert_eq!(config.theme.name, "Nord");
    }

    #[test]
    fn une_config_a_qui_il_manque_des_sections_garde_ce_qu_elle_porte() {
        // Le `#[serde(default)]` du conteneur `AppConfig` lui-meme. Une
        // `settings.json` ecrite par une version anterieure n'a pas toutes les
        // sections d'aujourd'hui : sans ce defaut, elle est integralement perdue.
        let config = AppConfig::depuis_json(Some(
            r#"{"constructo_migration":1,"recent":{"files":["a.dwg"],"limit":7}}"#,
        ));
        assert_eq!(config.recent.files, vec!["a.dwg".to_string()]);
        assert_eq!(config.recent.limit, 7);
    }

    #[test]
    fn un_compteur_venu_du_futur_ne_relance_pas_la_migration() {
        // Le test voisin ne verifie QUE le compteur, sur une config deja au theme
        // clair : remplacer `< 1` par `!= CONSTRUCTO_MIGRATION` le laissait vert.
        // Scenario : l'onglet A (bundle neuf) ecrit 8 et l'utilisateur choisit
        // Oxocarbon ; l'onglet B (bundle d'hier) charge, `8 != 1`, et son theme
        // est ecrase. Le compteur etait protege, le theme qu'il protege ne
        // l'etait pas.
        let mut futur = AppConfig {
            constructo_migration: CONSTRUCTO_MIGRATION + 7,
            theme: UiThemeConfig {
                name: THEME_PAR_DEFAUT_AMONT.to_string(),
                palette: UiThemePalette::from_iced(iced::Theme::Oxocarbon.seed()),
            },
            ..AppConfig::default()
        };
        futur.migrer_constructo();
        assert_eq!(
            futur.theme.name, THEME_PAR_DEFAUT_AMONT,
            "un compteur venu du futur ne doit pas relancer la migration"
        );
    }

    #[test]
    fn la_migration_epargne_un_theme_choisi_avant_le_fork() {
        // La clause de nom n'avait aucun test : la retirer laissait le banc vert.
        // Elle tranche pourtant une decision produit -- un utilisateur reste sur
        // `Dracula` garde-t-il son theme ? -- et les autres tests n'exercent
        // qu'un seul theme d'avant le fork, Oxocarbon.
        let mut valeur = serde_json::to_value(AppConfig::default()).unwrap();
        let objet = valeur.as_object_mut().unwrap();
        assert!(
            objet.remove("constructo_migration").is_some(),
            "la cle persistee a change de nom : ce banc ne mesurait plus rien"
        );
        objet.insert(
            "theme".to_string(),
            serde_json::to_value(UiThemeConfig {
                name: "Dracula".to_string(),
                palette: UiThemePalette::from_iced(iced::Theme::Dracula.seed()),
            })
            .unwrap(),
        );
        let migree = AppConfig::depuis_json(Some(&valeur.to_string()));
        assert_eq!(
            migree.theme.name, "Dracula",
            "un theme CHOISI avant le fork survit ; seul le defaut amont bascule"
        );
        assert_eq!(migree.constructo_migration, CONSTRUCTO_MIGRATION);
    }

    #[test]
    fn le_nom_du_theme_d_amont_est_celui_qu_iced_ecrit_vraiment() {
        // `THEME_PAR_DEFAUT_AMONT` est une CHAINE, et rien ne la rattachait a
        // `iced`. Les six autres tests fabriquent le nom depuis cette meme
        // constante ou depuis le meme litteral : ils restent donc verts,
        // coherents avec eux-memes, le jour ou `iced` renomme son affichage au
        // prochain rebase.
        //
        // ⚠️ CE COMMENTAIRE A ANNONCE LA MAUVAISE CONSEQUENCE, et la mesure du
        // 2026-09-20 la corrige. Il disait que « migrer_constructo ne
        // reconnaitrait plus personne ». C'est faux : `:604` compare
        // `self.theme.name` -- la chaine PERSISTEE, ecrite dans le fichier il y
        // a des mois -- a la constante de CE fork. Un renommage chez `iced` ne
        // touche ni l'une ni l'autre, et la migration continue de marcher.
        //
        // CE QUI CASSE VRAIMENT, c'est la RESOLUTION : `:158`
        // `builtin_theme(&self.name).unwrap_or_else(fusion_white)`. Celui qui
        // avait choisi Oxocarbon DELIBEREMENT se retrouve en Fusion White au
        // demarrage suivant, en silence, sans que rien ne le lui dise. C'est
        // cette assertion-ci qui le voit :
        assert!(
            builtin_theme(THEME_PAR_DEFAUT_AMONT).is_some(),
            "le nom du theme d'amont ne se resout plus : toute config qui le \
             porte basculera silencieusement sur Fusion White (config.rs:158)"
        );
        // Et celle-ci previent du renommage lui-meme.
        //
        // 🔴 SI ELLE ROUGIT, NE PAS METTRE LA CONSTANTE A JOUR. Elle ne decrit
        // pas ce qu'`iced` affiche aujourd'hui : elle decrit CE QUE CONTIENNENT
        // LES FICHIERS DE CONFIG DEJA ECRITS, une valeur historique et gelee.
        // La changer casserait la migration pour toute la population existante,
        // dont les fichiers portent l'ancien nom. Le bon geste est d'ajouter
        // une seconde valeur reconnue, pas de remplacer celle-ci.
        assert_eq!(
            THEME_PAR_DEFAUT_AMONT,
            iced::Theme::Oxocarbon.to_string(),
            "iced a renomme son theme : AJOUTER le nouveau nom, ne pas \
             remplacer la constante (voir le commentaire au-dessus)"
        );
    }

    #[test]
    fn la_garde_suit_la_constante_et_non_un_litteral() {
        // `CONSTRUCTO_MIGRATION = 1` et la garde `< 1` portent le meme nombre par
        // COINCIDENCE DE FRAPPE : passer la constante a 2 laissait le banc vert.
        // Le jour ou la migration 2 arrive, la population qui la demande
        // (compteur = 1) ne la recevrait pas, `1 < 1` etant faux -- la classe de
        // defaut que le compteur existe pour servir, desarmee en silence.
        let mut jamais_migree = AppConfig {
            constructo_migration: CONSTRUCTO_MIGRATION - 1,
            theme: UiThemeConfig {
                name: THEME_PAR_DEFAUT_AMONT.to_string(),
                palette: UiThemePalette::from_iced(iced::Theme::Oxocarbon.seed()),
            },
            ..AppConfig::default()
        };
        jamais_migree.migrer_constructo();
        assert_eq!(
            jamais_migree.theme.name,
            crate::ui::style::fusion_theme::FUSION_WHITE,
            "une config d'un cran sous la constante doit migrer"
        );
    }

    #[test]
    fn ce_qui_est_enregistre_porte_le_numero_courant() {
        // `current_config` (`update/file.rs`) rebatit la config depuis l'etat
        // vivant, qui ne transporte pas le compteur : c'est la constante qui
        // part en storage. Ce test tient le contrat des deux cotes — s'il
        // changeait, un choix delibere serait efface au rechargement suivant.
        let json = serde_json::to_string(&AppConfig::default()).unwrap();
        let relue: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(relue.constructo_migration, CONSTRUCTO_MIGRATION);
        assert_eq!(relue.theme.name, crate::ui::style::fusion_theme::FUSION_WHITE);
    }
}
