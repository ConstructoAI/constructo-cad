// Small platform shims for things the desktop build does natively but the web
// (wasm) build must handle differently or skip.

/// Drawing hyperlinks may open web pages, never local files or custom handlers.
pub(crate) fn web_hyperlink(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(value.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        .then(|| url.into())
}

#[test]
fn drawing_hyperlinks_only_open_web_pages() {
    for value in ["https://example.com/path", " HTTP://example.com "] {
        assert!(web_hyperlink(value).is_some(), "{value}");
    }
    for value in [
        "", "https://", "javascript:alert(1)", "data:text/html,example",
        "file:///tmp/program.desktop", "/tmp/program.desktop", "custom:run",
        "mailto:user@example.com", "https://example.com/\npath",
    ] {
        assert!(web_hyperlink(value).is_none(), "{value}");
    }
}

/// Open a URL in the user's browser.
///
/// Wayland requires an xdg-activation token from the source window before it
/// will let an existing browser window take focus. Linux obtains that token
/// from the OCS surface and passes it to xdg-open.
#[cfg(target_os = "linux")]
pub fn open_url<Message: Send + 'static>(
    url: &str,
    parent: Option<iced::window::Id>,
) -> iced::Task<Message> {
    let url = url.to_string();
    match parent {
        Some(parent) => iced::window::run(parent, move |window| {
            open_url_linux(&url, linux_activation_token(window));
        })
        .discard(),
        None => {
            let _ = open::that_detached(url);
            iced::Task::none()
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_activation_token(window: &dyn iced::window::Window) -> Option<String> {
    use iced::window::raw_window_handle::{RawDisplayHandle, RawWindowHandle};

    let RawWindowHandle::Wayland(window_handle) =
        window.window_handle().ok()?.as_raw()
    else {
        return None;
    };
    let RawDisplayHandle::Wayland(display_handle) =
        window.display_handle().ok()?.as_raw()
    else {
        return None;
    };

    // Keep the iced window borrowed while ashpd uses its raw Wayland surface.
    // The returned token is an owned string and is immediately handed to the
    // child process.
    unsafe {
        iced::futures::executor::block_on(
            ashpd::ActivationToken::from_wayland_raw(
                None,
                window_handle.surface.as_ptr(),
                display_handle.display.as_ptr(),
            ),
        )
    }
    .map(String::from)
}

#[cfg(target_os = "linux")]
fn open_url_linux(url: &str, activation_token: Option<String>) {
    let Some(token) = activation_token else {
        let _ = open::that_detached(url);
        return;
    };

    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(url)
        .env("XDG_ACTIVATION_TOKEN", &token)
        .env("DESKTOP_STARTUP_ID", token);

    if let Ok(mut child) = command.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    } else {
        let _ = open::that_detached(url);
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "linux")))]
pub fn open_url<Message>(
    url: &str,
    _parent: Option<iced::window::Id>,
) -> iced::Task<Message> {
    let _ = open::that_detached(url);
    iced::Task::none()
}

/// Web opens the tab synchronously so the browser still sees the click as a
/// user gesture and does not block it as a pop-up.
#[cfg(target_arch = "wasm32")]
pub fn open_url<Message>(
    url: &str,
    _parent: Option<iced::window::Id>,
) -> iced::Task<Message> {
    if let Some(window) = web_sys::window() {
        let _ = window.open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer");
    }
    iced::Task::none()
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static BEFORE_UNLOAD_WARNING: std::cell::RefCell<
        Option<wasm_bindgen::closure::Closure<dyn FnMut(web_sys::BeforeUnloadEvent)>>
    > = const { std::cell::RefCell::new(None) };
}

/// Enable the browser's standard leave-page confirmation while drawings have
/// unsaved changes. Browsers control the dialog text; preventing the event and
/// setting `returnValue` are the portable signal that a warning is required.
///
/// The callback is installed only while needed so clean sessions remain
/// eligible for the browser back/forward cache.
#[cfg(target_arch = "wasm32")]
pub fn set_unsaved_changes_warning(active: bool) {
    use wasm_bindgen::JsCast;

    let Some(window) = web_sys::window() else {
        return;
    };
    BEFORE_UNLOAD_WARNING.with(|warning| {
        let mut warning = warning.borrow_mut();
        if active == warning.is_some() {
            return;
        }
        if active {
            let callback = wasm_bindgen::closure::Closure::new(
                move |event: web_sys::BeforeUnloadEvent| {
                    event.prevent_default();
                    event.set_return_value("");
                },
            );
            window.set_onbeforeunload(Some(callback.as_ref().unchecked_ref()));
            *warning = Some(callback);
        } else {
            window.set_onbeforeunload(None);
            warning.take();
        }
    });
}

/// Reveal a saved drawing in the native platform's file manager.
#[cfg(not(target_arch = "wasm32"))]
pub fn reveal_in_file_manager(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("explorer.exe")
        .arg("/select,")
        .arg(path)
        .status();

    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .status();

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let folder = path
            .parent()
            .ok_or_else(|| format!("Path has no parent folder: {}", path.display()))?;
        return open::that(folder).map_err(|e| e.to_string());
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("File manager exited with {status}")),
        Err(error) => Err(error.to_string()),
    }
}

/// Copy the rendered web canvas during the frame callback, before the browser
/// clears its drawing buffer. Canvas readback avoids Iced's synchronous GPU map.
#[cfg(target_arch = "wasm32")]
pub fn capture_canvas() -> Option<iced::window::Screenshot> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window()?;
    let document = window.document()?;
    let source = document
        .query_selector("canvas")
        .ok()??
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()?;
    let (width, height) = (source.width(), source.height());
    if width == 0 || height == 0 {
        return None;
    }
    let copy = document
        .create_element("canvas")
        .ok()?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()?;
    copy.set_width(width);
    copy.set_height(height);
    let context = copy
        .get_context("2d")
        .ok()??
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .ok()?;
    context
        .draw_image_with_html_canvas_element(&source, 0.0, 0.0)
        .ok()?;
    let pixels = context
        .get_image_data(0.0, 0.0, width as f64, height as f64)
        .ok()?;
    Some(iced::window::Screenshot::new(
        pixels.data().0,
        iced::Size::new(width, height),
        window.device_pixel_ratio() as f32,
    ))
}

/// Web: read text from the system clipboard via the async Clipboard API.
/// iced's own `clipboard::read` is a no-op on the web (the browser clipboard is
/// async + permission-gated), so the editor paste paths use this instead. The
/// Ctrl+V keypress that drives it is a user gesture, so the read is permitted.
/// Returns `None` when denied, empty, or unsupported.
#[cfg(target_arch = "wasm32")]
pub async fn read_clipboard_text() -> Option<String> {
    let clipboard = web_sys::window()?.navigator().clipboard();
    let value = wasm_bindgen_futures::JsFuture::from(clipboard.read_text())
        .await
        .ok()?;
    value.as_string()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(module = "/web/clipboard.js")]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = copyHistory)]
    pub fn copy_history_text(text: &str, fallback_label: &str, close_label: &str) -> js_sys::Promise;
}

