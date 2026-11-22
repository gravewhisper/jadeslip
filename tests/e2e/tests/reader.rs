use serde_json::{Value, json};
use jadeslip_core::{DEFAULT_LIMITS, Reader};
use jadeslip_e2e::{
    App, data_files_besides_covers, place_name, KEY_DOWN, KEY_ENTER, KEY_ESCAPE, KEY_F11, KEY_LEFT, KEY_PAGE_DOWN, KEY_RIGHT, KEY_SPACE,
    KEY_UP, MASK_CTRL, MASK_SHIFT, book_label, window_fullscreen_of, generated_book, largest_real_book, real_books, scratch_dir,
};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const KEY_K: u32 = 0x6b;
const KEY_P_UPPER: u32 = 0x50;
const KEY_F: u32 = 0x66;
const KEY_E: u32 = 0x65;
const KEY_S: u32 = 0x73;
const KEY_T: u32 = 0x74;
const KEY_F2: u32 = 0xffbf;

const INSTALL: &str = r"(async()=>{
  window.__main = await import('/main.js');
  window.__map = await import('/textmap.js');
  const element = (id) => document.getElementById(id);
  const scroller = element('scroller');
  const shadow = () => element('book-host').shadowRoot;
  const body = () => shadow().querySelector('.ss-body');
  const line = () => parseFloat(getComputedStyle(element('book-host')).lineHeight);
  window.__point = () => {
    const box = scroller.getBoundingClientRect();
    const left = body().getBoundingClientRect().left + 2;
    for (let y = box.top + 1; y < box.bottom; y += 4) {
      const found = document.caretPositionFromPoint(left, y, { shadowRoots: [shadow()] });
      if (!found || !shadow().contains(found.offsetNode) || found.offsetNode.nodeType !== Node.TEXT_NODE) continue;
      if (!/\S/.test(found.offsetNode.data.slice(found.offset, found.offset + 1))) continue;
      const range = document.createRange();
      range.setStart(found.offsetNode, found.offset);
      range.setEnd(found.offsetNode, found.offset + 1);
      const rect = range.getClientRects()[0];
      if (!rect || rect.top < box.top - 0.5) continue;
      const parent = found.offsetNode.parentElement.closest('[data-ss-b]');
      if (!parent) continue;
      return { block: Number(parent.getAttribute('data-ss-b')), units: window.__map.unitsBefore(parent, found.offsetNode, found.offset), top: rect.top - box.top };
    }
    return null;
  };
  window.__topOf = (point) => {
    const top = window.__map.topOfPoint(body(), { block: point.block, offset_u16: point.units });
    return top === null ? null : top - scroller.getBoundingClientRect().top;
  };
  window.__view = () => ({
    top: scroller.scrollTop,
    room: scroller.scrollHeight - scroller.clientHeight,
    height: scroller.clientHeight,
    line: line(),
    where: element('where-title').title,
    title: element('where-title').textContent.length,
    pct: element('pct').textContent,
    fill: element('fill').style.width,
    mode: element('mode-edit').classList.contains('on') ? 'edit' : 'read',
    unsaved: !element('unsaved').hidden,
    dialog: !element('dialog').hidden,
    palette: !element('palette').hidden,
    comfort: !element('comfort').hidden,
    focus: document.querySelector('.app').classList.contains('focus'),
    theme: document.querySelector('.app').classList.contains('light') ? 'light' : 'dark',
    toast: element('toast').hidden ? '' : element('toast').textContent,
    active: document.activeElement?.id ?? '',
    filter: element('filter').value,
    count: element('count').textContent,
    fontPx: getComputedStyle(element('book-host')).fontSize,
    measure: getComputedStyle(document.querySelector('.sheet')).maxWidth,
    contents: getComputedStyle(document.querySelector('.rail.toc')).display !== 'none',
  });
  window.__rail = () => {
    const rows = [...document.querySelectorAll('#toc-spacer .row:not([hidden])')];
    const list = element('toc-list');
    const listBox = list.getBoundingClientRect();
    return {
      slots: document.querySelectorAll('#toc-spacer .row').length,
      top: list.scrollTop,
      rows: rows.map((row) => {
        const box = row.getBoundingClientRect();
        return {
          entry: Number(row.dataset.entry),
          vol: row.classList.contains('vol'),
          cur: row.classList.contains('cur'),
          num: row.querySelector('.num').textContent,
          t: row.querySelector('.t').textContent,
          chev: row.querySelector('.chev').textContent,
          y: box.top + box.height / 2,
          x: box.left + 120,
          seen: box.top >= listBox.top && box.bottom <= listBox.bottom,
          indent: parseFloat(row.style.paddingLeft),
        };
      }).sort((a, b) => a.entry - b.entry),
      shapes: rows.map((row) => [...row.childNodes].map((node) => node.localName ?? '#text').join(',')),
      strangers: [...element('toc-spacer').querySelectorAll('*')].filter((node) => !['button', 'span'].includes(node.localName)).map((node) => node.localName),
    };
  };
  window.__click = (id) => { element(id).click(); return true; };
  window.__layout = () => {
    const sheet = document.querySelector('.sheet');
    const box = sheet.getBoundingClientRect();
    const text = body().getBoundingClientRect();
    const sheetStyle = getComputedStyle(sheet);
    const p = getComputedStyle(shadow().querySelector('.ss-body p'));
    return {
      maxWidth: sheetStyle.maxWidth,
      pad: [sheetStyle.paddingTop, sheetStyle.paddingRight, sheetStyle.paddingBottom, sheetStyle.paddingLeft].join(' '),
      textWidth: text.width,
      textLeft: text.left - box.left,
      textTop: text.top - box.top,
      lineHeight: parseFloat(p.lineHeight),
      gap: parseFloat(p.marginBottom),
      indent: parseFloat(p.textIndent),
      face: p.fontFamily,
      sliders: [...document.querySelectorAll('#comfort input[type=range]')].map((input) => input.id + '=' + input.value).join(' '),
      font: element('layout-font').value,
    };
  };
  window.__slide = (id, value) => {
    const input = element(id);
    input.value = value;
    input.dispatchEvent(new Event('input'));
    input.dispatchEvent(new Event('change'));
    return true;
  };
  window.__pick = (id, value) => {
    const input = element(id);
    input.value = value;
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
        "(()=>{const c=document.getElementById('count').textContent;return c.includes('entries')})()",
        "the contents loading",
    );
}

fn launch(book: &Path) -> App {
    let mut app = App::launch(Some(book), &[]);
    ready(&mut app);
    app
}

fn view(app: &mut App) -> Value {
    app.js("window.__view()")
}

fn go(app: &mut App, ix: u64) {
    app.js(&format!("window.__main.goTo({ix}).then(()=>true)"));
}

fn wait_where(app: &mut App, wanted: &str) {
    let script = format!(
        "document.getElementById('where-title').title==={}",
        json!(wanted)
    );
    wait_until(app, &script, wanted);
}

fn wait_view(app: &mut App, script: &str, what: &str) -> Value {
    wait_until(
        app,
        &format!("(()=>{{const v=window.__view();return ({script})?v:false}})()"),
        what,
    )
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values
}

fn at(sorted: &[f64], share: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * share).round() as usize]
}

fn floats(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(Value::as_f64)
        .collect()
}

const SCROLL_FRAMES: &str = r"((frames, jumpEvery) => new Promise((done) => {
  const list = document.getElementById('toc-list');
  const deltas = [];
  const costs = [];
  let blanks = 0;
  let wrong = 0;
  let frame = 0;
  let last = null;
  let seed = 7;
  const random = () => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed / 0x7fffffff; };
  const room = () => list.scrollHeight - list.clientHeight;
  const check = () => {
    const box = list.getBoundingClientRect();
    const seen = document.elementFromPoint(box.left + 100, box.top + 3)?.closest('.row');
    if (!seen || seen.querySelector('.t').textContent.length === 0) blanks += 1;
    else if (Math.abs(seen.getBoundingClientRect().top - box.top) > 34) wrong += 1;
  };
  const tick = (now) => {
    if (last !== null) { deltas.push(now - last); check(); }
    last = now;
    frame += 1;
    if (frame > frames) { done({ deltas, costs, blanks, wrong, moved: list.scrollTop, slots: document.querySelectorAll('#toc-spacer .row').length }); return; }
    setTimeout(() => {
      const started = performance.now();
      list.scrollTop = jumpEvery > 0 && frame % jumpEvery === 0 ? random() * room() : (list.scrollTop + 170) % room();
      list.dispatchEvent(new Event('scroll'));
      void list.offsetHeight;
      costs.push(performance.now() - started);
    }, 0);
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}))";

const FILTER_FRAMES: &str = r"((frames, every) => new Promise((done) => {
  const input = document.getElementById('filter');
  const queries = ['c', 'ch', 'cha', 'chap', 'chapt', 'chapte', 'chapter', 'chapter 1', '', '1', '12', '123', '1234', '', 'a', 'e', 'th', 'the', 'zzzzqqq', ''];
  const deltas = [];
  const costs = [];
  const counts = new Set();
  let frame = 0;
  let last = null;
  let used = 0;
  const tick = (now) => {
    if (last !== null) deltas.push(now - last);
    last = now;
    frame += 1;
    if (frame > frames) {
      input.value = '';
      input.dispatchEvent(new Event('input', { bubbles: true }));
      done({ deltas, costs, used, distinctCounts: counts.size });
      return;
    }
    if (frame % every === 0) {
      const started = performance.now();
      input.value = queries[used % queries.length];
      used += 1;
      input.dispatchEvent(new Event('input', { bubbles: true }));
      void input.offsetHeight;
      costs.push(performance.now() - started);
      counts.add(document.getElementById('count').textContent);
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}))";

