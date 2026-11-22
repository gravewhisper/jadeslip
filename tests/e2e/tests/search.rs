use serde_json::{Value, json};
use jadeslip_e2e::{
    App, KEY_ESCAPE, MASK_CTRL, generated_book, largest_real_book, repo_root, scratch_dir,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const KEY_F: u32 = 0x66;
const KEY_E: u32 = 0x65;
const KEY_T: u32 = 0x74;
const QUADRATIC: &str = "(?s)(?=.*QQQ)x";
const FRAME_MS: f64 = 16.7;

const INSTALL: &str = r"(async()=>{
  const main = await import('/main.js');
  const ipc = await import('/ipc.js');
  const element = (id) => document.getElementById(id);
  const input = element('search-input');
  const highlight = (name) => {
    const found = CSS.highlights.get(name);
    return found ? [...found.values()].map((range) => range.toString()) : [];
  };
  let frames = null;
  window.__errors = [];
  window.addEventListener('error', (event) => window.__errors.push(event.message));
  window.addEventListener('unhandledrejection', (event) => window.__errors.push(String(event.reason?.message ?? event.reason)));
  window.__s = {
    main, ipc,
    state: () => {
      const pane = element('search-pane');
      const rows = [...element('search-results').querySelectorAll('.result')].filter((row) => !row.hidden);
      const list = element('search-results').getBoundingClientRect();
      const visible = rows.map((row) => {
        const box = row.getBoundingClientRect();
        const middle = box.top + box.height / 2;
        const shown = middle > list.top && middle < list.bottom;
        return {
          index: Number(row.dataset.index),
          where: row.querySelector('.where').textContent,
          mark: row.querySelector('.mark').textContent,
          text: row.querySelector('.snip').textContent,
          x: box.left + box.width / 2,
          y: box.top + box.height / 2,
          top: box.top,
          shown,
        };
      });
      const current = CSS.highlights.get('ss-search-current');
      let inView = null;
      if (current?.size) {
        const spot = [...current.values()][0].getBoundingClientRect();
        const page = element('scroller').getBoundingClientRect();
        inView = spot.height > 0 && spot.top >= page.top && spot.bottom <= page.bottom;
      }
      return {
        open: !pane.hidden,
        toggleOn: element('toggle-search').classList.contains('on'),
        recentShown: !element('search-memory').hidden,
        focused: document.activeElement === input,
        value: input.value,
        footer: element('search-count').textContent,
        scanned: element('search-status').hidden ? '' : element('search-scanned').textContent,
        scannedFits: element('search-scanned').scrollWidth <= element('search-scanned').clientWidth,
        matchCase: element('search-case')?.getAttribute('aria-pressed') ?? 'missing',
        range: Object.fromEntries(['before', 'here', 'after'].map((side) => {
          const button = element(`search-${side}`);
          return [side, { label: button.textContent, hint: button.title, checked: button.classList.contains('on'), disabled: button.disabled }];
        })),
        placeholder: input.placeholder,
        caseLabel: element('search-case')?.getAttribute('aria-label') ?? '',
        action: element('search-action').textContent,
        message: element('search-message').hidden ? '' : element('search-message').textContent,
        rows: visible,
        domRows: rows.length,
        spacer: element('search-spacer').getBoundingClientRect().height,
        where: element('where-title').title,
        current: highlight('ss-search-current'),
        hits: highlight('ss-search'),
        inView,
        readOn: element('mode-read').classList.contains('on'),
        editOn: element('mode-edit').classList.contains('on'),
        focusOn: document.querySelector('.app').classList.contains('focus'),
        dialog: !element('dialog').hidden,
        unsaved: !element('unsaved').hidden,
        toast: element('toast').hidden ? '' : element('toast').textContent,
        errors: window.__errors,
      };
    },
    setQuery: (text) => {
      input.value = text;
      input.dispatchEvent(new Event('input', { bubbles: true }));
      return true;
    },
    setMode: (regex) => { if (element('search-regex').classList.contains('on') !== regex) element('search-regex').click(); return true; },
    clickCase: () => { element('search-case')?.click(); return true; },
    tick: (side, on) => {
      const button = element(`search-${side}`);
      if (button.classList.contains('on') !== on) button.click();
      return window.__s.state();
    },
    goTo: (ix) => main.goTo(ix).then(() => true),
    scrollList: (top) => { element('search-results').scrollTop = top; return true; },
    rowsMissingText: () => {
      const list = element('search-results');
      const box = list.getBoundingClientRect();
      const shown = [...list.querySelectorAll('.result')].filter((row) => {
        if (row.hidden) return false;
        const at = row.getBoundingClientRect();
        return at.bottom > box.top && at.top < box.bottom;
      });
      const expected = Math.floor(list.clientHeight / 84);
      return { shown: shown.length, expected, blank: shown.filter((row) => row.querySelector('.mark').textContent.length === 0).length };
    },
    startFrames: () => {
      const sampling = { times: [], running: true };
      frames = sampling;
      const tick = (now) => {
        if (!sampling.running) return;
        sampling.times.push(now);
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
      return true;
    },
    stopFrames: () => {
      frames.running = false;
      const gaps = frames.times.slice(1).map((at, ix) => at - frames.times[ix]).sort((a, b) => a - b);
      const at = (share) => gaps[Math.min(gaps.length - 1, Math.floor(gaps.length * share))] ?? 0;
      return { frames: gaps.length, p50: at(0.5), p99: at(0.99), max: gaps[gaps.length - 1] ?? 0 };
    },
    timeSearch: async (id, text, regex) => {
      const before = performance.now();
      await ipc.searchStart(id, { text, regex, caseSensitive: false, wholeWord: false }, { kind: 'whole' });
      let hits = 0;
      for (;;) {
        const batch = await ipc.searchNext(id);
        for (const group of batch.groups) hits += group.hits.length;
        if (batch.end) return { ms: performance.now() - before, hits, kind: batch.end.kind, done: batch.done, total: batch.total };
      }
    },
    renderLatency: async (ix) => {
      const before = performance.now();
      await ipc.renderChapter(ix);
      return performance.now() - before;
    },
    cancelLatency: async (id, text) => {
      await ipc.searchStart(id, { text, regex: false, caseSensitive: false, wholeWord: false }, { kind: 'whole' });
      await new Promise((done) => setTimeout(done, 60));
      const before = performance.now();
      await ipc.searchCancel(id);
      for (;;) {
        const batch = await ipc.searchNext(id);
        if (batch.end) return { ms: performance.now() - before, kind: batch.end.kind, done: batch.done };
      }
    },
  };
  return true;
})()";

