use serde_json::Value;
use jadeslip_e2e::{
    App, KEY_LEFT, KEY_RIGHT, generated_book, headless_output, largest_real_book,
    plain_release_binary, repo_root, scratch_dir, window_monitor_of,
};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const OPEN_STAGE_BOMBS: &[&str] = &[
    "bomb-book-total",
    "bomb-deep-xml-opf",
    "bomb-huge-entry-count",
    "bomb-lying-extent",
    "bomb-overlap-nested",
    "bomb-overlap-shared",
    "bomb-traversal-absolute",
    "bomb-traversal-backslash",
    "bomb-traversal-container",
    "bomb-traversal-dotdot",
    "bomb-traversal-nul",
    "bomb-traversal-opf-href",
];

const NOTICE: &str = "(()=>{const n=document.getElementById('notice');return {hidden:n.hidden,error:n.classList.contains('error'),text:n.textContent}})()";
const CARD: &str = "(()=>{const c=document.getElementById('open-failure');return {hidden:c.hidden,title:document.getElementById('open-failure-title').textContent,text:document.getElementById('open-failure-text').textContent}})()";
const WHERE: &str = "document.getElementById('where-title').title";
const BOOK_TEXT: &str =
    "(document.getElementById('book-host').shadowRoot?.textContent ?? '').length";

fn launch_ready(book: Option<&Path>) -> App {
    let mut app = App::launch(book, &[]);
    app.wait_first_text();
    app.assert_on_headless_output();
    app
}

fn wait_for_where(app: &mut App, previous: &Value) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let now = app.js(WHERE);
        if &now != previous {
            return now;
        }
        assert!(Instant::now() < deadline, "the chapter did not change");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh shell"]