fn report_frames(label: &str, result: &Value) -> (f64, f64, f64) {
    let deltas = sorted(&floats(&result["deltas"]));
    let costs = sorted(&floats(&result["costs"]));
    let (p50, p99, max) = (
        at(&deltas, 0.5),
        at(&deltas, 0.99),
        deltas[deltas.len() - 1],
    );
    println!(
        "{label}: {} frames, frame p50 {p50:.1} p99 {p99:.1} max {max:.1} ms. JS and layout per step p50 {:.2} p99 {:.2} max {:.2} ms",
        deltas.len(),
        at(&costs, 0.5),
        at(&costs, 0.99),
        costs[costs.len() - 1],
    );
    (p50, p99, max)
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn the_contents_stay_at_60_hz_on_the_largest_book_during_scroll_and_filter() {
    let book = largest_real_book();
    let mut app = launch(&book);
    let entries = app.js("(async()=>{const ipc=await import('/ipc.js');const model=await import('/toc-model.js');const a=performance.now();const v=await ipc.loadToc();const b=performance.now();const t=model.buildToc(v);const c=performance.now();return {entries:t.entries.length,linear:t.linear.length,source:t.source,ipc:b-a,build:c-b}})()");
    println!("contents on {}: {entries}", book_label(&book));
    assert_eq!(entries["entries"], json!(6189), "the spike's baseline book");
    for round in 0..3 {
        let scroll = app.js(&format!("{SCROLL_FRAMES}(300, 0)"));
        let (p50, p99, max) = report_frames(
            &format!("round {round} steady scroll, 5 rows a frame"),
            &scroll,
        );
        assert_eq!(
            scroll["blanks"],
            json!(0),
            "a frame showed an empty top row"
        );
        assert_eq!(scroll["wrong"], json!(0));
        assert!(scroll["moved"].as_f64().unwrap() > 0.0, "the list moved");
        assert!(
            scroll["slots"].as_u64().unwrap() < 80,
            "the list is virtual: {}",
            scroll["slots"]
        );
        assert!(
            p50 < 18.0 && p99 < 25.0,
            "steady scroll p50 {p50} p99 {p99} max {max}"
        );

        let jumps = app.js(&format!("{SCROLL_FRAMES}(240, 4)"));
        let (p50, p99, max) = report_frames(
            &format!("round {round} random jumps every 4 frames"),
            &jumps,
        );
        assert_eq!(jumps["blanks"], json!(0), "a jump showed an empty top row");
        assert!(
            p50 < 18.0 && p99 < 25.0,
            "jumps p50 {p50} p99 {p99} max {max}"
        );

        let filter = app.js(&format!("{FILTER_FRAMES}(300, 6)"));
        let (p50, p99, max) =
            report_frames(&format!("round {round} filter every 6 frames"), &filter);
        println!(
            "  {} queries, {} distinct result counts",
            filter["used"], filter["distinctCounts"]
        );
        assert!(filter["used"].as_u64().unwrap() >= 40, "the filter ran");
        assert!(
            filter["distinctCounts"].as_u64().unwrap() > 5,
            "the filter changed the list"
        );
        assert!(
            p50 < 18.0 && p99 < 25.0,
            "filter p50 {p50} p99 {p99} max {max}"
        );
    }
    app.op(&json!({"op": "key", "keyval": 0}));
    app.js("(()=>{const i=document.getElementById('filter');i.focus();window.__keys=[];i.addEventListener('keydown',()=>window.__keys.push(performance.now()),true);window.__painted=[];i.addEventListener('input',()=>requestAnimationFrame(()=>requestAnimationFrame(()=>window.__painted.push(performance.now()))));return true})()");
    for key in "chapter1".chars() {
        app.key(u32::from(key));
        settle(60);
    }
    let timing = app.js("window.__keys.map((k,i)=>window.__painted[i]-k)");
    let times = sorted(&floats(&timing));
    println!(
        "real key events to second frame after filter on {} keys: p50 {:.1} max {:.1} ms",
        times.len(),
        at(&times, 0.5),
        times[times.len() - 1]
    );
    assert_eq!(times.len(), 8);
    app.close();
}

#[test]
#[ignore = "real books, counts only"]
fn every_real_toc_entry_resolves_or_is_a_parent() {
    println!("book       source entries targets parents parent_only dead depth linear covered");
    for book in real_books() {
        let reader = Reader::open(File::open(&book).unwrap(), DEFAULT_LIMITS).unwrap();
        let started = Instant::now();
        let toc = reader.toc().unwrap();
        let parse = started.elapsed();
        let entries = &toc.entries;
        let is_parent = |at: usize| {
            entries
                .get(at + 1)
                .is_some_and(|next| next.depth > entries[at].depth)
        };
        let targets = entries
            .iter()
            .filter(|entry| entry.target.is_some())
            .count();
        let parents = (0..entries.len()).filter(|at| is_parent(*at)).count();
        let parent_only = (0..entries.len())
            .filter(|at| is_parent(*at) && entries[*at].target.is_none())
            .count();
        let dead = (0..entries.len())
            .filter(|at| !is_parent(*at) && entries[*at].target.is_none())
            .count();
        let depth = entries.iter().map(|entry| entry.depth).max().unwrap_or(0);
        let mut hit: Vec<u32> = entries
            .iter()
            .filter_map(|entry| entry.target.map(|ix| ix.0))
            .collect();
        hit.sort_unstable();
        hit.dedup();
        for ix in &hit {
            assert!((*ix as usize) < reader.rows().len());
        }
        let linear = reader
            .rows()
            .iter()
            .filter(|row| row.linear_ordinal.is_some())
            .count();
        let covered = reader
            .rows()
            .iter()
            .filter(|row| row.linear_ordinal.is_some() && hit.binary_search(&row.ix.0).is_ok())
            .count();
        println!(
            "{} {:?} {} {targets} {parents} {parent_only} {dead} {depth} {linear} {covered} parsed in {:.1} ms",
            book_label(&book),
            toc.source,
            entries.len(),
            parse.as_secs_f64() * 1e3
        );
        assert_eq!(
            dead,
            0,
            "{} has a leaf that resolves to no chapter",
            book_label(&book)
        );
    }
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn clicking_real_contents_rows_opens_the_row_chapter() {
    for book in [
        largest_real_book(),
        real_books()
            .into_iter()
            .find(|p| book_label(p) == "tech_b")
            .unwrap(),
    ] {
        let reader = Reader::open(File::open(&book).unwrap(), DEFAULT_LIMITS).unwrap();
        let toc = reader.toc().unwrap();
        let targets: Vec<(usize, u32)> = toc
            .entries
            .iter()
            .enumerate()
            .filter_map(|(at, entry)| entry.target.map(|ix| (at, ix.0)))
            .collect();
        let step = (targets.len() / 60).max(1);
        let mut app = launch(&book);
        let mut opened = 0;
        for (at, ix) in targets.iter().step_by(step) {
            let row = app.js(&format!("(()=>{{const l=document.getElementById('toc-list');l.scrollTop={at}*40;l.dispatchEvent(new Event('scroll'));const r=[...document.querySelectorAll('#toc-spacer .row:not([hidden])')].find(x=>Number(x.dataset.entry)==={at});if(!r)return null;const b=r.getBoundingClientRect();return [b.left+150,b.top+20]}})()"));
            let (x, y) = (
                row[0].as_f64().expect("the row is rendered"),
                row[1].as_f64().unwrap(),
            );
            app.click(x, y);
            wait_until(
                &mut app,
                &format!("window.__main.shownChapter()==={ix}"),
                "the chapter of the row",
            );
            settle(40);
            opened += 1;
        }
        println!(
            "{}: {opened} real rows clicked, each opened its chapter",
            book_label(&book)
        );
        app.close();
    }
}

fn rail(app: &mut App) -> Value {
    app.js("window.__rail()")
}

fn row_titled(seen: &Value, title: &str) -> Value {
    seen["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["t"] == title)
        .unwrap_or_else(|| panic!("no row titled {title}"))
        .clone()
}

fn click_row(app: &mut App, title: &str) {
    let row = row_titled(&rail(app), title);
    app.click(row["x"].as_f64().unwrap(), row["y"].as_f64().unwrap());
}

fn nested_checks(name: &str) {
    let mut app = launch(&generated_book(name));
    let seen = rail(&mut app);
    let titles: Vec<&str> = seen["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["t"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        [
            "Part One",
            "Chapter One",
            "Chapter Two",
            "Scene Two",
            "Part Two",
            "Book Three",
            "Chapter Three",
            "Chapter Four",
            "Chapter Five",
            "Chapter Six"
        ],
        "{name}"
    );
    let part = row_titled(&seen, "Part One");
    assert_eq!(part["vol"], true, "a parent is styled as a volume");
    assert_eq!(part["num"], "", "titles that carry no numbers give a parent no range");
    assert_eq!(row_titled(&seen, "Chapter One")["num"], "", "a chapter shows no position column");
    let scene = row_titled(&seen, "Scene Two");
    assert_eq!(scene["vol"], false);
    assert!(
        scene["indent"].as_f64().unwrap()
            > row_titled(&seen, "Chapter Two")["indent"].as_f64().unwrap() - 0.1
    );
    assert_eq!(
        row_titled(&seen, "Chapter One")["cur"],
        true,
        "the first chapter is current"
    );
    assert_eq!(view(&mut app)["where"], "Position 1 of 6");

    click_row(&mut app, "Part Two");
    settle(120);
    assert_eq!(
        view(&mut app)["where"],
        "Position 1 of 6",
        "a parent with no link opens nothing"
    );
    let folded = rail(&mut app);
    let titles: Vec<&str> = folded["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["t"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        [
            "Part One",
            "Chapter One",
            "Chapter Two",
            "Scene Two",
            "Part Two",
            "Chapter Six"
        ],
        "{name}: folded"
    );

    click_row(&mut app, "Chapter Six");
    wait_where(&mut app, "Position 6 of 6");
    assert_eq!(row_titled(&rail(&mut app), "Chapter Six")["cur"], true);
    go(&mut app, chapter_ix_of(&generated_book(name), 4));
    wait_where(&mut app, "Position 4 of 6");
    assert_eq!(
        row_titled(&rail(&mut app), "Part Two")["cur"],
        true,
        "a folded parent stands in for its child"
    );
    click_row(&mut app, "Part Two");
    settle(50);
    assert_eq!(row_titled(&rail(&mut app), "Chapter Four")["cur"], true);

    click_row(&mut app, "Scene Two");
    wait_where(&mut app, "Position 2 of 6");
    assert!(
        view(&mut app)["top"].as_f64().unwrap() > 0.0,
        "the fragment scrolled"
    );
    assert_eq!(
        row_titled(&rail(&mut app), "Chapter Two")["cur"],
        true,
        "the plain entry owns the chapter"
    );
    app.close();
}

fn chapter_ix_of(book: &Path, number: u32) -> u64 {
    let reader = Reader::open(File::open(book).unwrap(), DEFAULT_LIMITS).unwrap();
    let row = reader
        .rows()
        .iter()
        .find(|row| row.linear_ordinal == Some(number))
        .unwrap();
    u64::from(row.ix.0)
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn nested_nav_keeps_parents_and_nesting() {
    nested_checks("toc-nested-nav");
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn nested_ncx_keeps_parents_and_nesting() {
    nested_checks("toc-nested-ncx");
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn no_toc_starts_with_placeholders_and_fills_titles_behind_the_reader() {
    let book = generated_book("toc-none");
    let mut app = App::launch(Some(&book), &[]);
    ready(&mut app);
    wait_until(
        &mut app,
        "!document.getElementById('count').textContent.includes('Titling')",
        "the fill finishing",
    );
    let seen = rail(&mut app);
    let rows = seen["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 6);
    assert_eq!(
        rows[3]["t"], "Chapter 4",
        "a chapter with no heading keeps its placeholder"
    );
    for at in [0, 1, 2, 4, 5] {
        assert_ne!(
            rows[at]["t"],
            format!("Chapter {}", at + 1),
            "row {at} was titled from its heading"
        );
    }
    app.js("(()=>{const i=document.getElementById('filter');i.value='Chapter 4';i.dispatchEvent(new Event('input',{bubbles:true}));return true})()");
    assert_eq!(
        rail(&mut app)["rows"].as_array().unwrap().len(),
        1,
        "the filter sees the placeholder"
    );
    let title = rows[0]["t"].as_str().unwrap().to_owned();
    app.js(&format!("(()=>{{const i=document.getElementById('filter');i.value={};i.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()", json!(title)));
    assert!(
        !rail(&mut app)["rows"].as_array().unwrap().is_empty(),
        "the filter sees the heading titles"
    );
    app.close();
}

fn strip_toc(from: &Path, to: &Path) {
    let script = r#"
import re, sys, zipfile
src, dst = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(src) as zin, zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as zout:
    for item in zin.infolist():
        data = zin.read(item.filename)
        if item.filename.endswith(".opf"):
            text = data.decode("utf-8")
            text = re.sub(r'\s+toc="[^"]*"', "", text)
            text = re.sub(r'(<item\b[^>]*?)\s+properties="nav"', r"\1", text)
            text = re.sub(r"<item\b[^>]*application/x-dtbncx\+xml[^>]*/>", "", text)
            data = text.encode("utf-8")
        zout.writestr(item, data)
"#;
    let done = std::process::Command::new("python3")
        .args(["-I", "-c", script])
        .arg(from)
        .arg(to)
        .status()
        .expect("python runs");
    assert!(done.success());
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn a_real_book_with_its_toc_removed_fills_titles_while_the_reader_stays_live() {
    let copy = scratch_dir().join("reader-no-toc.epub");
    strip_toc(&largest_real_book(), &copy);
    let mut app = App::launch(Some(&copy), &[]);
    let started = app.spawned_at_ms;
    let shown = app.wait_first_text();
    println!(
        "first text with no toc: {:.0} ms after spawn",
        shown - started
    );
    app.js(INSTALL);
    let source = wait_until(
        &mut app,
        "document.getElementById('count').textContent.includes('entries')?document.getElementById('count').textContent.length:false",
        "the placeholders",
    );
    assert!(source.as_u64().unwrap() > 0);
    let placeholders = app.js("(async()=>{const ipc=await import('/ipc.js');const v=await ipc.loadToc();return {source:v.source,total:v.entries.length,plain:v.entries.filter((e)=>/^Chapter [0-9]+$/.test(e.label)).length}})()");
    println!("what the command gives first: {placeholders}");
    assert_eq!(placeholders["source"], "headings");
    assert_eq!(
        placeholders["plain"], placeholders["total"],
        "every row starts as a placeholder"
    );
    wait_until(
        &mut app,
        "!document.getElementById('count').textContent.includes('Titling')",
        "the fill finishing",
    );
    let renamed = app.js("[...document.querySelectorAll('#toc-spacer .row:not([hidden]) .t')].filter((n)=>!/^Chapter [0-9]+$/.test(n.textContent)).length");
    assert!(renamed.as_u64().unwrap() > 0, "the rows were renamed");
    let reader = Reader::open(File::open(&copy).unwrap(), DEFAULT_LIMITS).unwrap();
    let begun = Instant::now();
    let mut titled = 0;
    reader.heading_titles(&jadeslip_core::CancelToken::default(), &mut |_, _| {
        titled += 1
    });
    println!(
        "the fill on one thread: {titled} titles in {:.0} ms",
        begun.elapsed().as_secs_f64() * 1e3
    );
    app.js("(async()=>{const ipc=await import('/ipc.js');window.__fills=0;window.__filling=true;let id=1000;(async()=>{while(window.__filling){id+=1;await ipc.headingsStart(id);for(;;){const b=await ipc.headingsNext(id);if(b.done)break}window.__fills+=1}})();return true})()");
    let during = app.js(&format!("{SCROLL_FRAMES}(180, 0)"));
    let (p50, p99, max) = report_frames("scroll while the fill runs over and over", &during);
    let turned = app.js("(async()=>{const a=performance.now();await window.__main.goTo(50);return performance.now()-a})()").as_f64().unwrap();
    let fills = app.js("(()=>{window.__filling=false;return window.__fills})()");
    println!(
        "a chapter turn during the same load: {turned:.1} ms. Full fills finished during the run: {fills}"
    );
    assert!(fills.as_u64().unwrap() >= 1, "the fill really ran");
    assert_eq!(during["blanks"], json!(0));
    assert!(
        p50 < 18.0 && p99 < 25.0,
        "scroll while filling p50 {p50} p99 {p99} max {max}"
    );
    app.close();
    let _ = std::fs::remove_file(copy);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn the_current_row_stays_in_view_and_a_click_in_edit_asks_first() {
    let book = generated_book("large-h1-p");
    let mut app = launch(&book);
    let current_seen = "(()=>{const l=document.getElementById('toc-list').getBoundingClientRect();const r=document.querySelector('#toc-spacer .row.cur');if(!r)return false;const b=r.getBoundingClientRect();return b.top>=l.top&&b.bottom<=l.bottom})()";
    assert_eq!(app.js(current_seen), true);
    go(&mut app, 4000);
    assert_eq!(
        app.js(current_seen),
        true,
        "a far jump brought the row into view"
    );
    let top = rail(&mut app)["top"].as_f64().unwrap();
    assert!(top > 100_000.0, "{top}");
    app.key(KEY_RIGHT);
    wait_where(
        &mut app,
        &format!("Position {} of 6,000", number_of(&book, 4001)),
    );
    assert_eq!(app.js(current_seen), true);
    let near = rail(&mut app)["top"].as_f64().unwrap();
    assert!(
        (near - top).abs() < 1.0,
        "a next chapter still in view does not move the list"
    );
    app.js("(()=>{const i=document.getElementById('filter');i.value='zzzzqqq';i.dispatchEvent(new Event('input',{bubbles:true}));return true})()");
    assert_eq!(
        app.js("document.getElementById('toc-empty').hidden"),
        false,
        "no match says so"
    );
    assert_eq!(rail(&mut app)["rows"].as_array().unwrap().len(), 0);
    assert!(
        view(&mut app)["count"]
            .as_str()
            .unwrap()
            .starts_with("0 of ")
    );
    app.js("(()=>{document.getElementById('filter').focus();return true})()");
    app.key(KEY_ESCAPE);
    assert_eq!(view(&mut app)["filter"], "", "Esc clears the filter first");
    assert_eq!(view(&mut app)["active"], "filter");
    assert_eq!(
        app.js(current_seen),
        true,
        "clearing brings the current row back"
    );
    app.key(KEY_ESCAPE);
    assert_ne!(
        view(&mut app)["active"],
        "filter",
        "the second Esc leaves the field"
    );
    app.key(KEY_RIGHT);
    wait_where(
        &mut app,
        &format!("Position {} of 6,000", number_of(&book, 4002)),
    );

    app.key(KEY_E);
    wait_view(&mut app, "v.mode==='edit'", "Edit");
    settle(40);
    app.type_text("zz");
    settle(40);
    let row = rail(&mut app)["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["seen"] == true && r["cur"] == false)
        .unwrap()
        .clone();
    app.click(row["x"].as_f64().unwrap(), row["y"].as_f64().unwrap());
    wait_view(&mut app, "v.dialog", "the unsaved prompt");
    assert_eq!(
        view(&mut app)["where"],
        format!("Position {} of 6,000", number_of(&book, 4002))
    );
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.dialog&&v.mode==='edit'", "keeping the edit");
    app.close();
}

fn number_of(book: &Path, ix: u32) -> String {
    let reader = Reader::open(File::open(book).unwrap(), DEFAULT_LIMITS).unwrap();
    let number = reader.rows()[ix as usize].linear_ordinal.unwrap();
    let digits = number.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn long_chapters(book: &Path, wanted: usize) -> Vec<u32> {
    let reader = Reader::open(File::open(book).unwrap(), DEFAULT_LIMITS).unwrap();
    let rows: Vec<u32> = reader
        .rows()
        .iter()
        .filter(|row| {
            row.linear_ordinal.is_some() && row.rev.is_some_and(|rev| rev.inflated_len > 8_000)
        })
        .map(|row| row.ix.0)
        .collect();
    let step = (rows.len() / wanted).max(1);
    rows.into_iter().step_by(step).take(wanted).collect()
}

fn scroll_to(app: &mut App, share: f64) -> Value {
    app.js(&format!("(()=>{{const s=document.getElementById('scroller');s.scrollTop=(s.scrollHeight-s.clientHeight)*{share};return true}})()"));
    settle(260);
    app.js("window.__point()")
}

const COMFORT_STEPS: &[&str] = &[
    "window.__click('font-larger')",
    "window.__click('font-larger')",
    "window.__click('font-larger')",
    "window.__slide('layout-width',816)",
    "window.__click('font-larger')",
    "window.__slide('layout-width',516)",
    "window.__slide('layout-marginTop',120)",
    "window.__slide('layout-marginLeft',100)",
    "window.__slide('layout-lineHeight',230)",
    "window.__slide('layout-paraGap',200)",
    "window.__slide('layout-indent',20)",
    "window.__pick('layout-font','mono')",
    "window.__click('font-smaller')",
    "window.__click('font-smaller')",
    "window.__click('font-smaller')",
    "window.__click('font-smaller')",
    "window.__click('layout-reset')",
];

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn the_same_text_stays_on_top_across_text_and_layout_changes_on_real_chapters() {
    let mut worst_px: f64 = 0.0;
    let mut worst_lines: f64 = 0.0;
    let mut changes = 0;
    let mut clamped = 0;
    let mut drift_px: f64 = 0.0;
    let books: Vec<PathBuf> = real_books()
        .into_iter()
        .filter(|p| ["serial_a", "serial_b1", "tech_b"].contains(&book_label(p).as_str()))
        .collect();
    assert_eq!(books.len(), 3);
    for book in books {
        let chapters = long_chapters(&book, 12);
        let mut app = launch(&book);
        let mut looked = 0;
        for ix in &chapters {
            go(&mut app, u64::from(*ix));
            let shown = view(&mut app);
            if shown["room"].as_f64().unwrap() < shown["height"].as_f64().unwrap() * 2.0 {
                continue;
            }
            let Some(point) = Some(scroll_to(&mut app, 0.41)).filter(|point| {
                !point.is_null() && point["top"].as_f64().unwrap() < shown["line"].as_f64().unwrap()
            }) else {
                continue;
            };
            looked += 1;
            let start_top = point["top"].as_f64().unwrap();
            let start_scroll = view(&mut app)["top"].as_f64().unwrap();
            for step in COMFORT_STEPS {
                app.js(step);
                let after = view(&mut app);
                let top = app
                    .js(&format!("window.__topOf({point})"))
                    .as_f64()
                    .expect("the point resolves");
                changes += 1;
                let at_bottom =
                    after["top"].as_f64().unwrap() >= after["room"].as_f64().unwrap() - 1.0;
                if at_bottom && top > start_top {
                    clamped += 1;
                    assert!(
                        top < after["height"].as_f64().unwrap(),
                        "the text stayed on screen"
                    );
                    continue;
                }
                let moved = (top - start_top).abs();
                worst_px = worst_px.max(moved);
                worst_lines = worst_lines.max(moved / after["line"].as_f64().unwrap());
                assert!(moved <= 2.0, "chapter {ix} moved {moved} px after {step}");
            }
            drift_px = drift_px.max((view(&mut app)["top"].as_f64().unwrap() - start_scroll).abs());
        }
        println!("{}: {looked} real chapters", book_label(&book));
        assert!(
            looked >= 5,
            "too few long chapters on {}",
            book_label(&book)
        );
        app.close();
    }
    println!(
        "{changes} changes. The unit that topped the view moved at most {worst_px:.2} px ({worst_lines:.3} lines), {clamped} clamped at chapter end. After undoing every change the scroll offset differed by at most {drift_px:.2} px"
    );
}

fn wait_file(path: &Path, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if std::fs::read_to_string(path).is_ok_and(|text| text.contains(needle)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} never held {needle}",
            path.display()
        );
        settle(20);
    }
}

fn file_hash(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn fresh_data(name: &str) -> PathBuf {
    let dir = scratch_dir().join(format!("reader-data-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn the_place_comes_back_across_a_launch_on_real_chapters_without_parsing_the_toc_first() {
    for book in real_books()
        .into_iter()
        .filter(|p| ["serial_a", "tech_b"].contains(&book_label(p).as_str()))
    {
        let before = file_hash(&book);
        let data = fresh_data(&book_label(&book));
        let chapters = long_chapters(&book, 6);
        let mut worst: f64 = 0.0;
        let mut looked = 0;
        for ix in chapters {
            let mut app = App::launch_with_data(&book, &data);
            ready(&mut app);
            go(&mut app, u64::from(ix));
            let shown = view(&mut app);
            if shown["room"].as_f64().unwrap() < shown["height"].as_f64().unwrap() {
                app.close();
                continue;
            }
            let point = scroll_to(&mut app, 0.63);
            if point.is_null() {
                app.close();
                continue;
            }
            let where_before = view(&mut app)["where"].clone();
            let places = data.join(place_name(&data, &book));
            wait_file(&places, "\"offset_u16\"");
            settle(450);
            app.close();

            let mut next = App::launch_with_data(&book, &data);
            let first_text = next.wait_first_text();
            let shown_at = next.phases();
            assert!(
                shown_at.iter().any(|(name, _)| name == "first_render"),
                "the phases were recorded"
            );
            assert!(
                !shown_at
                    .iter()
                    .any(|(name, _)| name == "toc_parsed_to_start"),
                "a saved place starts without parsing the contents"
            );
            let toc_called = shown_at
                .iter()
                .find(|(name, _)| name == "toc_called")
                .map(|(_, at)| *at);
            assert!(
                toc_called.is_none_or(|at| at >= first_text - 40.0),
                "the contents loaded before first text"
            );
            next.js(INSTALL);
            let again = view(&mut next);
            assert_eq!(again["where"], where_before);
            next.js("document.fonts.ready.then(()=>new Promise((done)=>requestAnimationFrame(()=>requestAnimationFrame(()=>done(true)))))");
            settle(150);
            let top = next
                .js(&format!("window.__topOf({point})"))
                .as_f64()
                .expect("the point resolves");
            let moved = top.abs();
            worst = worst.max(moved);
            assert!(
                moved <= again["line"].as_f64().unwrap() + 2.0,
                "moved {moved} px on launch"
            );
            looked += 1;
            next.close();
        }
        println!(
            "{}: {looked} launches. The unit on top came back within {worst:.2} px",
            book_label(&book)
        );
        assert!(looked >= 3);
        assert_eq!(before, file_hash(&book), "the book never changes");
        assert_eq!(
            data_files_besides_covers(&data),
            [place_name(&data, &book)],
            "nothing but the place is written for a reader"
        );
        let _ = std::fs::remove_dir_all(&data);
    }
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn space_and_arrows_at_the_end_of_a_chapter_in_a_field_and_in_edit() {
    let book = generated_book("large-h1-p");
    let mut app = launch(&book);
    let start = view(&mut app);
    let first_where = start["where"].as_str().unwrap().to_owned();
    assert!(start["room"].as_f64().unwrap() > start["height"].as_f64().unwrap());
    app.key(KEY_SPACE);
    let paged = wait_view(&mut app, "v.top>0", "Space paging down");
    assert_eq!(paged["where"], first_where.as_str());
    let page = paged["top"].as_f64().unwrap();
    assert!(
        page > paged["height"].as_f64().unwrap() * 0.8 && page < paged["height"].as_f64().unwrap(),
        "{page}"
    );
    app.key(KEY_PAGE_DOWN);
    wait_view(&mut app, &format!("v.top>{page}"), "PageDown");
    let scrolled = view(&mut app)["top"].as_f64().unwrap();
    app.key(KEY_UP);
    wait_view(&mut app, &format!("v.top<{scrolled}"), "ArrowUp");
    let up = view(&mut app)["top"].as_f64().unwrap();
    app.key(KEY_DOWN);
    wait_view(&mut app, &format!("v.top>{up}"), "ArrowDown");
    app.key_with(KEY_SPACE, 1);
    wait_view(&mut app, &format!("v.top<{up}"), "Shift Space");

    app.js("(()=>{const s=document.getElementById('scroller');s.scrollTop=s.scrollHeight;return true})()");
    settle(50);
    assert_eq!(
        view(&mut app)["where"],
        first_where.as_str(),
        "scrolling alone turns no chapter"
    );
    let progress_end = view(&mut app)["fill"]
        .as_str()
        .unwrap()
        .trim_end_matches('%')
        .parse::<f64>()
        .unwrap();
    app.key(KEY_SPACE);
    let turned = wait_view(
        &mut app,
        &format!("v.where!=={}", json!(first_where)),
        "Space at the end",
    );
    assert_eq!(turned["top"], 0.0, "the next chapter starts at its top");
    let second_where = turned["where"].as_str().unwrap().to_owned();
    let progress_next = turned["fill"]
        .as_str()
        .unwrap()
        .trim_end_matches('%')
        .parse::<f64>()
        .unwrap();
    assert!(
        progress_next <= progress_end + 0.05 && progress_next > 0.0,
        "progress counts position inside the chapter: {progress_end} then {progress_next}"
    );

    app.js("(()=>{const s=document.getElementById('scroller');s.scrollTop=s.scrollHeight;return true})()");
    settle(30);
    for _ in 0..4 {
        app.key(KEY_SPACE);
    }
    settle(150);
    let held = view(&mut app);
    assert_ne!(held["where"], second_where.as_str());
    let number = |text: &str| {
        text.split_whitespace()
            .nth(1)
            .unwrap()
            .replace(',', "")
            .parse::<u32>()
            .unwrap()
    };
    assert_eq!(
        number(held["where"].as_str().unwrap()),
        number(&second_where) + 1,
        "held Space turned one chapter and then paged"
    );

    app.key(KEY_LEFT);
    wait_view(
        &mut app,
        &format!("v.where==={}", json!(second_where)),
        "ArrowLeft",
    );
    app.key(KEY_RIGHT);
    wait_view(
        &mut app,
        &format!("v.where!=={}", json!(second_where)),
        "ArrowRight",
    );
    let here = view(&mut app);

    app.js("(()=>{document.getElementById('filter').focus();return true})()");
    for key in [KEY_RIGHT, KEY_LEFT, KEY_DOWN, KEY_SPACE] {
        app.key(key);
    }
    app.type_text("e");
    settle(120);
    let in_field = view(&mut app);
    assert_eq!(in_field["where"], here["where"], "arrows stay in the field");
    assert_eq!(in_field["top"], here["top"], "Space stays in the field");
    assert_eq!(
        in_field["filter"], " e",
        "Space and letters type in the field"
    );
    assert_eq!(in_field["mode"], "read", "E types in the field");
    app.key(KEY_ESCAPE);
    app.key(KEY_ESCAPE);

    app.js("(()=>{document.getElementById('next').focus();return true})()");
    let before_button = view(&mut app);
    app.key(KEY_SPACE);
    let via_button = wait_view(
        &mut app,
        &format!("v.where!=={}", json!(before_button["where"])),
        "Space on a focused button",
    );
    assert_eq!(via_button["top"], 0.0, "the button acted, not paging");
    app.js("(()=>{document.activeElement.blur();return true})()");

    let editing_at = view(&mut app);
    app.key(KEY_E);
    wait_view(&mut app, "v.mode==='edit'", "Edit");
    settle(40);
    let text_before = app.js("document.getElementById('book-host').shadowRoot.querySelector('.ss-body').textContent.length");
    app.key(KEY_SPACE);
    app.key(KEY_RIGHT);
    app.key(KEY_LEFT);
    app.key(KEY_DOWN);
    settle(100);
    let in_edit = view(&mut app);
    assert_eq!(
        in_edit["where"], editing_at["where"],
        "arrows move the caret in Edit"
    );
    assert_eq!(in_edit["mode"], "edit");
    let text_after = app.js("document.getElementById('book-host').shadowRoot.querySelector('.ss-body').textContent.length");
    assert_eq!(
        text_after.as_u64().unwrap(),
        text_before.as_u64().unwrap() + 1,
        "Space types in Edit"
    );
    app.close();

    let short = generated_book("toc-nested-nav");
    let mut last = launch(&short);
    go(&mut last, chapter_ix_of(&short, 6));
    wait_where(&mut last, "Position 6 of 6");
    last.js("(()=>{const s=document.getElementById('scroller');s.scrollTop=s.scrollHeight;return true})()");
    settle(30);
    last.key(KEY_SPACE);
    let done = wait_view(
        &mut last,
        "v.toast==='This is the last chapter.'",
        "the last chapter notice",
    );
    assert_eq!(done["where"], "Position 6 of 6");
    last.close();
}

const PALETTE_ROWS: &str = "[...document.querySelectorAll('#palette-rows .palrow')].map((r)=>({label:r.querySelector('.pallabel').textContent,hint:r.querySelector('.palhint').textContent,on:r.classList.contains('on')}))";

const TIME_PAINT: &str = r"(()=>{
  window.__key = null;
  window.__times = [];
  document.addEventListener('keydown', () => { window.__key = performance.now(); }, true);
  const mark = () => {
    const from = window.__key;
    if (from === null) return;
    window.__key = null;
    requestAnimationFrame(() => requestAnimationFrame(() => window.__times.push(performance.now() - from)));
  };
  new MutationObserver(mark).observe(document.getElementById('palette'), { attributes: true, attributeFilter: ['hidden'] });
  document.getElementById('palette-input').addEventListener('input', mark);
  return true;
})()";

fn palette_type(app: &mut App, text: &str) {
    app.type_text(text);
    settle(30);
}

fn palette_clear(app: &mut App) {
    app.js("(()=>{const i=document.getElementById('palette-input');i.value='';i.dispatchEvent(new Event('input',{bubbles:true}));return true})()");
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader"]
fn ctrl_shift_p_opens_and_closes_the_palette_like_ctrl_k() {
    let mut app = launch(&generated_book("toc-none"));
    app.key_with(KEY_P_UPPER, MASK_CTRL | MASK_SHIFT);
    wait_view(
        &mut app,
        "v.palette&&v.active==='palette-input'",
        "Ctrl+Shift+P opening the palette",
    );
    app.key_with(KEY_P_UPPER, MASK_CTRL | MASK_SHIFT);
    wait_view(&mut app, "!v.palette", "Ctrl+Shift+P closing the palette");
    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "Ctrl+K still opening the palette");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn quick_jump_finds_numbers_titles_and_commands_fast_on_the_largest_book() {
    let book = largest_real_book();
    let reader = Reader::open(File::open(&book).unwrap(), DEFAULT_LIMITS).unwrap();
    let mut app = launch(&book);
    app.js(TIME_PAINT);

    app.key_with(KEY_K, MASK_CTRL);
    wait_view(
        &mut app,
        "v.palette&&v.active==='palette-input'",
        "the palette opening",
    );
    assert!(
        app.js(&format!("{PALETTE_ROWS}.length")).as_u64().unwrap() >= 4,
        "the commands are offered"
    );

    palette_type(&mut app, "3000");
    let rows = app.js(PALETTE_ROWS);
    let label = rows[0]["label"].as_str().unwrap().to_owned();
    assert!(label.starts_with("Chapter 3000:"), "the title carrying the number leads: {rows}");
    let hint = rows[0]["hint"].as_str().unwrap().to_owned();
    assert!(hint.starts_with("Position "), "{hint}");
    assert_eq!(rows[0]["on"], true);
    assert!(rows.as_array().unwrap().len() <= 7);
    app.key(KEY_ENTER);
    wait_where(&mut app, &format!("{hint} of 6,191"));
    let shown = view(&mut app);
    assert!(!shown["palette"].as_bool().unwrap());
    let position: u32 = hint["Position ".len()..].replace(',', "").parse().unwrap();
    let row = reader
        .rows()
        .iter()
        .find(|row| row.linear_ordinal == Some(position))
        .unwrap();
    assert_eq!(app.js("window.__main.shownChapter()"), json!(row.ix.0));

    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    palette_type(&mut app, "3,0");
    palette_clear(&mut app);
    palette_type(&mut app, "chapter 42");
    assert!(
        app.js(PALETTE_ROWS)[0]["label"].as_str().unwrap().starts_with("Chapter 42 "),
        "words before a number still jump"
    );
    palette_clear(&mut app);
    palette_type(&mut app, "999999");
    assert!(
        app.js(&format!(
            "{PALETTE_ROWS}.filter((r)=>r.hint==='Position 999,999').length"
        ))
        .as_u64()
        .unwrap()
            == 0
    );

    let toc = reader.toc().unwrap();
    let wanted = toc.entries[4000].label.clone();
    let word = wanted.split_whitespace().last().unwrap().to_owned();
    app.js(&format!("(()=>{{const i=document.getElementById('palette-input');i.value={};i.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()", json!(wanted)));
    let rows = app.js(PALETTE_ROWS);
    let picked = rows
        .as_array()
        .unwrap()
        .iter()
        .position(|row| row["label"] == wanted.as_str());
    assert!(picked.is_some(), "the title is offered");
    let hint = rows[picked.unwrap()]["hint"].as_str().unwrap().to_owned();
    let lower_word = word.to_lowercase();
    app.js(&format!("(()=>{{const i=document.getElementById('palette-input');i.value={};i.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()", json!(lower_word)));
    assert!(
        app.js(&format!("{PALETTE_ROWS}.length")).as_u64().unwrap() >= 1,
        "matching ignores case"
    );
    app.js(&format!("(()=>{{const i=document.getElementById('palette-input');i.value={};i.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()", json!(wanted)));
    for _ in 0..picked.unwrap() {
        app.key(KEY_DOWN);
    }
    app.key(KEY_ENTER);
    wait_where(&mut app, &format!("{} of 6,191", hint));

    let theme_before = view(&mut app)["theme"].clone();
    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    palette_type(&mut app, "theme");
    assert_eq!(app.js(PALETTE_ROWS)[0]["label"], "Switch theme");
    assert_eq!(app.js(PALETTE_ROWS)[0]["hint"], "Command");
    app.key(KEY_ENTER);
    let after_theme = wait_view(&mut app, "!v.palette", "the command closing the palette");
    assert_ne!(after_theme["theme"], theme_before, "the command ran");

    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    palette_type(&mut app, "zzzzqqqq");
    assert_eq!(
        app.js("document.getElementById('palette-empty').hidden"),
        false,
        "no match says so"
    );
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.palette", "Esc closing");
    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "!v.palette", "Ctrl K closing it");

    for _ in 0..12 {
        app.key_with(KEY_K, MASK_CTRL);
        settle(50);
        app.key(KEY_ESCAPE);
        settle(50);
    }
    let open = sorted(&floats(&app.js("window.__times.splice(0)")));
    app.key_with(KEY_K, MASK_CTRL);
    settle(60);
    app.js("window.__times.splice(0)");
    for text in [
        "1", "2", "3", "4", "a", "e", "i", "o", "u", "n", "s", "t", "r", "l", "z", "q", "x",
    ] {
        app.key(u32::from(text.chars().next().unwrap()));
        settle(50);
    }
    let typed = sorted(&floats(&app.js("window.__times.splice(0)")));
    println!(
        "real key to second frame. Open and close, {} marks: p50 {:.1} max {:.1} ms. Typing, {} keys: p50 {:.1} max {:.1} ms. Worst query scans {} entries",
        open.len(),
        at(&open, 0.5),
        open[open.len() - 1],
        typed.len(),
        at(&typed, 0.5),
        typed[typed.len() - 1],
        toc.entries.len(),
    );
    assert!(at(&typed, 0.5) < 34.0 && typed[typed.len() - 1] < 60.0);
    assert!(at(&open, 0.5) < 34.0);
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn quick_jump_in_edit_asks_first_and_gives_the_caret_back() {
    let book = generated_book("toc-nested-nav");
    let mut app = launch(&book);
    app.key(KEY_E);
    wait_view(&mut app, "v.mode==='edit'", "Edit");
    settle(40);
    app.type_text("qq");
    let caret = app.js("(()=>{const s=document.getSelection();const r=s.getComposedRanges({shadowRoots:[document.getElementById('book-host').shadowRoot]})[0];return r.startOffset})()");
    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.palette&&v.mode==='edit'", "still editing");
    app.type_text("w");
    let body_has = |text: &str| {
        format!("document.getElementById('book-host').shadowRoot.querySelector('.ss-body').textContent.includes('{text}')")
    };
    wait_until(&mut app, &body_has("qqw"), "the caret coming back to the same place");
    assert!(caret.as_u64().is_some());

    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    app.type_text("5");
    settle(30);
    app.key(KEY_ENTER);
    let asked = wait_view(&mut app, "v.dialog&&!v.palette", "the unsaved prompt");
    assert_eq!(asked["where"], "Position 1 of 6", "no move while asking");
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.dialog&&v.mode==='edit'", "keeping the edit");
    app.type_text("x");
    wait_until(&mut app, &body_has("qqwx"), "the caret surviving the prompt");

    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    app.type_text("theme");
    settle(30);
    app.key(KEY_ENTER);
    wait_view(
        &mut app,
        "!v.palette&&v.mode==='edit'&&v.unsaved",
        "a command leaving the edit alone",
    );
    app.type_text("y");
    wait_until(&mut app, &body_has("qqwxy"), "the caret staying through a command");
    app.close();
}

const VISIBLE_CHROME: &str = r"['.top', '.toolbar', '.rail.toc', '#search-pane', '.footrow', '.offerbar', '.pop'].filter((name) => { const node = document.querySelector(name); return node && getComputedStyle(node).display !== 'none'; })";

fn in_focus(app: &mut App) -> bool {
    view(app)["focus"].as_bool().unwrap()
}

fn top_stays(app: &mut App, point: &Value, start: f64, what: &str) {
    let top = app
        .js(&format!("window.__topOf({point})"))
        .as_f64()
        .unwrap();
    assert!(
        (top - start).abs() <= 2.0,
        "{what}: moved {} px",
        (top - start).abs()
    );
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn focus_hides_all_but_the_text_and_every_exit_brings_the_panes_back() {
    let book = generated_book("large-h1-p");
    let mut app = launch(&book);
    let width = app.js("window.innerWidth").as_f64().unwrap();
    let height = app.js("window.innerHeight").as_f64().unwrap();
    let chrome_with_panes = app.js(VISIBLE_CHROME);
    assert!(chrome_with_panes.to_string().contains(".top"));
    let point = scroll_to(&mut app, 0.5);
    let start = point["top"].as_f64().unwrap();

    app.key(KEY_F);
    wait_view(&mut app, "v.focus", "F entering focus");
    assert_eq!(app.js(VISIBLE_CHROME), json!([]), "only the text is left");
    assert_eq!(
        app.js("document.getElementById('exit-focus').hidden"),
        false
    );
    top_stays(&mut app, &point, start, "entering focus");

    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.focus", "Esc leaving focus");
    assert_eq!(app.js(VISIBLE_CHROME), chrome_with_panes);
    top_stays(&mut app, &point, start, "leaving focus");

    app.key(KEY_F);
    wait_view(&mut app, "v.focus", "F");
    app.key(KEY_F);
    wait_view(&mut app, "!v.focus", "F again toggling off");
    app.key_with(KEY_F11, MASK_SHIFT);
    wait_view(&mut app, "v.focus", "Shift F11");
    app.key_with(KEY_F11, MASK_SHIFT);
    wait_view(&mut app, "!v.focus", "Shift F11 again");

    app.js("window.__click('toggle-focus')");
    wait_view(&mut app, "v.focus", "the Focus button");
    app.js("window.__click('exit-focus')");
    wait_view(&mut app, "!v.focus", "the Exit focus button");

    let middle = (width / 2.0, height / 2.0);
    let edges = [
        ("top", middle.0, 2.0),
        ("left", 2.0, middle.1),
        ("bottom", middle.0, height - 2.0),
    ];
    for (name, x, y) in edges {
        app.key(KEY_F);
        wait_view(&mut app, "v.focus", "F");
        app.move_pointer(x, y);
        settle(60);
        assert!(
            in_focus(&mut app),
            "{name}: a pointer already at the edge when focus began stays quiet"
        );
        app.move_pointer(middle.0, middle.1);
        settle(40);
        assert!(in_focus(&mut app), "{name}: the middle stays quiet");
        app.move_pointer(x, y);
        wait_view(&mut app, "!v.focus", name);
        assert_eq!(app.js(VISIBLE_CHROME), chrome_with_panes, "{name}");
        app.move_pointer(middle.0, middle.1);
        settle(30);
    }
    app.key(KEY_F);
    wait_view(&mut app, "v.focus", "F");
    app.move_pointer(middle.0, middle.1);
    settle(30);
    app.move_pointer(width - 2.0, middle.1);
    settle(80);
    assert!(
        in_focus(&mut app),
        "the right edge belongs to the scrollbar"
    );
    app.move_pointer(middle.0, middle.1);
    app.key(KEY_RIGHT);
    let turned = wait_view(&mut app, "v.top===0", "a chapter turn inside focus");
    assert!(
        turned["focus"].as_bool().unwrap(),
        "turning a chapter keeps focus"
    );
    app.key(KEY_SPACE);
    wait_view(&mut app, "v.top>0", "Space in focus");

    app.key_with(KEY_K, MASK_CTRL);
    let over = wait_view(&mut app, "v.palette", "the palette over focus");
    assert!(over["focus"].as_bool().unwrap());
    app.key(KEY_ESCAPE);
    let closed = wait_view(&mut app, "!v.palette", "Esc closes only the palette");
    assert!(
        closed["focus"].as_bool().unwrap(),
        "focus survives the palette's Esc"
    );
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.focus", "the next Esc leaves focus");

    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    app.type_text("focus");
    settle(30);
    app.key(KEY_ENTER);
    wait_view(&mut app, "v.focus&&!v.palette", "the Focus mode command");
    app.key_with(KEY_F, MASK_CTRL);
    wait_until(
        &mut app,
        "!document.getElementById('search-pane').hidden",
        "Ctrl F opening the pane",
    );
    assert!(in_focus(&mut app), "Ctrl F keeps focus");
    assert_eq!(
        app.js(VISIBLE_CHROME),
        json!(["#search-pane"]),
        "only the pane joins the text"
    );
    app.key(KEY_ESCAPE);
    wait_until(
        &mut app,
        "document.getElementById('search-pane').hidden",
        "Esc closing the pane",
    );
    assert!(in_focus(&mut app), "the first Esc keeps focus");
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.focus", "the next Esc leaving focus");
    app.close();
}

fn edit_text(app: &mut App) -> String {
    app.js("document.getElementById('book-host').shadowRoot.querySelector('.ss-body').textContent")
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn in_focus_esc_saves_the_edit_first_and_only_then_brings_the_panes_back() {
    let original = generated_book("toc-nested-nav");
    let dir = scratch_dir().join(format!("reader-focus-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("focus-book.epub");
    std::fs::copy(&original, &copy).unwrap();
    let before = file_hash(&copy);
    let mut app = launch(&copy);

    app.key(KEY_E);
    wait_view(&mut app, "v.mode==='edit'", "Edit");
    settle(40);
    app.key(KEY_F);
    settle(30);
    assert!(!in_focus(&mut app), "plain F types in Edit");
    assert!(edit_text(&mut app).contains('f'));
    app.key_with(KEY_F11, MASK_SHIFT);
    let hidden = wait_view(
        &mut app,
        "v.focus&&v.mode==='edit'",
        "Shift F11 entering focus in Edit",
    );
    assert_eq!(
        app.js(VISIBLE_CHROME),
        json!([]),
        "toolbar and panes are hidden in Edit"
    );
    assert_eq!(
        hidden["active"], "book-host",
        "the caret keeps the keyboard"
    );
    assert_eq!(
        app.js("document.getElementById('exit-focus-key').textContent"),
        "Shift F11",
        "the button tells the truth about the key"
    );
    app.type_text("zq");
    settle(30);
    assert!(
        edit_text(&mut app).contains("zq"),
        "typing goes on in focus"
    );

    app.key(KEY_ESCAPE);
    let saved = wait_view(
        &mut app,
        "v.mode==='read'&&!v.unsaved",
        "the first Esc saving and leaving Edit",
    );
    assert!(
        saved["focus"].as_bool().unwrap(),
        "the first Esc keeps focus"
    );
    assert_ne!(before, file_hash(&copy), "the first Esc wrote the book");
    assert_eq!(
        app.js("document.getElementById('exit-focus-key').textContent"),
        "Esc"
    );
    let after_save = file_hash(&copy);
    app.key(KEY_ESCAPE);
    wait_view(
        &mut app,
        "!v.focus&&v.mode==='read'",
        "the second Esc bringing the panes back",
    );
    assert_eq!(
        after_save,
        file_hash(&copy),
        "the second Esc writes nothing"
    );

    app.key(KEY_F);
    wait_view(&mut app, "v.focus", "F in Read");
    app.key(KEY_E);
    wait_view(
        &mut app,
        "v.focus&&v.mode==='edit'",
        "E inside focus stays in focus",
    );
    settle(40);
    app.type_text("k");
    app.key_with(KEY_F11, MASK_SHIFT);
    let out = wait_view(
        &mut app,
        "!v.focus&&v.mode==='edit'",
        "Shift F11 leaving focus without saving",
    );
    assert!(
        out["unsaved"].as_bool().unwrap(),
        "leaving focus does not save"
    );
    assert_eq!(after_save, file_hash(&copy));
    assert_eq!(
        app.js(VISIBLE_CHROME)
            .as_array()
            .unwrap()
            .iter()
            .filter(|name| *name == ".toolbar")
            .count(),
        1
    );
    app.type_text("m");
    settle(30);
    assert!(edit_text(&mut app).contains("km"), "the caret stayed");

    app.key_with(KEY_F11, MASK_SHIFT);
    wait_view(&mut app, "v.focus", "Shift F11");
    app.move_pointer(300.0, 300.0);
    settle(30);
    app.move_pointer(300.0, 2.0);
    let edge = wait_view(&mut app, "!v.focus", "the edge in Edit");
    assert_eq!(edge["mode"], "edit", "the edge changes no mode");
    assert!(edge["unsaved"].as_bool().unwrap());
    app.move_pointer(300.0, 300.0);
    settle(30);

    app.key_with(KEY_F11, MASK_SHIFT);
    wait_view(&mut app, "v.focus", "Shift F11");
    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    app.type_text("3");
    settle(30);
    app.key(KEY_ENTER);
    wait_view(&mut app, "v.dialog", "a jump with unsaved text asking");
    app.key(KEY_ESCAPE);
    let kept = wait_view(
        &mut app,
        "!v.dialog&&v.mode==='edit'",
        "Esc keeping the edit at the prompt",
    );
    assert!(
        kept["focus"].as_bool().unwrap(),
        "the prompt's Esc leaves focus alone"
    );
    app.key_with(KEY_S, MASK_CTRL);
    wait_view(&mut app, "!v.unsaved", "Ctrl S in focus");
    app.key(KEY_ESCAPE);
    wait_view(
        &mut app,
        "v.mode==='read'&&v.focus",
        "Esc with nothing to save",
    );
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.focus", "the panes");
    app.close();
    let _ = std::fs::remove_dir_all(dir);
}

fn prefs_path(data: &Path) -> PathBuf {
    data.join("prefs.json")
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn prefs_survive_a_restart_and_corrupt_files_fall_back_without_a_crash() {
    let book = generated_book("toc-nested-nav");
    let before = file_hash(&book);
    let data = fresh_data("prefs");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    let defaults = view(&mut app);
    assert_eq!(defaults["fontPx"], "18px");
    assert_eq!(defaults["measure"], "700px");
    assert_eq!(defaults["theme"], "dark");
    app.js("window.__click('toggle-comfort')");
    wait_view(&mut app, "v.comfort", "the text controls opening");
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.comfort", "Esc closing the text controls");
    for step in [
        "window.__click('font-larger')",
        "window.__click('font-larger')",
        "window.__click('font-larger')",
        "window.__slide('layout-width',816)",
        "window.__click('prefs-theme-light')",
        "window.__click('toggle-contents')",
    ] {
        app.js(step);
    }
    wait_file(&prefs_path(&data), "\"contents\":false");
    let chosen = view(&mut app);
    assert_eq!(chosen["fontPx"], "21px");
    assert_eq!(chosen["measure"], "880px");
    assert_eq!(chosen["theme"], "light");
    assert_eq!(chosen["contents"], false);
    app.close();

    let mut again = App::launch_with_data(&book, &data);
    again.wait_page_started();
    again.wait_first_text();
    again.js(INSTALL);
    let shown = view(&mut again);
    assert_eq!(shown["fontPx"], "21px");
    assert_eq!(shown["measure"], "880px");
    assert_eq!(shown["theme"], "light");
    assert_eq!(shown["contents"], false);
    assert_eq!(
        again.js("getComputedStyle(document.body).backgroundColor"),
        "rgb(244, 245, 243)"
    );
    again.js("window.__click('toggle-comfort')");
    assert_eq!(
        again.js("document.getElementById('font-now').textContent"),
        "21px"
    );
    assert_eq!(
        again.js("document.getElementById('layout-width').value"),
        "816"
    );
    assert_eq!(
        again.js("document.getElementById('prefs-theme-light').classList.contains('on')"),
        true
    );
    for _ in 0..40 {
        again.js("window.__click('font-larger')");
    }
    assert_eq!(
        view(&mut again)["fontPx"],
        "28px",
        "the control stops at the range"
    );
    assert_eq!(
        again.js("document.getElementById('font-larger').disabled"),
        true
    );
    again.close();

    for (name, bytes) in [
        ("garbage", b"\xff\xfe\x00not json".as_slice()),
        ("empty", b"".as_slice()),
        ("array", b"[1,2,3]".as_slice()),
        (
            "hostile",
            br#"{"theme":"<img src=x>","fontPx":1e30,"width":"; display:none","contents":null,"marginTop":"1px;color:red","marginLeft":-50,"lineHeight":"x","paraGap":null,"indent":"2em","font":"x;}*{display:none"}"#
                .as_slice(),
        ),
    ] {
        let data = fresh_data(&format!("corrupt-{name}"));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(prefs_path(&data), bytes).unwrap();
        std::fs::write(data.join("library.json"), bytes).unwrap();
        let mut broken = App::launch_with_data(&book, &data);
        ready(&mut broken);
        let shown = view(&mut broken);
        assert_eq!(shown["fontPx"], "18px", "{name}");
        assert_eq!(shown["measure"], "700px", "{name}");
        assert_eq!(shown["theme"], "dark", "{name}");
        assert_eq!(shown["where"], "Position 1 of 6", "{name}");
        assert_eq!(
            broken.js("(()=>{const s=getComputedStyle(document.querySelector('.sheet'));const h=getComputedStyle(document.getElementById('book-host'));return [s.paddingTop,s.paddingRight,s.paddingBottom,s.paddingLeft,h.lineHeight,h.fontFamily.split(',')[0]].join(' ')})()"),
            "44px 32px 72px 32px 28.799999px Literata",
            "{name}"
        );
        assert!(
            broken
                .js("document.getElementById('book-host').shadowRoot.textContent.length")
                .as_u64()
                .unwrap()
                > 100,
            "{name}"
        );
        broken.js("window.__click('font-larger')");
        wait_file(&prefs_path(&data), "\"fontPx\":19");
        broken.close();
        let _ = std::fs::remove_dir_all(&data);
    }

    let blocked = fresh_data("blocked");
    std::fs::create_dir_all(blocked.parent().unwrap()).unwrap();
    std::fs::write(&blocked, b"a file where the data folder should be").unwrap();
    let mut stuck = App::launch_with_data(&book, &blocked);
    ready(&mut stuck);
    stuck.js("window.__click('font-larger')");
    let told = wait_view(
        &mut stuck,
        "v.toast.includes('settings could not be saved')",
        "a readable message",
    );
    assert_eq!(
        told["fontPx"], "19px",
        "the change still applies this session"
    );
    assert!(!told["toast"].as_str().unwrap().contains("os error"), "{told}");
    stuck.close();
    let _ = std::fs::remove_file(&blocked);
    let _ = std::fs::remove_dir_all(&data);
    assert_eq!(before, file_hash(&book));
}

fn layout(app: &mut App) -> Value {
    app.js("window.__layout()")
}

fn number(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap_or_else(|| panic!("{key} in {value}"))
}

fn near(value: &Value, key: &str, wanted: f64) {
    let got = number(value, key);
    assert!((got - wanted).abs() <= 0.5, "{key} is {got}, wanted {wanted}");
}

fn web_novel() -> PathBuf {
    real_books()
        .into_iter()
        .find(|path| book_label(path) == "serial_b1")
        .expect("the web novel fixture")
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn layout_controls_reshape_the_page_come_back_after_a_relaunch_and_reset() {
    let book = web_novel();
    let ix = u64::from(long_chapters(&book, 1)[0]);
    let data = fresh_data("layout");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    go(&mut app, ix);
    let start = layout(&mut app);
    println!("defaults: {start}");
    assert_eq!(start["maxWidth"], "700px", "today's sheet width");
    assert_eq!(start["pad"], "44px 32px 72px 32px", "today's margins");
    near(&start, "lineHeight", 18.0 * 1.6);
    near(&start, "gap", 18.0 * 0.6);
    near(&start, "indent", 0.0);
    assert!(start["face"].as_str().unwrap().contains("Literata"));

    app.js("window.__slide('layout-width',500)");
    let narrow = layout(&mut app);
    near(&narrow, "textWidth", 500.0);
    assert_eq!(narrow["maxWidth"], "564px");

    app.js("window.__slide('layout-marginTop',100)");
    app.js("window.__slide('layout-marginLeft',80)");
    app.js("window.__slide('layout-marginRight',60)");
    app.js("window.__slide('layout-marginBottom',150)");
    let margins = layout(&mut app);
    assert_eq!(margins["pad"], "100px 60px 150px 80px");
    near(&margins, "textTop", number(&narrow, "textTop") + 56.0);
    near(&margins, "textLeft", number(&narrow, "textLeft") + 48.0);
    near(&margins, "textWidth", 500.0);

    app.js("window.__slide('layout-lineHeight',200)");
    app.js("window.__slide('layout-paraGap',200)");
    app.js("window.__slide('layout-indent',20)");
    let spaced = layout(&mut app);
    near(&spaced, "lineHeight", 36.0);
    near(&spaced, "gap", 36.0);
    near(&spaced, "indent", 36.0);

    for (font, family) in [
        ("plexSans", "IBM Plex Sans"),
        ("serif", "serif"),
        ("sans", "sans-serif"),
        ("mono", "monospace"),
    ] {
        app.js(&format!("window.__pick('layout-font','{font}')"));
        let face = layout(&mut app)["face"].as_str().unwrap().to_owned();
        assert!(face.starts_with(family) || face.starts_with(&format!("\"{family}\"")), "{font} gave {face}");
    }
    wait_file(&prefs_path(&data), "\"font\":\"mono\"");
    let chosen = layout(&mut app);
    println!("chosen: {chosen}");
    app.close();

    let mut again = App::launch_with_data(&book, &data);
    ready(&mut again);
    go(&mut again, ix);
    let back = layout(&mut again);
    for key in ["maxWidth", "pad", "textWidth", "textLeft", "textTop", "lineHeight", "gap", "indent", "face", "sliders", "font"] {
        assert_eq!(back[key], chosen[key], "{key} after a relaunch");
    }

    again.js("window.__click('layout-reset')");
    wait_file(&prefs_path(&data), "\"width\":636");
    let reset = layout(&mut again);
    for key in ["maxWidth", "pad", "textWidth", "textLeft", "textTop", "lineHeight", "gap", "indent", "face", "sliders", "font"] {
        assert_eq!(reset[key], start[key], "{key} after Reset layout");
    }
    again.close();
    let _ = std::fs::remove_dir_all(&data);
}

fn wait_fullscreen(app: &mut App, on: bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if app.state()["fullscreen"] == json!(on) {
            break;
        }
        assert!(Instant::now() < deadline, "{what}: the window never became fullscreen={on}");
        settle(20);
    }
    println!(
        "{what}: window fullscreen={on}, compositor fullscreen={:?}",
        window_fullscreen_of(app.pid)
    );
}

fn full_button_on(app: &mut App) -> bool {
    app.js("document.getElementById('toggle-fullscreen').classList.contains('on')")
        .as_bool()
        .unwrap()
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn full_screen_follows_f11_the_panel_and_the_palette_and_esc_leaves_it() {
    let book = generated_book("large-h1-p");
    let mut app = launch(&book);
    wait_fullscreen(&mut app, false, "at launch");
    let point = scroll_to(&mut app, 0.5);
    let start = point["top"].as_f64().unwrap();

    app.key(KEY_F11);
    wait_fullscreen(&mut app, true, "F11 in Read");
    assert!(full_button_on(&mut app));
    assert!(!in_focus(&mut app), "full screen alone keeps the panes");
    top_stays(&mut app, &point, start, "entering full screen");
    app.key(KEY_ESCAPE);
    wait_fullscreen(&mut app, false, "Esc in Read");
    assert!(!full_button_on(&mut app));
    top_stays(&mut app, &point, start, "leaving full screen");

    app.js("window.__click('toggle-comfort')");
    app.js("window.__click('toggle-fullscreen')");
    wait_fullscreen(&mut app, true, "the panel button");
    app.js("window.__click('toggle-fullscreen')");
    wait_fullscreen(&mut app, false, "the panel button again");
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.comfort", "Esc closing the panel");

    app.key_with(KEY_K, MASK_CTRL);
    wait_view(&mut app, "v.palette", "the palette");
    app.type_text("full screen");
    settle(30);
    app.key(KEY_ENTER);
    wait_fullscreen(&mut app, true, "the Full screen command");
    app.key(KEY_F);
    wait_view(&mut app, "v.focus", "F adding focus");
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "!v.focus", "one Esc leaving focus");
    wait_fullscreen(&mut app, false, "the same Esc leaving full screen");

    app.key(KEY_E);
    wait_view(&mut app, "v.mode==='edit'", "Edit");
    app.key(KEY_F11);
    wait_fullscreen(&mut app, true, "F11 in Edit");
    assert!(!view(&mut app)["unsaved"].as_bool().unwrap(), "F11 typed nothing");
    app.key_with(KEY_F11, MASK_SHIFT);
    wait_view(&mut app, "v.focus", "Shift F11 adding focus in Edit");
    app.key_with(KEY_F11, MASK_SHIFT);
    wait_view(&mut app, "!v.focus", "Shift F11 leaving focus in Edit");
    app.key(KEY_F11);
    wait_fullscreen(&mut app, false, "F11 again in Edit");
    app.close();
}

const CSS_AUDIT: &str = r"(()=>{
  const shadow = document.getElementById('book-host').shadowRoot;
  const body = shadow.querySelector('.ss-body');
  const range = document.createRange();
  range.selectNodeContents(body);
  const box = range.getBoundingClientRect();
  const p = shadow.querySelector('p');
  const style = getComputedStyle(p);
  const chain = (node) => { for (let at = node; at; at = at.parentElement) { const c = getComputedStyle(at); if (c.display === 'none' || c.visibility === 'hidden' || Number(c.opacity) === 0) return false; } return true; };
  return { h: box.height, w: box.width, left: box.left, shown: chain(p), color: style.color, back: getComputedStyle(body).backgroundColor, margin: style.marginTop + ' ' + style.marginBottom, indent: style.textIndent, link: shadow.querySelector('a') ? getComputedStyle(shadow.querySelector('a')).color : '' };
})()";

fn chapter_of(reader: &Reader, id: &str) -> u64 {
    let row = reader
        .rows()
        .iter()
        .find(|row| {
            row.href
                .as_ref()
                .is_some_and(|href| href.as_str().ends_with(&format!("/{id}.xhtml")))
        })
        .unwrap();
    u64::from(row.ix.0)
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn the_book_cannot_hide_its_chapter_and_its_own_css_and_fonts_still_apply() {
    let book = generated_book("reader-css");
    let reader = Reader::open(File::open(&book).unwrap(), DEFAULT_LIMITS).unwrap();
    let mut app = launch(&book);
    for id in [
        "hide_body",
        "hide_important",
        "hide_scope",
        "hide_strong",
        "hide_outrank",
        "hide_inline",
    ] {
        go(&mut app, chapter_of(&reader, id));
        let seen = app.js(CSS_AUDIT);
        assert!(
            seen["h"].as_f64().unwrap() > 40.0 && seen["w"].as_f64().unwrap() > 100.0,
            "{id}: {seen}"
        );
        assert!(seen["left"].as_f64().unwrap() > 0.0, "{id}: {seen}");
        assert_eq!(seen["shown"], true, "{id}: {seen}");
        println!("{id}: shown");
    }
    go(&mut app, chapter_of(&reader, "hidden_note"));
    assert_eq!(app.js("getComputedStyle(document.getElementById('book-host').shadowRoot.querySelector('p.note')).display"), "none", "a book may still hide a note");
    go(&mut app, chapter_of(&reader, "own_margin"));
    let own = app.js(CSS_AUDIT);
    assert_eq!(
        own["margin"], "0px 0px",
        "the book's margin beats the shell's default"
    );
    assert_eq!(own["indent"], "36px");

    go(&mut app, chapter_of(&reader, "ink_dark"));
    let dark = app.js(CSS_AUDIT);
    assert_eq!(
        dark["color"], "rgb(232, 230, 225)",
        "dark theme ink wins over book ink"
    );
    assert_eq!(
        dark["back"], "rgba(0, 0, 0, 0)",
        "dark theme drops the book's white page"
    );
    assert_eq!(dark["link"], "rgb(143, 211, 182)");
    go(&mut app, chapter_of(&reader, "ink_outrank"));
    let outranked = app.js(CSS_AUDIT);
    assert_eq!(
        outranked["color"], "rgb(232, 230, 225)",
        "dark theme ink wins over a book rule with more ids than the guard"
    );
    go(&mut app, chapter_of(&reader, "ink_dark"));
    app.js("window.__click('prefs-theme-light')");
    let light = app.js(CSS_AUDIT);
    assert_eq!(
        light["color"], "rgb(0, 0, 0)",
        "light theme keeps the book's colours"
    );
    assert_eq!(light["link"], "rgb(0, 0, 238)");
    app.js("window.__click('prefs-theme-dark')");

    let chrome_width = "(()=>{const r=document.createRange();r.selectNodeContents(document.getElementById('count'));const n=document.getElementById('crumb');r.selectNodeContents(n);return r.getBoundingClientRect().width})()";
    let before = app.js(chrome_width).as_f64().unwrap();
    go(&mut app, chapter_of(&reader, "font_face"));
    wait_until(
        &mut app,
        "(async()=>{await document.fonts.ready;return true})()",
        "fonts",
    );
    let font = app.js(r"(()=>{const s=document.getElementById('book-host').shadowRoot;return {probe:s.getElementById('probe').getBoundingClientRect().width,control:s.getElementById('control').getBoundingClientRect().width,family:getComputedStyle(s.getElementById('probe')).fontFamily}})()");
    println!("book font: {font}");
    let expected = 10.0 * 0.6 * 40.0;
    assert!(
        (font["probe"].as_f64().unwrap() - expected).abs() < 2.0,
        "the book font loaded: {font}"
    );
    assert!(
        (font["control"].as_f64().unwrap() - expected).abs() > 20.0,
        "the control proves the width differs: {font}"
    );

    go(&mut app, chapter_of(&reader, "font_shell_name"));
    wait_until(
        &mut app,
        "(async()=>{await document.fonts.ready;return true})()",
        "fonts",
    );
    settle(150);
    let after = app.js(chrome_width).as_f64().unwrap();
    println!(
        "shell text width {before:.1} before and {after:.1} after a book font borrowed its name"
    );
    assert!(
        (after - before).abs() < 0.6,
        "a book font changed the shell: {before} then {after}"
    );
    assert_eq!(app.state()["pwned"], json!([]));
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output, real books, counts only"]
fn dark_theme_text_is_readable_on_real_books() {
    const AUDIT: &str = r"(()=>{
      const shadow = document.getElementById('book-host').shadowRoot;
      const root = shadow.querySelector('.ss-body');
      const out = { total: 0, low: 0 };
      if (!root) return out;
      const parse = (text) => { const m = text.match(/rgba?\(([^)]+)\)/); return m ? m[1].split(',').map(Number) : null; };
      const lum = ([r, g, b]) => { const f = (v) => { const c = v / 255; return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
      const page = parse(getComputedStyle(document.querySelector('.app')).backgroundColor);
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      while (walker.nextNode() && out.total < 3000) {
        const node = walker.currentNode;
        if (!/\S/.test(node.data)) continue;
        const style = getComputedStyle(node.parentElement);
        if (style.display === 'none' || style.visibility === 'hidden') continue;
        out.total += 1;
        let back = null;
        for (let at = node.parentElement; at && !back; at = at.parentElement) { const c = parse(getComputedStyle(at).backgroundColor); if (c && (c.length < 4 || c[3] > 0)) back = c; }
        const a = lum(parse(style.color));
        const b = lum(back ?? page);
        if ((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05) < 3) out.low += 1;
      }
      return out;
    })()";
    for book in real_books() {
        let rows = Reader::open(File::open(&book).unwrap(), DEFAULT_LIMITS)
            .unwrap()
            .rows()
            .len();
        let mut app = launch(&book);
        let step = (rows / 40).max(1);
        let (mut total, mut low) = (0, 0);
        for ix in (0..rows).step_by(step) {
            go(&mut app, ix as u64);
            let seen = app.js(AUDIT);
            total += seen["total"].as_u64().unwrap();
            low += seen["low"].as_u64().unwrap();
        }
        println!(
            "{}: {total} text runs sampled, {low} under 3:1 on the dark theme",
            book_label(&book)
        );
        assert_eq!(low, 0, "{}", book_label(&book));
        app.close();
    }
}

const TEXT_SIZES: &str = r"[...document.getElementById('book-host').shadowRoot.querySelectorAll('[data-ss-b]')].filter((b)=>b.textContent.trim().length>0).map((b)=>parseFloat(getComputedStyle(b).fontSize))";

fn sizes(app: &mut App) -> Vec<f64> {
    app.js(TEXT_SIZES)
        .as_array()
        .unwrap()
        .iter()
        .map(|size| size.as_f64().unwrap())
        .collect()
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output, real books, counts only"]
fn the_size_control_changes_every_text_block_on_every_real_book() {
    for book in real_books() {
        let mut app = launch(&book);
        let mut blocks = 0;
        let mut stuck = 0;
        for ix in long_chapters(&book, 3) {
            go(&mut app, u64::from(ix));
            let before = sizes(&mut app);
            for _ in 0..4 {
                app.js("window.__click('font-larger')");
            }
            let after = sizes(&mut app);
            for _ in 0..4 {
                app.js("window.__click('font-smaller')");
            }
            assert_eq!(before.len(), after.len());
            blocks += before.len();
            stuck += before
                .iter()
                .zip(&after)
                .filter(|(was, now)| **now < **was * 1.1)
                .count();
        }
        println!(
            "{}: {stuck} of {blocks} text blocks ignored the size control",
            book_label(&book)
        );
        assert_eq!(stuck, 0, "{}: blocks kept their size", book_label(&book));
        app.close();
    }
}

fn wheel_down(app: &mut App, steps: u32) {
    let spot = app.js("(()=>{const b=document.getElementById('scroller').getBoundingClientRect();return {x:b.left+b.width/2,y:b.top+b.height/2}})()");
    app.op(&json!({"op": "wheel", "x": spot["x"], "y": spot["y"], "steps": steps}));
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn a_wheel_scroll_right_before_closing_is_the_place_that_comes_back() {
    let book = real_books()
        .into_iter()
        .find(|p| book_label(p) == "serial_a")
        .expect("the real book");
    let data = fresh_data("last-wheel");
    let ix = long_chapters(&book, 4)[1];
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    go(&mut app, u64::from(ix));
    scroll_to(&mut app, 0.2);
    settle(700);
    let start = view(&mut app)["top"].as_f64().unwrap();
    wheel_down(&mut app, 5);
    settle(700);
    let travel = view(&mut app)["top"].as_f64().unwrap() - start;
    assert!(travel > 200.0, "the wheel scrolled {travel} px");
    scroll_to(&mut app, 0.4);
    settle(700);
    let start = view(&mut app)["top"].as_f64().unwrap();
    wheel_down(&mut app, 5);
    app.request_close_through_the_compositor();
    assert!(app.wait_exit(Duration::from_secs(5)), "the window closed");
    drop(app);

    let mut next = App::launch_with_data(&book, &data);
    ready(&mut next);
    next.js("document.fonts.ready.then(()=>new Promise((done)=>requestAnimationFrame(()=>requestAnimationFrame(()=>done(true)))))");
    settle(150);
    let shown = view(&mut next);
    let back = shown["top"].as_f64().unwrap() - start;
    println!("the wheel travels {travel} px; after a close mid-scroll the place came back {back} px past the start");
    assert!(
        (back - travel).abs() <= 2.0 * shown["line"].as_f64().unwrap() + 2.0,
        "the scroll right before closing was lost: {back} of {travel} px"
    );
    next.close();
    let _ = std::fs::remove_dir_all(&data);
}

const EXPANDED: &str = "['toggle-contents','hide-contents'].map((id)=>document.getElementById(id).getAttribute('aria-expanded'))";

fn text_width(app: &mut App) -> f64 {
    app.js("document.getElementById('book-host').shadowRoot.querySelector('.ss-body').getBoundingClientRect().width").as_f64().unwrap()
}

fn moved_from(app: &mut App, point: &Value) -> f64 {
    let top = app
        .js(&format!("window.__topOf({point})"))
        .as_f64()
        .expect("the point resolves");
    (top - point["top"].as_f64().unwrap()).abs()
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn contents_hide_and_show_from_ctrl_t_the_top_bar_and_the_rail_keeping_the_place() {
    let book = real_books()
        .into_iter()
        .find(|p| book_label(p) == "serial_a")
        .expect("the real book");
    let before = file_hash(&book);
    let data = fresh_data("contents-toggle");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    app.js("window.__slide('layout-width',1400)");
    let mut point = Value::Null;
    for ix in long_chapters(&book, 6) {
        go(&mut app, u64::from(ix));
        let shown = view(&mut app);
        if shown["room"].as_f64().unwrap() < shown["height"].as_f64().unwrap() * 2.0 {
            continue;
        }
        point = scroll_to(&mut app, 0.41);
        if !point.is_null() {
            break;
        }
    }
    assert!(!point.is_null(), "a long chapter to read in");
    assert_eq!(app.js(EXPANDED), json!(["true", "true"]));
    let with_rail = text_width(&mut app);

    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "!v.contents", "Ctrl T hiding the contents");
    assert_eq!(app.js(EXPANDED), json!(["false", "false"]));
    let without_rail = text_width(&mut app);
    assert!(
        without_rail > with_rail,
        "the text column widens: {with_rail} to {without_rail}"
    );
    let moved = moved_from(&mut app, &point);
    assert!(moved <= 2.0, "hiding moved the text {moved} px");
    wait_file(&prefs_path(&data), "\"contents\":false");

    app.js("window.__click('toggle-contents')");
    wait_view(&mut app, "v.contents", "the top bar showing the contents");
    assert_eq!(app.js(EXPANDED), json!(["true", "true"]));
    let back = text_width(&mut app);
    assert!((back - with_rail).abs() <= 1.0, "the column narrows back: {back}");
    let moved = moved_from(&mut app, &point);
    assert!(moved <= 2.0, "showing moved the text {moved} px");

    app.js("window.__click('hide-contents')");
    wait_view(&mut app, "!v.contents", "the rail button hiding the contents");
    assert_eq!(app.js(EXPANDED), json!(["false", "false"]));
    wait_file(&prefs_path(&data), "\"contents\":false");
    app.close();

    let mut again = App::launch_with_data(&book, &data);
    again.wait_page_started();
    again.wait_first_text();
    again.js(INSTALL);
    assert_eq!(view(&mut again)["contents"], false, "hidden after a restart");
    assert_eq!(again.js(EXPANDED), json!(["false", "false"]));
    again.close();
    assert_eq!(file_hash(&book), before, "the book is untouched");
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn ctrl_t_works_in_edit_and_fields_and_rename_and_focus_mode_bring_the_contents_back() {
    let book = generated_book("toc-nested-nav");
    let before = file_hash(&book);
    let data = fresh_data("contents-edit");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);

    app.js("document.getElementById('filter').focus()");
    wait_view(&mut app, "v.active==='filter'", "the filter focused");
    app.key_with(KEY_T, MASK_CTRL);
    wait_view(
        &mut app,
        "!v.contents && v.active!=='filter'",
        "Ctrl T in the filter hiding the contents and leaving the field",
    );
    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "v.contents", "Ctrl T showing the contents");

    app.key(KEY_E);
    wait_view(&mut app, "v.mode==='edit'", "Edit");
    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "!v.contents && v.mode==='edit'", "Ctrl T hiding the contents in Edit");
    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "v.contents && v.mode==='edit'", "Ctrl T showing the contents in Edit");
    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "!v.contents", "the contents hidden again");
    app.key(KEY_F2);
    wait_view(
        &mut app,
        "v.contents && v.active==='rail-rename'",
        "F2 showing the contents with the rename field focused",
    );
    assert_eq!(app.js(EXPANDED), json!(["true", "true"]));
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "v.active!=='rail-rename' && v.mode==='edit'", "Esc leaving the rename field");
    app.key(KEY_ESCAPE);
    wait_view(&mut app, "v.mode==='read' && !v.unsaved", "Read");

    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "!v.contents", "the contents hidden in Read");
    app.key(KEY_F);
    wait_view(&mut app, "v.focus", "focus mode");
    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "!v.focus && v.contents", "Ctrl T leaving focus mode with the contents shown");
    assert_eq!(app.js(EXPANDED), json!(["true", "true"]));
    let pane = "!document.getElementById('search-pane').hidden";
    let query = "document.getElementById('search-input').value";
    app.key_with(KEY_F, MASK_CTRL);
    wait_until(&mut app, pane, "Ctrl F opening Search in the panel");
    app.js("(()=>{const i=document.getElementById('search-input');i.value='the';i.dispatchEvent(new Event('input',{bubbles:true}));return true})()");
    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "!v.contents", "one Ctrl T on the Search tab closing the panel");
    assert_eq!(app.js(pane), json!(false), "Search closes with the panel");
    app.key_with(KEY_T, MASK_CTRL);
    wait_until(&mut app, pane, "Ctrl T reopening the panel on Search");
    assert_eq!(app.js(query), json!("the"), "the search is kept");
    assert_eq!(
        app.js("document.querySelector('.app').classList.contains('find-bar')"),
        json!(false),
        "Search comes back docked"
    );
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "document.getElementById('search-pane').hidden", "Esc closing Search");
    app.js("document.getElementById('tab-contents').click()");
    wait_view(&mut app, "v.contents", "the Contents tab");
    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "!v.contents", "Ctrl T closing the panel on Contents");
    app.key_with(KEY_T, MASK_CTRL);
    wait_view(&mut app, "v.contents", "Ctrl T reopening the panel on Contents");
    assert_eq!(app.js(pane), json!(false), "Contents comes back, not Search");
    app.close();
    assert_eq!(file_hash(&book), before, "the book is untouched");
    let _ = std::fs::remove_dir_all(&data);
}

const KEY_F1: u32 = 0xffbe;
const KEY_D: u32 = 0x64;

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn f1_lists_every_shortcut_and_the_header_tooltips_name_their_keys() {
    let mut app = launch(&generated_book("large-h1-p"));
    let tips = app.js("Object.fromEntries(['toggle-contents','open-palette','toggle-search','toggle-notes','toggle-focus','mode-edit','go-library'].map((id)=>[id,document.getElementById(id).title]))");
    assert_eq!(tips["toggle-contents"], "Contents (Ctrl T)");
    assert_eq!(tips["open-palette"], "Jump (Ctrl K or Ctrl Shift P)");
    assert_eq!(tips["toggle-search"], "Search (Ctrl F)");
    assert_eq!(tips["toggle-notes"], "Notes (Ctrl D)");
    assert_eq!(tips["toggle-focus"], "Focus (F)");
    assert_eq!(tips["mode-edit"], "Edit (E)");
    assert_eq!(tips["go-library"], "Library (Ctrl L)");
    let names = app.js("[...document.querySelectorAll('#reader .top button')].filter((b)=>!b.textContent.trim()&&!b.getAttribute('aria-label')).map((b)=>b.id)");
    assert_eq!(names, json!([]), "every icon button has a name");

    app.key(KEY_F1);
    let sheet = wait_until(
        &mut app,
        "(async()=>{const keys=await import('/keys.js');return !document.getElementById('keys-dialog').hidden&&{rows:document.querySelectorAll('#keys-list .keyrow').length,listed:keys.BINDINGS.length,text:document.getElementById('keys-list').textContent}})()",
        "the shortcut sheet",
    );
    assert_eq!(sheet["rows"], sheet["listed"], "one row per binding");
    assert!(sheet["text"].as_str().unwrap().contains("Ctrl T"));
    app.key(KEY_ESCAPE);
    wait_until(&mut app, "document.getElementById('keys-dialog').hidden", "Esc closing the sheet");

    app.key_with(KEY_D, MASK_CTRL);
    wait_until(&mut app, "!document.getElementById('notes-pane').hidden&&document.getElementById('toggle-notes').classList.contains('on')", "Ctrl D opening notes");
    app.key_with(KEY_D, MASK_CTRL);
    wait_until(&mut app, "document.getElementById('notes-pane').hidden", "Ctrl D closing notes");
    app.key(KEY_E);
    wait_view(&mut app, "v.mode==='edit'", "Edit");
    assert_eq!(
        app.js("document.getElementById('toggle-focus').title"),
        "Focus (Shift F11)",
        "the tooltip follows the mode"
    );
    app.close();
}

const KEY_EQUAL: u32 = 0x3d;
const KEY_MINUS: u32 = 0x2d;
const KEY_ZERO: u32 = 0x30;

const WORDS_ON_SCREEN: &str = r"(()=>{
  const host = document.getElementById('book-host');
  const view = document.getElementById('scroller').getBoundingClientRect();
  const walker = document.createTreeWalker(host.shadowRoot, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  let words = 0;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    for (const match of node.data.matchAll(/\S+/g)) {
      range.setStart(node, match.index);
      range.setEnd(node, match.index + match[0].length);
      const box = range.getBoundingClientRect();
      if (box.width > 0 && box.top >= view.top && box.bottom <= view.bottom) words += 1;
    }
  }
  return words;
})()";

const CHROME: &str = r"(()=>({
  zoomed: window.innerWidth,
  ratio: window.devicePixelRatio,
  scale: window.visualViewport.scale,
  button: document.getElementById('toggle-contents').getBoundingClientRect().height,
  bar: document.querySelector('#reader .top').getBoundingClientRect().height,
  readout: document.getElementById('size-readout').hidden ? '' : document.getElementById('size-readout').textContent,
}))()";

fn serial() -> PathBuf {
    real_books()
        .into_iter()
        .find(|path| book_label(path) == "serial_a")
        .expect("serial_a in fixtures/real")
}

fn ctrl_wheel(app: &mut App, up: bool, steps: u32) {
    let spot = app.js("(()=>{const b=document.getElementById('scroller').getBoundingClientRect();return {x:b.left+b.width/2,y:b.top+b.height/2}})()");
    app.op(&json!({"op": "wheel", "x": spot["x"], "y": spot["y"], "steps": steps, "state": MASK_CTRL, "up": up}));
}

fn words_at_top(app: &mut App) -> u64 {
    app.js("document.getElementById('scroller').scrollTop=0");
    app.js("(async()=>{await document.fonts.ready;await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));return true})()");
    app.js(WORDS_ON_SCREEN).as_u64().unwrap()
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn ctrl_wheel_and_ctrl_keys_size_the_text_keep_the_place_and_never_zoom_the_page() {
    let book = serial();
    let mut app = launch(&book);
    go(&mut app, u64::from(long_chapters(&book, 1)[0]));
    let point = scroll_to(&mut app, 0.41);
    assert!(!point.is_null(), "a line at the top");
    let start_top = point["top"].as_f64().unwrap();
    let chrome = app.js(CHROME);
    let zoom = app.state()["zoom"].clone();
    assert_eq!(zoom, json!(1.0), "WebKit starts unzoomed");
    let mut checks = 0;
    for (step, size, readout) in [
        ("wheel up", "19px", "19 px"),
        ("wheel up", "20px", "20 px"),
        ("wheel down", "19px", "19 px"),
        ("ctrl -", "18px", "18 px"),
        ("ctrl -", "17px", "17 px"),
        ("ctrl =", "18px", "18 px"),
        ("wheel up", "19px", "19 px"),
        ("ctrl 0", "18px", "18 px"),
    ] {
        match step {
            "wheel up" => ctrl_wheel(&mut app, true, 1),
            "wheel down" => ctrl_wheel(&mut app, false, 1),
            "ctrl -" => app.key_with(KEY_MINUS, MASK_CTRL),
            "ctrl =" => app.key_with(KEY_EQUAL, MASK_CTRL),
            _ => app.key_with(KEY_ZERO, MASK_CTRL),
        }
        let shown = wait_view(&mut app, &format!("v.fontPx === '{size}'"), step);
        let now = app.js(CHROME);
        assert_eq!(now["readout"], readout, "{step}: the size shows briefly");
        for key in ["zoomed", "ratio", "scale", "button", "bar"] {
            assert_eq!(now[key], chrome[key], "{step}: the chrome keeps its {key}");
        }
        assert_eq!(app.state()["zoom"], zoom, "{step}: WebKit's page zoom never moves");
        let top = app.js(&format!("window.__topOf({point})")).as_f64().unwrap();
        assert!((top - start_top).abs() <= 2.0, "{step}: the top line moved {} px {shown}", top - start_top);
        checks += 1;
    }
    wait_until(&mut app, "document.getElementById('size-readout').hidden", "the readout going away");

    app.key(KEY_F);
    wait_view(&mut app, "v.focus", "focus mode");
    ctrl_wheel(&mut app, true, 1);
    wait_view(&mut app, "v.fontPx === '19px'", "Ctrl and the wheel in focus");
    app.key(KEY_E);
    wait_view(&mut app, "v.mode === 'edit'", "Edit");
    ctrl_wheel(&mut app, false, 2);
    let edit = wait_view(&mut app, "v.fontPx === '17px'", "Ctrl and the wheel in Edit");
    assert_eq!(edit["mode"], "edit");
    assert_eq!(app.state()["zoom"], zoom, "no page zoom in Edit");
    println!("ctrl size: {checks} steps held the top line, chrome {chrome}");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh reader, headless output"]
fn spacing_presets_and_new_defaults_fit_more_words_on_a_web_novel_screen() {
    let book = serial();
    let data = fresh_data("presets");
    let mut app = App::launch_with_data(&book, &data);
    ready(&mut app);
    go(&mut app, 1840);
    wait_until(&mut app, "document.getElementById('book-host').shadowRoot.textContent.includes('Chapter 1818')", "chapter 1818 at spine item 1840");
    let on = |app: &mut App, id: &str| app.js(&format!("document.getElementById('{id}').classList.contains('on')"));
    let defaults = view(&mut app);
    assert!((defaults["line"].as_f64().unwrap() - 18.0 * 1.6).abs() < 0.5, "{defaults}");
    assert_eq!(on(&mut app, "preset-comfortable"), true, "the defaults are Comfortable");
    assert_eq!(on(&mut app, "preset-compact"), false);
    let comfortable = words_at_top(&mut app);

    app.js("window.__click('preset-compact')");
    let compact_view = wait_view(&mut app, "v.fontPx === '17px'", "Compact");
    assert!((compact_view["line"].as_f64().unwrap() - 17.0 * 1.5).abs() < 0.5, "{compact_view}");
    assert_eq!(on(&mut app, "preset-compact"), true);
    assert_eq!(on(&mut app, "preset-comfortable"), false);
    wait_file(&prefs_path(&data), "\"fontPx\":17");
    let saved = std::fs::read_to_string(prefs_path(&data)).unwrap();
    assert!(saved.contains("\"lineHeight\":150") && saved.contains("\"paraGap\":40"), "{saved}");
    let compact_words = words_at_top(&mut app);

    app.js("window.__click('font-larger')");
    wait_view(&mut app, "v.fontPx === '18px'", "a size step");
    assert_eq!(on(&mut app, "preset-compact"), false, "a changed value leaves the preset");

    app.js("window.__click('font-larger')");
    app.js("window.__slide('layout-lineHeight',172)");
    app.js("window.__slide('layout-paraGap',105)");
    wait_view(&mut app, "v.fontPx === '19px'", "the first defaults");
    let first = words_at_top(&mut app);
    app.js("window.__click('preset-comfortable')");
    wait_view(&mut app, "v.fontPx === '18px'", "Comfortable");
    assert_eq!(words_at_top(&mut app), comfortable, "Comfortable is the default look");
    println!("words on screen, chapter 1818 at 1256x750: first defaults {first}, comfortable {comfortable}, compact {compact_words} {compact_view}");
    assert!(comfortable as f64 >= first as f64 * 1.2, "{first} -> {comfortable}");
    assert!(compact_words > comfortable, "{comfortable} -> {compact_words}");
    app.close();
}
