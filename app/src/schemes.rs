use crate::session::Session;
use jadeslip_core::{Library, Theme, parse_key};
use std::borrow::Cow;
use tauri::http::{Request, Response, Uri, header};

pub const UI_SCHEME: &str = "jadeslip";
pub const BOOK_SCHEME: &str = "jadeslip-book";
pub const UI_URL: &str = "jadeslip://localhost/index.html";

pub const UI_CSP: &str = "default-src 'none'; script-src 'self'; \
    style-src 'self' 'unsafe-inline'; img-src jadeslip-book:; font-src 'self' jadeslip-book:; \
    media-src jadeslip-book:; connect-src ipc: http://ipc.localhost; frame-src 'none'; \
    object-src 'none'; base-uri 'none'; form-action 'none'";

const RESOURCE_CSP: &str =
    "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src jadeslip-book:";

macro_rules! font {
    ($name:literal) => {
        (
            concat!("fonts/", $name, ".woff2"),
            "font/woff2",
            include_bytes!(concat!("../ui/fonts/", $name, ".woff2")).as_slice(),
        )
    };
}

const UI_FILES: &[(&str, &str, &[u8])] = &[
    (
        "index.html",
        "text/html; charset=utf-8",
        include_bytes!("../ui/index.html"),
    ),
    (
        "app.css",
        "text/css; charset=utf-8",
        include_bytes!("../ui/app.css"),
    ),
    (
        "shelf.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/shelf.js"),
    ),
    (
        "shelf-model.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/shelf-model.js"),
    ),
    (
        "marks.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/marks.js"),
    ),
    (
        "search-memory.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/search-memory.js"),
    ),
    (
        "search-memory-model.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/search-memory-model.js"),
    ),
    (
        "undo-bar.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/undo-bar.js"),
    ),
    (
        "cleanup.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/cleanup.js"),
    ),
    (
        "format-pill.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/format-pill.js"),
    ),
    (
        "cleanup-model.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/cleanup-model.js"),
    ),
    (
        "revert.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/revert.js"),
    ),
    (
        "history.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/history.js"),
    ),
    (
        "chapters.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/chapters.js"),
    ),
    (
        "chapters-model.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/chapters-model.js"),
    ),
    (
        "search.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/search.js"),
    ),
    (
        "replace.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/replace.js"),
    ),
    (
        "replace-model.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/replace-model.js"),
    ),
    (
        "fix-offer.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/fix-offer.js"),
    ),
    (
        "search-model.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/search-model.js"),
    ),
    (
        "main.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/main.js"),
    ),
    (
        "ipc.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/ipc.js"),
    ),
    (
        "draft.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/draft.js"),
    ),
    (
        "spelling.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/spelling.js"),
    ),
    (
        "chapter-view.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/chapter-view.js"),
    ),
    (
        "textmap.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/textmap.js"),
    ),
    (
        "edit-machine.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/edit-machine.js"),
    ),
    (
        "edit-session.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/edit-session.js"),
    ),
    (
        "shortcuts.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/shortcuts.js"),
    ),
    (
        "word-count.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/word-count.js"),
    ),
    (
        "failure-card.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/failure-card.js"),
    ),
    (
        "toc.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/toc.js"),
    ),
    (
        "toc-model.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/toc-model.js"),
    ),
    (
        "palette.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/palette.js"),
    ),
    (
        "palette-model.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/palette-model.js"),
    ),
    (
        "chapter-number.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/chapter-number.js"),
    ),
    (
        "back.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/back.js"),
    ),
    (
        "focus.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/focus.js"),
    ),
    (
        "prefs.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/prefs.js"),
    ),
    (
        "preferences.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/preferences.js"),
    ),
    (
        "preferences-model.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/preferences-model.js"),
    ),
    (
        "place.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/place.js"),
    ),
    (
        "pages.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/pages.js"),
    ),
    (
        "keys.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../ui/keys.js"),
    ),
    font!("literata-latin-400-normal"),
    font!("literata-latin-400-italic"),
    font!("literata-latin-600-normal"),
    font!("literata-latin-ext-400-normal"),
    font!("literata-latin-ext-400-italic"),
    font!("literata-latin-ext-600-normal"),
    font!("ibm-plex-sans-latin-400-normal"),
    font!("ibm-plex-sans-latin-500-normal"),
    font!("ibm-plex-sans-latin-600-normal"),
    font!("ibm-plex-mono-latin-400-normal"),
    font!("ibm-plex-mono-latin-500-normal"),
];