/// Web: write text to the system clipboard (fire-and-forget). Backs Ctrl+C in
/// plain text_input fields, whose iced-internal copy is a no-op on the web
/// (#346). The triggering keypress is a user gesture, so the write is allowed.
#[cfg(target_arch = "wasm32")]
pub fn write_clipboard_text(text: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.navigator().clipboard().write_text(text);
    }
}

/// Web: replay `text` into the focused iced widget as synthetic KeyboardEvents
/// dispatched on the winit canvas. This is the only route INTO a focused
/// text_input on the web — iced's clipboard read is a no-op there and it
/// exposes no insert-text operation (#346). Control characters are dropped
/// and the length capped so a runaway clipboard can't wedge the event loop.
#[cfg(target_arch = "wasm32")]
pub fn synthesize_typing(text: &str) {
    let Some(canvas) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector("canvas").ok().flatten())
    else {
        return;
    };
    for ch in text.chars().filter(|c| !c.is_control()).take(1024) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(&ch.to_string());
        init.set_bubbles(true);
        init.set_cancelable(true);
        for kind in ["keydown", "keyup"] {
            if let Ok(ev) =
                web_sys::KeyboardEvent::new_with_keyboard_event_init_dict(kind, &init)
            {
                let _ = canvas.dispatch_event(&ev);
            }
        }
    }
}

/// Turn an `rfd` file handle into a `PathBuf` the rest of the app keys on.
///
/// Desktop returns the real filesystem path. The browser has no path, so we
/// synthesize one from the file name — enough for the app to compile and track
/// the document name; actual byte I/O on the web reads the handle directly
/// (a follow-up).
#[cfg(not(target_arch = "wasm32"))]
pub fn handle_path(h: &rfd::FileHandle) -> std::path::PathBuf {
    let p = h.path().to_path_buf();
    // Every dialog result funnels through here — remember its folder so the
    // next dialog opens where the user left off.
    crate::config::remember_dialog_dir(&p);
    p
}

#[cfg(target_arch = "wasm32")]
pub fn handle_path(h: &rfd::FileHandle) -> std::path::PathBuf {
    std::path::PathBuf::from(h.file_name())
}

/// New async file dialog seeded with the last directory a dialog was used in.
/// All pickers should start from this instead of `rfd::AsyncFileDialog::new()`.
#[cfg(not(target_arch = "wasm32"))]
pub fn file_dialog() -> rfd::AsyncFileDialog {
    let dlg = rfd::AsyncFileDialog::new();
    match crate::config::last_dialog_dir() {
        Some(dir) => dlg.set_directory(dir),
        None => dlg,
    }
}

/// Blocking native file dialog seeded with the last-used directory.
///
/// Use this through `iced::window::run` when the picker needs the main
/// window's raw handle. Portal-based desktops can otherwise reject or lose a
/// parentless save request without ever showing a dialog (#537).
#[cfg(not(target_arch = "wasm32"))]
pub fn blocking_file_dialog() -> rfd::FileDialog {
    let dlg = rfd::FileDialog::new();
    match crate::config::last_dialog_dir() {
        Some(dir) => dlg.set_directory(dir),
        None => dlg,
    }
}

/// Web: no filesystem paths, nothing to remember.
#[cfg(target_arch = "wasm32")]
pub fn file_dialog() -> rfd::AsyncFileDialog {
    rfd::AsyncFileDialog::new()
}

/// Trigger a browser download of `bytes` as `name`. Builds a Blob, points a
/// hidden `<a download>` at it and clicks it programmatically — because this
/// runs inside the Save button's click (a user gesture), the file downloads
/// immediately with no extra "click to download" link. Web only.
#[cfg(target_arch = "wasm32")]
pub fn download_bytes(name: &str, bytes: &[u8]) {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else { return };
    let Some(document) = window.document() else { return };
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(&parts) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };
    if let Ok(el) = document.create_element("a") {
        let a: web_sys::HtmlAnchorElement = el.unchecked_into();
        a.set_href(&url);
        a.set_download(name);
        a.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// Short platform string for bug reports: OS + architecture on the desktop,
/// the browser user-agent on the web.
#[cfg(not(target_arch = "wasm32"))]
pub fn platform_info() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(target_arch = "wasm32")]
pub fn platform_info() -> String {
    web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .map(|ua| format!("Web — {ua}"))
        .unwrap_or_else(|| "Web (wasm)".to_string())
}

/// Percent-encode `s` for use in a URL query value (e.g. a GitHub issue
/// `?body=`). Encodes everything outside the unreserved set.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// FORK CONSTRUCTO : les deux predicats du garde-fou graphique.
///
/// Ils sont PURS — une chaine entre, un booleen sort — et compiles sur TOUTES
/// les cibles, alors que le module `web_diag` qui les emploie est
/// `#[cfg(target_arch = "wasm32")]`.
///
/// 🔴 C'EST LA RAISON D'ETRE DE CE MODULE. Tant qu'ils vivaient dans
/// `web_diag`, aucun test de ce depot ne pouvait les compiler : la seule facon
/// de les mesurer etait d'en recopier une REPLIQUE a cote, c'est-a-dire de
/// mesurer la copie et non le code servi. Les deux defauts corriges plus bas —
/// la sous-chaine « naga » et les chemins de la dependance git `iced` — ont
/// justement ete trouves sur une replique, faute de mieux. Ici, la CI les
/// execute.
pub mod garde_graphique {
    /// Vrai pour les enregistrements de journal venus de la pile de rendu.
    ///
    /// On juge la CIBLE du journal, pas le texte du message : la cible est une
    /// donnee structuree (`wgpu::backend::wgpu_core`), le texte est de la prose
    /// qui change avec la version et la langue du pilote.
    ///
    /// L'egalite est ancree sur la frontiere de module (`::`) plutot que sur un
    /// simple prefixe : `starts_with("naga")` accepterait une caisse nommee
    /// `nagareru`, et ce fichier a deja paye le prix d'une sous-chaine trois
    /// fois (voir `panique_de_rendu`).
    ///
    /// LES QUATRE PREMIERES SONT LES SEULES DONT ON AIT MESURE DES SITES
    /// `error!` : mesure du 2026-09-20 sur les sources du cache reel — `wgpu`
    /// 13, `wgpu-core` 15, `wgpu-hal` 77, `naga` 54. `cosmic-text` en a 1.
    /// `iced_wgpu`, `iced_graphics`, `cryoglyph` et `lyon_tessellation` n'en ont
    /// AUCUN aujourd'hui ; on les garde parce qu'elles sont gratuites et qu'une
    /// version future peut en emettre, mais il ne faut pas lire cette liste
    /// comme une mesure de ce qui arrive vraiment.
    ///
    /// ⚠️ `glyphon` a ete RETIRE : `grep -c glyphon Cargo.lock` rend **0**.
    /// Cette caisse n'est pas une dependance de ce projet — `cryoglyph` l'a
    /// remplacee. Une entree qui ne peut jamais correspondre se lit comme une
    /// couverture ; c'en est le contraire.
    pub fn est_de_la_pile_graphique(cible: &str) -> bool {
        const CAISSES: [&str; 10] = [
            // Mesurees comme emettrices.
            "wgpu",
            "wgpu_core",
            "wgpu_hal",
            "naga",
            "cosmic_text",
            // Muettes aujourd'hui, gardees par precaution.
            "iced_wgpu",
            "iced_graphics",
            "cryoglyph",
            "lyon_tessellation",
            "glow",
        ];
        CAISSES
            .iter()
            .any(|c| cible == *c || cible.starts_with(&format!("{c}::")))
    }

