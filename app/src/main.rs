mod cli;
mod commands;
#[cfg(feature = "e2e")]
mod e2e;
mod external;
mod headings;
mod history;
mod library;
mod schemes;
mod search;
mod session;
mod spell;
mod storage;

use schemes::{BOOK_SCHEME, UI_SCHEME, UI_URL};
use session::Session;
use jadeslip_core::Theme;
use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{Url, WebviewUrl, WebviewWindowBuilder};

const DARK_BACKGROUND: Color = Color(20, 22, 27, 255);
const LIGHT_BACKGROUND: Color = Color(244, 245, 243, 255);

#[macro_export]
macro_rules! mark {
    ($name:literal) => {
        #[cfg(feature = "e2e")]
        $crate::e2e::mark($name);
    };
}

fn navigation_allowed(url: &Url) -> bool {
    let allowed = url.as_str() == UI_URL;
    #[cfg(feature = "e2e")]
    let allowed = e2e::navigation_decision(url, allowed);
    allowed
}

fn main() {
    if std::env::var_os("APPIMAGE").is_some()
        && let Some(launched_from) = std::env::var_os("OWD")
    {
        let _ = std::env::set_current_dir(launched_from);
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if cli::wants_cli(&args) {
        std::process::exit(cli::run(&args));
    }
    unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    mark!("main");
    #[cfg(feature = "e2e")]
    e2e::before_gtk();

    let session = Session::start(args.into_iter().next());
    storage::sweep_later(session.clone());
    let book_session = session.clone();
    let theme_session = session.clone();
    let window_background = match session.saved.prefs().theme {
        Theme::Dark => DARK_BACKGROUND,
        Theme::Light => LIGHT_BACKGROUND,
    };
    mark!("before_builder");
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(session)
        .manage(spell::Spelling::default())
        .register_uri_scheme_protocol(UI_SCHEME, move |_context, request| {
            schemes::ui_response(&request, theme_session.saved.prefs().theme)
        })
        .register_asynchronous_uri_scheme_protocol(
            BOOK_SCHEME,
            move |_context, request, responder| {
                let session = book_session.clone();
                std::thread::spawn(move || {
                    responder.respond(schemes::book_response(&session, &request))
                });
            },
        )
        .on_window_event(commands::on_window_event)
        .setup(move |app| {
            mark!("setup");
            let url = UI_URL.parse().expect("the UI URL is valid");
            WebviewWindowBuilder::new(app, "main", WebviewUrl::CustomProtocol(url))
                .title("Jadeslip")
                .inner_size(1200.0, 800.0)
                .background_color(window_background)
                .on_navigation(navigation_allowed)
                .on_new_window(|_url, _features| NewWindowResponse::Deny)
                .on_download(|_webview, _event| false)
                .build()?;
            mark!("window_built");
            #[cfg(feature = "e2e")]
            e2e::start_control(app.handle().clone());
            Ok(())
        });

    #[cfg(not(feature = "e2e"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        commands::first_text,
        commands::render_chapter,
        commands::follow_link,
        commands::save_chapter,
        commands::draft_write,
        commands::draft_offer,
        commands::draft_recover,
        commands::draft_discard,
        commands::draft_drop,
        commands::search_start,
        commands::search_next,
        commands::search_cancel,
        commands::locate_hits,
        commands::replace_start,
        commands::replace_next,
        commands::replace_cancel,
        commands::apply_start,
        commands::apply_next,
        commands::apply_cancel,
        commands::cleanup_rules,
        commands::toc,
        commands::headings_start,
        commands::headings_next,
        commands::headings_cancel,
        commands::prefs_get,
        commands::prefs_set,
        commands::searches_get,
        commands::searches_set,
        commands::save_place,
        commands::manage,
        commands::backup_status,
        commands::new_book,
        commands::reload_book,
        commands::changed_on_disk,
        commands::set_close_guard,
        commands::close_asked,
        commands::close_window,
        commands::set_fullscreen,
        library::shelf,
        library::fill_covers,
        library::open_shelf_book,
        library::relink_book,
        library::pick_and_open,
        library::open_dropped,
        library::add_dropped,
        library::pick_and_add,
        library::add_open_book,
        library::library_moving,
        library::close_book,
        library::set_own,
        library::remove_from_shelf,
        library::export_copy_dialog,
        library::export_shelf_copy,
        library::reveal_shelf_book,
        library::removal_info,
        library::remove_book,
        library::revert_preview,
        library::revert_book,
        history::history_list,
        history::history_changes,
        history::history_snapshot,
        history::history_diff,
        history::history_restore,
        library::marks,
        library::add_mark,
        library::delete_mark,
        library::resolve_mark,
        spell::spell_chapter,
        spell::spell_block,
        spell::spell_suggest,
        spell::spell_add,
        spell::spell_languages,
        spell::spell_language_set,
        storage::storage_info,
        storage::storage_reveal,
        storage::storage_preview,
        storage::storage_clean,
        storage::delete_original_backup,
        storage::dictionaries_info,
    ]);
    #[cfg(feature = "e2e")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        commands::first_text,
        commands::render_chapter,
        commands::follow_link,
        commands::save_chapter,
        commands::draft_write,
        commands::draft_offer,
        commands::draft_recover,
        commands::draft_discard,
        commands::draft_drop,
        commands::search_start,
        commands::search_next,
        commands::search_cancel,
        commands::locate_hits,
        commands::replace_start,
        commands::replace_next,
        commands::replace_cancel,
        commands::apply_start,
        commands::apply_next,
        commands::apply_cancel,
        commands::cleanup_rules,
        commands::toc,
        commands::headings_start,
        commands::headings_next,
        commands::headings_cancel,
        commands::prefs_get,
        commands::prefs_set,
        commands::searches_get,
        commands::searches_set,
        commands::save_place,
        commands::manage,
        commands::backup_status,
        commands::new_book,
        commands::reload_book,
        commands::changed_on_disk,
        commands::set_close_guard,
        commands::close_asked,
        commands::close_window,
        commands::set_fullscreen,
        library::shelf,
        library::fill_covers,
        library::open_shelf_book,
        library::relink_book,
        library::pick_and_open,
        library::open_dropped,
        library::add_dropped,
        library::pick_and_add,
        library::add_open_book,
        library::library_moving,
        library::close_book,
        library::set_own,
        library::remove_from_shelf,
        library::export_copy_dialog,
        library::export_shelf_copy,
        library::reveal_shelf_book,
        library::removal_info,
        library::remove_book,
        library::revert_preview,
        library::revert_book,
        history::history_list,
        history::history_changes,
        history::history_snapshot,
        history::history_diff,
        history::history_restore,
        library::marks,
        library::add_mark,
        library::delete_mark,
        library::resolve_mark,
        spell::spell_chapter,
        spell::spell_block,
        spell::spell_suggest,
        spell::spell_add,
        spell::spell_languages,
        spell::spell_language_set,
        storage::storage_info,
        storage::storage_reveal,
        storage::storage_preview,
        storage::storage_clean,
        storage::delete_original_backup,
        storage::dictionaries_info,
        e2e::ctl_reply,
        e2e::pwned,
    ]);

    if let Err(error) = builder.run(tauri::generate_context!()) {
        eprintln!("jadeslip: the window could not start: {error}");
        std::process::exit(1);
    }
}