fn not_found() -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(404)
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Cow::Borrowed(b"not found".as_slice()))
        .expect("static response headers are valid")
}

fn csp_enabled() -> bool {
    #[cfg(feature = "e2e")]
    {
        crate::e2e::switches().csp
    }
    #[cfg(not(feature = "e2e"))]
    {
        true
    }
}

const DARK_APP: &str = "class=\"app dark\"";
const LIGHT_APP: &str = "class=\"app light\"";

fn themed(path: &str, bytes: &'static [u8], theme: Theme) -> Cow<'static, [u8]> {
    if path != "index.html" || theme == Theme::Dark {
        return Cow::Borrowed(bytes);
    }
    let page = String::from_utf8_lossy(bytes).replacen(DARK_APP, LIGHT_APP, 1);
    Cow::Owned(page.into_bytes())
}

pub fn ui_response(request: &Request<Vec<u8>>, theme: Theme) -> Response<Cow<'static, [u8]>> {
    let path = request.uri().path().trim_start_matches('/');
    let Some((_, media_type, bytes)) = UI_FILES.iter().find(|(name, ..)| *name == path) else {
        return not_found();
    };
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, *media_type)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    if csp_enabled() {
        response = response.header(header::CONTENT_SECURITY_POLICY, UI_CSP);
    }
    response
        .body(themed(path, bytes, theme))
        .expect("static response headers are valid")
}

const COVER_HOST: &str = "cover";

pub fn cover_response(library: &Library, uri: &Uri) -> Response<Cow<'static, [u8]>> {
    let key = uri.path().strip_prefix('/').and_then(parse_key);
    let Some(bytes) = key.and_then(|key| library.cover_bytes(key)) else {
        return not_found();
    };
    Response::builder()
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, RESOURCE_CSP)
        .body(Cow::Owned(bytes))
        .unwrap_or_else(|_| not_found())
}

pub fn book_response(
    session: &Session,
    request: &Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    if request.uri().host() == Some(COVER_HOST) {
        return match session.library() {
            Ok(library) => cover_response(library, request.uri()),
            Err(_) => not_found(),
        };
    }
    let Ok(reader) = session.reader() else {
        return not_found();
    };
    let Some(entry) = reader.entry_of_url(&request.uri().to_string()) else {
        return not_found();
    };
    let Ok(resource) = reader.resource(&entry) else {
        return not_found();
    };
    Response::builder()
        .header(header::CONTENT_TYPE, resource.media_type)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, RESOURCE_CSP)
        .body(Cow::Owned(resource.bytes))
        .unwrap_or_else(|_| not_found())
}

#[cfg(test)]
mod tests {
    use super::{DARK_APP, LIGHT_APP, RESOURCE_CSP, UI_CSP, UI_FILES, cover_response, ui_response};
    use jadeslip_core::{BookKey, Library, LibraryConfig, Theme, key_hex};
    use tauri::http::{Request, Uri};