    /// Les FAMILLES de caisses de rendu, telles qu'elles apparaissent dans un
    /// chemin de caisse (`wgpu-core`, `lyon_tessellation`, ...).
    ///
    /// 🔴 DES FAMILLES, ET PLUS UNE LISTE DE NOMS. Une liste ecrite a la main
    /// en comptait 11 ; `Cargo.lock` en epingle **24**. Les 13 manquantes
    /// n'etaient pas exotiques : `wgpu-types`, `wgpu-naga-bridge` et les quatre
    /// `wgpu-core-deps-*` sont au coeur du rendu, et la version d'AMONT les
    /// attrapait — c'est mon propre correctif qui les avait perdues en exigeant
    /// un chiffre juste apres `wgpu-`. Mesure du 2026-09-20 : 3 chemins sur 7
    /// de la seule famille `wgpu` etaient devenus invisibles.
    ///
    /// `lyon` couvre `lyon`, `lyon_algorithms`, `lyon_geom`, `lyon_path` et
    /// `lyon_tessellation` grace au separateur `_`.
    const FAMILLES: [&str; 10] = [
        "wgpu",
        "naga",
        "cosmic-text",
        "cryoglyph",
        "etagere",
        "glow",
        "guillotiere",
        "lyon",
        "swash",
        // Le SECOND moteur de rendu, celui qu`iced` emprunte quand wgpu
        // echoue -- donc celui qui casse en second sur un GPU mobile. Il
        // manquait, avec ses 417 sites de panique (`tiny-skia` 205,
        // `tiny-skia-path` 212).
        "tiny-skia",
    ];

    /// ⚠️ CE QUE CES FAMILLES NE COUVRENT PAS, ET C'EST DELIBERE.
    ///
    /// La campagne de mutation du 2026-09-20 a releve des sites de panique dans
    /// `ttf-parser` (165), `rustybuzz` (116), `fontdb`, `zeno`, `kurbo`,
    /// `arrayvec` (116) et `bytemuck` (63). Les trois dernieres sont des caisses
    /// utilitaires que NOTRE propre code emploie aussi : les classer « rendu »
    /// ferait remplacer la page sur nos bogues a nous. Les quatre premieres
    /// sont la pile de POLICES ; elles cassent aussi bien a la mise en page
    /// d'un texte de dessin qu'au rendu, et rien ne mesure aujourd'hui laquelle
    /// des deux. On les laisse au bandeau d'amont, avec son bouton Copier.

    /// Reconnait une panique venue de la pile de rendu.
    ///
    /// Ici on n'a que le TEXTE — une panique ne porte pas de cible de journal.
    /// Ce qu'on y cherche est donc le CHEMIN DE FICHIER, qui est fige a la
    /// compilation et qui a une FORME, pas de la prose.
    ///
    /// 🔴 QUATRE FUITES SUCCESSIVES, TOUTES LA MEME CLASSE, ET CHAQUE CORRECTIF
    /// A ENGENDRE LA SUIVANTE. A lire avant de toucher a cette fonction.
    ///
    /// 1. `"naga"` NU. `layer "managa-01" has no linetype` contient « naga ».
    /// 2. `"naga-"`, cense ancrer par le tiret de version. `managa-01` porte un
    ///    tiret : la fuite est intacte.
    /// 3. `"naga-"` a une FRONTIERE DE MOT a gauche. 9 faux positifs sur 14
    ///    charges, dont `bloc introuvable : mur-naga-12` — une frontiere de mot
    ///    est exactement ce que `mur-naga-12` possede.
    /// 4. Separateur a gauche + CHIFFRE a droite. Ferme les faux positifs, mais
    ///    **ouvre un faux negatif** : `/wgpu-types-29.0.4/` n'a pas de chiffre
    ///    apres `wgpu-`. Trois caisses de rendu sur sept perdues.
    ///
    /// ➜ LA REGLE QUI EN SORT : on ne cherche plus une SOUS-CHAINE, on decoupe
    /// le texte en SEGMENTS DE CHEMIN et on juge chaque segment ENTIER. Un
    /// repertoire de caisse a la forme `<nom>-<version>` ou la version commence
    /// par un chiffre ET contient un point. Et il n'est jamais le DERNIER
    /// segment : un repertoire a toujours quelque chose apres lui.
    ///
    /// Ce que ces trois conditions ecartent, chacune une charge reelle :
    ///   - segment entier  -> `mur-naga-12` donne le nom `mur-naga`, pas `naga`
    ///   - le point        -> un dossier client `wgpu-2024` n'est pas une version
    ///   - pas le dernier  -> `/Projets/naga-2024.dwg` est un FICHIER
    ///
    /// Le cout d'un faux positif est concret : sur wasm la panique est
    /// terminale, `show_fatal` REMPLACE la page et supprime le canevas. Un
    /// client dont un calque s'appelle `naga-01` perdrait son seul diagnostic.
    pub fn panique_de_rendu(texte: &str) -> bool {
        /// Textes, pas des chemins.
        const MESSAGES: [&str; 2] = ["wgpu error", "Shader compilation failed"];
        /// Sous-repertoires de rendu de la dependance git `iced`.
        ///
        /// Ils ne comptent QUE dans un chemin qui contient deja un checkout
        /// `iced-<empreinte>` : sans cette seconde condition,
        /// `C:\Chantiers\graphics\src\facade.dwg` serait classe « rendu ». Et on
        /// ne prend pas `iced` en bloc : une panique de widget reste NOTRE
        /// bogue, avec le bandeau et son bouton Copier.
        const ICED_RENDU: [&str; 4] = ["wgpu", "graphics", "renderer", "tiny_skia"];

        if MESSAGES.iter().any(|m| texte.contains(m)) {
            return true;
        }
        let segments: Vec<&str> = texte.split(['/', '\\']).collect();
        // Le DERNIER segment est un nom de fichier, jamais un repertoire de
        // caisse. C'est lui qui distingue `…/naga-29.0.4/src/lib.rs` de
        // `impossible d ouvrir /Projets/naga-2024.dwg`.
        let repertoires = &segments[..segments.len().saturating_sub(1)];
        if repertoires.iter().any(|s| est_repertoire_de_caisse(s)) {
            return true;
        }
        // Les caisses de rendu prises en dependance GIT n'ont pas de version
        // dans leur repertoire, mais une empreinte : `cryoglyph-9f2a1c53ba3e8`.
        if repertoires
            .iter()
            .any(|s| FAMILLES.iter().any(|f| est_checkout_git(s, f)))
        {
            return true;
        }
        // `iced` est une dependance GIT : son repertoire est `iced-<empreinte>`,
        // sans version. On exige donc le checkout ET un sous-repertoire de rendu
        // APRES lui, dans le MEME chemin.
        //
        // ⚠️ « Apres lui » n'est pas un detail. Une premiere version posait deux
        // `any` independants sur la liste entiere : une panique citant DEUX
        // chemins -- `xref /Chantiers/iced-2024/plan.dwg introuvable depuis
        // /Chantiers/graphics/src/facade.dwg` -- satisfaisait les deux
        // conditions par deux moities sans rapport, et remplacait la page d'un
        // client. Chaque moitie seule rendait `false` : c'est la CONJONCTION
        // qui etait decomposable.
        //
        // ⚠️ ET « APRES » NE SUFFIT PAS NON PLUS : `iced-2024abc` est une
        // empreinte hexadecimale parfaitement valide, donc un dossier de
        // chantier nomme ainsi, suivi PLUS LOIN dans le texte d'un second
        // chemin contenant `graphics`, satisfaisait encore la conjonction.
        // On exige donc la POSITION EXACTE du gabarit de cargo :
        // `iced-<empreinte>/<revision>/<repertoire de rendu>`, soit i+2.
        repertoires
            .iter()
            .position(|s| est_checkout_git(s, "iced"))
            .and_then(|i| repertoires.get(i + 2))
            .is_some_and(|s| ICED_RENDU.contains(s))
    }

