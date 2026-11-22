use jadeslip_e2e::{App, KEY_ENTER, KEY_LEFT, MASK_CTRL, largest_real_book};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const KEY_K: u32 = 0x6b;
const KEY_F: u32 = 0x66;
const MASK_ALT: u32 = 8;

const INSTALL: &str = r"(()=>{
  const element = (id) => document.getElementById(id);
  const inside = (node, frame) => {
    if (!node) return false;
    const a = node.getBoundingClientRect();
    const b = frame.getBoundingClientRect();
    return a.top >= b.top && a.bottom <= b.bottom;
  };
  window.__nav = () => {
    const current = CSS.highlights.get('ss-search-current');
    const range = current?.size ? [...current.values()][0] : null;
    const scroller = element('scroller');
    const page = scroller.getBoundingClientRect();
    const cur = document.querySelector('#toc-spacer .row.cur');
    return {
      title: element('where-title').textContent,
      where: element('where-title').title,
      back: element('back').hidden ? '' : element('back-label').textContent,
      rail: cur?.querySelector('.t').textContent ?? '',
      railInView: inside(cur, element('toc-list')),
      railTop: element('toc-list').scrollTop,
      typed: element('palette-input').value,
      palette: [...document.querySelectorAll('#palette-rows .palrow')].map((row) => [row.querySelector('.pallabel').textContent, row.querySelector('.palhint').textContent]),
      footer: element('search-count').textContent,
      matched: range ? range.toString() : '',
      hitTop: range ? range.getBoundingClientRect().top - page.top + scroller.scrollTop : null,
      scrollTop: scroller.scrollTop,
      room: scroller.scrollHeight - scroller.clientHeight,
    };
  };
  return true;
})()";

fn settle(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn nav(app: &mut App) -> Value {
    app.js("window.__nav()")
}

fn wait_nav(app: &mut App, what: &str, test: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let now = nav(app);
        if test(&now) {
            return now;
        }
        assert!(Instant::now() < deadline, "{what} never happened: {now}");
        settle(20);
    }
}

fn ready(app: &mut App) {
    app.wait_first_text();
    app.assert_on_headless_output();
    app.js(INSTALL);
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.js("document.getElementById('count').textContent.includes('entries')") != json!(true) {
        assert!(Instant::now() < deadline, "the contents never loaded");
        settle(20);
    }
}

fn chapter_of(title: &str) -> Option<u32> {
    title
        .strip_prefix("Chapter ")?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn jump(app: &mut App, typed: &str) -> Value {
    app.key_with(KEY_K, MASK_CTRL);
    settle(60);
    app.type_text(typed);
    let offered = wait_nav(app, "the palette offering rows", |now| {
        now["typed"] == typed && now["palette"].as_array().is_some_and(|rows| !rows.is_empty())
    });
    app.key(KEY_ENTER);
    offered
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh navigation"]
fn jump_lands_on_the_chapter_the_title_numbers_and_alt_left_comes_back() {
    let mut app = App::launch(Some(&largest_real_book()), &[]);
    ready(&mut app);
    let start = nav(&mut app);
    assert_eq!(start["back"], "", "no way back before any jump");

    let offered = jump(&mut app, "1818");
    let first = &offered["palette"][0];
    assert!(
        first[0].as_str().unwrap().starts_with("Chapter 1818"),
        "the title carrying 1818 comes first: {offered}"
    );
    let hint = first[1].as_str().unwrap();
    assert!(hint.starts_with("Position ") && hint != "Position 1,818", "the position is only a hint: {hint}");

    let landed = wait_nav(&mut app, "the jump", |now| {
        now["title"].as_str().is_some_and(|title| title.starts_with("Chapter 1818"))
    });
    assert_eq!(landed["where"], format!("{hint} of 6,191").as_str(), "the footer keeps the position on hover");
    assert_eq!(landed["rail"], landed["title"], "the contents follow the jump");
    assert_eq!(landed["railInView"], true);
    let back = landed["back"].as_str().unwrap();
    assert!(back.starts_with("Back to "), "a way back after the jump: {landed}");

    app.key_with(KEY_LEFT, MASK_ALT);
    let returned = wait_nav(&mut app, "Alt+Left", |now| now["title"] == start["title"]);
    assert_eq!(returned["back"], "", "the chip goes once used");
    assert_eq!(returned["where"], start["where"]);

    jump(&mut app, "1818");
    wait_nav(&mut app, "the jump again", |now| now["railTop"].as_f64().is_some_and(|top| top > 1000.0));
    app.js("(async()=>{await (await import('/main.js')).goTo(1);return true})()");
    wait_nav(&mut app, "the page before the contents", |now| {
        now["title"].as_str().is_some_and(|title| !title.starts_with("Chapter "))
    });
    let cover = wait_nav(&mut app, "the contents following to the top", |now| now["railTop"] == 0);
    assert_eq!(cover["rail"], "", "no entry claims a page the contents do not list");
    assert_eq!(cover["back"], "", "reading elsewhere drops the chip");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh navigation"]
fn enter_in_search_steps_from_the_reading_place_and_the_chip_comes_back() {
    let mut app = App::launch(Some(&largest_real_book()), &[]);
    ready(&mut app);
    jump(&mut app, "1818");
    let deep = wait_nav(&mut app, "chapter 1818", |now| {
        now["title"].as_str().is_some_and(|title| title.starts_with("Chapter 1818"))
    });
    app.key_with(KEY_F, MASK_CTRL);
    settle(300);
    let room = nav(&mut app)["room"].as_f64().unwrap();
    assert!(room > 400.0, "the chapter scrolls: {deep}");
    app.js(&format!("(document.getElementById('scroller').scrollTop={}, true)", (room / 2.0).round()));
    settle(400);
    let read_to = nav(&mut app)["scrollTop"].as_f64().unwrap();
    println!("read to {read_to} of {room}");

    app.type_text("said");
    app.key(KEY_ENTER);
    let found = wait_nav(&mut app, "the first step", |now| {
        now["footer"].as_str().is_some_and(|text| text.starts_with("Match ")) && now["matched"] != ""
    });
    println!("stepped to: {found}");
    assert!(!found["footer"].as_str().unwrap().starts_with("Match 1 of"), "not the first match in the book: {found}");
    let chapter = chapter_of(found["title"].as_str().unwrap()).expect("a numbered chapter");
    assert!(chapter >= 1818, "the step stays at or after the reading place: {found}");
    if chapter == 1818 {
        assert!(found["hitTop"].as_f64().unwrap() >= read_to - 1.0, "the match is below the place read to: {found}");
    }
    let found = wait_nav(&mut app, "the contents following the step", |now| now["rail"] == now["title"]);
    assert_eq!(found["railInView"], true);
    assert_eq!(found["back"], "Back to Chapter 1818");

    app.key(KEY_ENTER);
    let second = wait_nav(&mut app, "the second step", |now| now["footer"] != found["footer"] && now["matched"] != "");
    assert_eq!(second["back"], "Back to Chapter 1818", "stepping on keeps the first place");

    app.key_with(KEY_LEFT, MASK_ALT);
    println!("second step: {second}");
    let returned = wait_nav(&mut app, "Alt+Left", |now| {
        now["back"] == "" && now["title"].as_str().is_some_and(|title| title.starts_with("Chapter 1818"))
    });
    let top = returned["scrollTop"].as_f64().unwrap();
    assert!((top - read_to).abs() < 40.0, "back to the place read to, {read_to}: {returned}");
    app.close();
}
