use serde_json::{Value, json};
use jadeslip_e2e::{
    App, KEY_DOWN, KEY_ESCAPE, KEY_LEFT, KEY_RIGHT, KEY_UP, MASK_SHIFT, generated_book, largest_real_book, repo_root, scratch_dir,
};
use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const KEY_E: u32 = 0x65;
const KEY_ENTER: u32 = 0xff0d;
const KEY_MENU: u32 = 0xff67;
const KEY_F10: u32 = 0xffc7;
const MASK_ALT: u32 = 8;

const INSTALL: &str = r"(()=>{
  const element = (id) => document.getElementById(id);
  const shown = (id) => !element(id).hidden;
  window.__box = (id) => {
    const rect = element(id).getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2, height: rect.height };
  };
  window.__books = () => [...document.querySelectorAll('#shelf-grid:not([hidden]) .card, #shelf-list:not([hidden]) .trow')].map((node) => {
    const button = node.matches('button') ? node : node.querySelector('.cardgo');
    const rect = button.getBoundingClientRect();
    const meta = node.querySelector('.cardgo > .dotted') ?? node.querySelector('.td:nth-child(2)');
    return {
      key: node.dataset.key,
      title: (node.querySelector('.cardtitle') ?? node.querySelector('.rt')).textContent,
      meta: [...meta.children].map((part) => part.textContent).filter(Boolean).join(' · '),
      x: rect.left + rect.width / 2,
      y: rect.top + Math.min(rect.height / 2, 60),
      sel: node.classList.contains('sel'),
      focused: document.activeElement === button,
    };
  });
  window.__marks = () => [...document.querySelectorAll('#notes-list .note')].map((node) => ({
    id: Number(node.dataset.mark),
    state: node.dataset.state,
    where: node.querySelector('.notewhere').textContent,
    text: node.querySelector('.notetext').textContent,
  }));
  window.__view = () => ({
    onShelf: shown('library'),
    inReader: shown('reader'),
    mode: element('mode-edit').classList.contains('on') ? 'edit' : 'read',
    where: element('where-title').title,
    title: element('where-title').textContent,
    crumb: element('crumb').textContent,
    dialog: shown('dialog'),
    toast: element('toast').hidden ? '' : element('toast').textContent,
    notice: element('notice').hidden ? '' : element('notice').textContent,
    undo: shown('undo-bar'),
    deleted: element('deleted-open').hidden ? '' : element('deleted-open').textContent,
    text: element('book-host').shadowRoot?.querySelector('.ss-body')?.textContent ?? '',
    links: element('book-host').shadowRoot?.querySelectorAll('.ss-body a').length ?? 0,
    kicker: element('hero-kicker')?.textContent ?? '',
    heroTitle: element('hero-title')?.textContent ?? '',
    line: element('hero-line')?.textContent ?? '',
    quote: element('hero-quote')?.textContent ?? '',
    stat: document.querySelector('#hero .stat')?.textContent ?? '',
    go: element('hero-go')?.textContent ?? '',
    recent: [...document.querySelectorAll('#recent-cards .rtitle')].map((node) => node.textContent),
    empty: shown('shelf-empty'),
    none: shown('shelf-none'),
    list: shown('shelf-list'),
    tab: document.querySelector('#shelf-tabs .tab.on')?.id ?? '',
    sort: element('shelf-sort-label').textContent,
    menu: shown('book-menu') ? [...element('book-menu').querySelectorAll('[role=menuitem]')].map((node) => node.textContent) : [],
    menuFocus: document.activeElement?.closest?.('#book-menu') ? document.activeElement.id : '',
    removeOpen: shown('remove-dialog'),
    removeText: element('remove-title').textContent + ' ' + element('remove-text').textContent,
    removeData: element('remove-data-label').textContent,
    removeFile: shown('remove-file-row') ? element('remove-file-label').textContent : '',
    removeError: shown('remove-error') ? element('remove-error').textContent : '',
    failure: shown('shelf-failure') ? element('shelf-failure-text').textContent : '',
    card: shown('open-failure') ? element('open-failure-text').textContent : '',
    cardTitle: element('open-failure-title').textContent,
    cardButtons: ['failure-remove', 'failure-back', 'failure-find'].filter(shown),
    inert: ['#reader .top .tools', '#reader .top .seg', '#contents-rail', '#reader .footrow'].every((selector) => document.querySelector(selector).inert),
    revertOpen: shown('revert-dialog'),
    revertText: element('revert-text').textContent,
    revertChapter: shown('revert-chapter'),
    revertChapterOff: element('revert-chapter').disabled,
    revertWhole: shown('revert-whole'),
    revertCancel: element('revert-cancel').textContent,
    restores: [...element('revert-restores').querySelectorAll('.deleted-row span')].map((row) => row.textContent),
    add: element('note-add').textContent,
    current: CSS.highlights.has('ss-search-current'),
  });
})()";

fn install(app: &mut App) {
    app.js(INSTALL);
}

fn view(app: &mut App) -> Value {
    app.js("window.__view()")
}

fn books(app: &mut App) -> Vec<Value> {
    app.js("window.__books()").as_array().unwrap().clone()
}

fn marks(app: &mut App) -> Vec<Value> {
    app.js("window.__marks()").as_array().unwrap().clone()
}

fn click_id(app: &mut App, id: &str) {
    let spot = app.js(&format!("window.__box('{id}')"));
    assert!(
        spot["height"].as_f64().unwrap_or(0.0) > 0.0,
        "{id} is not on screen"
    );
    app.click(spot["x"].as_f64().unwrap(), spot["y"].as_f64().unwrap());
}