    /// Vrai si `segment` est un repertoire de caisse d'une famille de rendu :
    /// `<nom>-<version>`, version commencant par un chiffre et contenant un
    /// point.
    fn est_repertoire_de_caisse(segment: &str) -> bool {
        let Some((nom, version)) = segment.rsplit_once('-') else {
            return false;
        };
        if !version.starts_with(|c: char| c.is_ascii_digit()) || !version.contains('.') {
            return false;
        }
        FAMILLES.iter().any(|f| {
            nom == *f
                || nom.starts_with(&format!("{f}-"))
                || nom.starts_with(&format!("{f}_"))
        })
    }

    /// Vrai si `segment` est un checkout git de `caisse` : `<nom>-<empreinte>`,
    /// ou l'empreinte fait au moins SEPT caracteres hexadecimaux DONT UNE
    /// LETTRE.
    ///
    /// La lettre n'est pas un ornement : sans elle, `/Projets/naga-123456/` --
    /// un numero de dossier de chantier -- serait un checkout git, parce que les
    /// chiffres sont aussi des caracteres hexadecimaux. C'est la meme classe que
    /// les quatre fuites de `panique_de_rendu`, une cinquieme fois.
    fn est_checkout_git(segment: &str, caisse: &str) -> bool {
        segment
            .strip_prefix(caisse)
            .and_then(|r| r.strip_prefix('-'))
            .is_some_and(|h| {
                h.len() >= 7
                    && h.chars().all(|c| c.is_ascii_hexdigit())
                    && h.chars().any(|c| c.is_ascii_alphabetic())
            })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Chemins REELS, batis sur les versions que `Cargo.lock` epingle.
        const PANIQUES_DE_RENDU: [&str; 14] = [
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/wgpu-29.0.4/src/backend/wgpu_core.rs:685:\nwgpu error: Validation Error",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/naga-29.0.4/src/back/glsl/mod.rs:1204:\nnot implemented",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/wgpu-hal-29.0.4/src/gles/device.rs:220:\nboom",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/wgpu-core-29.0.4/src/device/resource.rs:1:\nboom",
            // Les trois que le correctif precedent avait PERDUES.
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/wgpu-types-29.0.4/src/lib.rs:1:\nboom",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/wgpu-naga-bridge-29.0.4/src/lib.rs:1:\nboom",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/wgpu-core-deps-wasm-29.0.4/src/lib.rs:1:\nboom",
            // Les familles ajoutees.
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/etagere-0.2.15/src/allocator.rs:412:\nattempt to subtract with overflow",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/guillotiere-0.7.0/src/lib.rs:88:\nassertion failed",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/lyon_path-1.0.19/src/builder.rs:77:\nunwrap on None",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/cosmic-text-0.19.0/src/font/system.rs:405:\nno face",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/swash-0.2.9/src/scale/mod.rs:12:\nboom",
            "panicked at /home/runner/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tiny-skia-0.11.4/src/pipeline/highp.rs:1:\nboom",
            // Poste de developpement Windows.
            r"panicked at C:\Users\dev\.cargo\registry\src\index.crates.io-6f17d22bba15001f\wgpu-29.0.4\src\lib.rs:1: boom",
        ];

        /// Les sous-repertoires de rendu d'`iced`, dependance GIT.
        const PANIQUES_ICED_RENDU: [&str; 5] = [
            "/home/runner/.cargo/git/checkouts/iced-3ba1e2e5c1e0e1a1/23604ff/wgpu/src/text.rs:406: Render text",
            "/home/runner/.cargo/git/checkouts/iced-3ba1e2e5c1e0e1a1/23604ff/graphics/src/compositor.rs:88: no adapter",
            "/home/runner/.cargo/git/checkouts/iced-3ba1e2e5c1e0e1a1/23604ff/renderer/src/fallback.rs:331: internal error",
            "/home/runner/.cargo/git/checkouts/iced-3ba1e2e5c1e0e1a1/23604ff/tiny_skia/src/window/compositor.rs:44: boom",
            "/home/runner/.cargo/git/checkouts/cryoglyph-9f2a1c53ba3e8/53ba3e8/src/text_atlas.rs:102: unwrap on None",
        ];

        /// Paniques qui ne sont PAS du rendu.
        ///
        /// Les douze premieres sont les charges qui ont FAIT TOMBER les quatre
        /// versions precedentes de cette garde. Elles restent ici a vie.
        const PANIQUES_ORDINAIRES: [&str; 19] = [
            "panicked at src/entities/layer.rs:88:\nlayer \"managa-01\" has no linetype",
            "panicked at src/entities/layer.rs:88:\nlayer \"naga-01\" has no linetype",
            "panicked at src/entities/layer.rs:88:\nlayer \"wgpu-02\" has no linetype",
            "bloc introuvable : mur-naga-12",
            "bloc introuvable : /Projets/mur-naga-12/plan.dwg",
            "cannot open naga-plan.dwg",
            "impossible d ouvrir /Projets/naga-2024.dwg",
            "naga::: cle absente",
            "calque \u{00ab}\u{00a0}naga-01\u{00a0}\u{00bb} introuvable",
            "calque \u{2014}naga-01 introuvable",
            r"impossible d ouvrir C:\Projets\wgpu\src\plan.dwg",
            r"impossible d ouvrir C:\Chantiers\graphics\src\facade.dwg",
            // Un dossier client dont le nom ressemble a une caisse : le point
            // manquant dans « 2024 » le sauve.
            r"impossible d ouvrir C:\Projets\wgpu-2024\plan.dwg",
            // Un numero de dossier de chantier n'est pas une empreinte git :
            // les chiffres sont hexadecimaux, il faut au moins une lettre.
            "impossible d ouvrir /Projets/naga-123456/plan.dwg",
            // Un widget d'`iced` reste notre bogue.
            "/home/runner/.cargo/git/checkouts/iced-3ba1e2e5c1e0e1a1/23604ff/widget/src/text_input.rs:12: index out of bounds",
            "\u{56fe}\u{5c42}naga-01 introuvable",
            "panicked at src/scene/mod.rs:1204:\nattempt to divide by zero",
            "panicked at src/io/dwg.rs:77:\ncalled `Option::unwrap()` on a `None` value",
            "panicked at src/app/update/mod.rs:9:\nno document",
        ];

        #[test]
        fn une_panique_de_rendu_est_reconnue_sur_toutes_les_familles_de_chemin() {
            for texte in PANIQUES_DE_RENDU {
                assert!(panique_de_rendu(texte), "ratee : {texte}");
            }
        }

        /// 🔴 LA BRANCHE `MESSAGES`, SEULE. Elle n'etait surveillee par RIEN :
        /// les deux charges du banc qui portaient ces phrases portaient AUSSI un
        /// chemin de registre, donc six mutations sur six y survivaient — dont
        /// `"wgpu error"` reduit a `"error"`, qui aurait fait remplacer la page
        /// sur CHAQUE `unreachable!()` de notre propre code.
        #[test]
        fn les_deux_messages_suffisent_seuls_et_ne_sont_pas_generiques() {
            // Sans aucun chemin : c'est le message qui doit decider.
            assert!(panique_de_rendu("wgpu error: Validation Error"));
            assert!(panique_de_rendu("Shader compilation failed"));
            // Et ils ne doivent pas etre elargis a un mot generique : voici le
            // texte exact que produit `unreachable!()` chez nous.
            assert!(!panique_de_rendu(
                "panicked at src/scene/mod.rs:1204:\ninternal error: entered unreachable code"
            ));
            assert!(!panique_de_rendu("error"));
            assert!(!panique_de_rendu("compilation failed"));
            // La casse compte : ce sont des chaines d'amont, pas de la prose.
            assert!(!panique_de_rendu("WGPU ERROR: boom"));
        }

        /// Les charges du rapport de mutation qui distinguaient un survivant de
        /// l'original. Chacune a coute une mutation qui passait le banc.
        #[test]
        fn les_ancres_de_caisse_ne_se_laissent_pas_affaiblir() {
            // Chiffre ASCII, pas `is_numeric` : le `²` et le bengali sont des
            // chiffres Unicode.
            assert!(!panique_de_rendu("/x/naga-\u{00b2}.0/src/lib.rs"));
            assert!(!panique_de_rendu("/x/swash-\u{09e9}.0/src/lib.rs"));
            // Chiffre, pas hexadecimal : `blanc` commence par un `b`.
            assert!(!panique_de_rendu("/plans/etagere-blanc/dessin.dwg"));
            // Frontiere de module en PREFIXE, pas n'importe ou.
            assert!(!est_de_la_pile_graphique("constructo::wgpu::pont"));
            assert!(!est_de_la_pile_graphique("wgpu:backend"));
            // DEUX occurrences : la premiere echoue, la seconde est la vraie.
            assert!(panique_de_rendu(
                "cannot open /naga-plan.dwg while loading /naga-29.0.4/src/lib.rs"
            ));
            // Empreinte git : il faut une LETTRE, sinon un numero de dossier
            // passe. Et elle peut commencer par une lettre.
            assert!(panique_de_rendu("/co/cryoglyph-a1b2c3d4e5f60789/rev/src/x.rs"));
            assert!(!panique_de_rendu("/co/cryoglyph-1234567/rev/src/x.rs"));
        }

        /// 🔴 LES CHEMINS D'UN VRAI CLIENT. `etagere` et `guillotiere` sont des
        /// caisses Rust ; « etagere » et « guillotine » sont des mots de
        /// MENUISERIE, et le tenant de ce depot est un atelier de menuiserie.
        /// Une version precedente classait les neuf premiers en « rendu » : le
        /// dessin etait retire de l'ecran et remplace par « ce module demande un
        /// ordinateur ».
        #[test]
        fn un_nom_de_fichier_de_menuisier_n_est_pas_une_caisse_de_rendu() {
            for chemin in [
                r"C:\Chantiers\Cuisine Tremblay\etagere-3-tablettes.dwg",
                r"C:\Armoires\etagere-24po.dwg",
                r"C:\Ilots\etagere-9-cases.dwg",
                r"C:\Garde-robe\etagere-1.dwg",
                r"C:\Projets\etagere-2\plan.dwg",
                "https://app.constructoai.ca/plans/etagere-2.dwg",
                "/plans/etagere-3.dwg",
                "/Volumes/Chantiers/Cabanon/etagere-4.dwg",
                r"C:\Atelier\guillotiere-2.dwg",
                r"C:\Rive-Sud-2024\plan-etage-2.dwg",
                r"C:\Chantiers\solive-2x10-16.dwg",
            ] {
                assert!(!panique_de_rendu(chemin), "faux positif : {chemin}");
            }
            // TEMOIN : la vraie caisse, elle, reste reconnue.
            assert!(panique_de_rendu(
                "at /registry/src/x/etagere-0.2.15/src/allocator.rs:1:"
            ));
        }

        /// La conjonction de la branche `iced` ne doit pas se satisfaire de deux
        /// moities sans rapport.
        #[test]
        fn la_branche_iced_exige_un_seul_et_meme_chemin() {
            assert!(!panique_de_rendu(
                "xref /Chantiers/iced-2024abc/plan.dwg introuvable depuis \
                 /Chantiers/graphics/src/facade.dwg"
            ));
            // Le sous-repertoire AVANT le checkout ne compte pas non plus.
            assert!(!panique_de_rendu("/x/graphics/src/y/iced-3ba1e2e5c1e0e1a1/z.rs"));
            // Mais le vrai chemin, lui, compte.
            assert!(panique_de_rendu(
                "/co/iced-3ba1e2e5c1e0e1a1/23604ff/graphics/src/compositor.rs:88:"
            ));
        }

        /// Chaque entree de `CAISSES` doit avoir sa charge. Deux n'en avaient
        /// aucune, et une entree sans temoin peut etre retiree sans rougeur.
        #[test]
        fn chaque_cible_de_journal_gardee_a_sa_charge() {
            for (cible, caisse) in [
                ("wgpu::backend::x", "wgpu"),
                ("wgpu_core::device::x", "wgpu_core"),
                ("wgpu_hal::gles::x", "wgpu_hal"),
                ("naga::valid::x", "naga"),
                ("cosmic_text::font::x", "cosmic_text"),
                ("iced_wgpu::window::x", "iced_wgpu"),
                ("iced_graphics::compositor", "iced_graphics"),
                ("cryoglyph::text_atlas", "cryoglyph"),
                ("lyon_tessellation::fill", "lyon_tessellation"),
                ("glow::native", "glow"),
            ] {
                assert!(
                    est_de_la_pile_graphique(cible),
                    "l'entree `{caisse}` n'est gardee par rien"
                );
            }
        }

        #[test]
        fn les_sous_repertoires_de_rendu_d_iced_sont_reconnus() {
            for texte in PANIQUES_ICED_RENDU {
                assert!(panique_de_rendu(texte), "ratee : {texte}");
            }
        }

        #[test]
        fn une_panique_ordinaire_garde_le_bandeau_et_son_bouton_copier() {
            for texte in PANIQUES_ORDINAIRES {
                assert!(!panique_de_rendu(texte), "faux positif : {texte}");
            }
        }

        /// Les TROIS conditions, isolees. C'est le correctif lui-meme : toute
        /// forme qui perd l'une des trois doit basculer.
        #[test]
        fn un_repertoire_de_caisse_demande_un_segment_entier_un_point_et_une_suite() {
            assert!(panique_de_rendu("at /naga-29.0.4/src/lib.rs"));
            assert!(panique_de_rendu(r"at \naga-29.0.4\src\lib.rs"));
            // Segment entier : `mur-naga` n'est pas `naga`.
            assert!(!panique_de_rendu("/x/mur-naga-12/y"));
            // Le point : `2024` n'est pas une version de caisse.
            assert!(!panique_de_rendu("/x/wgpu-2024/y"));
            // Pas le dernier segment : un fichier n'est pas un repertoire.
            assert!(!panique_de_rendu("/x/naga-29.0.4"));
        }

        /// 🔴 LE CLIQUET CONTRE LA DERIVE. `FAMILLES` est ecrit a la main et
        /// `Cargo.lock` bouge a chaque rebase sur amont. Ce test lit le VRAI
        /// `Cargo.lock` a la compilation et refuse les deux derives :
        ///   - une caisse de rendu du graphe qu'aucune famille ne couvre ;
        ///   - une famille qui ne couvre plus rien (elle ment sur ce qu'elle garde).
        ///
        /// Sans lui, les 13 caisses perdues par le correctif precedent seraient
        /// restees invisibles : les tests figes ci-dessus ne connaissent que les
        /// chemins qu'on a pense a y ecrire.
        #[test]
        fn les_familles_couvrent_ce_que_cargo_lock_epingle() {
            let lock = include_str!("../Cargo.lock");
            let mut paquets: Vec<(String, String)> = Vec::new();
            let mut nom = String::new();
            for ligne in lock.lines() {
                if let Some(v) = ligne.strip_prefix("name = \"") {
                    nom = v.trim_end_matches('"').to_string();
                } else if let Some(v) = ligne.strip_prefix("version = \"") {
                    if !nom.is_empty() {
                        paquets.push((nom.clone(), v.trim_end_matches('"').to_string()));
                        nom.clear();
                    }
                }
            }
            // Temoin : un Cargo.lock qu'on lit mal rend zero paquet, et un
            // « aucune caisse manquante » sur zero paquet ne prouve rien.
            assert!(
                paquets.len() > 200,
                "lecture de Cargo.lock cassee : {} paquets",
                paquets.len()
            );

            let de_rendu: Vec<&(String, String)> = paquets
                .iter()
                .filter(|(n, _)| {
                    FAMILLES.iter().any(|f| {
                        n == f
                            || n.starts_with(&format!("{f}-"))
                            || n.starts_with(&format!("{f}_"))
                    })
                })
                .collect();
            assert!(
                de_rendu.len() >= 20,
                "seulement {} caisses de rendu trouvees : les familles ont derive",
                de_rendu.len()
            );

            // Chaque caisse de rendu epinglee doit etre reconnue dans un chemin.
            for (n, v) in &de_rendu {
                let chemin = format!("panicked at /home/x/.cargo/registry/src/idx/{n}-{v}/src/lib.rs:1:");
                assert!(
                    panique_de_rendu(&chemin),
                    "caisse de rendu non couverte : {n}-{v}"
                );
            }

            // Et chaque famille doit couvrir au moins une caisse reelle.
            for f in FAMILLES {
                assert!(
                    de_rendu.iter().any(|(n, _)| {
                        n == f
                            || n.starts_with(&format!("{f}-"))
                            || n.starts_with(&format!("{f}_"))
                    }),
                    "la famille `{f}` ne couvre aucune caisse de Cargo.lock : \
                     elle se lit comme une couverture et n'en est pas une"
                );
            }
        }

        #[test]
        fn la_cible_de_journal_designe_la_pile_de_rendu() {
            for cible in [
                "wgpu",
                "wgpu::backend::wgpu_core",
                "wgpu_core::device::resource",
                "wgpu_hal::gles::adapter",
                "naga::valid::interface",
                "cosmic_text::font::system",
                "iced_wgpu::window::compositor",
                "cryoglyph::text_atlas",
                "glow::native",
            ] {
                assert!(est_de_la_pile_graphique(cible), "ratee : {cible}");
            }
        }

        #[test]
        fn la_cible_de_journal_ne_designe_pas_notre_propre_code() {
            for cible in [
                "OpenCADStudio::entities::layer",
                "OpenCADStudio",
                "iced_winit::program",
                "acadrust::io",
                // Un prefixe nu aurait avale les trois suivantes.
                "wgpuzzle::core",
                "nagareru",
                "cosmic_textile::x",
            ] {
                assert!(!est_de_la_pile_graphique(cible), "faux positif : {cible}");
            }
        }

        /// `glyphon` n'est pas une dependance de ce projet (`Cargo.lock` : 0
        /// occurrence). Une entree qui ne peut jamais correspondre se lit comme
        /// une couverture ; c'en est le contraire.
        #[test]
        fn glyphon_n_est_pas_gardee_car_elle_n_est_pas_une_dependance() {
            assert!(!est_de_la_pile_graphique("glyphon::text_atlas"));
            assert!(!panique_de_rendu("/x/glyphon-0.9.0/src/lib.rs"));
            assert!(!include_str!("../Cargo.lock").contains("\"glyphon\""));
        }
    }
}

