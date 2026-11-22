use jadeslip_core::{DEFAULT_LIMITS, Reader};
use jadeslip_e2e::{
    App, place_name, KEY_ENTER, KEY_ESCAPE, KEY_LEFT, KEY_PAGE_DOWN, KEY_RIGHT, KEY_SPACE, MASK_CTRL, MASK_SHIFT, book_label,
    largest_real_book, real_books, scratch_dir,
};
use serde_json::{Value, json};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const KEY_E: u32 = 0x65;
const KEY_PAGE_UP: u32 = 0xff55;

const INSTALL: &str = r"(async()=>{
  window.__main = await import('/main.js');
  window.__map = await import('/textmap.js');
  const element = (id) => document.getElementById(id);
  const host = element('book-host');
  const body = () => host.shadowRoot.querySelector('.ss-body');
  window.__area = () => {
    const sheet = document.querySelector('.sheet');
    const box = sheet.getBoundingClientRect();
    const style = getComputedStyle(sheet);
    const view = element('scroller').getBoundingClientRect();
    return {
      left: box.left + parseFloat(style.paddingLeft),
      right: box.right - parseFloat(style.paddingRight),
      top: Math.max(view.top, box.top + parseFloat(style.paddingTop)),
      bottom: Math.min(view.bottom, box.bottom - parseFloat(style.paddingBottom)),
      gap: parseFloat(getComputedStyle(host).columnGap) || 0,
      viewLeft: view.left, viewRight: view.right, viewTop: view.top, viewBottom: view.bottom,
    };
  };
  window.__pg = () => ({
    paged: document.querySelector('.app').classList.contains('paged'),
    two: document.querySelector('.app').classList.contains('two-pages'),
    label: element('page-where').hidden ? '' : element('page-where').textContent,
    where: element('where-title').title,
    mode: element('mode-edit').classList.contains('on') ? 'edit' : 'read',
    font: getComputedStyle(host).fontSize,
    contents: getComputedStyle(element('contents-rail')).display !== 'none',
    toast: element('toast').hidden ? '' : element('toast').textContent,
    top: element('scroller').scrollTop,
  });
  window.__textAt = (x, from, to) => {
    for (let y = from + 1; y < to; y += 3) {
      const found = document.caretPositionFromPoint(x, y, { shadowRoots: [host.shadowRoot] });
      if (!found || !host.shadowRoot.contains(found.offsetNode) || found.offsetNode.nodeType !== Node.TEXT_NODE) continue;
      if (!/\S/.test(found.offsetNode.data.slice(found.offset, found.offset + 1))) continue;
      const block = found.offsetNode.parentElement.closest('[data-ss-b]');
      if (!block) continue;
      return { block: Number(block.getAttribute('data-ss-b')), units: window.__map.unitsBefore(block, found.offsetNode, found.offset) };
    }
    return null;
  };
  window.__first = () => { const a = window.__area(); return window.__textAt(a.left + 2, a.top, a.bottom); };
  window.__rectOf = (point) => {
    const block = body().querySelector(`[data-ss-b='${point.block}']`);
    const at = window.__map.pointAt(block, point.units);
    const range = document.createRange();
    range.setStart(at.node, at.offset);
    range.setEnd(at.node, Math.min(at.offset + 1, at.node.length ?? at.offset));
    const box = range.getClientRects()[0] ?? range.getBoundingClientRect();
    return { left: box.left, right: box.right, top: box.top, bottom: box.bottom };
  };
  window.__seen = (point) => {
    const box = window.__rectOf(point);
    const a = window.__area();
    return box.left >= a.left - 1 && box.right <= a.right + 1 && box.top >= a.top - 1 && box.bottom <= a.bottom + 1;
  };
  window.__caret = () => {
    const [range] = document.getSelection().getComposedRanges({ shadowRoots: [host.shadowRoot] });
    if (!range) return null;
    const block = (range.startContainer.nodeType === Node.ELEMENT_NODE ? range.startContainer : range.startContainer.parentElement).closest('[data-ss-b]');
    return block ? { block: Number(block.getAttribute('data-ss-b')), units: window.__map.unitsBefore(block, range.startContainer, range.startOffset) } : null;
  };
  window.__centre = (id) => {
    const box = element(id).getBoundingClientRect();
    return { x: box.left + box.width / 2, y: box.top + box.height / 2, bottom: box.bottom, height: window.innerHeight };
  };
  window.__click = (id) => { element(id).click(); return true; };
  window.__slide = (id, value) => {
    const input = element(id);
    input.value = value;
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new Event('change'));
    return true;
  };
  return true;
})()";