fn launch_with(book: &Path, env: &[(&str, &str)]) -> App {
    let mut command = Command::new(jadeslip_e2e::e2e_binary());
    command.arg(book);
    for (name, value) in env {
        command.env(name, value);
    }
    let mut app = App::spawn(command, &[]);
    app.wait_first_text();
    assert_eq!(app.js(INSTALL), json!(true));
    app
}

fn launch(book: &Path) -> App {
    launch_with(book, &[])
}

fn state(app: &mut App) -> Value {
    app.js("window.__s.state()")
}

fn without_book_text(state: &Value) -> Value {
    json!({
        "open": state["open"],
        "action": state["action"],
        "scanned": state["scanned"],
        "domRows": state["domRows"],
        "dialog": state["dialog"],
        "readOn": state["readOn"],
        "editOn": state["editOn"],
        "focusOn": state["focusOn"],
        "errors": state["errors"],
    })
}

fn wait_for(app: &mut App, what: &str, limit: Duration, test: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let now = state(app);
        if test(&now) {
            return now;
        }
        assert!(
            started.elapsed() < limit,
            "timed out waiting for {what}: {}",
            without_book_text(&now)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn finished(state: &Value) -> bool {
    state["action"] == "Rerun" || !state["message"].as_str().unwrap_or_default().is_empty()
}

fn set_query(app: &mut App, text: &str) {
    app.js(&format!("window.__s.setQuery({})", json!(text)));
}

fn search_and_wait(app: &mut App, text: &str) -> Value {
    set_query(app, text);
    std::thread::sleep(Duration::from_millis(5));
    wait_for(
        app,
        "the search to finish",
        Duration::from_secs(60),
        |now| {
            now["value"] == text
                && finished(now)
                && !now["scanned"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("0 of 0")
        },
    )
}

fn footer_count(state: &Value) -> u64 {
    state["footer"]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .find_map(|word| word.trim_end_matches('.').replace(',', "").parse().ok())
        .unwrap_or(0)
}

fn truth_of(name: &str) -> Value {
    let path = repo_root()
        .join("fixtures/generated")
        .join(format!("{name}.truth.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn workers_of(parent: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let Ok(tasks) = fs::read_dir(format!("/proc/{parent}/task")) else {
        return found;
    };
    for task in tasks.flatten() {
        if let Ok(text) = fs::read_to_string(task.path().join("children")) {
            for pid in text
                .split_whitespace()
                .filter_map(|pid| pid.parse::<u32>().ok())
            {
                let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
                if comm.starts_with("jadeslip-wor") {
                    found.push(pid);
                }
            }
        }
    }
    found
}

fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit(')')
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            != Some("Z")
    })
}

fn rss_kib(pid: u32) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmRSS"))
                .and_then(|line| line.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn normal(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn pixels_matching(path: &Path, rgb: [u8; 3]) -> impl Fn(&Value) -> u32 {
    let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    let channels = info.color_type.samples();
    let (width, stride) = (info.width as usize, info.line_size);
    move |rect: &Value| {
        let bound = |name: &str| rect[name].as_f64().unwrap() as usize;
        let mut found = 0;
        for y in bound("top")..bound("bottom") {
            for x in bound("left")..bound("right").min(width) {
                let pixel = &buffer[y * stride + x * channels..][..3];
                if pixel
                    .iter()
                    .zip(rgb)
                    .all(|(have, want)| have.abs_diff(want) <= 12)
                {
                    found += 1;
                }
            }
        }
        found
    }
}

fn hit_rect(app: &mut App, name: &str) -> Value {
    let script = format!(
        "(()=>{{const box=[...CSS.highlights.get('{name}').values()][0].getBoundingClientRect();return {{left:box.left,right:box.right,top:box.top,bottom:box.bottom}}}})()"
    );
    app.js(&script)
}

fn click_row(app: &mut App, row: &Value) {
    let (x, y) = (row["x"].as_f64().unwrap(), row["y"].as_f64().unwrap());
    app.click(x, y);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn ctrl_f_opens_the_pane_and_typing_streams_results_that_navigate_and_highlight() {
    let name = "large-h1-p";
    let truth = truth_of(name);
    let mut app = launch(&generated_book(name));
    app.assert_on_headless_output();
    let (term, facts) = truth["terms"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, facts)| facts["class"] == "plain")
        .unwrap();
    assert!(!state(&mut app)["open"].as_bool().unwrap());
    app.key_with(KEY_F, MASK_CTRL);
    let opened = wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true && now["focused"] == true
    });
    assert_eq!(opened["toggleOn"], true);
    app.type_text(term);
    let done = wait_for(&mut app, "results", Duration::from_secs(30), |now| {
        finished(now)
            && now["value"] == term.as_str()
            && !now["rows"].as_array().unwrap().is_empty()
    });
    let want = facts["insensitive_total"].as_u64().unwrap();
    assert_eq!(footer_count(&done), want, "{}", done["footer"]);
    assert_eq!(done["scanned"], "6,000 of 6,000 chapters searched");
    assert_eq!(done["spacer"].as_f64().unwrap(), want as f64 * 84.0);

    let rows = done["rows"].as_array().unwrap().clone();
    let target = rows
        .iter()
        .find(|row| row["shown"] == true && row["index"].as_u64() > Some(1))
        .unwrap();
    let wanted_chapter: u32 = target["where"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .trim_end_matches([':', '.'])
        .replace(',', "")
        .parse()
        .unwrap();
    click_row(&mut app, target);
    let moved = wait_for(
        &mut app,
        "the click to land",
        Duration::from_secs(5),
        |now| {
            now["current"]
                .as_array()
                .is_some_and(|current| current.len() == 1)
        },
    );
    assert_eq!(
        moved["where"],
        format!("Position {wanted_chapter} of 6,000"),
        "the reader landed on the row's chapter"
    );
    assert_eq!(normal(moved["current"][0].as_str().unwrap()), normal(term));
    assert_eq!(moved["inView"], true);
    let in_chapter = facts["insensitive"][(wanted_chapter - 1).to_string()]
        .as_u64()
        .unwrap() as usize;
    assert_eq!(moved["hits"].as_array().unwrap().len() + 1, in_chapter);

    let current_box = hit_rect(&mut app, "ss-search-current");
    app.js("new Promise((done)=>requestAnimationFrame(()=>requestAnimationFrame(done))).then(()=>true)");
    let shot = scratch_dir().join("search-pane.png");
    app.snapshot_png(&shot);
    let accent = pixels_matching(&shot, [0x8F, 0xD3, 0xB6]);
    let painted = accent(&current_box);
    println!(
        "pane picture at {}, {painted} accent pixels on the hit",
        shot.display()
    );
    assert!(painted > 50, "the clicked hit is painted, {painted} pixels");

    app.js("window.__s.setQuery('')");
    let cleared = wait_for(&mut app, "the clear", Duration::from_secs(3), |now| {
        now["footer"] == ""
            && now["hits"].as_array().unwrap().is_empty()
            && now["current"].as_array().unwrap().is_empty()
    });
    assert_eq!(cleared["domRows"], 0);
    assert_eq!(cleared["recentShown"], true, "the clicked search is listed as recent");
    app.key(KEY_ESCAPE);
    let list_closed = wait_for(&mut app, "Esc to close the recent list", Duration::from_secs(3), |now| {
        now["recentShown"] == false
    });
    assert_eq!(list_closed["open"], true, "the first Esc closes only the list");
    app.key(KEY_ESCAPE);
    wait_for(
        &mut app,
        "the pane to close",
        Duration::from_secs(3),
        |now| now["open"] == false,
    );
    assert!(state(&mut app)["errors"].as_array().unwrap().is_empty());
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn every_planted_term_counts_true_and_highlights_exactly_its_text_in_webkit() {
    let name = "large-h1-div-br";
    let truth = truth_of(name);
    let mut app = launch(&generated_book(name));
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    let mut checked = 0;
    for (term, facts) in truth["terms"].as_object().unwrap() {
        let want = facts["insensitive_total"].as_u64().unwrap();
        let done = search_and_wait(&mut app, term);
        if want == 0 {
            assert_eq!(done["footer"], "No matches.", "{term}");
            continue;
        }
        assert_eq!(footer_count(&done), want, "{term}: {}", done["footer"]);
        let row = done["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["shown"] == true)
            .unwrap()
            .clone();
        assert_eq!(
            normal(row["mark"].as_str().unwrap()),
            normal(term),
            "{term}"
        );
        click_row(&mut app, &row);
        let moved = wait_for(&mut app, "the highlight", Duration::from_secs(5), |now| {
            now["current"]
                .as_array()
                .is_some_and(|current| current.len() == 1)
        });
        assert_eq!(
            normal(moved["current"][0].as_str().unwrap()),
            normal(term),
            "{term}"
        );
        for painted in moved["hits"].as_array().unwrap() {
            assert_eq!(normal(painted.as_str().unwrap()), normal(term), "{term}");
        }
        assert_eq!(moved["inView"], true, "{term}");
        checked += 1;
    }
    println!("terms checked in the real app: {checked}");
    assert!(checked > 10);
    assert!(state(&mut app)["errors"].as_array().unwrap().is_empty());
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_hundred_thousand_results_stay_smooth_and_the_list_keeps_few_rows() {
    let mut app = launch(&generated_book("large-h1-p"));
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    let idle = app.memory();
    app.js("window.__s.startFrames()");
    let started = Instant::now();
    set_query(&mut app, "the");
    let done = wait_for(&mut app, "the cap", Duration::from_secs(60), |now| {
        now["footer"]
            .as_str()
            .unwrap_or_default()
            .contains("Results stop at")
            && finished(now)
    });
    let streamed_in = started.elapsed();
    let streaming = app.js("window.__s.stopFrames()");
    let loaded = app.memory();
    println!(
        "100k memory: tree RSS {} to {} MiB, PSS {} to {} MiB",
        idle.rss_kb / 1024,
        loaded.rss_kb / 1024,
        idle.pss_kb / 1024,
        loaded.pss_kb / 1024
    );
    assert_eq!(footer_count(&done), 100_000);
    assert_eq!(done["spacer"].as_f64().unwrap(), 8_400_000.0);
    assert!(
        done["domRows"].as_u64().unwrap() < 40,
        "{}",
        done["domRows"]
    );

    app.js("window.__s.startFrames()");
    let script = r"(async()=>{
      const max = 8400000;
      const list = document.getElementById('search-results');
      let blank = 0;
      let short = 0;
      const steps = 240;
      for (let step = 0; step < steps; step += 1) {
        list.scrollTop = step % 2 === 0 ? (step / steps) * max : max - (step / steps) * max;
        await new Promise(requestAnimationFrame);
        const seen = window.__s.rowsMissingText();
        blank += seen.blank;
        if (seen.shown < seen.expected) short += 1;
      }
      return { blank, short, steps };
    })()";
    let scrolling = app.js(script);
    let frames = app.js("window.__s.stopFrames()");
    println!(
        "100k: delivered in {streamed_in:?}; frames while streaming {streaming}; frames while jumping {frames}; {scrolling}"
    );
    assert_eq!(scrolling["blank"], 0);
    assert_eq!(scrolling["short"], 0);
    let last_row = app.js(
        "(async()=>{document.getElementById('search-results').scrollTop=9e9;await new Promise(requestAnimationFrame);await new Promise(requestAnimationFrame);return window.__s.state().rows.at(-1).index})()",
    );
    assert_eq!(last_row, 99_999);
    assert!(frames["p99"].as_f64().unwrap() < 3.0 * FRAME_MS, "{frames}");
    assert!(state(&mut app)["errors"].as_array().unwrap().is_empty());
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn the_backend_cancels_within_a_frame_seen_from_the_page() {
    let mut app = launch_with(
        &generated_book("large-h1-p"),
        &[("JADESLIP_E2E_SEARCH_DELAY", "1000000:4")],
    );
    let mut latencies = Vec::new();
    for id in 0..11u64 {
        let script = format!("window.__s.cancelLatency({}, 'zzzqqzzz')", 1_000 + id);
        let measured = app.js(&script);
        assert_eq!(measured["kind"], "cancelled", "{measured}");
        latencies.push(measured["ms"].as_f64().unwrap());
    }
    latencies.sort_by(f64::total_cmp);
    println!("cancel round trip from the page, ms: {latencies:?}");
    assert!(latencies[latencies.len() / 2] < FRAME_MS, "{latencies:?}");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_new_query_or_closing_the_pane_stops_the_old_search_and_drops_its_rows() {
    let mut app = launch_with(
        &generated_book("large-h1-p"),
        &[("JADESLIP_E2E_SEARCH_DELAY", "1000000:20")],
    );
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    set_query(&mut app, "the");
    wait_for(&mut app, "first rows", Duration::from_secs(5), |now| {
        !now["rows"].as_array().unwrap().is_empty() && now["action"] == "Stop"
    });
    set_query(&mut app, "zzzqqzzz");
    let replaced = wait_for(
        &mut app,
        "the old rows to go",
        Duration::from_secs(3),
        |now| now["domRows"] == 0 && now["action"] == "Stop",
    );
    assert_eq!(replaced["value"], "zzzqqzzz");

    app.js("window.__s.setMode(true)");
    set_query(&mut app, "(?=zzzqq)x");
    let pid = app.pid;
    let started = Instant::now();
    while workers_of(pid).len() < 4 {
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the search runs in workers"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        state(&mut app)["action"],
        "Stop",
        "the search is still running"
    );
    app.key(KEY_ESCAPE);
    wait_for(
        &mut app,
        "the pane to close",
        Duration::from_secs(3),
        |now| now["open"] == false,
    );
    let started = Instant::now();
    let closing = workers_of(pid);
    while !workers_of(pid).is_empty() {
        if started.elapsed() >= Duration::from_secs(1) {
            let left = workers_of(pid);
            let states: Vec<String> = left
                .iter()
                .map(|worker| {
                    fs::read_to_string(format!("/proc/{worker}/stat"))
                        .ok()
                        .and_then(|stat| {
                            Some(stat.rsplit(')').next()?.trim().chars().next()?.to_string())
                        })
                        .unwrap_or_default()
                })
                .collect();
            let now = state(&mut app);
            panic!(
                "closing the pane left workers: at close {closing:?}, now {left:?} states {states:?} action {} scanned {} footer {} message {} errors {}",
                now["action"], now["scanned"], now["footer"], now["message"], now["errors"]
            );
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    app.key_with(KEY_F, MASK_CTRL);
    let reopened = wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    assert_eq!(reopened["action"], "Rerun");
    assert_eq!(reopened["value"], "(?=zzzqq)x");
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn the_quadratic_pattern_is_limited_on_the_largest_real_book_and_the_app_stays_responsive() {
    let book = largest_real_book();
    let mut app = launch(&book);
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    app.js("window.__s.setMode(true)");
    let baseline = app.js("window.__s.renderLatency(10)").as_f64().unwrap();
    app.js("window.__s.startFrames()");
    let before_rss = rss_kib(app.pid);
    set_query(&mut app, QUADRATIC);
    let started = Instant::now();
    let mut peak_workers = 0;
    let mut peak_sum = 0;
    let mut parent_peak = 0;
    let mut renders = Vec::new();
    let mut ix = 20;
    let done = loop {
        let now = state(&mut app);
        let workers = workers_of(app.pid);
        peak_workers = peak_workers.max(workers.len());
        peak_sum = peak_sum.max(workers.iter().map(|pid| rss_kib(*pid)).sum::<u64>());
        parent_peak = parent_peak.max(rss_kib(app.pid));
        if finished(&now) {
            break now;
        }
        renders.push(
            app.js(&format!("window.__s.renderLatency({ix})"))
                .as_f64()
                .unwrap(),
        );
        ix += 37;
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{}",
            without_book_text(&now)
        );
    };
    let elapsed = started.elapsed();
    let frames = app.js("window.__s.stopFrames()");
    renders.sort_by(f64::total_cmp);
    let footer = done["footer"].as_str().unwrap().to_owned();
    println!(
        "quadratic: {elapsed:?}, footer tail: limited text present {}; frames {frames}; render during, ms, median {:.1} max {:.1} over {} calls against {baseline:.1} idle; workers {peak_workers}, worker peak sum {} MiB, parent {} to {} MiB",
        footer.contains("too slow"),
        renders[renders.len() / 2],
        renders[renders.len() - 1],
        renders.len(),
        peak_sum / 1024,
        before_rss / 1024,
        parent_peak / 1024,
    );
    assert!(footer.contains("too slow"), "{footer}");
    assert!(elapsed < Duration::from_secs(10));
    assert!(peak_workers >= 2);
    assert!(frames["p99"].as_f64().unwrap() < 3.0 * FRAME_MS, "{frames}");
    assert!(renders[renders.len() / 2] < 100.0, "{renders:?}");
    assert!(workers_of(app.pid).is_empty());
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_real_sigkill_of_the_app_leaves_no_worker() {
    let mut app = launch(&generated_book("large-h1-p"));
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    app.js("window.__s.setMode(true)");
    set_query(&mut app, QUADRATIC);
    let pid = app.pid;
    let started = Instant::now();
    while workers_of(pid).len() < 4 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "workers never started"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let workers = workers_of(pid);
    let status = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let killed = Instant::now();
    while workers.iter().any(|worker| alive(*worker)) {
        assert!(
            killed.elapsed() < Duration::from_secs(2),
            "workers outlived the app"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    println!(
        "{} workers gone {:?} after SIGKILL",
        workers.len(),
        killed.elapsed()
    );
    assert!(app.wait_exit(Duration::from_secs(5)));
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn hostile_and_broken_patterns_leave_the_app_working() {
    let mut app = launch(&generated_book("large-h1-p"));
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    app.js("window.__s.setMode(true)");
    for pattern in ["(", "[a", r"\", "(?<=a", r"\p{Nope}"] {
        let done = wait_after_query(&mut app, pattern);
        assert!(!done["message"].as_str().unwrap().is_empty(), "{pattern}");
    }
    let deep = "(".repeat(5_000) + &")".repeat(5_000);
    let too_long = "a".repeat(100_000);
    for pattern in [
        deep.as_str(),
        too_long.as_str(),
        "(a+)+$",
        "(?s).*",
        "(?s).",
        "x*",
        r"(?s)(?:(?=.).)*Q",
        "a{100000}{100000}{100000}",
        "(a{1,1000}){1,1000}",
    ] {
        let label: String = pattern.chars().take(20).collect();
        let done = wait_after_query(&mut app, pattern);
        println!(
            "{label}: action {} footer present {}",
            done["action"],
            !done["footer"].as_str().unwrap().is_empty()
        );
        assert!(app.js("window.__s.renderLatency(3)").as_f64().unwrap() < 2_000.0);
    }
    app.js("window.__s.setMode(false)");
    let done = search_and_wait(&mut app, "the fox");
    assert!(finished(&done));
    assert!(state(&mut app)["errors"].as_array().unwrap().is_empty());
    assert!(workers_of(app.pid).is_empty());
    app.close();
}

fn wait_after_query(app: &mut App, pattern: &str) -> Value {
    set_query(app, pattern);
    std::thread::sleep(Duration::from_millis(200));
    wait_for(app, "the pattern to end", Duration::from_secs(30), |now| {
        now["value"] == pattern && finished(now)
    })
}

fn open_and_search(app: &mut App, term: &str) -> Value {
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    search_and_wait(app, term)
}

fn enter_edit_with_change(app: &mut App, dirty: bool) {
    app.js("document.activeElement.blur(), true");
    app.key(KEY_E);
    wait_for(app, "Edit", Duration::from_secs(3), |now| {
        now["editOn"] == true
    });
    if dirty {
        app.type_text("zz");
        wait_for(app, "the unsaved mark", Duration::from_secs(3), |now| {
            now["unsaved"] == true
        });
    }
}

fn copy_of(name: &str, label: &str) -> PathBuf {
    let path = scratch_dir().join(format!("search-{label}-{}.epub", std::process::id()));
    let _ = fs::remove_file(&path);
    fs::copy(generated_book(name), &path).unwrap();
    path
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_hit_clicked_with_unsaved_edits_prompts_and_each_answer_behaves() {
    let book = copy_of("large-h1-p", "prompt");
    let mut app = launch(&book);
    let done = open_and_search(&mut app, "the");
    assert!(footer_count(&done) > 3);
    let start_where = done["where"].as_str().unwrap().to_owned();
    enter_edit_with_change(&mut app, true);
    let row = state(&mut app)["rows"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|row| row["shown"] == true)
        .unwrap()
        .clone();

    click_row(&mut app, &row);
    let asked = wait_for(&mut app, "the prompt", Duration::from_secs(3), |now| {
        now["dialog"] == true
    });
    assert_eq!(asked["where"].as_str().unwrap(), start_where);
    assert!(asked["current"].as_array().unwrap().is_empty());
    app.key(KEY_ESCAPE);
    let kept = wait_for(
        &mut app,
        "the prompt to close",
        Duration::from_secs(3),
        |now| now["dialog"] == false,
    );
    assert_eq!(kept["editOn"], true);
    assert_eq!(kept["unsaved"], true);
    assert_eq!(kept["where"].as_str().unwrap(), start_where);

    click_row(&mut app, &row);
    wait_for(&mut app, "the prompt", Duration::from_secs(3), |now| {
        now["dialog"] == true
    });
    app.js("document.getElementById('dialog-discard').click(), true");
    let landed = wait_for(&mut app, "the hit", Duration::from_secs(5), |now| {
        now["current"]
            .as_array()
            .is_some_and(|current| current.len() == 1)
    });
    assert_eq!(landed["readOn"], true);
    assert_eq!(landed["unsaved"], false);
    assert_eq!(normal(landed["current"][0].as_str().unwrap()), "the");
    assert_eq!(landed["inView"], true);
    assert!(state(&mut app)["errors"].as_array().unwrap().is_empty());
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_hit_clicked_in_a_clean_edit_leaves_edit_as_the_reference_does() {
    let book = copy_of("large-h1-p", "clean");
    let mut app = launch(&book);
    open_and_search(&mut app, "the");
    enter_edit_with_change(&mut app, false);
    let in_edit = state(&mut app);
    let row = in_edit["rows"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|row| row["shown"] == true)
        .unwrap()
        .clone();
    assert!(
        in_edit["hits"].as_array().unwrap().is_empty(),
        "entering Edit clears highlights"
    );
    click_row(&mut app, &row);
    let landed = wait_for(&mut app, "the hit", Duration::from_secs(5), |now| {
        now["current"]
            .as_array()
            .is_some_and(|current| current.len() == 1)
    });
    assert_eq!(landed["dialog"], false);
    assert_eq!(landed["readOn"], true);
    assert_eq!(landed["inView"], true);
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn saving_then_clicking_an_old_hit_says_the_chapter_changed() {
    let book = copy_of("large-h1-p", "stale");
    let mut app = launch(&book);
    let done = open_and_search(&mut app, "the");
    let current_where = done["where"].as_str().unwrap().to_owned();
    let row = done["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["shown"] == true)
        .unwrap()
        .clone();
    let number = current_where.split_whitespace().nth(1).unwrap().to_owned();
    let row_where = row["where"].as_str().unwrap();
    assert!(
        row_where.contains(&format!("Chapter {number}")),
        "{row_where} {current_where}"
    );
    enter_edit_with_change(&mut app, true);
    app.key_with(0x73, MASK_CTRL);
    wait_for(&mut app, "the save", Duration::from_secs(5), |now| {
        now["unsaved"] == false
    });
    app.key(KEY_ESCAPE);
    app.js("document.getElementById('mode-read').click(), true");
    wait_for(&mut app, "Read", Duration::from_secs(5), |now| {
        now["readOn"] == true
    });
    let moved = state(&mut app)["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|now| now["index"] == row["index"])
        .unwrap()
        .clone();
    click_row(&mut app, &moved);
    let warned = wait_for(&mut app, "the message", Duration::from_secs(5), |now| {
        now["toast"]
            .as_str()
            .unwrap_or_default()
            .contains("changed since the search")
    });
    assert!(warned["current"].as_array().unwrap().is_empty());
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn keys_in_the_search_field_never_reach_the_chapter_during_edit() {
    let book = copy_of("large-h1-p", "keys");
    let mut app = launch(&book);
    let done = open_and_search(&mut app, "the");
    assert!(footer_count(&done) > 0);
    enter_edit_with_change(&mut app, false);
    let shape = "document.getElementById('book-host').shadowRoot.querySelector('.ss-body').textContent.length";
    let before = app.js(shape);
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the field", Duration::from_secs(3), |now| {
        now["focused"] == true
    });
    app.type_text("fox");
    app.key(jadeslip_e2e::KEY_ENTER);
    app.type_text("y");
    let during = wait_for(
        &mut app,
        "the field to hold every typed key",
        Duration::from_secs(3),
        |now| now["value"] == "foxy",
    );
    assert_eq!(during["editOn"], true);
    assert_eq!(during["unsaved"], false);
    assert_eq!(app.js(shape), before);
    app.key(KEY_ESCAPE);
    let closed = wait_for(
        &mut app,
        "the pane to close",
        Duration::from_secs(3),
        |now| now["open"] == false,
    );
    assert_eq!(
        closed["editOn"], true,
        "Esc closed the pane and stayed in Edit"
    );
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn keys_in_the_search_field_never_reach_the_book_while_reading() {
    let book = copy_of("large-h1-p", "read-keys");
    let mut app = launch(&book);
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the field", Duration::from_secs(3), |now| {
        now["focused"] == true && now["value"] == ""
    });
    app.type_text("fee");
    let typed = wait_for(
        &mut app,
        "the field to hold every typed key",
        Duration::from_secs(3),
        |now| now["value"] == "fee",
    );
    assert_eq!(typed["readOn"], true, "E in the field stayed in Read");
    assert_eq!(typed["editOn"], false, "E in the field stayed in Read");
    assert_eq!(
        typed["focusOn"], false,
        "F in the field left focus mode off"
    );
    assert_eq!(typed["focused"], true);
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn search_timing_and_memory_in_the_real_app_on_the_largest_real_book() {
    let book = largest_real_book();
    let mut app = launch(&book);
    let idle = app.memory();
    let cases = [
        ("literal rare", "zzzqqqz", false),
        ("literal common", "the", false),
        ("lookahead rare", "zzz(?=q)", true),
        ("lookahead common", r"\bthe(?= )", true),
    ];
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); cases.len()];
    let mut id = 5_000u64;
    let mut parent_peak = 0;
    for _ in 0..7 {
        for (at, (_, text, regex)) in cases.iter().enumerate() {
            id += 1;
            let script = format!("window.__s.timeSearch({id}, {}, {regex})", json!(text));
            let measured = app.js(&script);
            assert_eq!(measured["kind"], "done", "{}", measured["kind"]);
            times[at].push(measured["ms"].as_f64().unwrap());
            parent_peak = parent_peak.max(rss_kib(app.pid));
        }
    }
    let after = app.memory();
    for (at, (label, ..)) in cases.iter().enumerate() {
        times[at].sort_by(f64::total_cmp);
        println!(
            "in the app, {label}: median {} ms, range {} to {} over 7 runs, page to last row",
            times[at][3], times[at][0], times[at][6]
        );
    }
    println!(
        "tree RSS {} to {} MiB, PSS {} to {} MiB, parent peak {} MiB",
        idle.rss_kb / 1024,
        after.rss_kb / 1024,
        idle.pss_kb / 1024,
        after.pss_kb / 1024,
        parent_peak / 1024
    );
    assert!(times[0][3] < 500.0);
    assert!(workers_of(app.pid).is_empty());
    app.close();
}

fn search_mode_and_wait(app: &mut App, regex: bool, text: &str) -> Value {
    app.js(&format!("window.__s.setMode({regex})"));
    set_query(app, "");
    wait_for(app, "the clear", Duration::from_secs(3), |now| {
        now["footer"] == ""
    });
    search_and_wait(app, text)
}

fn wait_case(app: &mut App, pressed: &str) -> Value {
    wait_for(app, "Match case", Duration::from_secs(3), |now| {
        now["matchCase"] == pressed
    })
}

fn wait_prefs(data: &Path, needle: &str) {
    let path = data.join("prefs.json");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fs::read_to_string(&path).is_ok_and(|text| text.contains(needle)) {
        assert!(Instant::now() < deadline, "prefs.json never held {needle}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_lowercase_regex_finds_capitals_until_match_case_is_on_and_the_choice_survives_a_relaunch() {
    let name = "large-h1-p";
    let truth = truth_of(name);
    let facts = &truth["terms"]["zephyrquill"];
    let ignoring = facts["insensitive_total"].as_u64().unwrap();
    let exact = facts["exact_total"].as_u64().unwrap();
    assert!(exact < ignoring, "the fixture plants capitals");
    let data = scratch_dir().join(format!("search-case-data-{}", std::process::id()));
    let _ = fs::remove_dir_all(&data);
    let mut app = App::launch_with_data(&generated_book(name), &data);
    app.wait_first_text();
    app.assert_on_headless_output();
    assert_eq!(app.js(INSTALL), json!(true));
    app.key_with(KEY_F, MASK_CTRL);
    let opened = wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    let regex = search_mode_and_wait(&mut app, true, "zephyrquill");
    assert_eq!(
        footer_count(&regex),
        ignoring,
        "a lowercase regex finds the capitalised copies too: {}",
        regex["footer"]
    );
    assert_eq!(opened["matchCase"], "false", "Match case starts off");
    assert_eq!(opened["caseLabel"], "Match case");
    let plain = search_mode_and_wait(&mut app, false, "zephyrquill");
    assert_eq!(footer_count(&plain), ignoring, "{}", plain["footer"]);

    app.js("window.__s.clickCase()");
    wait_case(&mut app, "true");
    let plain_exact = search_mode_and_wait(&mut app, false, "zephyrquill");
    assert_eq!(footer_count(&plain_exact), exact, "{}", plain_exact["footer"]);
    let regex_exact = search_mode_and_wait(&mut app, true, "zephyrquill");
    assert_eq!(footer_count(&regex_exact), exact, "{}", regex_exact["footer"]);
    for row in regex_exact["rows"].as_array().unwrap() {
        assert_eq!(row["mark"], "zephyrquill", "Match case shows only the typed case");
    }
    wait_prefs(&data, "\"matchCase\":true");
    assert!(state(&mut app)["errors"].as_array().unwrap().is_empty());
    app.close();

    let mut again = App::launch_with_data(&generated_book(name), &data);
    again.wait_first_text();
    assert_eq!(again.js(INSTALL), json!(true));
    again.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut again, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    wait_case(&mut again, "true");
    let kept = search_mode_and_wait(&mut again, true, "zephyrquill");
    assert_eq!(footer_count(&kept), exact, "Match case after a relaunch: {}", kept["footer"]);
    again.js("window.__s.clickCase()");
    wait_case(&mut again, "false");
    let wide = wait_for(&mut again, "the rerun without case", Duration::from_secs(60), |now| {
        finished(now) && footer_count(now) == ignoring
    });
    assert_eq!(wide["value"], "zephyrquill", "turning Match case off reruns the same query");
    wait_prefs(&data, "\"matchCase\":false");
    assert!(state(&mut again)["errors"].as_array().unwrap().is_empty());
    again.close();
    let _ = fs::remove_dir_all(&data);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn the_top_counter_counts_chapters_and_the_footer_counts_matches() {
    let name = "large-h1-p";
    let truth = truth_of(name);
    let want = truth["terms"]["quillmarble"]["insensitive_total"]
        .as_u64()
        .unwrap();
    let mut app = launch(&generated_book(name));
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true && now["focused"] == true
    });
    let done = search_and_wait(&mut app, "quillmarble");
    assert_eq!(done["scanned"], "6,000 of 6,000 chapters searched");
    assert_eq!(done["scannedFits"], true, "the chapter counter is not cut off");
    let matches = format!("{} matches.", fmt_count(want));
    assert_eq!(done["footer"], matches.as_str());
    app.key(jadeslip_e2e::KEY_ENTER);
    let stepped = wait_for(&mut app, "the first match", Duration::from_secs(5), |now| {
        now["footer"].as_str().is_some_and(|text| text.starts_with("Match 1 of"))
    });
    assert_eq!(
        stepped["footer"],
        format!("Match 1 of {}.", fmt_count(want)).as_str()
    );
    assert_eq!(stepped["scanned"], "6,000 of 6,000 chapters searched");
    assert!(state(&mut app)["errors"].as_array().unwrap().is_empty());
    app.close();
}

fn fmt_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (at, ch) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn checked(state: &Value) -> (bool, bool) {
    let side = |name: &str| state["range"][name]["checked"].as_bool().unwrap();
    (side("before"), side("after"))
}

fn shown_total(state: &Value) -> u64 {
    let footer = state["footer"].as_str().unwrap_or_default();
    match footer.split_once(" of ") {
        Some((_, total)) if footer.starts_with("Match ") => {
            footer_count(&json!({ "footer": total }))
        }
        _ => footer_count(state),
    }
}

fn go_to(app: &mut App, ix: u32) {
    app.js(&format!("window.__s.goTo({ix})"));
    wait_for(app, "the chapter to show", Duration::from_secs(10), |now| {
        now["where"].as_str().is_some_and(|where_| {
            where_.starts_with(&format!("Position {} of", fmt_count(u64::from(ix) + 1)))
        })
    });
}

fn tick(app: &mut App, side: &str, on: bool) -> Value {
    app.js(&format!("window.__s.tick('{side}', {on})"))
}

fn whole_done(app: &mut App) -> Value {
    wait_for(app, "the whole-book search", Duration::from_secs(60), |now| {
        now["action"] == "Rerun"
            && now["scanned"]
                .as_str()
                .is_some_and(|scanned| scanned == "6,000 of 6,000 chapters searched")
    })
}

fn assert_filtered_in_place(now: &Value) {
    assert_eq!(now["action"], "Rerun", "no search started");
    assert_eq!(
        now["scanned"],
        "6,000 of 6,000 chapters searched",
        "the rows come from the finished whole-book search"
    );
    assert!(
        now["domRows"].as_u64().unwrap() > 0,
        "the rows show at once"
    );
}

fn truth_side(facts: &Value, keep: impl Fn(u32) -> bool) -> u64 {
    facts["insensitive"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(ix, _)| keep(ix.parse().unwrap()))
        .map(|(_, count)| count.as_u64().unwrap())
        .sum()
}

fn row_chapter(row: &Value) -> u32 {
    row["where"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .trim_end_matches([':', '.'])
        .replace(',', "")
        .parse()
        .unwrap()
}

fn plain_term(truth: &Value) -> (String, Value) {
    truth["terms"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, facts)| facts["class"] == "plain")
        .map(|(term, facts)| (term.clone(), facts.clone()))
        .unwrap()
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn before_and_after_filter_the_whole_book_results_without_searching_again() {
    let name = "large-h1-p";
    let truth = truth_of(name);
    let (term, facts) = plain_term(&truth);
    let total = facts["insensitive_total"].as_u64().unwrap();
    let mut app = launch(&generated_book(name));
    app.assert_on_headless_output();
    let here = 3000;
    go_to(&mut app, here);
    app.key_with(KEY_F, MASK_CTRL);
    let opened = wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    assert_eq!(checked(&opened), (true, true));
    assert_eq!(opened["range"]["before"]["label"], "Earlier");
    assert_eq!(opened["range"]["here"]["label"], "Ch. 3001");
    assert_eq!(opened["range"]["after"]["label"], "Later");
    assert_eq!(
        opened["range"]["before"]["hint"],
        "Hide matches before Chapter 3001"
    );
    assert_eq!(
        opened["range"]["after"]["hint"],
        "Hide matches after Chapter 3001"
    );
    assert_eq!(opened["placeholder"], "Search the whole book");

    set_query(&mut app, &term);
    let whole = whole_done(&mut app);
    assert_eq!(footer_count(&whole), total);

    let after = tick(&mut app, "before", false);
    assert_filtered_in_place(&after);
    assert_eq!(checked(&after), (false, true));
    assert_eq!(after["scannedFits"], true);
    assert_eq!(shown_total(&after), truth_side(&facts, |ix| ix >= here));
    assert_eq!(after["placeholder"], "Search ch. 3001 and after");
    let rows = after["rows"].as_array().unwrap().clone();
    assert!(
        rows.iter().all(|row| row_chapter(row) > here),
        "every row is from chapter 3001 on"
    );

    let target = rows
        .iter()
        .find(|row| row["shown"] == true && row_chapter(row) > here + 1)
        .cloned()
        .expect("a row in a later chapter");
    let later = row_chapter(&target);
    click_row(&mut app, &target);
    let moved = wait_for(&mut app, "the jump", Duration::from_secs(5), |now| {
        now["current"]
            .as_array()
            .is_some_and(|current| current.len() == 1)
            && now["range"]["here"]["hint"] == format!("Count from Chapter {later} instead")
    });
    assert_eq!(moved["scanned"], after["scanned"], "jumping does not rerun");
    assert_eq!(shown_total(&moved), shown_total(&after));
    assert_eq!(
        moved["placeholder"], "Search ch. 3001 and after",
        "the filter stays where it was set"
    );
    assert_eq!(moved["range"]["here"]["label"], "Ch. 3001");
    assert_eq!(moved["range"]["here"]["disabled"], false);

    let anchored = app.js("document.getElementById('search-here').click(), window.__s.state()");
    assert_filtered_in_place(&anchored);
    assert_eq!(anchored["range"]["here"]["label"], format!("Ch. {later}"));
    assert_eq!(anchored["range"]["here"]["disabled"], true);
    assert_eq!(shown_total(&anchored), truth_side(&facts, |ix| ix + 1 >= later));

    let only = tick(&mut app, "after", false);
    assert_filtered_in_place(&only);
    assert_eq!(checked(&only), (false, false));
    assert_eq!(shown_total(&only), truth_side(&facts, |ix| ix + 1 == later));
    assert!(
        only["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row_chapter(row) == later),
        "only the chapter being read"
    );
    assert_eq!(only["placeholder"], format!("Search ch. {later} only"));

    let before = tick(&mut app, "before", true);
    assert_filtered_in_place(&before);
    assert_eq!(shown_total(&before), truth_side(&facts, |ix| ix < later));

    let back = tick(&mut app, "after", true);
    assert_filtered_in_place(&back);
    assert_eq!(checked(&back), (true, true));
    assert_eq!(shown_total(&back), total, "every match is back");

    std::thread::sleep(Duration::from_millis(400));
    let settled = state(&mut app);
    assert_eq!(settled["scanned"], "6,000 of 6,000 chapters searched");
    assert_eq!(settled["action"], "Rerun", "nothing ran on its own");
    assert_eq!(shown_total(&settled), total);

    go_to(&mut app, here);
    tick(&mut app, "after", false);
    set_query(&mut app, &term);
    let typed = whole_done(&mut app);
    assert_eq!(
        footer_count(&typed),
        truth_side(&facts, |ix| ix <= here),
        "a new search covers the whole book and shows the range"
    );
    app.js("document.getElementById('search-action').click()");
    let rerun = whole_done(&mut app);
    assert_eq!(
        footer_count(&rerun),
        footer_count(&typed),
        "Rerun keeps the range"
    );
    assert_eq!(state(&mut app)["errors"], json!([]));
    app.close();
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn ctrl_t_hides_and_shows_the_pane_with_its_rows_and_range_unchanged() {
    let name = "large-h1-p";
    let truth = truth_of(name);
    let (term, facts) = plain_term(&truth);
    let mut app = launch(&generated_book(name));
    go_to(&mut app, 100);
    app.js("document.getElementById('tab-search').click()");
    wait_for(&mut app, "the pane", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    set_query(&mut app, &term);
    whole_done(&mut app);
    let scoped = tick(&mut app, "before", false);
    assert_filtered_in_place(&scoped);
    assert_eq!(shown_total(&scoped), truth_side(&facts, |ix| ix >= 100));

    app.key_with(KEY_T, MASK_CTRL);
    wait_for(
        &mut app,
        "the pane to close",
        Duration::from_secs(3),
        |now| now["open"] == false,
    );
    app.key_with(KEY_T, MASK_CTRL);
    let reopened = wait_for(&mut app, "the pane again", Duration::from_secs(3), |now| {
        now["open"] == true
    });
    assert_eq!(checked(&reopened), (false, true), "the range is kept");
    assert_eq!(reopened["value"], term.as_str());
    assert_eq!(reopened["scanned"], scoped["scanned"]);
    assert_eq!(reopened["footer"], scoped["footer"], "the rows are kept");
    assert_eq!(reopened["domRows"], scoped["domRows"]);
    assert_eq!(reopened["placeholder"], "Search ch. 101 and after");
    std::thread::sleep(Duration::from_millis(400));
    let later = state(&mut app);
    assert_eq!(later["action"], "Rerun", "nothing ran on its own");
    assert_eq!(later["footer"], scoped["footer"]);
    assert_eq!(later["errors"], json!([]));
    app.close();
}