/// Web renderer-error surface (#414): wgpu / naga report pipeline and shader
/// failures through the `log` facade and then leave the canvas empty — with no
/// logger installed the message is lost, so a broken GPU path looks like "the
/// app draws nothing" with a clean console. Mirror every Error-level record to
/// the browser console AND into a fixed DOM banner whose text is selectable
/// (the canvas UI is not), with a one-click Copy button, so a failing user can
/// paste the exact error into a bug report. Panics land in the same banner via
/// a chained hook.
#[cfg(target_arch = "wasm32")]
pub mod web_diag {
    use super::garde_graphique::{est_de_la_pile_graphique, panique_de_rendu};
    use std::sync::{Mutex, OnceLock};

    /// Cap on banner entries so a per-frame error can't grow the DOM forever.
    const MAX_LINES: u32 = 12;

    /// Last banner message + repeat count, for collapsing a hot error loop
    /// into one line with an `(xN)` suffix.
    static LAST: Mutex<(String, u32)> = Mutex::new((String::new(), 0));

    /// Les deux phrases de `show_fatal`, traduites, posees par `init`.
    ///
    /// Un `OnceLock` et non deux variables capturees : le crochet de panique est
    /// installe AVANT que les traductions soient resolues (voir `init`), donc il
    /// doit pouvoir lire « pas encore pret » sans se bloquer ni paniquer.
    static PHRASES: OnceLock<(String, String)> = OnceLock::new();