fn settle(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn wait_until(app: &mut App, script: &str, what: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let value = app.js(script);
        if value != Value::Null && value != json!(false) {
            return value;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        settle(10);
    }
}

fn ready(app: &mut App) {
    app.wait_first_text();
    app.assert_on_headless_output();
    app.js(INSTALL);
    wait_until(
        app,
        "document.getElementById('count').textContent.includes('entries')",
        "the contents loading",
    );
}

fn pg(app: &mut App) -> Value {
    app.js("window.__pg()")
}

fn wait_pg(app: &mut App, test: &str, what: &str) -> Value {
    wait_until(
        app,
        &format!("(()=>{{const v=window.__pg();return ({test})?v:false}})()"),
        what,
    )
}

fn wait_label(app: &mut App, wanted: &str) -> Value {
    wait_pg(app, &format!("v.label==={}", json!(wanted)), wanted)
}

fn label_total(value: &Value) -> u64 {
    value["label"]
        .as_str()
        .and_then(|label| label.rsplit(' ').next())
        .and_then(|total| total.replace(',', "").parse().ok())
        .unwrap_or_else(|| panic!("no page total in {value}"))
}

fn go(app: &mut App, ix: u64) {
    app.js(&format!("window.__main.goTo({ix}).then(()=>true)"));
}

fn click_at(app: &mut App, id: &str) {
    let at = app.js(&format!("window.__centre({})", json!(id)));
    app.click(at["x"].as_f64().unwrap(), at["y"].as_f64().unwrap());
}

fn web_novel() -> PathBuf {
    real_books()
        .into_iter()
        .find(|path| book_label(path) == "serial_b1")
        .expect("the web novel fixture")
}

fn long_chapters(book: &Path) -> Vec<u64> {
    let reader = Reader::open(File::open(book).unwrap(), DEFAULT_LIMITS).unwrap();
    reader
        .rows()
        .iter()
        .filter(|row| {
            row.linear_ordinal.is_some() && row.rev.is_some_and(|rev| rev.inflated_len > 12_000)
        })
        .map(|row| u64::from(row.ix.0))
        .collect()
}

fn following_long_pair(book: &Path) -> u64 {
    let long = long_chapters(book);
    long.windows(2)
        .find(|pair| pair[1] == pair[0] + 1 && pair[0] > 4)
        .map(|pair| pair[1])
        .expect("two long chapters in a row")
}

fn largest_item_of(book: &Path) -> u64 {
    let reader = Reader::open(File::open(book).unwrap(), DEFAULT_LIMITS).unwrap();
    reader
        .rows()
        .iter()
        .max_by_key(|row| row.rev.map_or(0, |rev| rev.inflated_len))
        .map_or(0, |row| u64::from(row.ix.0))
}

fn fresh_data(name: &str) -> PathBuf {
    let dir = scratch_dir().join(format!("pages-data-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn wait_file(path: &Path, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(path).is_ok_and(|text| text.contains(needle)) {
        assert!(
            Instant::now() < deadline,
            "{} never held {needle}",
            path.display()
        );
        settle(20);
    }
}

fn auto_one_page(app: &mut App) {
    app.js("window.__click('prefs-pages-auto')");
    wait_pg(app, "v.paged && !v.two && v.contents && v.font === '18px'", "Auto as one page beside the contents");
}

fn font_down_to_15(app: &mut App) {
    for _ in 0..3 {
        app.js("window.__click('font-smaller')");
    }
    wait_pg(app, "v.font === '15px'", "15 px text");
}

fn first(app: &mut App) -> Value {
    let point = app.js("window.__first()");
    assert!(!point.is_null(), "text at the top of the page");
    point
}

fn seen(app: &mut App, point: &Value) -> bool {
    app.js(&format!("window.__seen({point})")) == json!(true)
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn page_mode_is_chosen_in_the_panel_and_comes_back_after_a_relaunch() {
    let book = web_novel();
    let data = fresh_data("persist");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    assert_eq!(pg(&mut app)["paged"], false, "scrolling is the default");
    click_at(&mut app, "toggle-comfort");
    wait_until(
        &mut app,
        "!document.getElementById('comfort').hidden",
        "the panel opening",
    );
    for tab in ["text", "layout"] {
        click_at(&mut app, &format!("comfort-tab-{tab}"));
        wait_until(
            &mut app,
            &format!("!document.getElementById('comfort-{tab}').hidden"),
            "the tab showing",
        );
        let fits = app.js(&format!(
            "(()=>{{const pop=document.getElementById('comfort');const panel=document.getElementById('comfort-{tab}');const box=pop.getBoundingClientRect();const controls=[...panel.querySelectorAll('button,input,select')].filter((node)=>node.getClientRects().length>0);const low=Math.max(...controls.map((node)=>node.getBoundingClientRect().bottom));return {{shown:!panel.hidden,controls:controls.length,scroll:pop.scrollHeight,client:pop.clientHeight,low,bottom:box.bottom,window:window.innerHeight}}}})()"
        ));
        println!("{tab} tab: {fits}");
        assert!(
            fits["shown"] == true
                && fits["controls"].as_u64().unwrap() > 0
                && fits["scroll"].as_f64() <= fits["client"].as_f64()
                && fits["low"].as_f64() <= fits["bottom"].as_f64()
                && fits["low"].as_f64() <= fits["window"].as_f64(),
            "every control on the {tab} tab shows without scrolling: {fits}"
        );
    }
    app.key(KEY_ESCAPE);
    wait_until(
        &mut app,
        "document.getElementById('comfort').hidden",
        "the panel closing",
    );
    click_at(&mut app, "reader-more");
    wait_until(
        &mut app,
        "document.getElementById('open-prefs').getClientRects().length > 0",
        "the reader menu opening",
    );
    click_at(&mut app, "open-prefs");
    wait_until(
        &mut app,
        "!document.getElementById('prefs-dialog').hidden && !document.getElementById('prefs-section-general').hidden",
        "Preferences opening on General",
    );
    click_at(&mut app, "prefs-pages-auto");
    let shown = wait_pg(
        &mut app,
        "v.paged && v.label.startsWith('Page 1 of ')",
        "Auto pages",
    );
    println!("auto: {shown}");
    wait_file(&data.join("prefs.json"), "\"pages\":\"auto\"");
    app.close();

    let mut again = App::launch_with_data(&book, &data);
    ready(&mut again);
    let back = wait_pg(
        &mut again,
        "v.paged && v.label.startsWith('Page ')",
        "page mode after a relaunch",
    );
    assert_eq!(back["two"], false);
    again.js("window.__click('open-prefs')");
    assert_eq!(
        again.js("document.getElementById('prefs-pages-auto').classList.contains('on')"),
        true
    );
    again.js("window.__click('prefs-pages-scroll')");
    wait_pg(&mut again, "!v.paged && v.label === ''", "scrolling again");
    wait_file(&data.join("prefs.json"), "\"pages\":\"scroll\"");
    again.close();
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn a_saved_two_pages_choice_opens_as_auto() {
    let book = web_novel();
    let data = fresh_data("migrate");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("prefs.json"), r#"{"pages":"two","fontPx":18}"#).unwrap();
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    let shown = wait_pg(
        &mut app,
        "v.paged && !v.two && v.contents && v.label.startsWith('Page ')",
        "Auto as one page beside the contents",
    );
    let modes = app.js("[...document.querySelectorAll('[id^=prefs-pages-]')].map((b)=>b.id+(b.classList.contains('on')?'*':''))");
    println!("a saved two pages: {shown} {modes}");
    assert_eq!(modes, json!(["prefs-pages-scroll", "prefs-pages-auto*"]));
    app.js("window.__click('toggle-contents')");
    app.js("window.__click('toggle-contents')");
    wait_file(&data.join("prefs.json"), "\"pages\":\"auto\"");
    app.close();
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn keys_wheel_and_edge_clicks_turn_pages_and_cross_chapters_both_ways() {
    let book = web_novel();
    let ix = following_long_pair(&book);
    let mut app = App::launch(Some(&book), &[]);
    ready(&mut app);
    auto_one_page(&mut app);
    go(&mut app, ix);
    let start = wait_pg(
        &mut app,
        "v.label.startsWith('Page 1 of ')",
        "the chapter's first page",
    );
    let here = start["where"].clone();
    let total = label_total(&start);
    assert!(total >= 5, "a long chapter has pages: {start}");
    let page_one = first(&mut app);

    let steps: [(u32, u32, &str); 6] = [
        (KEY_RIGHT, 0, "Page 2"),
        (KEY_PAGE_DOWN, 0, "Page 3"),
        (KEY_SPACE, 0, "Page 4"),
        (KEY_LEFT, 0, "Page 3"),
        (KEY_PAGE_UP, 0, "Page 2"),
        (KEY_SPACE, MASK_SHIFT, "Page 1"),
    ];
    for (key, mask, wanted) in steps {
        app.key_with(key, mask);
        let now = wait_label(&mut app, &format!("{wanted} of {total}"));
        assert_eq!(now["where"], here, "{wanted} stays in the chapter");
        assert_eq!(now["top"], 0, "nothing scrolls in page mode");
    }
    assert_eq!(
        first(&mut app),
        page_one,
        "page 1 shows the same text again"
    );
    app.key(KEY_RIGHT);
    wait_label(&mut app, &format!("Page 2 of {total}"));
    assert!(!seen(&mut app, &page_one), "page 2 shows other text");
    app.key(KEY_LEFT);
    wait_label(&mut app, &format!("Page 1 of {total}"));

    app.key(KEY_LEFT);
    let before = wait_pg(
        &mut app,
        &format!("v.where !== {here}"),
        "the previous chapter",
    );
    let before_total = label_total(&before);
    assert!(before_total > 1, "the previous chapter has pages: {before}");
    assert_eq!(
        before["label"],
        format!("Page {before_total} of {before_total}"),
        "it opens on its last page"
    );
    app.key(KEY_RIGHT);
    let back = wait_pg(
        &mut app,
        &format!("v.where === {here}"),
        "the chapter again",
    );
    assert_eq!(
        back["label"],
        format!("Page 1 of {total}"),
        "the next chapter opens on its first page"
    );

    let area = app.js("window.__area()");
    let y = (area["top"].as_f64().unwrap() + area["bottom"].as_f64().unwrap()) / 2.0;
    let right_edge = (area["right"].as_f64().unwrap() + area["viewRight"].as_f64().unwrap()) / 2.0;
    let left_edge = (area["left"].as_f64().unwrap() + area["viewLeft"].as_f64().unwrap()) / 2.0;
    app.click(right_edge, y);
    wait_label(&mut app, &format!("Page 2 of {total}"));
    app.click(right_edge, y);
    wait_label(&mut app, &format!("Page 3 of {total}"));
    app.click(left_edge, y);
    wait_label(&mut app, &format!("Page 2 of {total}"));
    let middle = (area["left"].as_f64().unwrap() + area["right"].as_f64().unwrap()) / 2.0;
    app.click(middle, y);
    settle(300);
    assert_eq!(
        pg(&mut app)["label"],
        format!("Page 2 of {total}"),
        "a click on the text turns nothing"
    );

    app.op(&json!({"op": "wheel", "x": middle, "y": y, "steps": 1}));
    wait_label(&mut app, &format!("Page 3 of {total}"));
    settle(400);
    app.op(&json!({"op": "wheel", "x": middle, "y": y, "steps": 1}));
    wait_label(&mut app, &format!("Page 4 of {total}"));
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn auto_sets_two_pages_side_by_side_when_wide_and_one_beside_the_contents() {
    let book = web_novel();
    let ix = long_chapters(&book)[3];
    let mut app = App::launch(Some(&book), &[]);
    ready(&mut app);
    go(&mut app, ix);
    app.js("window.__click('toggle-contents')");
    app.js("window.__slide('layout-width',500)");
    app.js("window.__click('prefs-pages-auto')");
    font_down_to_15(&mut app);
    let wide = wait_pg(
        &mut app,
        "v.paged && v.two && v.label.startsWith('Pages 1-2 of ')",
        "two pages",
    );
    let area = app.js("window.__area()");
    let (left, right, top, bottom, gap) = (
        area["left"].as_f64().unwrap(),
        area["right"].as_f64().unwrap(),
        area["top"].as_f64().unwrap(),
        area["bottom"].as_f64().unwrap(),
        area["gap"].as_f64().unwrap(),
    );
    println!("two pages: {wide} area {area}");
    let column = (right - left - gap) / 2.0;
    assert!(
        (column - 500.0).abs() <= 1.0,
        "each page is the column width: {column}"
    );
    let first_page = app.js(&format!("window.__textAt({},{top},{bottom})", left + 2.0));
    let second_page = app.js(&format!(
        "window.__textAt({},{top},{bottom})",
        left + column + gap + 2.0
    ));
    assert!(
        !first_page.is_null() && !second_page.is_null(),
        "both pages hold text"
    );
    let (one, two) = (
        app.js(&format!("window.__rectOf({first_page})")),
        app.js(&format!("window.__rectOf({second_page})")),
    );
    let line = 60.0;
    assert!(
        one["top"].as_f64().unwrap() - top <= line && two["top"].as_f64().unwrap() - top <= line,
        "both pages start at the top: {one} {two}"
    );
    assert!(
        two["left"].as_f64().unwrap() - one["left"].as_f64().unwrap() >= column,
        "the second page is to the right"
    );
    let order = (
        first_page["block"].as_u64().unwrap(),
        first_page["units"].as_u64().unwrap(),
    ) < (
        second_page["block"].as_u64().unwrap(),
        second_page["units"].as_u64().unwrap(),
    );
    assert!(order, "the right page continues the left one");
    let total = label_total(&wide);
    app.key(KEY_RIGHT);
    wait_label(&mut app, &format!("Pages 2-3 of {total}"));
    let third = first(&mut app);
    assert!(!seen(&mut app, &first_page), "a turn moves the left page out");
    let moved = app.js(&format!("window.__rectOf({second_page})"));
    assert!(
        (moved["left"].as_f64().unwrap() - one["left"].as_f64().unwrap()).abs() <= 2.0,
        "the right page slides over to the left: {moved} {one}"
    );
    app.key(KEY_LEFT);
    wait_label(&mut app, &format!("Pages 1-2 of {total}"));
    assert!(seen(&mut app, &first_page), "a turn back brings the left page back");
    app.key(KEY_RIGHT);
    wait_label(&mut app, &format!("Pages 2-3 of {total}"));

    app.js("window.__click('toggle-contents')");
    let beside = wait_pg(&mut app, "v.paged && !v.two && v.contents", "one page beside the contents");
    assert!(
        beside["label"].as_str().unwrap().starts_with("Page "),
        "{beside}"
    );
    assert!(
        seen(&mut app, &third),
        "the same text stays on screen when two pages become one"
    );
    app.js("window.__click('toggle-contents')");
    let again = wait_pg(&mut app, "v.paged && v.two && !v.contents", "two pages again");
    assert!(seen(&mut app, &third), "and back: {again}");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn auto_turns_one_page_into_two_with_text_size_room_and_edit() {
    let book = web_novel();
    let data = fresh_data("auto");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    go(&mut app, long_chapters(&book)[3]);
    app.js("window.__click('toggle-contents')");
    app.js("window.__click('prefs-pages-auto')");
    let one = wait_pg(&mut app, "v.paged && !v.two && !v.contents && v.font === '18px'", "Auto at 18 px");
    wait_file(&data.join("prefs.json"), "\"pages\":\"auto\"");
    app.key(KEY_RIGHT);
    let turned = wait_pg(&mut app, "v.label.startsWith('Page 2 ')", "a page turn");
    let held = first(&mut app);
    let spot = app.js("(()=>{const b=document.getElementById('scroller').getBoundingClientRect();return {x:b.left+b.width/2,y:b.top+b.height/2}})()");
    app.op(&json!({"op": "wheel", "x": spot["x"], "y": spot["y"], "steps": 3, "state": MASK_CTRL, "up": false}));
    let two = wait_pg(&mut app, "v.paged && v.two && v.font === '15px'", "two pages at 15 px");
    assert!(seen(&mut app, &held), "Ctrl and the wheel keep the text on screen: {turned} {two}");
    app.op(&json!({"op": "wheel", "x": spot["x"], "y": spot["y"], "steps": 1, "state": MASK_CTRL, "up": true}));
    wait_pg(&mut app, "v.paged && !v.two && v.font === '16px'", "one page again at 16 px");
    assert!(seen(&mut app, &held), "and back");
    app.op(&json!({"op": "wheel", "x": spot["x"], "y": spot["y"], "steps": 1, "state": MASK_CTRL, "up": false}));
    wait_pg(&mut app, "v.paged && v.two && v.font === '15px'", "two pages at 15 px again");
    app.js("window.__click('toggle-contents')");
    wait_pg(&mut app, "v.paged && !v.two && v.contents", "one page once the contents take room");
    assert!(seen(&mut app, &held), "the text stays across the room change");
    app.js("window.__click('toggle-contents')");
    wait_pg(&mut app, "v.paged && v.two && !v.contents", "two pages when the contents go");
    app.key(KEY_E);
    let edit = wait_pg(&mut app, "v.mode === 'edit' && !v.paged", "Edit scrolls");
    app.op(&json!({"op": "wheel", "x": spot["x"], "y": spot["y"], "steps": 1, "state": MASK_CTRL, "up": false}));
    wait_pg(&mut app, "v.mode === 'edit' && !v.paged && v.font === '14px'", "Ctrl and the wheel in Edit never pages");
    app.key(KEY_ESCAPE);
    wait_pg(&mut app, "v.mode === 'read' && v.paged && v.two", "Auto pages again in Read");
    println!("auto: {one} -> {two}, edit {edit}");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn the_page_keeps_its_text_across_text_size_layout_and_a_relaunch() {
    let book = web_novel();
    let ix = long_chapters(&book)[5];
    let data = fresh_data("place");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    auto_one_page(&mut app);
    go(&mut app, ix);
    let start = wait_pg(
        &mut app,
        "v.label.startsWith('Page 1 of ')",
        "the first page",
    );
    let total = label_total(&start);
    for _ in 0..4 {
        app.key(KEY_RIGHT);
    }
    wait_label(&mut app, &format!("Page 5 of {total}"));
    let point = first(&mut app);
    for step in [
        "window.__click('font-larger')",
        "window.__click('font-larger')",
        "window.__slide('layout-width',520)",
        "window.__slide('layout-marginTop',120)",
        "window.__slide('layout-lineHeight',220)",
        "window.__click('font-smaller')",
    ] {
        app.js(step);
        let now = pg(&mut app);
        assert!(
            seen(&mut app, &point),
            "the text stayed on the page after {step}: {now}"
        );
    }
    let moved = pg(&mut app);
    println!("after the changes: {moved}");
    assert_ne!(
        moved["label"],
        format!("Page 5 of {total}"),
        "the changes repaginated"
    );
    wait_file(&data.join(place_name(&data, &book)), "\"offset_u16\"");
    settle(500);
    app.close();

    let mut again = App::launch_with_data(&book, &data);
    ready(&mut again);
    again.js("document.fonts.ready.then(()=>new Promise((done)=>requestAnimationFrame(()=>requestAnimationFrame(()=>done(true)))))");
    settle(150);
    let back = pg(&mut again);
    assert_eq!(back["paged"], true);
    assert_eq!(
        back["label"], moved["label"],
        "the same page after a relaunch"
    );
    assert!(seen(&mut again, &point), "the same text after a relaunch");
    again.close();
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn a_search_jump_lands_on_the_page_holding_the_match() {
    let book = web_novel();
    let ix = long_chapters(&book)[2];
    let mut app = App::launch(Some(&book), &[]);
    ready(&mut app);
    auto_one_page(&mut app);
    go(&mut app, ix);
    wait_pg(
        &mut app,
        "v.label.startsWith('Page 1 of ')",
        "the first page",
    );
    let term = app.js(
        "(()=>{const text=document.getElementById('book-host').shadowRoot.querySelector('.ss-body').textContent;const at=Math.floor(text.length*0.8);const words=text.slice(at,at+3000).match(/[A-Za-z]{9,}/g)??[];return words.sort((a,b)=>b.length-a.length)[0]})()",
    );
    let term = term
        .as_str()
        .expect("a long word late in the chapter")
        .to_owned();
    app.js("document.getElementById('toggle-search').click()");
    app.js(&format!(
        "(()=>{{const input=document.getElementById('search-input');input.value={};input.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()",
        json!(term)
    ));
    let rows = wait_until(
        &mut app,
        "(()=>{const rows=[...document.querySelectorAll('#search-results .result')].filter((row)=>{const b=row.getBoundingClientRect();const list=document.getElementById('search-results').getBoundingClientRect();return !row.hidden&&b.height>0&&b.top>=list.top&&b.bottom<=list.bottom}).slice(0,5).map((row)=>{const b=row.getBoundingClientRect();return {x:b.left+b.width/2,y:b.top+b.height/2}});return document.getElementById('search-action').textContent==='Rerun'&&rows.length>0?rows:false})()",
        "search results",
    );
    let mut beyond_first = 0;
    for row in rows.as_array().unwrap() {
        app.click(row["x"].as_f64().unwrap(), row["y"].as_f64().unwrap());
        let landed = wait_until(
            &mut app,
            "(()=>{const hit=CSS.highlights.get('ss-search-current');if(!hit||hit.size===0)return false;const box=[...hit.values()][0].getClientRects()[0];const a=window.__area();return {inside:box.left>=a.left-1&&box.right<=a.right+1&&box.top>=a.top-1&&box.bottom<=a.bottom+1,pg:window.__pg()}})()",
            "the current match",
        );
        println!("jump: {landed}");
        assert_eq!(
            landed["inside"], true,
            "the match is on the shown page: {landed}"
        );
        if !landed["pg"]["label"]
            .as_str()
            .unwrap()
            .starts_with("Page 1 of")
        {
            beyond_first += 1;
        }
        app.js("CSS.highlights.delete('ss-search-current')");
    }
    assert!(beyond_first > 0, "a match past the first page was tried");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn e_and_esc_carry_the_spot_between_pages_and_edit() {
    let book = web_novel();
    let ix = long_chapters(&book)[4];
    let mut app = App::launch(Some(&book), &[]);
    ready(&mut app);
    auto_one_page(&mut app);
    go(&mut app, ix);
    let start = wait_pg(
        &mut app,
        "v.label.startsWith('Page 1 of ')",
        "the first page",
    );
    let total = label_total(&start);
    for _ in 0..5 {
        app.key(KEY_RIGHT);
    }
    wait_label(&mut app, &format!("Page 6 of {total}"));
    let point = first(&mut app);
    app.key(KEY_RIGHT);
    app.key(KEY_RIGHT);
    wait_label(&mut app, &format!("Page 8 of {total}"));
    let later = first(&mut app);
    app.key(KEY_LEFT);
    app.key(KEY_LEFT);
    wait_label(&mut app, &format!("Page 6 of {total}"));

    app.key(KEY_E);
    let editing = wait_pg(
        &mut app,
        "v.mode === 'edit' && !v.paged",
        "Edit in a scrolling view",
    );
    assert_eq!(editing["label"], "", "no page count while editing");
    assert!(
        seen(&mut app, &point),
        "the page's first text is on screen in Edit"
    );
    assert_eq!(
        app.js("window.__caret()"),
        point,
        "the caret starts at the page's first text"
    );
    app.key(KEY_ESCAPE);
    wait_pg(&mut app, "v.mode === 'read' && v.paged", "page mode again");
    wait_label(&mut app, &format!("Page 6 of {total}"));
    assert!(seen(&mut app, &point));

    app.key(KEY_E);
    wait_pg(&mut app, "v.mode === 'edit' && !v.paged", "Edit again");
    let spot = app.js(&format!(
        "(()=>{{const s=document.getElementById('scroller');const b=window.__rectOf({later});s.scrollTop+=b.top-s.getBoundingClientRect().top-200;const r=window.__rectOf({later});return {{x:r.left+1,y:(r.top+r.bottom)/2}}}})()"
    ));
    app.click(spot["x"].as_f64().unwrap(), spot["y"].as_f64().unwrap());
    wait_until(
        &mut app,
        &format!("JSON.stringify(window.__caret())===JSON.stringify({later})"),
        "the caret on later text",
    );
    app.key(KEY_ESCAPE);
    wait_pg(
        &mut app,
        "v.mode === 'read' && v.paged",
        "page mode with the caret's page",
    );
    wait_label(&mut app, &format!("Page 8 of {total}"));
    assert!(seen(&mut app, &later), "the caret's text is on the page");
    app.close();
}

const TURNS: &str = r"(async (count, key) => {
  const frames = () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
  const label = () => document.getElementById('page-where').textContent;
  const times = [];
  const one = [];
  for (let at = 0; at < count; at += 1) {
    const before = label();
    const started = performance.now();
    document.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
    await new Promise(requestAnimationFrame);
    one.push(performance.now() - started);
    await new Promise(requestAnimationFrame);
    times.push(performance.now() - started);
    if (label() === before) return { stuck: before, at };
  }
  times.sort((a, b) => a - b);
  one.sort((a, b) => a - b);
  const pick = (list, share) => list[Math.round((list.length - 1) * share)];
  await frames();
  return { p50: pick(times, 0.5), p95: pick(times, 0.95), max: times[times.length - 1], firstFrameP95: pick(one, 0.95), label: label() };
})";

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn page_turns_on_the_longest_real_chapter_paint_within_two_frames() {
    let book = largest_real_book();
    let ix = largest_item_of(&book);
    let mut app = App::launch(Some(&book), &[]);
    ready(&mut app);
    let open = format!(
        "(async()=>{{await window.__main.goTo(0);const started=performance.now();await window.__main.goTo({ix});await new Promise((done)=>requestAnimationFrame(()=>requestAnimationFrame(done)));return performance.now()-started}})()"
    );
    let scrolling = app.js(&open);
    println!(
        "{} item {ix} scrolling: opened in {scrolling} ms",
        book_label(&book)
    );
    for (mode, two) in [("one page", false), ("two pages", true)] {
        app.js("window.__main.goTo(0).then(()=>true)");
        if two {
            app.js("window.__click('toggle-contents')");
            app.js("window.__slide('layout-width',560)");
            font_down_to_15(&mut app);
        }
        let entered = app.js(&format!(
            "(async()=>{{const started=performance.now();document.getElementById('prefs-pages-auto').click();await window.__main.goTo({ix});await new Promise((done)=>requestAnimationFrame(()=>requestAnimationFrame(done)));return performance.now()-started}})()"
        ));
        let shown = wait_pg(
            &mut app,
            &format!("v.paged && v.two === {two} && v.label.startsWith('Page')"),
            "Auto pages on the longest chapter",
        );
        let forward = app.js(&format!("({TURNS})(60,'ArrowRight')"));
        let backward = app.js(&format!("({TURNS})(30,'ArrowLeft')"));
        println!(
            "{} item {ix} in Auto as {mode} ({}): opened in {entered:.0} ms. 60 turns forward {forward}, 30 back {backward}",
            book_label(&book),
            shown["label"],
            entered = entered.as_f64().unwrap()
        );
        for result in [&forward, &backward] {
            assert!(
                result["stuck"].is_null(),
                "every key turned a page: {result}"
            );
            assert!(
                result["p95"].as_f64().unwrap() <= 36.0,
                "a turn paints within two frames: {result}"
            );
        }
    }
    app.close();
}

fn first_text_ms(book: &Path, data: &Path) -> f64 {
    let mut app = App::launch_with_data(book, data);
    let shown = app.wait_first_text();
    app.assert_on_headless_output();
    let paged = app.js("document.querySelector('.app').classList.contains('paged')");
    let ms = shown - app.spawned_at_ms;
    app.close();
    let wanted = std::fs::read_to_string(data.join("prefs.json"))
        .is_ok_and(|text| text.contains("\"pages\":\"auto\""));
    assert_eq!(paged, json!(wanted), "the launch used the chosen mode");
    ms
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn first_text_in_page_mode_stays_close_to_scrolling() {
    let book = largest_real_book();
    let scroll = fresh_data("first-scroll");
    let paged = fresh_data("first-paged");
    for (dir, pages) in [(&scroll, "scroll"), (&paged, "auto")] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("prefs.json"), format!("{{\"pages\":\"{pages}\"}}")).unwrap();
    }
    first_text_ms(&book, &scroll);
    first_text_ms(&book, &paged);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for _ in 0..9 {
        a.push(first_text_ms(&book, &scroll));
        b.push(first_text_ms(&book, &paged));
    }
    a.sort_by(f64::total_cmp);
    b.sort_by(f64::total_cmp);
    let (scrolling, pages) = (a[a.len() / 2], b[b.len() / 2]);
    println!(
        "{}: first text median scrolling {scrolling:.0} ms ({:.0} to {:.0}), one page {pages:.0} ms ({:.0} to {:.0}), 9 interleaved launches each",
        book_label(&book),
        a[0],
        a[a.len() - 1],
        b[0],
        b[b.len() - 1]
    );
    assert!(
        pages - scrolling <= 30.0,
        "page mode costs {:.0} ms at first text",
        pages - scrolling
    );
    let _ = std::fs::remove_dir_all(&scroll);
    let _ = std::fs::remove_dir_all(&paged);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn the_page_count_follows_images_that_load_after_the_chapter_shows() {
    let book = real_books()
        .into_iter()
        .find(|path| book_label(path) == "tech_b")
        .expect("the technical book fixture");
    let rows = Reader::open(File::open(&book).unwrap(), DEFAULT_LIMITS)
        .unwrap()
        .rows()
        .len() as u64;
    let mut app = App::launch(Some(&book), &[]);
    ready(&mut app);
    auto_one_page(&mut app);
    let mut checked = 0;
    for ix in 0..rows {
        if checked >= 6 {
            break;
        }
        go(&mut app, ix);
        let images = app
            .js("document.getElementById('book-host').shadowRoot.querySelectorAll('img').length");
        if images.as_u64().unwrap_or(0) < 2 {
            continue;
        }
        let shown = app.js("document.getElementById('page-where').textContent");
        wait_until(
            &mut app,
            "[...document.getElementById('book-host').shadowRoot.querySelectorAll('img')].every((img)=>img.complete)",
            "the images loading",
        );
        app.js(
            "new Promise((done)=>requestAnimationFrame(()=>requestAnimationFrame(()=>done(true))))",
        );
        settle(100);
        let loaded = pg(&mut app);
        app.js("window.__click('toggle-contents')");
        app.js("window.__click('toggle-contents')");
        let fresh = wait_pg(&mut app, "true", "a fresh count");
        println!(
            "item {ix}: {images} images, {shown} at mount, {} loaded, {} recounted",
            loaded["label"], fresh["label"]
        );
        assert_eq!(
            label_total(&loaded),
            label_total(&fresh),
            "the count after the images load is current for item {ix}"
        );
        checked += 1;
    }
    assert!(checked >= 3, "chapters with images were checked");
    app.close();
}

const KEY_F: u32 = 0x66;
const KEY_K: u32 = 0x6b;
const KEY_F3: u32 = 0xffc0;

const FOCUS_CHROME: &str = r"(()=>({focus:document.querySelector('.app').classList.contains('focus'),shown:['.top','.toolbar','.rail.toc','#search-pane','#notes-pane','.footrow','.offerbar','.pop'].filter((name)=>{const node=document.querySelector(name);return node&&getComputedStyle(node).display!=='none'}),active:document.activeElement?.id??'',count:document.getElementById('search-count').textContent}))()";

const CURRENT_MATCH: &str = r"(()=>{const hit=CSS.highlights.get('ss-search-current');if(!hit||hit.size===0)return null;const range=[...hit.values()][0];const box=range.getClientRects()[0];const a=window.__area();return {text:range.toString(),inside:box.left>=a.left-1&&box.right<=a.right+1&&box.top>=a.top-1&&box.bottom<=a.bottom+1,pg:window.__pg()}})()";

fn chrome(app: &mut App) -> Value {
    app.js(FOCUS_CHROME)
}

fn wait_chrome(app: &mut App, test: &str, what: &str) -> Value {
    wait_until(
        app,
        &format!("(()=>{{const v={FOCUS_CHROME};return ({test})?v:false}})()"),
        what,
    )
}

fn wide_two_pages_in_focus(app: &mut App) {
    app.js("window.__slide('layout-width',500)");
    app.js("window.__click('prefs-pages-auto')");
    font_down_to_15(app);
    app.key(KEY_F);
    wait_chrome(app, "v.focus", "F entering focus");
    wait_pg(app, "v.paged && v.two", "two pages in focus");
}

fn apart(a: &Value, b: &Value) -> bool {
    let x = |v: &Value, k: &str| v[k].as_f64().unwrap();
    x(a, "right") <= x(b, "left") || x(b, "right") <= x(a, "left")
}

fn rect_of(app: &mut App, selector: &str) -> Value {
    app.js(&format!(
        "(()=>{{const b=document.querySelector({}).getBoundingClientRect();return {{left:b.left,right:b.right,top:b.top,bottom:b.bottom}}}})()",
        json!(selector)
    ))
}

fn long_word_late_in_shown_chapter(app: &mut App) -> String {
    app.js(
        "(()=>{const text=document.getElementById('book-host').shadowRoot.querySelector('.ss-body').textContent;const at=Math.floor(text.length*0.8);const words=text.slice(at,at+3000).match(/[A-Za-z]{9,}/g)??[];return words.sort((a,b)=>b.length-a.length)[0]})()",
    )
    .as_str()
    .expect("a long word late in the chapter")
    .to_owned()
}

fn search_for(app: &mut App, pattern: &str) -> u64 {
    app.js(&format!(
        "(()=>{{const input=document.getElementById('search-input');input.value={};input.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()",
        json!(pattern)
    ));
    let done = wait_until(
        app,
        "(()=>{const text=document.getElementById('search-count').textContent;return document.getElementById('search-action').textContent==='Rerun'&&/^[0-9,]+ match/.test(text)?text:false})()",
        "the search finishing",
    );
    done.as_str()
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .replace(',', "")
        .parse()
        .unwrap()
}

fn first_step(app: &mut App, total: u64) -> u64 {
    app.js("CSS.highlights.delete('ss-search-current')");
    app.key(KEY_ENTER);
    let landed = wait_until(
        app,
        &format!("(()=>{{const c={FOCUS_CHROME};const m={CURRENT_MATCH};return /^Match [0-9,]+ of /.test(c.count)&&m&&m.inside?c.count:false}})()"),
        "the first step",
    );
    let text = landed.as_str().unwrap();
    assert!(text.ends_with(&format!(" of {total}.")), "{text}");
    text["Match ".len()..].split(' ').next().unwrap().replace(',', "").parse().unwrap()
}

fn current_step(app: &mut App, n: u64, total: u64) -> Value {
    let wanted = format!("Match {n} of {total}.");
    wait_until(
        app,
        &format!(
            "(()=>{{const c={FOCUS_CHROME};const m={CURRENT_MATCH};return c.count==={}&&m&&m.inside&&c.active==='search-input'?{{c,m}}:false}})()",
            json!(wanted)
        ),
        &wanted,
    )
}

fn step_to(app: &mut App, keyval: u32, state: u32, n: u64, total: u64) -> Value {
    app.js("CSS.highlights.delete('ss-search-current')");
    app.key_with(keyval, state);
    current_step(app, n, total)
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn in_focus_ctrl_f_shows_only_the_search_pane_and_esc_closes_it_before_focus() {
    let book = web_novel();
    let ix = long_chapters(&book)[3];
    let mut app = App::launch(Some(&book), &[]);
    ready(&mut app);
    go(&mut app, ix);
    wide_two_pages_in_focus(&mut app);
    let start = wait_pg(
        &mut app,
        "v.label.startsWith('Pages 1-2 of ')",
        "the first spread",
    );
    let total = label_total(&start);
    app.key(KEY_RIGHT);
    wait_label(&mut app, &format!("Pages 2-3 of {total}"));
    let point = first(&mut app);

    app.key_with(KEY_F, MASK_CTRL);
    let open = wait_chrome(
        &mut app,
        "v.shown.includes('#search-pane')",
        "Ctrl F opening the pane",
    );
    println!("open in focus: {open}");
    assert_eq!(open["focus"], true, "Ctrl F keeps focus");
    assert_eq!(
        open["shown"],
        json!(["#search-pane"]),
        "only the search pane shows"
    );
    assert_eq!(open["active"], "search-input");
    let narrow = wait_pg(
        &mut app,
        "v.paged && !v.two",
        "one page beside the pane",
    );
    assert!(
        seen(&mut app, &point),
        "the same text stays when two pages become one: {narrow}"
    );
    let pane = rect_of(&mut app, "#search-pane");
    let area = app.js("window.__area()");
    assert!(apart(&area, &pane), "the pane does not cover the text: {area} {pane}");
    let exit = rect_of(&mut app, "#exit-focus");
    assert!(apart(&exit, &pane), "the Exit focus button stays off the pane: {exit} {pane}");

    let height = app.js("window.innerHeight").as_f64().unwrap();
    let pane_x = f64::midpoint(
        pane["left"].as_f64().unwrap(),
        pane["right"].as_f64().unwrap(),
    );
    let text_x = f64::midpoint(
        area["left"].as_f64().unwrap(),
        area["right"].as_f64().unwrap(),
    );
    for (name, y) in [("top", 2.0), ("bottom", height - 2.0)] {
        app.move_pointer(text_x, height / 2.0);
        settle(40);
        app.move_pointer(pane_x, height / 2.0);
        settle(40);
        app.move_pointer(pane_x, y);
        settle(80);
        let under = app.js(&format!(
            "(()=>{{const n=document.elementFromPoint({pane_x},{y});return [n?(n.id||n.className||n.localName):null]}})()"
        ));
        assert_eq!(
            chrome(&mut app)["focus"],
            true,
            "the pane's {name} edge keeps focus, pointer over {under} at {y} of {height}"
        );
    }
    app.move_pointer(pane_x, height / 2.0);

    app.key(KEY_ESCAPE);
    let closed = wait_chrome(
        &mut app,
        "!v.shown.includes('#search-pane')",
        "Esc closing the pane",
    );
    assert_eq!(closed["focus"], true, "the first Esc keeps focus");
    let back = wait_pg(&mut app, "v.paged && v.two", "two pages again");
    assert!(seen(&mut app, &point), "and back to two pages: {back}");

    app.key(KEY_ESCAPE);
    wait_chrome(&mut app, "!v.focus", "the next Esc leaving focus");

    app.key(KEY_F);
    wait_chrome(&mut app, "v.focus", "F entering focus again");
    app.key_with(KEY_K, MASK_CTRL);
    settle(60);
    app.type_text("search this");
    settle(30);
    app.key(KEY_ENTER);
    let command = wait_chrome(
        &mut app,
        "v.shown.includes('#search-pane')",
        "the Search this book command",
    );
    assert_eq!(command["focus"], true, "the palette's search keeps focus");
    assert_eq!(command["shown"], json!(["#search-pane"]));
    assert_eq!(command["active"], "search-input");

    app.js("document.activeElement.blur()");
    let before = pg(&mut app);
    app.key(KEY_RIGHT);
    let turned = wait_pg(
        &mut app,
        &format!("v.label!=={}", json!(before["label"])),
        "a page turn with the pane open",
    );
    println!("turned beside the pane: {turned}");
    app.key(KEY_ESCAPE);
    let from_page = wait_chrome(
        &mut app,
        "!v.shown.includes('#search-pane')",
        "Esc from the page closing the pane",
    );
    assert_eq!(
        from_page["focus"], true,
        "Esc from the page closes the pane first"
    );
    app.key(KEY_ESCAPE);
    wait_chrome(&mut app, "!v.focus", "the next Esc leaving focus");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh pages, headless output"]
fn enter_and_f3_walk_regex_matches_across_pages_and_chapters_in_focus() {
    let book = web_novel();
    let long = long_chapters(&book);
    let (ix_a, ix_b) = (long[2], long[6]);
    let mut app = App::launch(Some(&book), &[]);
    ready(&mut app);
    go(&mut app, ix_b);
    wait_until(
        &mut app,
        "!!document.getElementById('book-host').shadowRoot?.querySelector('.ss-body')",
        "chapter b",
    );
    let word_b = long_word_late_in_shown_chapter(&mut app);
    go(&mut app, ix_a);
    wide_two_pages_in_focus(&mut app);
    wait_pg(
        &mut app,
        "v.label.startsWith('Pages 1-2 of ')",
        "the first spread",
    );
    let word_a = long_word_late_in_shown_chapter(&mut app);
    assert!(!word_a.eq_ignore_ascii_case(&word_b), "two different words");
    let pattern = format!(r"\b(?:{word_a}|{word_b})\b");

    app.key_with(KEY_F, MASK_CTRL);
    wait_chrome(
        &mut app,
        "v.focus && v.shown.includes('#search-pane')",
        "the pane in focus",
    );
    app.js("window.__click('search-regex')");
    app.js("document.getElementById('search-input').focus()");
    let total = search_for(&mut app, &pattern);
    println!("{pattern}: {total} matches");
    assert!((2..=400).contains(&total), "a short walk: {total}");

    let first = first_step(&mut app, total);
    assert!(first > 1, "the walk starts at the reading place, not the first match in the book: {first}");
    let nth = |at: u64| (first - 1 + at) % total + 1;
    let mut wheres = std::collections::BTreeSet::new();
    let mut past_first = 0;
    for at in 0..total {
        let n = nth(at);
        let landed = if at == 0 { current_step(&mut app, n, total) } else { step_to(&mut app, KEY_ENTER, 0, n, total) };
        let text = landed["m"]["text"].as_str().unwrap();
        assert!(
            text.eq_ignore_ascii_case(&word_a) || text.eq_ignore_ascii_case(&word_b),
            "the current match is the pattern, case ignored: {landed}"
        );
        assert_eq!(landed["c"]["focus"], true, "the walk stays in focus");
        let label = landed["m"]["pg"]["label"].as_str().unwrap();
        assert!(
            label.starts_with("Page "),
            "one page beside the pane: {landed}"
        );
        if !label.starts_with("Page 1 of") {
            past_first += 1;
        }
        wheres.insert(landed["m"]["pg"]["where"].as_str().unwrap().to_owned());
    }
    println!("chapters walked: {wheres:?}");
    assert!(wheres.len() >= 2, "the walk crossed chapters: {wheres:?}");
    assert!(past_first > 0, "a match past the first page was reached");

    let last = nth(total - 1);
    let wrapped = step_to(&mut app, KEY_ENTER, 0, first, total);
    println!("around again: {wrapped}");
    let back = step_to(&mut app, KEY_ENTER, MASK_SHIFT, last, total);
    println!("back again: {back}");
    step_to(&mut app, KEY_F3, MASK_SHIFT, nth(total - 2), total);
    step_to(&mut app, KEY_F3, 0, last, total);
    step_to(&mut app, KEY_F3, 0, first, total);

    app.js("document.activeElement.blur()");
    let before = pg(&mut app);
    app.key(KEY_RIGHT);
    wait_pg(
        &mut app,
        &format!(
            "v.label!=={} || v.where!=={}",
            json!(before["label"]),
            json!(before["where"])
        ),
        "the arrow key turning the page off the field",
    );
    app.close();
}