fn wait_until(app: &mut App, what: &str, mut ready: impl FnMut(&mut App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready(app) {
        if Instant::now() >= deadline {
            let shown = view(app);
            panic!(
                "{what} never happened: reader {} shelf {} mode {}",
                shown["inReader"], shown["onShelf"], shown["mode"]
            );
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

fn wait_reader(app: &mut App) {
    wait_until(app, "the reader", |app| {
        let shown = view(app);
        shown["inReader"] == json!(true) && !shown["text"].as_str().unwrap_or("").is_empty()
    });
}

fn wait_shelf(app: &mut App) {
    wait_until(app, "the shelf", |app| view(app)["onShelf"] == json!(true));
}

fn wait_mode(app: &mut App, mode: &str) {
    wait_until(app, &format!("mode {mode}"), |app| {
        view(app)["mode"] == json!(mode)
    });
}

fn wait_toast(app: &mut App, part: &str) {
    wait_until(app, &format!("toast {part}"), |app| {
        view(app)["toast"].as_str().unwrap_or("").contains(part)
    });
}

fn dir_for(label: &str) -> PathBuf {
    let dir = scratch_dir().join(format!("lib-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn writable_copy(source: &Path, path: &Path) -> PathBuf {
    fs::copy(source, path).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(path, perms).unwrap();
    path.to_owned()
}

fn copy_of(name: &str, dir: &Path, as_name: &str) -> PathBuf {
    writable_copy(&generated_book(name), &dir.join(as_name))
}

fn differing_entries(one: &Path, two: &Path) -> Vec<String> {
    let script = "import json,sys,zipfile\na=zipfile.ZipFile(sys.argv[1]);b=zipfile.ZipFile(sys.argv[2])\nnames=sorted(set(a.namelist())|set(b.namelist()))\nout=[n for n in names if n not in a.namelist() or n not in b.namelist() or a.read(n)!=b.read(n)]\nprint(json.dumps(out))";
    let done = Command::new("python3")
        .args(["-I", "-c", script])
        .arg(one)
        .arg(two)
        .output()
        .unwrap();
    serde_json::from_slice(&done.stdout).unwrap()
}

fn structure(path: &Path) -> Value {
    let done = Command::new("python3")
        .arg("-I")
        .arg(repo_root().join("tests/epub_structure.py"))
        .arg(path)
        .output()
        .unwrap();
    serde_json::from_slice(&done.stdout).unwrap()
}

fn assert_valid(path: &Path) {
    let found = structure(path);
    assert_eq!(found["problems"], json!([]), "{found}");
    assert_eq!(found["dangling_links"], json!(0));
    let meta = Command::new("ebook-meta").arg(path).output().unwrap();
    assert!(meta.status.success(), "Calibre could not open the book");
}

fn palette_run(app: &mut App, label: &str) {
    click_id(app, "open-palette");
    wait_until(app, "the palette", |app| {
        app.js("!document.getElementById('palette').hidden") == json!(true)
    });
    app.js(&format!(
        "(()=>{{const input=document.getElementById('palette-input');input.value={};input.dispatchEvent(new Event('input'))}})()",
        json!(label)
    ));
    app.key(KEY_ENTER);
}

fn type_at_start(app: &mut App, text: &str) {
    app.key(KEY_E);
    wait_mode(app, "edit");
    app.type_text(text);
}

fn save_and_read(app: &mut App) {
    app.key(KEY_ESCAPE);
    wait_mode(app, "read");
    wait_toast(app, "Saved");
    wait_until(app, "the save to finish", |app| {
        app.js("document.getElementById('unsaved').hidden") == json!(true)
    });
}

fn go_to(app: &mut App, ix: u32) {
    app.js(&format!(
        "(async()=>{{await (await import('/main.js')).goTo({ix})}})()"
    ));
}

fn select_in_block(app: &mut App, block: usize, from: usize, to: usize) -> String {
    let script = format!(
        "(()=>{{const s=document.getElementById('book-host').shadowRoot;const b=s.querySelectorAll('[data-ss-b]')[{block}];const w=document.createTreeWalker(b,NodeFilter.SHOW_TEXT);const t=w.nextNode();document.getSelection().setBaseAndExtent(t,{from},t,{to});return t.data.slice({from},{to})}})()"
    );
    let text = app.js(&script).as_str().unwrap().to_owned();
    wait_until(app, "the passage button", |app| {
        view(app)["add"] == json!("Bookmark this passage")
    });
    text
}

struct Watch {
    fd: i32,
}

impl Watch {
    fn on(paths: &[&Path]) -> Watch {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK) };
        assert!(fd >= 0);
        for path in paths {
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            let mask = libc::IN_OPEN | libc::IN_ACCESS;
            assert!(unsafe { libc::inotify_add_watch(fd, name.as_ptr(), mask) } >= 0);
        }
        Watch { fd }
    }

    fn events(&self) -> usize {
        let mut buffer = [0u8; 65536];
        let mut total = 0;
        loop {
            let got = unsafe { libc::read(self.fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if got <= 0 {
                return total;
            }
            total += got as usize / std::mem::size_of::<libc::inotify_event>();
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

fn open_fds_naming_epub(pid: u32) -> usize {
    fs::read_dir(format!("/proc/{pid}/fd"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| fs::read_link(entry.path()).ok())
                .filter(|target| target.to_string_lossy().ends_with(".epub"))
                .count()
        })
        .unwrap_or(0)
}

fn shelf_records(data: &Path) -> Vec<Value> {
    fs::read(data.join("library.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|root| root["books"].as_object().map(|books| books.values().cloned().collect()))
        .unwrap_or_default()
}

fn copy_titled(data: &Path, title: &str) -> PathBuf {
    shelf_records(data)
        .into_iter()
        .find(|record| record["title"] == json!(title))
        .and_then(|record| record["path"].as_str().map(PathBuf::from))
        .unwrap_or_else(|| panic!("no shelf record titled {title}"))
}

fn shelve(data: &Path, books: &[&Path]) {
    let mut app = App::launch_with_data_opt(None, data);
    app.wait_first_text();
    install(&mut app);
    app.op(&json!({"op": "drop_paths", "paths": books}));
    wait_until(&mut app, "every book shelved with its cover read", |_| {
        let records = shelf_records(data);
        records.len() == books.len() && records.iter().all(|record| !record["cover"].is_null())
    });
    app.close();
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn launch_opens_no_epub_and_opening_one_is_seen_by_the_same_watch() {
    let dir = dir_for("noepub");
    let data = dir.join("data");
    let small = copy_of("toc-nav-only", &dir, "small.epub");
    let covered = copy_of("cover-epub3", &dir, "covered.epub");
    let large = writable_copy(&largest_real_book(), &dir.join("large.epub"));
    shelve(&data, &[&small, &covered, &large]);

    let copies: Vec<PathBuf> = shelf_records(&data)
        .iter()
        .map(|record| PathBuf::from(record["path"].as_str().unwrap()))
        .collect();
    let mut watched: Vec<&Path> = vec![&small, &covered, &large];
    watched.extend(copies.iter().map(PathBuf::as_path));
    let watch = Watch::on(&watched);
    assert_eq!(watch.events(), 0, "nothing is reading the books yet");
    let mut app = App::launch_with_data_opt(None, &data);
    let shown_at = app.wait_first_text();
    app.assert_on_headless_output();
    install(&mut app);
    assert_eq!(view(&mut app)["onShelf"], json!(true));
    assert_eq!(books(&mut app).len(), 3);
    let ms = shown_at - app.spawned_at_ms;
    wait_until(&mut app, "the cover of the covered book", |app| {
        card_titled(&cover_states(app), "Cover Epub3")["loaded"] == json!(true)
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(watch.events(), 0, "the launch opened or read an EPUB");
    assert_eq!(open_fds_naming_epub(app.pid), 0, "an EPUB is held open");
    let phases = app.phases();
    assert!(
        !phases.iter().any(|(name, _)| name == "book_open"),
        "a book was opened on the launch path"
    );
    println!("shelf shown {ms:.0} ms after spawn with 3 books");

    let opened_at = Instant::now();
    click_id(&mut app, "hero-go");
    wait_reader(&mut app);
    println!(
        "shelf to first text {:.0} ms",
        opened_at.elapsed().as_secs_f64() * 1e3
    );
    assert!(watch.events() > 0, "the control: opening a book is seen");
    assert!(open_fds_naming_epub(app.pid) > 0, "the open book is held");
    assert!(app.phases().iter().any(|(name, _)| name == "book_open"));
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_book_missing_from_disk_still_shows_and_can_be_removed() {
    let dir = dir_for("missing");
    let data = dir.join("data");
    let path = copy_of("toc-nav-only", &dir, "gone.epub");
    shelve(&data, &[&path]);
    let copy = copy_titled(&data, "Toc Nav Only");
    let moved = dir.join("moved.epub");
    fs::rename(&copy, &moved).unwrap();
    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    install(&mut app);
    assert_eq!(books(&mut app).len(), 1);
    click_id(&mut app, "hero-go");
    wait_until(&mut app, "the card", |app| {
        !view(app)["card"].as_str().unwrap_or("").is_empty()
    });
    let card = view(&mut app);
    assert_eq!(
        card["card"],
        json!(format!(
            "Jadeslip cannot find the book at {}. It may have been moved, renamed or deleted.",
            copy.display()
        ))
    );
    assert_eq!(card["cardTitle"], "Book not found");
    assert_eq!(card["cardButtons"], json!(["failure-remove", "failure-back", "failure-find"]));
    assert_eq!(card["inert"], true, "the reader controls are off while the card shows");
    assert_eq!(card["onShelf"], false);
    click_id(&mut app, "failure-back");
    wait_shelf(&mut app);
    assert_eq!(books(&mut app).len(), 1, "Back to shelf keeps the book");
    click_id(&mut app, "hero-go");
    wait_until(&mut app, "the card again", |app| {
        !view(app)["card"].as_str().unwrap_or("").is_empty()
    });
    click_id(&mut app, "failure-remove");
    wait_shelf(&mut app);
    wait_until(&mut app, "an empty shelf", |app| books(app).is_empty());
    assert_eq!(view(&mut app)["empty"], json!(true));
    assert!(moved.exists(), "removing from the shelf leaves the file");
    assert!(path.exists(), "the added file is never touched");
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_missing_copy_shows_the_card_and_find_the_file_copies_the_same_book_back_in() {
    let dir = dir_for("relink");
    let data = dir.join("data");
    let path = copy_of("toc-nav-only", &dir, "found.epub");
    let other = copy_of("cover-epub3", &dir, "other.epub");
    shelve(&data, &[&path]);
    let copy = copy_titled(&data, "Toc Nav Only");
    fs::remove_file(&copy).unwrap();

    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    install(&mut app);
    click_id(&mut app, "hero-go");
    wait_until(&mut app, "the card", |app| {
        !view(app)["card"].as_str().unwrap_or("").is_empty()
    });
    let card = view(&mut app);
    println!("missing copy card: {card}");
    assert_eq!(
        card["card"],
        json!(format!(
            "Jadeslip cannot find the book at {}. It may have been moved, renamed or deleted.",
            copy.display()
        ))
    );
    assert!(!card["card"].as_str().unwrap().contains("os error"));
    assert_eq!(card["cardButtons"], json!(["failure-remove", "failure-back", "failure-find"]));
    assert_eq!(card["inert"], true);
    app.key(KEY_E);
    assert_eq!(view(&mut app)["mode"], "read", "keys are off while the card shows");

    app.op(&json!({"op": "arm_save_answer", "path": other}));
    click_id(&mut app, "failure-find");
    wait_until(&mut app, "the different-book sentence", |app| {
        view(app)["card"].as_str().unwrap_or("").contains("is a different book")
    });
    assert_eq!(
        view(&mut app)["card"],
        json!(format!("{} is a different book. Pick the file for Toc Nav Only.", other.display()))
    );

    let before = fs::read(&path).unwrap();
    app.op(&json!({"op": "arm_save_answer", "path": path}));
    click_id(&mut app, "failure-find");
    wait_reader(&mut app);
    let opened = view(&mut app);
    assert_eq!(opened["card"], "");
    assert_eq!(opened["inert"], false, "the reader controls come back");
    app.close();
    assert_eq!(fs::read(&copy).unwrap(), before, "the found file was copied back in");
    assert_eq!(fs::read(&path).unwrap(), before);

    let mut again = App::launch_with_data_opt(None, &data);
    again.wait_first_text();
    install(&mut again);
    assert_eq!(books(&mut again).len(), 1, "one shelf record, healed, not a second book");
    click_id(&mut again, "hero-go");
    wait_reader(&mut again);
    again.close();
    let _ = fs::remove_dir_all(&dir);
}

fn search(app: &mut App, text: &str) {
    app.js(&format!(
        "(()=>{{const f=document.getElementById('shelf-filter');f.value={};f.dispatchEvent(new Event('input'));return true}})()",
        json!(text)
    ));
}

fn selected_title(app: &mut App) -> Value {
    books(app)
        .into_iter()
        .find(|row| row["sel"] == json!(true))
        .map_or(Value::Null, |row| row["title"].clone())
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn the_shelf_leads_with_the_last_book_and_names_the_place_by_its_title() {
    let dir = dir_for("shelf");
    let data = dir.join("data");
    let small = copy_of("toc-nav-only", &dir, "small.epub");
    let large = writable_copy(&largest_real_book(), &dir.join("large.epub"));
    shelve(&data, &[&small, &large]);

    let mut app = App::launch_with_data(&large, &data);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    let large_title = view(&mut app)["crumb"].as_str().unwrap().to_owned();
    go_to(&mut app, 1840);
    wait_until(&mut app, "chapter 1818", |app| {
        view(app)["title"].as_str().unwrap_or("").contains("1818")
    });
    let chapter_title = view(&mut app)["title"].as_str().unwrap().to_owned();
    let place = view(&mut app)["where"].clone();
    click_id(&mut app, "go-library");
    wait_shelf(&mut app);
    let shown = view(&mut app);
    assert_eq!(shown["heroTitle"], json!(large_title), "the last book opened leads");
    assert_eq!(shown["recent"].as_array().unwrap().len(), 2);
    assert_eq!(shown["kicker"], json!("Continue reading"));
    assert_eq!(shown["go"], json!("Resume"));
    assert_eq!(
        shown["line"],
        json!(format!("{chapter_title}.")),
        "the place reads by the chapter's own title"
    );
    assert!(!shown["line"].as_str().unwrap().contains("6,1"), "{}", shown["line"]);
    assert!(!shown["quote"].as_str().unwrap().is_empty(), "a passage shows");
    assert!(shown["stat"].as_str().unwrap().ends_with('%'));
    let rows = books(&mut app);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["title"], json!(large_title), "Last opened sorts first");
    assert_eq!(rows[0]["sel"], json!(true));
    assert!(rows[0]["meta"].as_str().unwrap().contains("% · Ch "), "{}", rows[0]["meta"]);
    assert!(rows[1]["meta"].as_str().unwrap().starts_with("New · "), "{}", rows[1]["meta"]);
    assert_eq!(shown["tab"], json!("shelf-tab-all"));
    assert_eq!(shown["sort"], json!("Last opened"));

    click_id(&mut app, "shelf-tab-new");
    wait_until(&mut app, "the Not started tab", |app| books(app).len() == 1);
    assert_eq!(books(&mut app)[0]["title"], json!("Toc Nav Only"));
    click_id(&mut app, "shelf-tab-all");
    search(&mut app, "zzzzzz");
    wait_until(&mut app, "no match", |app| view(app)["none"] == json!(true));
    assert!(books(&mut app).is_empty());
    search(&mut app, "TOC NAV");
    wait_until(&mut app, "the search", |app| books(app).len() == 1);
    search(&mut app, "");
    wait_until(&mut app, "the search to clear", |app| books(app).len() == 2);

    app.key(KEY_RIGHT);
    wait_until(&mut app, "the arrow to move", |app| {
        selected_title(app) == json!("Toc Nav Only")
    });
    assert!(books(&mut app)[1]["focused"] == json!(true));
    app.key(KEY_LEFT);
    wait_until(&mut app, "the arrow back", |app| selected_title(app) == json!(large_title));
    app.key(KEY_RIGHT);
    wait_until(&mut app, "the small book", |app| selected_title(app) == json!("Toc Nav Only"));
    app.key(KEY_MENU);
    wait_until(&mut app, "the book menu", |app| !view(app)["menu"].as_array().unwrap().is_empty());
    let shown = view(&mut app);
    assert_eq!(
        shown["menu"],
        json!(["Start readingEnter", "Open in EditE", "This is my own book", "Show file in folder", "Export a copy", "Clean up book…", "Remove from shelf"])
    );
    assert_eq!(shown["menuFocus"], json!("menu-go"));
    app.key(KEY_DOWN);
    wait_until(&mut app, "the menu focus to move", |app| view(app)["menuFocus"] == json!("menu-alt"));
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "the menu to close", |app| view(app)["menu"].as_array().unwrap().is_empty());
    assert_eq!(view(&mut app)["onShelf"], json!(true), "Escape closes only the menu");
    app.key_with(KEY_F10, MASK_SHIFT);
    wait_until(&mut app, "Shift F10 opens the menu", |app| !view(app)["menu"].as_array().unwrap().is_empty());
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "the menu to close again", |app| view(app)["menu"].as_array().unwrap().is_empty());

    app.key(KEY_LEFT);
    wait_until(&mut app, "the large book again", |app| selected_title(app) == json!(large_title));
    app.key(KEY_ENTER);
    wait_reader(&mut app);
    wait_until(&mut app, "the saved chapter", |app| view(app)["where"] == place);
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn the_list_view_sorts_shows_the_selected_book_and_is_remembered() {
    let dir = dir_for("list");
    let data = dir.join("data");
    let small = copy_of("toc-nav-only", &dir, "small.epub");
    let ncx = copy_of("toc-ncx-only", &dir, "ncx.epub");
    let covered = copy_of("cover-epub3", &dir, "covered.epub");
    shelve(&data, &[&small, &ncx, &covered]);

    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    install(&mut app);
    click_id(&mut app, "view-list");
    wait_until(&mut app, "the list", |app| view(app)["list"] == json!(true));
    let titles = |app: &mut App| -> Vec<Value> { books(app).iter().map(|row| row["title"].clone()).collect() };
    assert_eq!(titles(&mut app), vec![json!("Cover Epub3"), json!("Toc Ncx Only"), json!("Toc Nav Only")]);
    assert_eq!(app.js("document.getElementById('detail-title').textContent"), json!("Cover Epub3"));
    app.js("document.querySelector('#shelf-head [data-sort=title]').click()");
    wait_until(&mut app, "the title sort", |app| view(app)["sort"] == json!("Title"));
    assert_eq!(titles(&mut app), vec![json!("Cover Epub3"), json!("Toc Nav Only"), json!("Toc Ncx Only")]);
    let rows = books(&mut app);
    app.click(rows[2]["x"].as_f64().unwrap(), rows[2]["y"].as_f64().unwrap());
    wait_until(&mut app, "the detail panel", |app| {
        app.js("document.getElementById('detail-title').textContent") == json!("Toc Ncx Only")
    });
    assert_eq!(view(&mut app)["onShelf"], json!(true), "one click in the list selects");
    app.key(KEY_UP);
    wait_until(&mut app, "the row above", |app| {
        app.js("document.getElementById('detail-title').textContent") == json!("Toc Nav Only")
    });
    click_id(&mut app, "recent-fold");
    wait_until(&mut app, "the strip to fold", |app| {
        app.js("document.getElementById('recent-cards').hidden") == json!(true)
    });
    app.close();

    let mut again = App::launch_with_data_opt(None, &data);
    again.wait_first_text();
    install(&mut again);
    let shown = view(&mut again);
    assert_eq!(shown["list"], json!(true), "the list view is remembered");
    assert_eq!(shown["sort"], json!("Title"), "the sort is remembered");
    assert_eq!(again.js("document.getElementById('recent-cards').hidden"), json!(true));
    click_id(&mut again, "view-grid");
    wait_until(&mut again, "the grid", |app| view(app)["list"] == json!(false));
    assert_eq!(
        again.js("document.getElementById('recent-cards').hidden"),
        json!(false),
        "the strip always shows in the grid"
    );
    again.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn an_own_book_continues_writing_and_reads_it() {
    let dir = dir_for("own");
    let data = dir.join("data");
    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    install(&mut app);
    assert_eq!(
        view(&mut app)["empty"],
        json!(true),
        "a first launch has an empty shelf"
    );
    click_id(&mut app, "shelf-new");
    wait_until(&mut app, "the new book dialog", |app| {
        app.js("!document.getElementById('newbook-dialog').hidden") == json!(true)
    });
    app.js("document.getElementById('newbook-name').value='My Draft'");
    click_id(&mut app, "newbook-create");
    wait_mode(&mut app, "edit");
    app.type_text("Words I wrote. ");
    click_id(&mut app, "add-chapter");
    wait_until(&mut app, "the second chapter", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 2 of 2")
    });
    wait_mode(&mut app, "edit");
    app.type_text("More words. ");
    app.key(KEY_ESCAPE);
    wait_mode(&mut app, "read");
    wait_toast(&mut app, "Saved");
    go_to(&mut app, 0);
    wait_until(&mut app, "reading the first chapter", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 1 of 2")
    });
    click_id(&mut app, "go-library");
    wait_shelf(&mut app);
    let shown = view(&mut app);
    assert_eq!(shown["kicker"], json!("Continue writing"));
    assert_eq!(shown["go"], json!("Continue writing"));
    assert!(shown["line"].as_str().unwrap().ends_with(". 2 chapters."), "{}", shown["line"]);
    assert!(!shown["stat"].as_str().unwrap().contains('%'), "{}", shown["stat"]);
    assert_eq!(books(&mut app)[0]["sel"], json!(true));
    click_id(&mut app, "hero-go");
    wait_reader(&mut app);
    wait_mode(&mut app, "edit");
    assert!(
        view(&mut app)["where"]
            .as_str()
            .unwrap()
            .starts_with("Position 2 of 2"),
        "Continue writing goes to where the writing stopped, not the last chapter read"
    );
    app.key(KEY_ESCAPE);
    wait_mode(&mut app, "read");
    go_to(&mut app, 0);
    wait_until(&mut app, "reading the first chapter", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 1 of 2")
    });
    click_id(&mut app, "go-library");
    wait_shelf(&mut app);
    app.key_with(KEY_F10, MASK_SHIFT);
    wait_until(&mut app, "the book menu", |app| !view(app)["menu"].as_array().unwrap().is_empty());
    assert_eq!(
        view(&mut app)["menu"].as_array().unwrap()[..3],
        [json!("Continue writingEnter"), json!("Read itE"), json!("Not my own book")]
    );
    click_id(&mut app, "menu-alt");
    wait_reader(&mut app);
    std::thread::sleep(Duration::from_millis(100));
    let shown = view(&mut app);
    assert_eq!(shown["mode"], json!("read"));
    assert!(
        shown["where"]
            .as_str()
            .unwrap()
            .starts_with("Position 1 of 2"),
        "Read it goes to the reading place"
    );
    app.close();
    let path = copy_titled(&data, "My Draft");
    assert!(path.starts_with(data.join("library")), "a new book lives in the library: {}", path.display());
    assert_valid(&path);
    let mut shelf = App::launch_with_data_opt(None, &data);
    shelf.wait_first_text();
    install(&mut shelf);
    let stat = view(&mut shelf)["stat"].as_str().unwrap().to_owned();
    let words: u64 = stat.strip_suffix(" words").unwrap_or_else(|| panic!("{stat}")).replace(',', "").parse().unwrap();
    assert!(words >= 5, "the typed words are counted: {stat}");
    assert_eq!(books(&mut shelf)[0]["meta"], json!(stat), "the card shows words, not a percent");
    shelf.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn open_reads_in_place_and_add_copies_into_the_library() {
    let dir = dir_for("open");
    let data = dir.join("data");
    let one = copy_of("toc-nav-only", &dir, "one.epub");
    let two = copy_of("toc-ncx-only", &dir, "two.epub");
    let three = copy_of("toc-none", &dir, "three.epub");
    let junk = dir.join("junk.epub");
    fs::write(&junk, b"this is no zip").unwrap();
    let stamp = |path: &Path| (fs::read(path).unwrap(), fs::metadata(path).unwrap().modified().unwrap());
    let originals: Vec<_> = [&one, &two, &three].iter().map(|path| stamp(path)).collect();

    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    install(&mut app);
    app.op(&json!({"op": "arm_save_answer"}));
    click_id(&mut app, "shelf-open");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(view(&mut app)["onShelf"], json!(true), "a cancelled picker stays on the shelf");
    app.op(&json!({"op": "arm_save_answer", "path": junk}));
    click_id(&mut app, "shelf-open");
    wait_until(&mut app, "a readable failure", |app| {
        !view(app)["failure"].as_str().unwrap_or("").is_empty()
    });
    assert_eq!(view(&mut app)["onShelf"], json!(true));
    app.op(&json!({"op": "arm_save_answer", "path": one}));
    click_id(&mut app, "shelf-open");
    wait_reader(&mut app);
    assert_eq!(app.js("!document.getElementById('add-to-shelf').hidden"), json!(true), "an opened book offers Add to shelf");
    click_id(&mut app, "go-library");
    wait_shelf(&mut app);
    assert!(books(&mut app).is_empty(), "Open EPUB does not shelve");
    assert!(view(&mut app)["recent"].as_array().unwrap().is_empty(), "nor does it join the strip");

    app.op(&json!({"op": "arm_save_answer", "path": one}));
    click_id(&mut app, "shelf-add");
    wait_until(&mut app, "one book added", |app| books(app).len() == 1);
    wait_toast(&mut app, "Added Toc Nav Only to your shelf.");
    assert_eq!(view(&mut app)["onShelf"], json!(true), "adding stays on the shelf");

    app.op(&json!({"op": "drop_paths", "paths": [two, three, junk]}));
    wait_until(&mut app, "the drop added", |app| books(app).len() == 3);
    wait_toast(&mut app, "Added 2 books to your shelf.");
    wait_until(&mut app, "the junk named", |app| view(app)["failure"].as_str().unwrap_or("").contains("junk.epub was not added"));
    assert_eq!(view(&mut app)["onShelf"], json!(true), "a drop on the shelf adds and does not open");

    app.op(&json!({"op": "drop_paths", "paths": [one]}));
    wait_until(&mut app, "the duplicate offer", |app| app.js("!document.getElementById('dup-dialog').hidden") == json!(true));
    assert_eq!(
        app.js("document.getElementById('dup-text').textContent"),
        json!("Toc Nav Only is already on your shelf, so no second copy was made.")
    );
    assert_eq!(books(&mut app).len(), 3);
    click_id(&mut app, "dup-open");
    wait_reader(&mut app);
    assert_eq!(view(&mut app)["crumb"], json!("Toc Nav Only"));
    assert_eq!(app.js("document.getElementById('add-to-shelf').hidden"), json!(true), "the shelf copy needs no Add");
    app.close();

    let library = data.join("library");
    let copies = fs::read_dir(&library).unwrap().count();
    assert_eq!(copies, 3, "one copy per book in {}", library.display());
    for ((path, title), (bytes, modified)) in [(&one, "Toc Nav Only"), (&two, "Toc Ncx Only"), (&three, "Toc None")].iter().zip(&originals) {
        let copy = copy_titled(&data, title);
        assert!(copy.starts_with(&library), "{}", copy.display());
        assert_eq!(&fs::read(&copy).unwrap(), bytes);
        assert_eq!(&stamp(path), &(bytes.clone(), *modified), "{} was not touched", path.display());
    }

    let lone = copy_of("cover-epub3", &dir, "lone.epub");
    let mut again = App::launch_with_data(&lone, &data);
    again.wait_first_text();
    install(&mut again);
    wait_reader(&mut again);
    assert_eq!(view(&mut again)["crumb"], json!("Cover Epub3"), "the book named on the command line opens");
    click_id(&mut again, "go-library");
    wait_shelf(&mut again);
    assert_eq!(books(&mut again).len(), 3, "a command-line book is not shelved");
    again.close();
    assert_eq!(fs::read_dir(&library).unwrap().count(), 3, "and not copied");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn the_native_open_picker_opens_on_the_test_output() {
    let dir = dir_for("picker");
    let data = dir.join("data");
    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    app.assert_on_headless_output();
    install(&mut app);
    let monitors = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .unwrap();
    let monitors: Value = serde_json::from_slice(&monitors.stdout).unwrap();
    let test_monitor = monitors
        .as_array()
        .unwrap()
        .iter()
        .find(|monitor| monitor["name"] == json!(jadeslip_e2e::headless_output()))
        .and_then(|monitor| monitor["id"].as_i64())
        .unwrap();
    app.op(&json!({"op": "arm_save_answer", "real": true}));
    click_id(&mut app, "shelf-open");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let output = Command::new("hyprctl")
            .args(["clients", "-j"])
            .output()
            .unwrap();
        let clients: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        let titled = clients.as_array().is_some_and(|list| {
            list.iter().any(|client| {
                client["pid"].as_u64() == Some(u64::from(app.pid))
                    && client["title"] == json!("Open an EPUB")
            })
        });
        if titled {
            app.assert_on_headless_output();
            let off_output = clients.as_array().unwrap().iter().any(|client| {
                client["pid"].as_u64() == Some(u64::from(app.pid))
                    && client["monitor"].as_i64() != Some(test_monitor)
            });
            assert!(
                !off_output,
                "a window of the app opened off the test output"
            );
            assert_eq!(
                app.js("!document.getElementById('picker-wait').hidden"),
                json!(true),
                "the page waits while the picker is open"
            );
            break;
        }
        assert!(Instant::now() < deadline, "the open picker never appeared");
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(app);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn export_writes_a_byte_equal_copy_and_keeps_the_book_open() {
    let dir = dir_for("export");
    let path = copy_of("toc-nav-only", &dir, "book.epub");
    let copy = dir.join("copy.epub");
    let mut app = App::launch_with_data_opt(Some(&path), &dir.join("data"));
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    app.op(&json!({"op": "arm_save_answer", "path": copy}));
    palette_run(&mut app, "Export a copy");
    wait_toast(&mut app, "Copy saved");
    assert!(
        !view(&mut app)["text"].as_str().unwrap().is_empty(),
        "the book stays open"
    );
    assert_eq!(fs::read(&copy).unwrap(), fs::read(&path).unwrap());
    app.op(&json!({"op": "arm_save_answer", "path": path}));
    palette_run(&mut app, "Export a copy");
    wait_toast(&mut app, "cannot replace the book");
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn revert_restores_one_chapter_then_the_whole_book_with_the_restore_list() {
    let dir = dir_for("revert");
    let path = copy_of("toc-nav-only", &dir, "book.epub");
    let original = dir.join("original.epub");
    fs::copy(&path, &original).unwrap();
    let mut app = App::launch_with_data_opt(Some(&path), &dir.join("data"));
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);

    go_to(&mut app, 2);
    wait_until(&mut app, "chapter 3", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 3 ")
    });
    type_at_start(&mut app, "QQQONE");
    save_and_read(&mut app);
    go_to(&mut app, 3);
    wait_until(&mut app, "chapter 4", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 4 ")
    });
    type_at_start(&mut app, "QQQTWO");
    save_and_read(&mut app);
    assert_eq!(differing_entries(&path, &original).len(), 2);

    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    click_id(&mut app, "tool-revert");
    wait_until(&mut app, "the revert dialog", |app| {
        view(app)["revertOpen"] == json!(true)
    });
    let shown = view(&mut app);
    assert_eq!(shown["revertChapter"], json!(true));
    assert_eq!(shown["revertChapterOff"], json!(false));
    assert!(
        shown["revertText"]
            .as_str()
            .unwrap()
            .contains("before your first edit")
    );
    assert!(
        shown["revertText"]
            .as_str()
            .unwrap()
            .contains("2 files differ")
    );
    click_id(&mut app, "revert-chapter");
    wait_toast(&mut app, "Chapter reverted to the original.");
    assert!(
        {
            let text = view(&mut app)["text"].as_str().unwrap().to_owned();
            !text.is_empty() && !text.contains("QQQTWO")
        },
        "the page shows the original"
    );
    let left = differing_entries(&path, &original);
    assert_eq!(left.len(), 1, "the other edit stays: {left:?}");
    wait_mode(&mut app, "edit");
    app.key(KEY_ESCAPE);
    wait_mode(&mut app, "read");

    go_to(&mut app, 1);
    wait_until(&mut app, "chapter 2", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 2 ")
    });
    palette_run(&mut app, "Delete this chapter");
    wait_until(&mut app, "the delete dialog", |app| {
        app.js("!document.getElementById('chapter-dialog').hidden") == json!(true)
    });
    let deleted_title = app.js("document.getElementById('chapter-dialog-title').textContent");
    click_id(&mut app, "chapter-dialog-ok");
    wait_until(&mut app, "the deleted count", |app| {
        !view(app)["deleted"].as_str().unwrap_or("").is_empty()
    });
    palette_run(&mut app, "Revert to backup");
    wait_until(&mut app, "the revert dialog", |app| {
        view(app)["revertOpen"] == json!(true)
    });
    let shown = view(&mut app);
    assert_eq!(
        shown["restores"].as_array().unwrap().len(),
        1,
        "the restore list shows"
    );
    assert!(
        deleted_title
            .as_str()
            .unwrap()
            .contains(shown["restores"][0].as_str().unwrap())
    );
    click_id(&mut app, "revert-whole");
    wait_toast(&mut app, "The whole book is back to your original.");
    assert_eq!(
        fs::read(&path).unwrap(),
        fs::read(&original).unwrap(),
        "whole revert is byte-equal"
    );
    wait_until(&mut app, "the records to clear", |app| {
        view(app)["deleted"] == json!("")
    });
    wait_until(&mut app, "the chapter back on the page", |app| {
        let text = view(app)["text"].as_str().unwrap_or("").to_owned();
        !text.is_empty() && !text.contains("QQQ")
    });
    app.close();
    assert_valid(&path);
    let _ = fs::remove_dir_all(&dir);
}

fn slot_backup(data: &Path) -> PathBuf {
    let books = fs::read_dir(data.join("books"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let slot = fs::read_dir(books.join("backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    slot.join("book.epub")
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_leftover_or_damaged_backup_is_refused() {
    let dir = dir_for("leftover");
    let data = dir.join("data");
    let path = copy_of("toc-nav-only", &dir, "book.epub");
    let other_data = dir.join("other-data");
    let other = dir.join("other.epub");
    fs::copy(&path, &other).unwrap();

    let mut maker = App::launch_with_data_opt(Some(&other), &other_data);
    maker.wait_first_text();
    install(&mut maker);
    wait_reader(&mut maker);
    type_at_start(&mut maker, "QQQFOREIGN");
    save_and_read(&mut maker);
    maker.close();

    let mut first = App::launch_with_data_opt(Some(&path), &data);
    first.wait_first_text();
    install(&mut first);
    wait_reader(&mut first);
    type_at_start(&mut first, "QQQMINE");
    save_and_read(&mut first);
    first.close();

    fs::copy(&other, &path).unwrap();
    let swapped = fs::read(&path).unwrap();
    let mut app = App::launch_with_data_opt(Some(&path), &data);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    palette_run(&mut app, "Revert to backup");
    wait_until(&mut app, "the refusal", |app| {
        view(app)["revertOpen"] == json!(true)
    });
    let shown = view(&mut app);
    assert!(
        shown["revertText"]
            .as_str()
            .unwrap()
            .contains("different copy"),
        "{shown}"
    );
    assert_eq!(shown["revertWhole"], json!(false));
    assert_eq!(shown["revertChapter"], json!(false));
    assert_eq!(shown["revertCancel"], json!("Close"));
    click_id(&mut app, "revert-cancel");
    app.close();
    assert_eq!(
        fs::read(&path).unwrap(),
        swapped,
        "the refusal changes nothing"
    );

    let dir_two = dir_for("damaged");
    let data_two = dir_two.join("data");
    let book_two = copy_of("toc-nav-only", &dir_two, "book.epub");
    let mut made = App::launch_with_data_opt(Some(&book_two), &data_two);
    made.wait_first_text();
    install(&mut made);
    wait_reader(&mut made);
    type_at_start(&mut made, "QQQKEPT");
    save_and_read(&mut made);
    made.close();
    let backup = slot_backup(&data_two);
    let mut bytes = fs::read(&backup).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xFF;
    fs::write(&backup, &bytes).unwrap();
    let kept = fs::read(&book_two).unwrap();
    let mut app = App::launch_with_data_opt(Some(&book_two), &data_two);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    palette_run(&mut app, "Revert to backup");
    wait_until(&mut app, "the refusal", |app| {
        view(app)["revertOpen"] == json!(true)
    });
    assert_eq!(
        view(&mut app)["revertWhole"],
        json!(false),
        "a damaged backup is not offered"
    );
    app.close();
    assert_eq!(fs::read(&book_two).unwrap(), kept);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&dir_two);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn marks_survive_an_edit_a_move_and_a_relaunch_and_detach_without_being_dropped() {
    let dir = dir_for("marks");
    let data = dir.join("data");
    let path = copy_of("toc-nav-only", &dir, "book.epub");
    let mut app = App::launch_with_data_opt(Some(&path), &data);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    go_to(&mut app, 3);
    wait_until(&mut app, "chapter 4", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 4 ")
    });
    click_id(&mut app, "toggle-notes");
    wait_until(&mut app, "the notes pane", |app| {
        app.js("!document.getElementById('notes-pane').hidden") == json!(true)
    });
    assert_eq!(view(&mut app)["add"], json!("Bookmark this chapter"));

    let before = fs::read(&path).unwrap();
    select_in_block(&mut app, 2, 5, 45);
    click_id(&mut app, "note-text");
    wait_until(&mut app, "the note field", |app| {
        app.js("document.activeElement.id") == json!("note-text")
    });
    go_to(&mut app, 2);
    wait_until(&mut app, "chapter 3", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 3 ")
    });
    assert_eq!(
        view(&mut app)["add"],
        json!("Bookmark this chapter"),
        "a selection does not follow the reader to another chapter"
    );
    go_to(&mut app, 3);
    wait_until(&mut app, "chapter 4 again", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 4 ")
    });
    app.js("document.activeElement.blur()");
    let quote = select_in_block(&mut app, 2, 5, 45);
    app.js("document.getElementById('note-text').value='A note of mine'");
    click_id(&mut app, "note-add");
    wait_until(&mut app, "the first mark", |app| marks(app).len() == 1);
    let row = &marks(&mut app)[0];
    assert_eq!(row["state"], json!("exact"));
    assert_eq!(row["text"], json!("A note of mine"));
    assert!(row["where"].as_str().unwrap().starts_with("Chapter 4: "));

    app.js("document.getSelection().removeAllRanges()");
    wait_until(&mut app, "the chapter button", |app| {
        view(app)["add"] == json!("Bookmark this chapter")
    });
    click_id(&mut app, "note-add");
    wait_until(&mut app, "the chapter mark", |app| marks(app).len() == 2);
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "a mark never changes the EPUB"
    );
    let books = fs::read_dir(data.join("books"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(
        books.join("marks.json").is_file(),
        "marks live in the data folder"
    );

    let mark_id = marks(&mut app)[1]["id"].as_u64().unwrap();
    type_at_start(&mut app, "QQQEDIT ");
    app.key(KEY_ENTER);
    app.type_text("A whole new paragraph before the marked words. ");
    app.key(KEY_ENTER);
    save_and_read(&mut app);
    wait_until(&mut app, "the mark to report moved", |app| {
        marks(app)
            .iter()
            .any(|row| row["id"] == json!(mark_id) && row["state"] == json!("moved"))
    });

    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    app.key_with(KEY_UP, MASK_ALT);
    wait_until(&mut app, "the shown chapter to move", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 3 ")
    });
    app.key(KEY_ESCAPE);
    wait_mode(&mut app, "read");
    wait_until(&mut app, "the row to keep the chapter's title after the move", |app| {
        marks(app).iter().any(|row| {
            row["id"] == json!(mark_id)
                && row["where"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("Chapter 4: ")
        })
    });

    go_to(&mut app, 0);
    wait_until(&mut app, "chapter 1", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 1 ")
    });
    let jump = app.js(&format!(
        "(()=>{{const b=document.querySelector('[data-mark=\"{mark_id}\"] .notejump').getBoundingClientRect();return {{x:b.left+b.width/2,y:b.top+b.height/2}}}})()"
    ));
    app.click(jump["x"].as_f64().unwrap(), jump["y"].as_f64().unwrap());
    wait_until(&mut app, "the jump to highlight the passage", |app| {
        let shown = view(app);
        shown["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 3 ")
            && shown["current"] == json!(true)
    });
    let found = app.js("CSS.highlights.get('ss-search-current').values().next().value.toString()");
    assert_eq!(found.as_str().unwrap(), quote);

    palette_run(&mut app, "Delete this chapter");
    wait_until(&mut app, "the delete dialog", |app| {
        app.js("!document.getElementById('chapter-dialog').hidden") == json!(true)
    });
    click_id(&mut app, "chapter-dialog-ok");
    wait_until(&mut app, "both marks to detach", |app| {
        let rows = marks(app);
        rows.len() == 2 && rows.iter().all(|row| row["state"] == json!("detached"))
    });
    let row = marks(&mut app)[0].clone();
    assert!(row["where"].as_str().unwrap().starts_with("Detached."));
    app.close();

    let mut later = App::launch_with_data_opt(Some(&path), &data);
    later.wait_first_text();
    install(&mut later);
    wait_reader(&mut later);
    click_id(&mut later, "toggle-notes");
    wait_until(&mut later, "the marks after a relaunch", |app| {
        marks(app).len() == 2
    });
    assert!(
        marks(&mut later)
            .iter()
            .all(|row| row["state"] == json!("detached"))
    );
    palette_run(&mut later, "Restore a deleted chapter");
    wait_until(&mut later, "the deleted list", |app| {
        app.js("!document.getElementById('deleted-dialog').hidden") == json!(true)
    });
    later.js("document.querySelector('#deleted-rows button').click()");
    wait_until(&mut later, "the marks to come back", |app| {
        marks(app)
            .iter()
            .all(|row| row["state"] != json!("detached"))
    });
    let last = marks(&mut later)[1]["id"].as_u64().unwrap();
    later.js(&format!(
        "document.querySelector('[data-mark=\"{last}\"] .mini').click()"
    ));
    wait_until(&mut later, "the delete", |app| marks(app).len() == 1);
    later.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_delete_unwraps_links_in_the_mounted_chapter_and_undo_is_exact() {
    let dir = dir_for("unwrap");
    let path = copy_of("reader-css", &dir, "book.epub");
    let original = dir.join("original.epub");
    fs::copy(&path, &original).unwrap();
    let mut app = App::launch_with_data_opt(Some(&path), &dir.join("data"));
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    let reader = jadeslip_core::Reader::open(
        fs::File::open(&path).unwrap(),
        jadeslip_core::DEFAULT_LIMITS,
    )
    .unwrap();
    let names: Vec<String> = reader
        .rows()
        .iter()
        .filter_map(|row| row.href.as_ref().map(|href| href.as_str().to_owned()))
        .collect();
    let target = names
        .iter()
        .position(|name| name.ends_with("font_face.xhtml"))
        .unwrap() as u32;
    let linker = (0..names.len() as u32)
        .find(|ix| {
            *ix != target
                && match reader.extract(jadeslip_core::ChapterIx(*ix)) {
                    Ok(jadeslip_core::Extracted::Document(bytes)) => {
                        String::from_utf8_lossy(&bytes).contains("href=\"font_face.xhtml\"")
                    }
                    _ => false,
                }
        })
        .unwrap();
    drop(reader);
    go_to(&mut app, target);
    wait_until(&mut app, "the target chapter", |app| {
        view(app)["text"]
            .as_str()
            .unwrap_or("")
            .contains("font_face")
    });
    palette_run(&mut app, "Delete this chapter");
    wait_until(&mut app, "the delete dialog", |app| {
        app.js("!document.getElementById('chapter-dialog').hidden") == json!(true)
    });
    click_id(&mut app, "chapter-dialog-ok");
    wait_until(&mut app, "the undo bar", |app| {
        view(app)["undo"] == json!(true)
    });
    let found = structure(&path);
    assert_eq!(found["dangling_links"], json!(0), "{found}");
    assert_valid(&path);
    let changed = differing_entries(&path, &original);
    assert!(
        changed.len() >= 3,
        "the package and a linking chapter changed: {changed:?}"
    );
    let linker_after = linker - u32::from(linker > target);
    go_to(&mut app, linker_after);
    wait_until(&mut app, "the linking chapter", |app| {
        view(app)["text"]
            .as_str()
            .unwrap_or("")
            .contains("A link sits here.")
    });
    assert_eq!(
        view(&mut app)["links"],
        json!(0),
        "the link is plain text now"
    );

    type_at_start(&mut app, "");
    app.key(KEY_ESCAPE);
    wait_mode(&mut app, "read");
    click_id(&mut app, "undo-action");
    wait_until(&mut app, "the undo", |app| {
        view(app)["toast"]
            .as_str()
            .unwrap_or("")
            .contains("Chapter restored.")
    });
    assert_eq!(
        differing_entries(&path, &original),
        Vec::<String>::new(),
        "every entry is byte-equal after undo"
    );
    go_to(&mut app, linker);
    wait_until(&mut app, "the link back", |app| {
        view(app)["links"].as_u64().unwrap_or(0) > 0
    });
    app.close();
    assert_valid(&path);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn the_place_is_saved_on_close_and_when_leaving_for_the_shelf() {
    let dir = dir_for("close");
    let data = dir.join("data");
    let path = copy_of("toc-nav-only", &dir, "book.epub");
    shelve(&data, &[&path]);
    let mut app = App::launch_with_data_opt(Some(&path), &data);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    go_to(&mut app, 3);
    wait_until(&mut app, "chapter 4", |app| {
        view(app)["where"]
            .as_str()
            .unwrap_or("")
            .starts_with("Position 4 ")
    });
    std::thread::sleep(Duration::from_millis(600));
    app.js("(()=>{const s=document.getElementById('scroller');s.scrollTop=(s.scrollHeight-s.clientHeight)/2})()");
    let scrolled = app
        .js("document.getElementById('scroller').scrollTop")
        .as_f64()
        .unwrap();
    assert!(scrolled > 50.0, "the chapter must scroll for this test");
    app.request_close_through_the_compositor();
    assert!(
        app.wait_exit(Duration::from_secs(5)),
        "the window closes in Read"
    );

    let library: Value =
        serde_json::from_slice(&fs::read(data.join("library.json")).unwrap()).unwrap();
    let entry = library["books"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert!(
        entry["read_point"]["point"].is_object(),
        "the scroll made within the debounce was saved"
    );

    let mut again = App::launch_with_data_opt(None, &data);
    again.wait_first_text();
    install(&mut again);
    click_id(&mut again, "hero-go");
    wait_reader(&mut again);
    wait_until(&mut again, "the same place", |app| {
        let top = app
            .js("document.getElementById('scroller').scrollTop")
            .as_f64()
            .unwrap_or(0.0);
        (top - scrolled).abs() < 40.0
    });
    std::thread::sleep(Duration::from_millis(600));
    let point_of = |data: &Path| -> Value {
        let library: Value =
            serde_json::from_slice(&fs::read(data.join("library.json")).unwrap()).unwrap();
        library["books"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()["read_point"]["point"]
            .clone()
    };
    let before_leaving = point_of(&data);
    again.js("(()=>{const s=document.getElementById('scroller');s.scrollTop=s.scrollTop/3})()");
    click_id(&mut again, "go-library");
    wait_shelf(&mut again);
    assert_ne!(
        point_of(&data),
        before_leaving,
        "leaving for the shelf saves the place at once"
    );
    again.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn leaving_for_the_shelf_with_unsaved_text_asks_first() {
    let dir = dir_for("unsaved");
    let path = copy_of("toc-nav-only", &dir, "book.epub");
    let original = fs::read(&path).unwrap();
    let mut app = App::launch_with_data_opt(Some(&path), &dir.join("data"));
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    type_at_start(&mut app, "QQQASK");
    wait_until(&mut app, "the text", |app| {
        view(app)["text"].as_str().unwrap_or("").contains("QQQASK")
    });
    click_id(&mut app, "go-library");
    wait_until(&mut app, "the unsaved dialog", |app| {
        view(app)["dialog"] == json!(true)
    });
    assert_eq!(view(&mut app)["onShelf"], json!(false));
    click_id(&mut app, "dialog-keep");
    wait_until(&mut app, "the dialog to close", |app| {
        view(app)["dialog"] == json!(false)
    });
    assert_eq!(view(&mut app)["mode"], json!("edit"));
    click_id(&mut app, "go-library");
    wait_until(&mut app, "the unsaved dialog", |app| {
        view(app)["dialog"] == json!(true)
    });
    click_id(&mut app, "dialog-discard");
    wait_shelf(&mut app);
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "a discard writes nothing"
    );
    app.op(&json!({"op": "arm_save_answer", "path": path}));
    click_id(&mut app, "shelf-open");
    wait_reader(&mut app);
    type_at_start(&mut app, "QQQKEEP");
    click_id(&mut app, "go-library");
    wait_until(&mut app, "the unsaved dialog", |app| {
        view(app)["dialog"] == json!(true)
    });
    click_id(&mut app, "dialog-save");
    wait_shelf(&mut app);
    assert_ne!(fs::read(&path).unwrap(), original, "a save writes the book");
    app.close();
    assert_valid(&path);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_mounted_linking_chapter_updates_on_delete_and_an_edit_warns_at_undo() {
    let dir = dir_for("mounted");
    let path = copy_of("reader-css", &dir, "book.epub");
    let mut app = App::launch_with_data_opt(Some(&path), &dir.join("data"));
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    let found = app.js("(async()=>{const m=await import('/main.js');for(let ix=0;ix<20;ix++){await m.goTo(ix);const s=document.getElementById('book-host').shadowRoot;const l=[...s.querySelectorAll('.ss-body a')].find((a)=>a.getAttribute('href')==='font_face.xhtml');if(l)return ix}return -1})()");
    assert!(
        found.as_i64().unwrap() >= 0,
        "the fixture has a linking chapter"
    );
    assert_eq!(view(&mut app)["links"], json!(1));
    app.key(KEY_E);
    wait_mode(&mut app, "edit");
    wait_until(&mut app, "the rail", |app| {
        app.js("document.querySelectorAll('#toc-spacer .row:not([hidden])').length")
            .as_u64()
            .unwrap_or(0)
            > 0
    });
    let row = app.js("(()=>{const r=[...document.querySelectorAll('#toc-spacer .row:not([hidden])')].find((n)=>n.querySelector('.t').textContent==='font_face');const b=r.getBoundingClientRect();return {x:b.left+b.width/2,y:b.top+b.height/2}})()");
    app.move_pointer(
        row["x"].as_f64().unwrap() - 60.0,
        row["y"].as_f64().unwrap(),
    );
    wait_until(&mut app, "the row actions", |app| {
        app.js("!document.getElementById('rail-actions').hidden") == json!(true)
    });
    click_id(&mut app, "rail-delete");
    wait_until(&mut app, "the delete dialog", |app| {
        app.js("!document.getElementById('chapter-dialog').hidden") == json!(true)
    });
    click_id(&mut app, "chapter-dialog-ok");
    wait_until(&mut app, "the undo bar", |app| {
        view(app)["undo"] == json!(true)
    });
    wait_until(&mut app, "the mounted chapter to lose its link", |app| {
        view(app)["links"] == json!(0)
    });
    assert_eq!(view(&mut app)["mode"], json!("edit"));
    app.type_text("QQQLATER");
    save_and_read(&mut app);
    assert_valid(&path);
    click_id(&mut app, "undo-action");
    wait_toast(&mut app, "stay as plain text");
    assert_valid(&path);
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_whole_revert_lands_on_a_real_chapter_when_the_shown_one_was_added_after_the_backup() {
    let dir = dir_for("landing");
    let path = copy_of("toc-nav-only", &dir, "book.epub");
    let original = dir.join("original.epub");
    fs::copy(&path, &original).unwrap();
    let mut app = App::launch_with_data_opt(Some(&path), &dir.join("data"));
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    palette_run(&mut app, "Add chapter");
    wait_mode(&mut app, "edit");
    wait_until(&mut app, "the revert button", |app| {
        app.js("window.__box('tool-revert').height")
            .as_f64()
            .unwrap_or(0.0)
            > 0.0
    });
    click_id(&mut app, "tool-revert");
    wait_until(&mut app, "the dialog", |app| {
        view(app)["revertOpen"] == json!(true)
    });
    assert_eq!(
        view(&mut app)["revertChapterOff"],
        json!(true),
        "an added chapter has no backup of its own"
    );
    click_id(&mut app, "revert-whole");
    wait_toast(&mut app, "The whole book is back to your original.");
    assert_eq!(fs::read(&path).unwrap(), fs::read(&original).unwrap());
    wait_until(&mut app, "a real chapter on the page", |app| {
        !view(app)["text"].as_str().unwrap_or("").is_empty()
    });
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_book_switch_closes_questions_about_the_old_book_and_the_picker_holds_the_keys() {
    let dir = dir_for("switch");
    let one = copy_of("toc-nav-only", &dir, "one.epub");
    let two = copy_of("toc-ncx-only", &dir, "two.epub");
    let two_before = fs::read(&two).unwrap();
    let mut app = App::launch_with_data_opt(Some(&one), &dir.join("data"));
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    let old_title = view(&mut app)["crumb"].clone();
    palette_run(&mut app, "Delete this chapter");
    wait_until(&mut app, "the delete dialog", |app| {
        app.js("!document.getElementById('chapter-dialog').hidden") == json!(true)
    });
    app.op(&json!({"op": "drop_paths", "paths": [two]}));
    wait_until(&mut app, "the new book", |app| {
        view(app)["crumb"] != old_title
    });
    assert_eq!(
        app.js("document.getElementById('chapter-dialog').hidden"),
        json!(true),
        "the question about the old book is gone"
    );
    assert_eq!(fs::read(&two).unwrap(), two_before);

    app.op(&json!({"op": "arm_save_answer", "real": true}));
    palette_run(&mut app, "Open an EPUB");
    wait_until(&mut app, "the wait while picking", |app| {
        app.js("!document.getElementById('picker-wait').hidden") == json!(true)
    });
    app.js("document.dispatchEvent(new KeyboardEvent('keydown',{key:'e',bubbles:true}))");
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        view(&mut app)["mode"],
        json!("read"),
        "E does nothing while the picker is open"
    );
    drop(app);
    let _ = fs::remove_dir_all(&dir);
}

const SHELF_AUDIT: &str = r#"(()=>{
  const classes = new Set(['article.rcard', 'div.cover', 'span.covertitle', 'img.', 'div.rbody', 'p.kick', 'span.', 'span.dot', 'h3.rtitle', 'p.pline', 'p.quote', 'div.prog', 'span.track', 'span.fill', 'span.stat', 'div.racts', 'button.btn', 'svg.icon', 'use.', 'button.tab', 'span.tabcount', 'div.card', 'button.cardgo', 'span.coverbar', 'span.coverfill', 'span.badge', 'span.cardtitle', 'span.dotted', 'button.trow', 'span.td', 'span.names', 'span.rt', 'span.ru', 'dt.', 'dd.', 'button.menuitem', 'span.grow']);
  const attributes = new Set(['class', 'id', 'data-key', 'role', 'tabindex', 'style', 'href', 'src', 'alt', 'loading', 'decoding', 'hidden', 'aria-label', 'aria-hidden', 'aria-haspopup', 'aria-expanded', 'aria-selected', 'aria-current', 'aria-checked']);
  const strangers = [];
  for (const root of ['recent-cards', 'shelf-tabs', 'shelf-grid', 'shelf-rows', 'shelf-status', 'detail-facts', 'sort-menu']) {
    for (const node of document.getElementById(root).querySelectorAll('*')) {
      const name = `${node.localName}.${(node.getAttribute('class') ?? '').split(' ')[0]}`;
      if (!classes.has(name)) strangers.push(name);
      for (const attribute of node.attributes) {
        if (!attributes.has(attribute.name)) strangers.push(`${name}@${attribute.name}`);
      }
      if (node.localName === 'use' && !/^#i-[a-z0-9-]+$/.test(node.getAttribute('href'))) strangers.push('use@href');
      if (node.localName === 'img' && !node.getAttribute('src').startsWith('jadeslip-book://cover/')) strangers.push('img@src');
    }
  }
  return strangers;
})()"#;

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn hostile_note_and_title_stay_plain_text() {
    let dir = dir_for("hostile");
    let data = dir.join("data");
    let path = copy_of("toc-nav-only", &dir, "book.epub");
    shelve(&data, &[&path]);
    let mut app = App::launch_with_data_opt(Some(&path), &data);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    click_id(&mut app, "toggle-notes");
    wait_until(&mut app, "the notes pane", |app| {
        app.js("!document.getElementById('notes-pane').hidden") == json!(true)
    });
    let payload = "<img src=x onerror=\"document.title='pwned'\"><b>bold</b>";
    app.js(&format!(
        "document.getElementById('note-text').value={}",
        json!(payload)
    ));
    click_id(&mut app, "note-add");
    wait_until(&mut app, "the mark", |app| marks(app).len() == 1);
    assert_eq!(marks(&mut app)[0]["text"], json!(payload));
    assert_eq!(
        app.js("document.querySelectorAll('#notes-list img, #notes-list b').length"),
        json!(0)
    );
    app.close();

    let file = data.join("library.json");
    let mut library: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    for entry in library["books"].as_object_mut().unwrap().values_mut() {
        entry["title"] = json!(payload);
        entry["read_point"] = json!({
            "href": "x",
            "point": null,
            "chapter_label": payload,
            "ordinal": 1,
            "progress": 0.5,
            "passage": payload,
        });
    }
    fs::write(&file, serde_json::to_vec(&library).unwrap()).unwrap();
    let mut shelf = App::launch_with_data_opt(None, &data);
    shelf.wait_first_text();
    install(&mut shelf);
    assert_eq!(view(&mut shelf)["heroTitle"], json!(payload));
    assert_eq!(view(&mut shelf)["quote"], json!(payload));
    assert_eq!(view(&mut shelf)["line"], json!(format!("{payload}.")));
    assert_eq!(
        shelf.js("document.querySelectorAll('#library img, #library b').length"),
        json!(0)
    );
    assert_eq!(shelf.js(SHELF_AUDIT), json!([]), "the grid holds only the shelf's own markup");
    click_id(&mut shelf, "view-list");
    wait_until(&mut shelf, "the list", |app| view(app)["list"] == json!(true));
    assert_eq!(shelf.js(SHELF_AUDIT), json!([]), "the list holds only the shelf's own markup");
    assert_eq!(shelf.js("document.querySelectorAll('#library img, #library b').length"), json!(0));
    assert_ne!(shelf.js("document.title"), json!("pwned"));
    shelf.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn time_the_shelf_open_and_revert_on_the_largest_book() {
    let dir = dir_for("timing");
    let data = dir.join("data");
    let large = writable_copy(&largest_real_book(), &dir.join("large.epub"));
    shelve(&data, &[&large]);
    let mut launches = Vec::new();
    let mut opens = Vec::new();
    for _ in 0..5 {
        let mut app = App::launch_with_data_opt(None, &data);
        let at = app.wait_first_text();
        launches.push(at - app.spawned_at_ms);
        install(&mut app);
        let started = Instant::now();
        click_id(&mut app, "hero-go");
        wait_reader(&mut app);
        opens.push(started.elapsed().as_secs_f64() * 1e3);
        app.close();
    }
    launches.sort_by(f64::total_cmp);
    opens.sort_by(f64::total_cmp);
    println!("shelf after spawn ms {launches:.0?}");
    println!("shelf click to text ms {opens:.0?}");

    let mut app = App::launch_with_data_opt(Some(&large), &data);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    type_at_start(&mut app, "QQQTIME");
    save_and_read(&mut app);
    let chapter = app.js("(async()=>{const ipc=await import('/ipc.js');const t=performance.now();await ipc.revertPreview(0);return performance.now()-t})()");
    println!(
        "revert preview first call ms {:.1}",
        chapter.as_f64().unwrap()
    );
    let timed = |app: &mut App, choice: &str| -> f64 {
        let script = format!(
            "(async()=>{{const ipc=await import('/ipc.js');const t=performance.now();await ipc.revertBook({choice},null);return performance.now()-t}})()"
        );
        app.try_js(&script, 60_000).unwrap().as_f64().unwrap()
    };
    let shown = app
        .js("(await import('/main.js')).shownChapter()")
        .as_u64()
        .unwrap();
    println!(
        "revert chapter ms {:.1}",
        timed(&mut app, &format!("{{kind:'chapter',ix:{shown}}}"))
    );
    println!(
        "revert whole book ms {:.1}",
        timed(&mut app, "{kind:'wholeBook'}")
    );
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

const COVER_STATES: &str = r"(()=>{
  const state = (node) => {
    const img = node.querySelector('img');
    return {
      classes: node.className,
      img: img !== null,
      loaded: img !== null && img.complete && img.naturalWidth > 0,
      width: img?.naturalWidth ?? 0,
      height: img?.naturalHeight ?? 0,
      src: img?.getAttribute('src') ?? '',
      alt: img?.getAttribute('alt') ?? null,
      lazy: img?.getAttribute('loading') ?? null,
    };
  };
  return {
    hero: state(document.querySelector('#hero .cover')),
    cards: [...document.querySelectorAll('#shelf-grid .card')].map((node) => ({ title: node.querySelector('.cardtitle').textContent, ...state(node.querySelector('.cover')) })),
  };
})()";

fn cover_states(app: &mut App) -> Value {
    app.js(COVER_STATES)
}

fn card_titled(states: &Value, title: &str) -> Value {
    states["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["title"] == json!(title))
        .unwrap_or_else(|| panic!("no card titled {title}"))
        .clone()
}

fn assert_thumbnail(state: &Value, what: &str) {
    assert_eq!(state["loaded"], json!(true), "{what}: {state}");
    assert!(
        state["width"].as_u64().unwrap() <= 400 && state["height"].as_u64().unwrap() <= 600,
        "{what} shows the app's thumbnail, not the book's image: {state}"
    );
    assert!(
        state["src"]
            .as_str()
            .unwrap()
            .starts_with("jadeslip-book://cover/"),
        "{what}: {state}"
    );
    assert_eq!(state["alt"], json!(""), "{what}");
}

fn assert_generated_look(state: &Value, what: &str) {
    assert_eq!(state["img"], json!(false), "{what}: {state}");
    let classes = state["classes"].as_str().unwrap();
    let tokens: Vec<&str> = classes.split(' ').collect();
    assert_eq!(tokens[0], "cover", "{what}: {classes}");
    assert!(!tokens.contains(&"has"), "{what}: {classes}");
    assert!(
        tokens[1].starts_with('b') && tokens[1].len() == 2,
        "{what} keeps its colour: {classes}"
    );
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn covers_show_on_the_shelf_and_books_without_one_keep_their_look() {
    let dir = dir_for("covers");
    let data = dir.join("data");
    let three = copy_of("cover-epub3", &dir, "three.epub");
    let plain = copy_of("toc-nav-only", &dir, "plain.epub");
    let two = copy_of("cover-epub2", &dir, "two.epub");
    shelve(&data, &[&three, &plain]);
    jadeslip_core::Library::open(jadeslip_core::LibraryConfig {
        data_dir: data.clone(),
    })
    .unwrap()
    .add_to_shelf(&two)
    .unwrap();

    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    install(&mut app);
    assert_eq!(view(&mut app)["heroTitle"], json!("Cover Epub2"));
    wait_until(&mut app, "the cover of a book shelved before covers", |app| {
        let states = cover_states(app);
        card_titled(&states, "Cover Epub2")["loaded"] == json!(true)
            && states["hero"]["loaded"] == json!(true)
    });
    wait_until(&mut app, "every cover", |app| {
        card_titled(&cover_states(app), "Cover Epub3")["loaded"] == json!(true)
    });
    let states = cover_states(&mut app);
    assert_thumbnail(&states["hero"], "the featured book");
    assert_eq!(states["hero"]["lazy"], json!(null), "the strip covers load at once");
    for title in ["Cover Epub3", "Cover Epub2"] {
        let card = card_titled(&states, title);
        assert_thumbnail(&card, title);
        assert_eq!(card["lazy"], json!("lazy"));
        assert!(card["classes"].as_str().unwrap().split(' ').any(|token| token == "has"));
    }
    assert_generated_look(&card_titled(&states, "Toc Nav Only"), "the plain card");
    assert_eq!(
        app.js("[...document.querySelectorAll('#lib img')].every((img) => img.getAttribute('src').startsWith('jadeslip-book://cover/'))"),
        json!(true),
        "every image on the shelf is a cover from the data folder"
    );

    let rows = books(&mut app);
    let plain_row = rows
        .iter()
        .find(|row| row["title"] == json!("Toc Nav Only"))
        .unwrap();
    app.click(
        plain_row["x"].as_f64().unwrap(),
        plain_row["y"].as_f64().unwrap(),
    );
    wait_reader(&mut app);
    click_id(&mut app, "go-library");
    wait_shelf(&mut app);
    wait_until(&mut app, "the plain book leads", |app| {
        view(app)["heroTitle"] == json!("Toc Nav Only")
    });
    let hero = cover_states(&mut app)["hero"].clone();
    assert_generated_look(&hero, "the plain book in the strip");
    assert_eq!(
        app.js("document.querySelector('#hero .covertitle').textContent"),
        json!("Toc Nav Only")
    );
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

const SHELF_BOOKS: u32 = 60;

const COVERS_SHOWN: &str = r"(async()=>{
  const box = document.getElementById('lib').getBoundingClientRect();
  const seen = (node) => { const rect = node.getBoundingClientRect(); return rect.bottom > box.top && rect.top < box.bottom; };
  const imgs = () => [...document.querySelectorAll('#lib .cover img')].filter(seen);
  for (;;) {
    const shown = imgs();
    if (shown.every((img) => img.complete && img.naturalWidth > 0)) return { count: shown.length, at: Date.now() };
    await new Promise((done) => setTimeout(done, 5));
  }
})()";

fn shelf_of_books(dir: &Path, data: &Path) -> Vec<PathBuf> {
    let library = jadeslip_core::Library::open(jadeslip_core::LibraryConfig {
        data_dir: data.to_owned(),
    })
    .unwrap();
    (0..SHELF_BOOKS)
        .map(|index| {
            let path = dir.join(format!("shelf-{index:02}.epub"));
            let mut zip = jadeslip_fixtures::zip::ZipWriter::new(fs::File::create(&path).unwrap());
            jadeslip_fixtures::books::shelf_book(index, index % 2 == 0, &mut zip).unwrap();
            zip.finish().unwrap();
            let key = library.add_to_shelf(&path).unwrap().key;
            let book = library.open_book(&library.entry(key).unwrap().path).unwrap();
            library.update_cover(key, &book.read()).unwrap();
            path
        })
        .collect()
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn time_a_shelf_of_sixty_books() {
    let dir = dir_for("sixty");
    let data = dir.join("data");
    let paths = shelf_of_books(&dir, &data);
    assert_eq!(paths.len(), SHELF_BOOKS as usize);
    let mut shelves = Vec::new();
    let mut covers = Vec::new();
    let mut counts = Vec::new();
    for _ in 0..5 {
        let mut app = App::launch_with_data_opt(None, &data);
        let at = app.wait_first_text();
        install(&mut app);
        assert_eq!(books(&mut app).len(), SHELF_BOOKS as usize);
        let shown = app.js(COVERS_SHOWN);
        shelves.push(at - app.spawned_at_ms);
        covers.push(shown["at"].as_f64().unwrap() - app.spawned_at_ms);
        counts.push(shown["count"].as_u64().unwrap());
        app.close();
    }
    println!(
        "sixty books: shelf after spawn ms {shelves:.0?}, median {:.0}",
        median(shelves.clone())
    );
    println!(
        "sixty books: visible covers {counts:?} complete after spawn ms {covers:.0?}, median {:.0}",
        median(covers.clone())
    );

    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    app.js("[...document.querySelectorAll('#shelf-grid .card')].find((card) => card.querySelector('.cardtitle').textContent === 'Shelf Book 58').querySelector('.cardgo').focus()");
    app.js(COVERS_SHOWN);
    let shots = scratch_dir().join("shelf-shots");
    fs::create_dir_all(&shots).unwrap();
    for theme in ["dark", "light"] {
        let path = shots.join(format!("shelf-{theme}.png"));
        app.snapshot_png(&path);
        println!("screenshot {}", path.display());
        app.js("document.getElementById('shelf-theme').click()");
        std::thread::sleep(Duration::from_millis(300));
    }
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

fn book_titled(app: &mut App, title: &str) -> Value {
    books(app)
        .into_iter()
        .find(|row| row["title"] == json!(title))
        .unwrap_or_else(|| panic!("no book titled {title}"))
}

fn menu_for(app: &mut App, title: &str) {
    let row = book_titled(app, title);
    app.right_click(row["x"].as_f64().unwrap(), row["y"].as_f64().unwrap());
    wait_until(app, "the book menu", |app| !view(app)["menu"].as_array().unwrap().is_empty());
}

fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn add_bookmark(app: &mut App) {
    click_id(app, "toggle-notes");
    wait_until(app, "the notes pane", |app| {
        app.js("!document.getElementById('notes-pane').hidden") == json!(true)
    });
    click_id(app, "note-add");
    wait_until(app, "the mark", |app| marks(app).len() == 1);
}

fn remove_with(app: &mut App, title: &str, data: bool, file: bool) {
    menu_for(app, title);
    click_id(app, "menu-remove");
    wait_until(app, "the remove dialog", |app| view(app)["removeOpen"] == json!(true));
    if data {
        click_id(app, "remove-data");
    }
    if file {
        click_id(app, "remove-file");
    }
    click_id(app, "remove-ok");
    wait_until(app, "the dialog to close", |app| view(app)["removeOpen"] == json!(false));
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn remove_keeps_everything_unless_asked_and_trashes_only_what_was_ticked() {
    let dir = dir_for("remove");
    let data = dir.join("data");
    let xdg = dir.join("xdg");
    let trash = xdg.join("Trash");
    let keep = copy_of("toc-nav-only", &dir, "keep.epub");
    let wiped = copy_of("toc-ncx-only", &dir, "wiped.epub");
    let binned = copy_of("toc-none", &dir, "binned.epub");
    shelve(&data, &[&binned, &wiped, &keep]);
    let binned_copy = copy_titled(&data, "Toc None");
    let keep_copy = copy_titled(&data, "Toc Nav Only");
    let env = [("XDG_DATA_HOME", xdg.as_path())];

    let mut app = App::launch_with_data_env(Some(&wiped), &data, &env);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    add_bookmark(&mut app);
    app.close();

    let mut app = App::launch_with_data_env(Some(&keep), &data, &env);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    go_to(&mut app, 2);
    wait_until(&mut app, "chapter 3", |app| {
        view(app)["where"].as_str().unwrap_or("").starts_with("Position 3 ")
    });
    add_bookmark(&mut app);
    click_id(&mut app, "go-library");
    wait_shelf(&mut app);
    let keep_key = book_titled(&mut app, "Toc Nav Only")["key"].as_str().unwrap().to_owned();

    menu_for(&mut app, "Toc Nav Only");
    click_id(&mut app, "menu-remove");
    wait_until(&mut app, "the remove dialog", |app| view(app)["removeOpen"] == json!(true));
    let asked = view(&mut app);
    assert!(asked["removeText"].as_str().unwrap().starts_with("Remove Toc Nav Only from the shelf? "), "{asked}");
    assert!(asked["removeText"].as_str().unwrap().contains("adding the book again brings them back"));
    assert!(asked["removeData"].as_str().unwrap().starts_with("Also delete Jadeslip's data for this book ("));
    assert!(asked["removeText"].as_str().unwrap().contains("The file you added it from is never touched."));
    assert!(asked["removeFile"].as_str().unwrap().starts_with("Also delete Jadeslip's copy of the EPUB ("), "{asked}");
    assert_eq!(app.js("document.getElementById('remove-data').checked || document.getElementById('remove-file').checked"), json!(false));
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "Escape to cancel", |app| view(app)["removeOpen"] == json!(false));
    assert_eq!(books(&mut app).len(), 3);
    remove_with(&mut app, "Toc Nav Only", false, false);
    wait_toast(&mut app, "Removed Toc Nav Only from the shelf.");
    wait_until(&mut app, "two books left", |app| books(app).len() == 2);
    assert!(keep.exists());
    assert!(keep_copy.exists(), "Jadeslip's copy stays unless asked");
    assert!(data.join("books").join(&keep_key).join("marks.json").exists(), "the notes stay");
    assert!(names_in(&trash.join("files")).is_empty(), "nothing went to the trash");

    app.op(&json!({"op": "drop_paths", "paths": [keep]}));
    wait_until(&mut app, "the book back on the shelf", |app| books(app).len() == 3);
    assert_eq!(copy_titled(&data, "Toc Nav Only"), keep_copy, "the kept copy comes back");
    assert_eq!(fs::read_dir(data.join("library")).unwrap().count(), 3, "no second copy");
    click_id(&mut app, "hero-go");
    wait_reader(&mut app);
    wait_until(&mut app, "the old place", |app| {
        view(app)["where"].as_str().unwrap_or("").starts_with("Position 3 ")
    });
    click_id(&mut app, "toggle-notes");
    wait_until(&mut app, "the old mark", |app| marks(app).len() == 1);
    click_id(&mut app, "go-library");
    wait_shelf(&mut app);
    assert_eq!(books(&mut app).len(), 3, "adding it again brings it back");

    let wiped_key = book_titled(&mut app, "Toc Ncx Only")["key"].as_str().unwrap().to_owned();
    assert!(data.join("books").join(&wiped_key).exists());
    remove_with(&mut app, "Toc Ncx Only", true, false);
    wait_toast(&mut app, "Moved its data to the trash.");
    wait_until(&mut app, "two books again", |app| books(app).len() == 2);
    assert!(wiped.exists(), "the file stays");
    assert!(!data.join("books").join(&wiped_key).exists(), "the data folder is gone from the data folder");
    assert_eq!(names_in(&trash.join("files")), vec![wiped_key.clone()]);
    assert!(trash.join("info").join(format!("{wiped_key}.trashinfo")).exists());

    let binned_bytes = fs::read(&binned).unwrap();
    remove_with(&mut app, "Toc None", false, true);
    wait_toast(&mut app, "Moved Jadeslip's copy of the EPUB to the trash.");
    wait_until(&mut app, "one book left", |app| books(app).len() == 1);
    assert!(!binned_copy.exists(), "Jadeslip's copy went to the trash");
    assert_eq!(fs::read(&binned).unwrap(), binned_bytes, "the added file is never touched");
    let mut wanted = vec![binned_copy.file_name().unwrap().to_string_lossy().into_owned(), wiped_key];
    wanted.sort();
    assert_eq!(names_in(&trash.join("files")), wanted);
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_failed_trash_leaves_the_book_on_the_shelf_and_says_so() {
    let dir = dir_for("trash-fails");
    let data = dir.join("data");
    let xdg = dir.join("xdg");
    fs::create_dir_all(xdg.join("Trash/files")).unwrap();
    fs::write(xdg.join("Trash/info"), b"not a folder").unwrap();
    let book = copy_of("toc-nav-only", &dir, "book.epub");
    shelve(&data, &[&book]);
    let mut app = App::launch_with_data_env(None, &data, &[("XDG_DATA_HOME", xdg.as_path())]);
    app.wait_first_text();
    install(&mut app);
    menu_for(&mut app, "Toc Nav Only");
    click_id(&mut app, "menu-remove");
    wait_until(&mut app, "the remove dialog", |app| view(app)["removeOpen"] == json!(true));
    click_id(&mut app, "remove-file");
    click_id(&mut app, "remove-ok");
    wait_until(&mut app, "the error", |app| !view(app)["removeError"].as_str().unwrap().is_empty());
    assert!(view(&mut app)["removeError"].as_str().unwrap().contains("nothing was removed"));
    click_id(&mut app, "remove-cancel");
    assert!(book.exists());
    assert!(copy_titled(&data, "Toc Nav Only").exists());
    assert_eq!(books(&mut app).len(), 1);
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn the_book_menu_shows_the_folder_exports_a_copy_and_marks_the_book_as_yours() {
    let dir = dir_for("menu");
    let data = dir.join("data");
    let book = copy_of("toc-nav-only", &dir, "book.epub");
    shelve(&data, &[&book]);
    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    install(&mut app);

    menu_for(&mut app, "Toc Nav Only");
    click_id(&mut app, "menu-folder");
    let folder = format!("reveal:{}", fs::canonicalize(data.join("library")).unwrap().display());
    wait_until(&mut app, "the folder to open", |app| {
        app.state()["external"].as_array().unwrap().contains(&json!(folder))
    });
    assert_eq!(app.state()["external"].as_array().unwrap().len(), 1);

    let copy = dir.join("copy.epub");
    app.op(&json!({"op": "arm_save_answer", "path": copy}));
    menu_for(&mut app, "Toc Nav Only");
    click_id(&mut app, "menu-export");
    wait_toast(&mut app, "Copy of Toc Nav Only saved.");
    assert_eq!(fs::read(&copy).unwrap(), fs::read(&book).unwrap());
    assert_eq!(view(&mut app)["onShelf"], json!(true), "exporting stays on the shelf");

    menu_for(&mut app, "Toc Nav Only");
    click_id(&mut app, "menu-own");
    wait_until(&mut app, "the Mine tab", |app| {
        app.js("document.getElementById('shelf-tab-mine').textContent") == json!("Mine1")
    });
    assert_eq!(view(&mut app)["kicker"], json!("Continue writing"));
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn a_shelf_of_two_hundred_books_scrolls_smoothly() {
    let dir = dir_for("two-hundred");
    let data = dir.join("data");
    fs::create_dir_all(&data).unwrap();
    let books_json: serde_json::Map<String, Value> = (0..200u32)
        .map(|index| {
            let key = format!("{:032x}", 0x5eed_0000_u128 + u128::from(index) * 0x1_0000_0001);
            let progress = f64::from(index % 7) / 7.0;
            let entry = json!({
                "path": dir.join(format!("book-{index:03}.epub")),
                "title": format!("Shelf Book {index:03}"),
                "author": if index % 5 == 0 { Value::Null } else { json!(format!("Author {}", index % 17)) },
                "inflated_spine_bytes": 40_000 * u64::from(index + 1),
                "linear_chapters": 10 + index * 31,
                "own": index % 23 == 0,
                "read_point": if index % 3 == 0 { Value::Null } else { json!({"href": "c.xhtml", "point": null, "chapter_label": format!("Chapter {}: A Title", index * 7 + 1), "ordinal": index * 7 + 1, "progress": progress, "passage": "He did not look back at the mountain gate."}) },
                "write_point": null,
                "added_at": 1_700_000_000_000_u64 + u64::from(index),
                "touched_at": 1_700_000_000_000_u64 + u64::from(index) * 3_600_000,
                "cover": {"file": "x", "found": "absent"},
            });
            (key, entry)
        })
        .collect();
    fs::write(
        data.join("library.json"),
        serde_json::to_vec(&json!({"version": 1, "books": books_json})).unwrap(),
    )
    .unwrap();
    let mut app = App::launch_with_data_opt(None, &data);
    let at = app.wait_first_text();
    install(&mut app);
    assert_eq!(books(&mut app).len(), 200);
    println!("two hundred books: shelf after spawn {:.0} ms", at - app.spawned_at_ms);
    let frames = app
        .try_js(
            "(async()=>{const lib=document.getElementById('lib');const gaps=[];let last=performance.now();const end=lib.scrollHeight-lib.clientHeight;for(let i=0;i<=120;i+=1){lib.scrollTop=end*i/120;await new Promise((r)=>requestAnimationFrame(r));const now=performance.now();gaps.push(now-last);last=now;}gaps.sort((a,b)=>a-b);return {median:gaps[60],p95:gaps[114],max:gaps[120],end}})()",
            60_000,
        )
        .unwrap();
    println!("two hundred books: scroll frame ms {frames}");
    assert!(frames["end"].as_f64().unwrap() > 5000.0, "the grid is long enough to scroll: {frames}");
    assert!(frames["p95"].as_f64().unwrap() < 34.0, "scrolling keeps up: {frames}");
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

fn reader_add_shown(app: &mut App) -> bool {
    app.js("!document.getElementById('add-to-shelf').hidden") == json!(true)
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn add_to_shelf_from_the_reader_moves_the_edits_to_the_copy() {
    let dir = dir_for("reader-add");
    let data = dir.join("data");
    let path = copy_of("toc-nav-only", &dir, "book.epub");
    let twin = copy_of("toc-nav-only", &dir, "twin.epub");
    let original = fs::read(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();

    let mut app = App::launch_with_data(&path, &data);
    app.wait_first_text();
    install(&mut app);
    wait_reader(&mut app);
    assert!(reader_add_shown(&mut app), "a book opened in place offers Add to shelf");
    go_to(&mut app, 2);
    wait_until(&mut app, "chapter 3", |app| {
        view(app)["where"].as_str().unwrap_or("").starts_with("Position 3 ")
    });
    click_id(&mut app, "add-to-shelf");
    wait_toast(&mut app, "Added to your shelf. Edits now go to Jadeslip's copy.");
    wait_until(&mut app, "Add to hide", |app| !reader_add_shown(app));
    assert!(view(&mut app)["where"].as_str().unwrap().starts_with("Position 3 "), "the place carries over");
    let copy = copy_titled(&data, "Toc Nav Only");
    assert!(copy.starts_with(data.join("library")));
    assert_eq!(fs::read(&copy).unwrap(), original);

    type_at_start(&mut app, "Copy only. ");
    save_and_read(&mut app);
    assert_ne!(fs::read(&copy).unwrap(), original, "the edit went to the copy");
    assert_eq!(fs::read(&path).unwrap(), original, "the added file is untouched");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    click_id(&mut app, "go-library");
    wait_shelf(&mut app);
    assert_eq!(books(&mut app).len(), 1);
    app.close();

    let mut again = App::launch_with_data(&twin, &data);
    again.wait_first_text();
    install(&mut again);
    wait_reader(&mut again);
    assert!(reader_add_shown(&mut again));
    click_id(&mut again, "add-to-shelf");
    wait_until(&mut again, "the duplicate offer", |app| app.js("!document.getElementById('dup-dialog').hidden") == json!(true));
    assert_eq!(
        again.js("[document.getElementById('dup-cancel').textContent, document.getElementById('dup-open').textContent]"),
        json!(["Keep reading this file", "Open the shelf copy"])
    );
    if let Ok(shots) = std::env::var("JADESLIP_SHOTS_DIR") {
        again.snapshot_png(&Path::new(&shots).join("after-03-duplicate-offer.png"));
    }
    click_id(&mut again, "dup-open");
    wait_until(&mut again, "the shelf copy", |app| !reader_add_shown(app) && view(app)["text"].as_str().unwrap_or("").contains("Copy only."));
    again.close();
    assert_eq!(fs::read_dir(data.join("library")).unwrap().count(), 1, "no second copy");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn an_old_shelf_is_copied_in_on_launch_and_nothing_outside_changes() {
    let dir = dir_for("migrate");
    let data = dir.join("data");
    let large = writable_copy(&largest_real_book(), &dir.join("large.epub"));
    let small = copy_of("toc-nav-only", &dir, "small.epub");
    let gone = copy_of("toc-ncx-only", &dir, "gone.epub");
    let core = jadeslip_core::Library::open(jadeslip_core::LibraryConfig { data_dir: data.clone() }).unwrap();
    let key = |path: &Path| jadeslip_core::key_hex(core.key_of_file(path).unwrap());
    let (large_key, small_key, gone_key) = (key(&large), key(&small), key(&gone));
    drop(core);
    fs::remove_file(&gone).unwrap();
    let place = json!({"href": "missing.xhtml", "point": null, "chapter_label": "Chapter 9", "ordinal": 9, "progress": 0.5, "passage": "Where the owner stopped."});
    let record = |path: &Path, title: &str, read: Value| json!({
        "path": path, "title": title, "author": null, "inflated_spine_bytes": 1000, "linear_chapters": 3,
        "own": false, "read_point": read, "write_point": null, "added_at": 1, "touched_at": 2,
    });
    let records = json!({
        large_key.clone(): record(&large, "Large", place.clone()),
        small_key.clone(): record(&small, "Toc Nav Only", Value::Null),
        gone_key.clone(): record(&gone, "Toc Ncx Only", Value::Null),
    });
    fs::create_dir_all(&data).unwrap();
    let old = serde_json::to_vec(&json!({"version": 1, "books": records})).unwrap();
    fs::write(data.join("library.json"), &old).unwrap();
    let stamp = |path: &Path| (fs::read(path).unwrap(), fs::metadata(path).unwrap().modified().unwrap());
    let before = [stamp(&large), stamp(&small)];

    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    install(&mut app);
    let notice = "document.getElementById('shelf-moving').hidden ? '' : document.getElementById('shelf-moving-text').textContent";
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let text = app.js(notice).as_str().unwrap_or("").to_owned();
        if !text.is_empty() && seen.last() != Some(&text) {
            seen.push(text.clone());
        }
        if text.starts_with("Jadeslip now keeps") {
            break;
        }
        assert!(Instant::now() < deadline, "the move never finished: {seen:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
    println!("migration notices: {seen:?}");
    assert_eq!(
        seen.last().unwrap(),
        "Jadeslip now keeps its own copy of 2 books on your shelf, so edits change that copy. The files you added them from were not changed."
    );
    if let Ok(shots) = std::env::var("JADESLIP_SHOTS_DIR") {
        app.snapshot_png(&Path::new(&shots).join("after-04-migration-notice.png"));
    }
    assert_eq!(books(&mut app).len(), 3);
    assert_eq!(fs::read(data.join("library.json.pre-managed")).unwrap(), old, "the old shelf is kept as it was");
    for ((path, title), (bytes, modified)) in [(&large, "Large"), (&small, "Toc Nav Only")].iter().zip(&before) {
        let copy = copy_titled(&data, title);
        assert!(copy.starts_with(data.join("library")), "{title} at {}", copy.display());
        assert_eq!(&fs::read(&copy).unwrap(), bytes);
        assert_eq!(&stamp(path), &(bytes.clone(), *modified), "{} was not touched", path.display());
    }
    assert_eq!(copy_titled(&data, "Toc Ncx Only"), gone, "a missing file stays a missing record");
    let large_record = shelf_records(&data).into_iter().find(|record| record["title"] == json!("Large")).unwrap();
    assert_eq!(large_record["read_point"]["passage"], json!("Where the owner stopped."));
    assert_eq!(large_record["touched_at"], json!(2), "moving a book does not count as opening it");
    app.close();

    let paths_of = || -> Vec<Value> { shelf_records(&data).into_iter().map(|record| record["path"].clone()).collect() };
    let paths_now = paths_of();
    let mut later = App::launch_with_data_opt(None, &data);
    later.wait_first_text();
    install(&mut later);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(later.js(notice), json!(""), "a second launch has nothing to move and says nothing");
    later.close();
    assert_eq!(paths_of(), paths_now, "a second launch moves nothing");
    assert_eq!(fs::read_dir(data.join("library")).unwrap().count(), 2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "real app: tests/e2e/headless.sh"]
fn time_adding_the_largest_real_book() {
    let dir = dir_for("add-time");
    let data = dir.join("data");
    let biggest = jadeslip_e2e::real_books()
        .into_iter()
        .max_by_key(|path| fs::metadata(path).unwrap().len())
        .unwrap();
    let large = writable_copy(&biggest, &dir.join("large.epub"));
    let bytes = fs::metadata(&large).unwrap().len();
    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    install(&mut app);
    let frames = "(()=>{window.__gaps=[];let last=performance.now();const tick=(now)=>{window.__gaps.push(now-last);last=now;if(window.__gaps.length<2000)requestAnimationFrame(tick)};requestAnimationFrame(tick);return true})()";
    app.js(frames);
    let started = Instant::now();
    app.op(&json!({"op": "drop_paths", "paths": [large]}));
    wait_until(&mut app, "the book added", |app| books(app).len() == 1);
    let ms = started.elapsed().as_secs_f64() * 1e3;
    let worst = app.js("Math.max(...window.__gaps.slice(1))").as_f64().unwrap();
    println!("added {:.1} MB in {ms:.0} ms, longest frame while adding {worst:.0} ms", bytes as f64 / 1e6);
    assert!(worst < 100.0, "the window kept drawing while the book was copied: {worst:.0} ms");
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "screenshots: set JADESLIP_SHOTS_DIR, and JADESLIP_SHOTS_BINARY with JADESLIP_SHOTS_PREFIX=before for an older build"]
fn managed_library_screenshots() {
    let Ok(shots) = std::env::var("JADESLIP_SHOTS_DIR") else {
        return;
    };
    let shots = PathBuf::from(shots);
    fs::create_dir_all(&shots).unwrap();
    let binary = std::env::var_os("JADESLIP_SHOTS_BINARY").map_or_else(jadeslip_e2e::e2e_binary, PathBuf::from);
    let prefix = std::env::var("JADESLIP_SHOTS_PREFIX").unwrap_or_else(|_| "after".to_owned());
    let after = prefix == "after";
    let shot = |app: &mut App, name: &str| {
        std::thread::sleep(Duration::from_millis(250));
        app.snapshot_png(&shots.join(format!("{prefix}-{name}.png")));
    };
    let dir = dir_for(&format!("shots-{prefix}"));
    let data = dir.join("data");
    let large = writable_copy(&largest_real_book(), &dir.join("Monsoon.epub"));
    let small = copy_of("toc-nav-only", &dir, "small.epub");
    let covered = copy_of("cover-epub3", &dir, "covered.epub");
    let loose = copy_of("toc-ncx-only", &dir, "loose.epub");
    let core = jadeslip_core::Library::open(jadeslip_core::LibraryConfig { data_dir: data.clone() }).unwrap();
    let records: serde_json::Map<String, Value> = [(&large, 3), (&small, 1), (&covered, 2)]
        .iter()
        .map(|(path, at)| {
            let key = core.key_of_file(path).unwrap();
            let reader = jadeslip_core::Reader::open(fs::File::open(path).unwrap(), jadeslip_core::Limits::default()).unwrap();
            let summary = reader.summary();
            (jadeslip_core::key_hex(key), json!({
                "path": path, "title": summary.title, "author": summary.author,
                "inflated_spine_bytes": summary.inflated_spine_bytes, "linear_chapters": summary.linear_chapters,
                "own": false, "read_point": null, "write_point": null, "added_at": at, "touched_at": at,
            }))
        })
        .collect();
    drop(core);
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("library.json"), serde_json::to_vec(&json!({"version": 1, "books": records})).unwrap()).unwrap();

    let mut app = App::launch_binary(&binary, None, &data);
    app.wait_first_text();
    install(&mut app);
    if after {
        wait_until(&mut app, "the move notice", |app| {
            app.js("document.getElementById('shelf-moving-text').textContent").as_str().unwrap_or("").starts_with("Jadeslip now keeps")
        });
    }
    shot(&mut app, "01-library");
    menu_for(&mut app, "Toc Nav Only");
    click_id(&mut app, "menu-remove");
    wait_until(&mut app, "the remove dialog", |app| view(app)["removeOpen"] == json!(true));
    shot(&mut app, "02-remove-dialog");
    click_id(&mut app, "remove-cancel");
    if after {
        app.op(&json!({"op": "drop_paths", "paths": [small]}));
        wait_until(&mut app, "the duplicate offer", |app| app.js("!document.getElementById('dup-dialog').hidden") == json!(true));
        shot(&mut app, "05-duplicate-on-the-shelf");
        click_id(&mut app, "dup-cancel");
    }
    app.close();

    let mut reader = App::launch_binary(&binary, Some(&loose), &data);
    reader.wait_first_text();
    install(&mut reader);
    wait_reader(&mut reader);
    shot(&mut reader, "03-reader-opened-in-place");
    reader.close();

    if after {
        let twin = copy_of("cover-epub3", &dir, "twin.epub");
        let mut again = App::launch_binary(&binary, Some(&twin), &data);
        again.wait_first_text();
        install(&mut again);
        wait_reader(&mut again);
        click_id(&mut again, "add-to-shelf");
        wait_until(&mut again, "the duplicate offer", |app| app.js("!document.getElementById('dup-dialog').hidden") == json!(true));
        shot(&mut again, "04-duplicate-from-the-reader");
        click_id(&mut again, "dup-cancel");
        click_id(&mut again, "go-library");
        wait_shelf(&mut again);
        again.close();
        let mut add = App::launch_binary(&binary, Some(&loose), &data);
        add.wait_first_text();
        install(&mut add);
        wait_reader(&mut add);
        click_id(&mut add, "add-to-shelf");
        wait_toast(&mut add, "Added to your shelf.");
        shot(&mut add, "06-added-from-the-reader");
        add.close();
    }
    println!("{prefix} shots in {}", shots.display());
    let _ = fs::remove_dir_all(&dir);
}