    /// Repli si une panique de rendu devance la resolution des traductions.
    /// C'est la source anglaise, la meme que celle du catalogue.
    const PHRASES_DE_SECOURS: (&str, &str) = (
        "The drawing module needs a computer: it does not open on this device.",
        "Open this link on a computer to draw.",
    );

    struct BannerLogger;

    impl log::Log for BannerLogger {
        fn enabled(&self, meta: &log::Metadata) -> bool {
            meta.level() <= log::Level::Error
        }
        fn log(&self, record: &log::Record) {
            if record.level() > log::Level::Error {
                return;
            }
            let msg = format!("[{}] {}", record.target(), record.args());
            web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&msg));
            // FORK CONSTRUCTO : ce qui vient de la pile graphique va dans la
            // console, jamais a l'ecran.
            //
            // Constate par Sylvain le 2026-09-19 sur iPhone : ouvrir le module
            // affichait un mur rouge portant un chemin de la machine de build de
            // GitHub et une invitation a ouvrir un bogue chez `gfx-rs/wgpu`.
            // C'est de l'interne d'amont servi a un client.
            //
            // La coupe est ici ET dans le crochet de panique : sans celle-ci, les
            // quatre lignes de `wgpu` s'affichent AVANT la panique et restent
            // visibles sous le message honnete. La console, elle, garde tout —
            // c'est nous qui en avons besoin, pas le visiteur.
            //
            // 🔴 ELLE SE TAIT, ET C'EST LE BON COMPORTEMENT. Une version
            // intermediaire affichait ici une phrase honnete « pour ne pas
            // laisser le silence ». Trois mesures du 2026-09-20 l'ont retiree,
            // et chacune suffisait :
            //
            // 1. LE SILENCE QU'ELLE VISAIT N'EST PAS ICI. Elle etait justifiee
            //    par « l'echec nominal du compositeur ne panique pas, donc seul
            //    ce journal peut parler ». Or `iced@23604ff` ne contient AUCUN
            //    `log::error!` dans `wgpu/`, `graphics/`, `renderer/` ni
            //    `winit/` : sur ce chemin, ce journal ne parle pas non plus. La
            //    phrase n'etait jamais atteinte. (Deux autres affirmations de
            //    cette justification etaient fausses : `run_web()` rend
            //    `Ok(())` sur wasm — `iced/winit/src/lib.rs:433-439` — et le
            //    voile de chargement est retire des qu'un canevas apparait,
            //    donc le visiteur voit une page blanche, pas un rotor.)
            // 2. ELLE ATTERRISSAIT DANS LE MAUVAIS CADRE. `show_banner` dessine
            //    le bandeau d'AMONT : fond #5c1a1a, monospace 12 px, titre
            //    « copiez ceci dans un rapport de bogue », bouton Copier. On
            //    retirait le chemin de la machine de build et on gardait tout
            //    ce qui en faisait de l'interne servi a un client.
            // 3. ELLE POUVAIT MENTIR. Un enregistrement `Error` n'est PAS
            //    terminal ici : `scene/pipeline/mod.rs:5661-5670` installe un
            //    `on_uncaptured_error` qui garde la session vivante. Un nuanceur
            //    qui echoue sur iPad aurait affiche « il ne s'ouvre pas sur cet
            //    appareil » AU-DESSUS du dessin ouvert.
            //
            // Ce qui reste vrai, et qui suffit : notre propre code n'emet AUCUN
            // `error!` (`grep -rnE '(log::)?error!\(' src crates` = 0). Chaque
            // ligne que ce bandeau pourrait afficher vient d'une caisse tierce,
            // et trois d'entre elles disent litteralement au client d'aller
            // ouvrir un bogue chez `gfx-rs/wgpu`. Il n'y a donc rien a sauver a
            // l'ecran, sur aucun appareil. La console garde tout.
            if est_de_la_pile_graphique(record.target()) {
                return;
            }
            show_banner(&msg);
        }
        fn flush(&self) {}
    }

    /// Install the logger + panic mirror. Call once at web startup, AFTER
    /// `console_error_panic_hook::set_once` so the chained hook keeps the
    /// console stack trace.
    pub fn init() {
        // 🔴 LE CROCHET EN PREMIER, AVANT DE RESOUDRE QUOI QUE CE SOIT.
        //
        // Ce qui suit peut paniquer : `crate::i18n::loader()` porte un
        // `.expect("fallback UI language must be embedded")` (`src/i18n.rs:218`).
        // Une version precedente resolvait les deux phrases AVANT `set_hook`, et
        // une panique la n'aurait rien montre du tout. L'amont reel
        // (`84836b0f:src/sys.rs`) avait `take_hook()` en premiere instruction ;
        // on revient a cet ordre.
        let console_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let texte = info.to_string();
            // Sur wasm il n'y a pas de deroulement de pile : une panique est
            // TOUJOURS terminale. « Panique attrapee » veut donc dire
            // « application morte », sans faux positif possible — c'est ce qui
            // rend ce rattrapage sans risque, et c'est ce qui distingue ce
            // chemin-ci du chemin du JOURNAL, ou un enregistrement `Error` peut
            // tres bien laisser la session vivante (voir `BannerLogger::log`).
            //
            // Une panique NON graphique garde le bandeau d'amont : ce sont nos
            // bogues a nous, et le bouton Copier y a de la valeur.
            if panique_de_rendu(&texte) {
                let (titre, sortie) = PHRASES
                    .get()
                    .map(|(t, s)| (t.as_str(), s.as_str()))
                    .unwrap_or(PHRASES_DE_SECOURS);
                show_fatal(titre, sortie);
            } else {
                show_banner(&texte);
            }
            // La trace complete continue d'aller dans la console, dans les deux
            // cas. Elle nous sert ; elle n'est simplement plus servie a l'ecran.
            console_hook(info);
        }));

        // Les phrases sont resolues MAINTENANT, une fois, pas dans le crochet :
        // rentrer dans le chargeur de traductions pendant une panique est un
        // risque qu'on peut supprimer sans rien couter.
        //
        // ⚠️ Elles sont donc figees dans la langue resolue au demarrage
        // (`Language::System`), tandis que `set_language` ne tourne qu'ensuite,
        // dans `app/mod.rs:3815`. Un utilisateur dont l'appareil est en anglais
        // mais qui a regle le module en francais verra ces deux phrases en
        // anglais. C'est assume : la seule facon de faire mieux serait de les
        // resoudre dans le crochet, ce que le paragraphe ci-dessus interdit.
        let _ = PHRASES.set((
            crate::t!("The drawing module needs a computer: it does not open on this device.")
                .into_owned(),
            crate::t!("Open this link on a computer to draw.").into_owned(),
        ));

        if log::set_boxed_logger(Box::new(BannerLogger)).is_ok() {
            log::set_max_level(log::LevelFilter::Error);
        }
    }


    /// Remplace la page par un message honnete. Rien de copiable, aucune trace,
    /// aucun lien vers un depot tiers.
    ///
    /// Elle REMPLACE au lieu d'ajouter une ligne : le bandeau d'amont empile, et
    /// empiler un message clair sous quatre lignes de `wgpu` ne repare rien. Elle
    /// retire aussi le canevas et le voile de chargement, qui resteraient sinon —
    /// l'un noir, l'autre en train de tourner pour une application deja morte.
    fn show_fatal(titre: &str, sortie: &str) {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        for id in ["ocs-err", "loading"] {
            if let Some(element) = doc.get_element_by_id(id) {
                element.remove();
            }
        }
        if let Ok(canevas) = doc.query_selector("canvas") {
            if let Some(canevas) = canevas {
                canevas.remove();
            }
        }
        let Some(body) = doc.body() else { return };
        let Ok(panneau) = doc.create_element("div") else {
            return;
        };
        panneau.set_id("ocs-fatal");
        let _ = panneau.set_attribute(
            "style",
            "position:fixed;inset:0;z-index:2147483647;display:flex;\
             flex-direction:column;align-items:center;justify-content:center;\
             gap:10px;padding:24px;text-align:center;background:#FAFAFA;\
             color:#1A1A1A;font:16px/1.5 system-ui,sans-serif;",
        );
        if let Ok(ligne) = doc.create_element("div") {
            let _ = ligne.set_attribute("style", "font-weight:600;max-width:34rem;");
            ligne.set_text_content(Some(titre));
            let _ = panneau.append_child(&ligne);
        }
        if let Ok(ligne) = doc.create_element("div") {
            let _ = ligne.set_attribute("style", "color:#555;max-width:34rem;");
            ligne.set_text_content(Some(sortie));
            let _ = panneau.append_child(&ligne);
        }
        let _ = body.append_child(&panneau);
    }

    /// Append `msg` to the on-page banner, creating the overlay on first use.
    fn show_banner(msg: &str) {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let pre = match doc.get_element_by_id("ocs-err-text") {
            Some(pre) => pre,
            None => {
                let Some(body) = doc.body() else { return };
                let Ok(overlay) = doc.create_element("div") else {
                    return;
                };
                overlay.set_id("ocs-err");
                let _ = overlay.set_attribute(
                    "style",
                    "position:fixed;top:0;left:0;right:0;z-index:2147483647;\
                     background:#5c1a1a;color:#ffdddd;font:12px monospace;\
                     padding:8px 12px;max-height:40vh;overflow:auto;\
                     user-select:text;cursor:text;",
                );
                overlay.set_inner_html(
                    "<div><b id=\"ocs-err-title\"></b> \
                     <button id=\"ocs-err-copy\" style=\"margin-left:8px\" onclick=\"navigator.clipboard.writeText(\
                     document.getElementById('ocs-err-text').innerText)\"></button> \
                     <button id=\"ocs-err-dismiss\" onclick=\"document.getElementById('ocs-err').remove()\"></button></div>\
                     <pre id=\"ocs-err-text\" style=\"margin:6px 0 0;\
                     white-space:pre-wrap;user-select:text;\"></pre>",
                );
                let _ = body.append_child(&overlay);
                for (id, label) in [
                    ("ocs-err-title", crate::t!("OpenCADStudio renderer error — copy this into a bug report:")),
                    ("ocs-err-copy", crate::t!("Copy")),
                    ("ocs-err-dismiss", crate::t!("Dismiss")),
                ] {
                    if let Some(element) = doc.get_element_by_id(id) {
                        element.set_text_content(Some(label.as_ref()));
                    }
                }
                match doc.get_element_by_id("ocs-err-text") {
                    Some(pre) => pre,
                    None => return,
                }
            }
        };
        // Collapse repeats: an error thrown every frame becomes one line with
        // a running (xN) counter instead of MAX_LINES copies of itself.
        let mut last = match LAST.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if last.0 == msg {
            last.1 += 1;
            if let Some(line) = pre.last_element_child() {
                line.set_text_content(Some(&format!("{msg} (x{})", last.1)));
            }
            return;
        }
        *last = (msg.to_string(), 1);
        if pre.child_element_count() >= MAX_LINES {
            if let Some(first) = pre.first_element_child() {
                first.remove();
            }
        }
        if let Ok(line) = doc.create_element("div") {
            line.set_text_content(Some(msg));
            let _ = pre.append_child(&line);
        }
    }
}