fn a_book_on_the_command_line_shows_its_first_chapter() {
    let mut app = launch_ready(Some(&generated_book("large-h1-div-br")));
    assert!(app.js(BOOK_TEXT).as_u64().unwrap() > 1000);
    assert_eq!(
        app.js("document.getElementById('crumb').textContent.length>0"),
        true
    );
    assert_eq!(app.js("typeof window.__TAURI__"), "undefined");
    let host_contain = app.js("getComputedStyle(document.getElementById('scroller')).contain");
    assert!(
        host_contain.as_str().unwrap().contains("paint"),
        "{host_contain}"
    );
    assert_eq!(
        app.js("document.getElementById('book-host').shadowRoot.querySelector('.ss-book').firstElementChild.namespaceURI"),
        "http://www.w3.org/1999/xhtml"
    );
    let ink = app.snapshot_ink();
    assert!(
        ink["ink"].as_u64().unwrap_or(0) > 500,
        "the view has no text pixels: {ink}"
    );
    println!("webkit snapshot {ink}");
    app.snapshot_png(&scratch_dir().join("generated-first-text.png"));
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh shell"]
fn next_and_previous_move_by_reading_order_from_buttons_and_keys() {
    let mut app = launch_ready(Some(&generated_book("large-h1-p")));
    let first = app.js(WHERE);
    let next_box = app.js("(()=>{const b=document.getElementById('next').getBoundingClientRect();return {x:b.x+b.width/2,y:b.y+b.height/2}})()");
    app.click(
        next_box["x"].as_f64().unwrap(),
        next_box["y"].as_f64().unwrap(),
    );
    let second = wait_for_where(&mut app, &first);
    app.key(KEY_RIGHT);
    let third = wait_for_where(&mut app, &second);
    app.key(KEY_LEFT);
    let back = wait_for_where(&mut app, &third);
    assert_eq!(back, second);
    println!("moved through {first} {second} {third}");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh shell"]
fn every_failure_is_a_readable_message_and_no_book_is_a_calm_page() {
    let mut app = launch_ready(None);
    assert_eq!(
        app.js("!document.getElementById('library').hidden"),
        serde_json::json!(true),
        "no book opens on the shelf"
    );
    assert_eq!(app.js(NOTICE)["hidden"], true);
    app.close();

    let missing = scratch_dir().join("no-such-book.epub");
    let mut books = vec![("missing file".to_owned(), missing)];
    for name in OPEN_STAGE_BOMBS {
        books.push(((*name).to_owned(), generated_book(name)));
    }
    for (label, book) in &books {
        let mut app = launch_ready(Some(book));
        let card = app.js(CARD);
        assert_eq!(card["hidden"], false, "{label}");
        let text = card["text"].as_str().unwrap();
        assert!(text.ends_with('.') && text.len() > 12, "{label}: {text}");
        assert!(
            text.contains(&book.display().to_string()),
            "{label}: the sentence names the path: {text}"
        );
        assert!(
            !text.contains("panic") && !text.contains("unwrap") && !text.contains("os error"),
            "{label}: {text}"
        );
        println!("{label}: {text}");
        app.close();
    }

    let mut app = launch_ready(Some(&generated_book("malformed-missing-chapter")));
    let rows = app.js("(async()=>{const m=await import('/main.js');const out=[];for(let ix=0;ix<8;ix++){await m.goTo(ix);const n=document.getElementById('notice');out.push(!n.hidden&&n.classList.contains('error'))}return out})()");
    assert!(
        rows.as_array().unwrap().iter().any(|failed| failed == true),
        "{rows}"
    );
    let notice = app.js(NOTICE);
    println!("missing chapter: {}", notice["text"]);
    assert_eq!(app.js("document.getElementById('next').disabled"), false);
    assert_eq!(app.js("document.getElementById('prev').disabled"), false);
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh shell"]
fn a_chapter_past_the_display_limit_is_refused_with_a_message() {
    let started = Instant::now();
    let mut app = launch_ready(Some(&generated_book("bomb-ratio")));
    let notice = app.js(NOTICE);
    println!(
        "bomb-ratio after {:?}: {}",
        started.elapsed(),
        notice["text"]
    );
    assert_eq!(notice["error"], true);
    assert!(
        notice["text"]
            .as_str()
            .unwrap()
            .contains("too large to display")
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the refusal was quick"
    );
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh shell"]
fn the_xml_mount_has_no_text_node_limit_up_to_the_display_limit() {
    let mut app = launch_ready(Some(&generated_book("images-mixed")));
    let result = app.js("(async()=>{const v=await import('/chapter-view.js');const host=document.getElementById('book-host');const units=16*1048576;const xhtml='<div xmlns=\"http://www.w3.org/1999/xhtml\" data-ss-b=\"0\">'+'aaaaaaa '.repeat(units/8)+'</div>';const t=performance.now();v.mountChapter(host,{xhtml,css:[]});return {units,mounted:host.shadowRoot.textContent.length,ms:Math.round(performance.now()-t)}})()");
    println!("16 MiB text node: {result}");
    assert_eq!(result["mounted"], result["units"]);
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh shell"]
fn a_shadow_root_gives_ids_no_named_access_on_window() {
    let mut app = launch_ready(Some(&generated_book("attack")));
    let result = app.js("(async()=>{const m=await import('/main.js');for(let ix=0;ix<160;ix++){await m.goTo(ix);const r=document.getElementById('book-host').shadowRoot;if(r.getElementById('__TAURI_INTERNALS__')){return {found:true,ix,internals:typeof window.__TAURI_INTERNALS__.invoke,global:typeof window.__TAURI__,direct:window.__TAURI_INTERNALS__ instanceof Element,ssRoot:'ss-root' in window,byIdInShadow:r.getElementById('__TAURI_INTERNALS__').localName}}}return {found:false}})()");
    println!("forge_id_clobber {result}");
    assert_eq!(
        result["found"], true,
        "the clobbering id survived to the DOM, so the check ran"
    );
    assert_eq!(result["internals"], "function");
    assert_eq!(result["global"], "undefined");
    assert_eq!(result["direct"], false);
    assert_eq!(result["ssRoot"], false);
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh shell"]
fn the_plain_release_build_opens_a_book_and_ignores_every_test_switch() {
    let binary = plain_release_binary();
    let bytes = std::fs::read(&binary).expect("build it first: tests/e2e/run.sh");
    for marker in [
        "JADESLIP_E2E",
        "ctl_reply",
        "jadeslip-test",
        "pwned",
        "nav_guard",
        "arm_save_answer",
        "take_save_answer",
    ] {
        let found = bytes
            .windows(marker.len())
            .any(|window| window == marker.as_bytes());
        assert!(!found, "the release build contains {marker}");
    }
    let renamed = scratch_dir().join("plain").join("jadeslip-test");
    std::fs::create_dir_all(renamed.parent().unwrap()).expect("scratch folder");
    std::fs::copy(&binary, &renamed).expect("the plain build copies under a test window class");
    let socket = scratch_dir().join("release-must-not-open-this.sock");
    let _ = std::fs::remove_file(&socket);
    let book = largest_real_book();
    let mut child = Command::new(&renamed)
        .arg(&book)
        .env("JADESLIP_E2E_SOCKET", &socket)
        .env("JADESLIP_E2E_OFF", "csp,sanitizer,nav_guard")
        .env("JADESLIP_DATA_DIR", scratch_dir().join("release-data"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("release build starts");
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(10);
    let monitor = loop {
        if let Some(monitor) = window_monitor_of(pid) {
            break monitor;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the release build opened no window");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    std::thread::sleep(Duration::from_millis(1500));
    let alive = child.try_wait().expect("child status").is_none();
    let opened_book = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .map(|fds| {
            fds.flatten()
                .filter_map(|fd| std::fs::read_link(fd.path()).ok())
                .any(|target| {
                    target == repo_root().join(&book) || target.ends_with(book.file_name().unwrap())
                })
        })
        .unwrap_or(false);
    let memory = jadeslip_e2e::memory_of_tree(pid);
    let socket_exists = socket.exists();
    let _ = Command::new("kill").arg(pid.to_string()).status();
    let _ = child.wait();
    println!(
        "release build: {} bytes, monitor {monitor}, opened book {opened_book}, processes {}, RSS {} MB, PSS {} MB",
        bytes.len(),
        memory.processes,
        memory.rss_kb / 1024,
        memory.pss_kb / 1024
    );
    assert_eq!(monitor, headless_output());
    assert!(alive, "the release build stayed up");
    assert!(opened_book, "the release build holds the book open");
    assert!(!socket_exists, "the release build has no control channel");
}