    fn get(url: &str) -> tauri::http::Response<std::borrow::Cow<'static, [u8]>> {
        ui_response(
            &Request::builder().uri(url).body(Vec::new()).unwrap(),
            Theme::Dark,
        )
    }

    fn page_in(theme: Theme) -> String {
        let request = Request::builder()
            .uri("jadeslip://localhost/index.html")
            .body(Vec::new())
            .unwrap();
        String::from_utf8(ui_response(&request, theme).body().to_vec()).unwrap()
    }

    #[test]
    fn the_saved_theme_is_in_the_page_before_any_script_runs() {
        assert!(page_in(Theme::Dark).contains(DARK_APP));
        assert!(!page_in(Theme::Dark).contains(LIGHT_APP));
        assert!(page_in(Theme::Light).contains(LIGHT_APP));
        assert!(!page_in(Theme::Light).contains(DARK_APP));
    }

    #[test]
    fn the_page_policy_blocks_frames_and_every_remote_origin() {
        assert!(UI_CSP.contains("frame-src 'none'"));
        assert!(UI_CSP.contains("default-src 'none'"));
        assert!(UI_CSP.contains("script-src 'self';"));
        for word in ["http:", "https:", "data:", "file:", "*"] {
            let without_ipc = UI_CSP.replace("http://ipc.localhost", "");
            assert!(!without_ipc.contains(word), "{word}");
        }
    }

    #[test]
    fn every_script_in_the_ui_folder_is_served() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        for entry in std::fs::read_dir(folder).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if name.ends_with(".js") {
                assert!(
                    UI_FILES.iter().any(|(listed, ..)| *listed == name),
                    "{name} is not in UI_FILES"
                );
            }
        }
    }

    #[test]
    fn only_listed_ui_files_are_served_and_each_carries_the_policy() {
        for (name, ..) in UI_FILES {
            let response = get(&format!("jadeslip://localhost/{name}"));
            assert_eq!(response.status(), 200);
            assert_eq!(
                response.headers()["content-security-policy"]
                    .to_str()
                    .unwrap(),
                UI_CSP
            );
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        }
        for path in ["../Cargo.toml", "fonts/LICENSE-Literata.txt", "", "secret"] {
            assert_eq!(
                get(&format!("jadeslip://localhost/{path}")).status(),
                404
            );
        }
    }

    #[test]
    fn no_ui_file_loads_anything_remote() {
        for (name, _, bytes) in UI_FILES {
            if name.ends_with(".woff2") {
                continue;
            }
            let text = std::str::from_utf8(bytes)
                .unwrap()
                .replace("http://www.w3.org/", "");
            assert!(!text.contains("http://"), "{name}");
            assert!(!text.contains("https://"), "{name}");
        }
    }

    #[test]
    fn a_cover_is_served_only_for_a_well_formed_key_with_a_thumbnail() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/scheme-tests")
            .join(format!("covers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let key = key_hex(BookKey(0x1234_5678_9abc_def0_0fed_cba9_8765_4321));
        let folder = data.join("books").join(&key);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("cover.jpg"), b"\xff\xd8 thumbnail").unwrap();
        std::fs::write(data.join("library.json"), b"{}").unwrap();
        let library = Library::open(LibraryConfig {
            data_dir: data.clone(),
        })
        .unwrap();
        let get = |url: &str| cover_response(&library, &url.parse::<Uri>().unwrap());

        let found = get(&format!("jadeslip-book://cover/{key}?v=1-2"));
        assert_eq!(found.status(), 200);
        assert_eq!(found.headers()["content-type"], "image/jpeg");
        assert_eq!(found.headers()["x-content-type-options"], "nosniff");
        assert_eq!(found.headers()["content-security-policy"], RESOURCE_CSP);
        assert_eq!(found.body().as_ref(), b"\xff\xd8 thumbnail");

        let unknown = key_hex(BookKey(7));
        for path in [
            unknown.as_str(),
            "",
            "zz",
            &key[1..],
            &format!("{key}0"),
            &format!("{key}/cover.jpg"),
            &format!("{}/../{}", &key[..16], &key[16..]),
            "../library.json",
            "%2e%2e%2flibrary.json",
            &format!("..%2f{key}"),
            &format!("{}%2f{}", &key[..16], &key[17..]),
            "books/../library.json",
        ] {
            assert_eq!(
                get(&format!("jadeslip-book://cover/{path}")).status(),
                404,
                "{path}"
            );
        }
        let _ = std::fs::remove_dir_all(&data);
    }
}
