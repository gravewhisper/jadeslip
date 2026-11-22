use jadeslip_core::{DEFAULT_LIMITS, Extracted, Reader};
use jadeslip_e2e::{App, KEY_ENTER, generated_book, largest_real_book, scratch_dir};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const FIXTURE: &str = "planted-junk";
const WAIT: Duration = Duration::from_secs(60);
const FRAME_MS: f64 = 16.7;
const RULES: [&str; 4] = ["watermark", "notes", "heading", "titles"];
const TIMED_RUNS: usize = 5;

const INSTALL: &str = r"(()=>{
  const element = (id) => document.getElementById(id);
  const shown = (id) => !element(id).hidden;
  const list = element('search-results');
  window.__errors = [];
  window.addEventListener('error', (event) => window.__errors.push(event.message));
  window.addEventListener('unhandledrejection', (event) => window.__errors.push(String(event.reason?.message ?? event.reason)));
  const centre = (node) => {
    const box = node.getBoundingClientRect();
    return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
  };
  const inView = (node) => {
    const box = node.getBoundingClientRect();
    const frame = list.getBoundingClientRect();
    return !node.hidden && box.height > 0 && box.top >= frame.top && box.bottom <= frame.bottom;
  };
  let frames = null;
  window.__c = {
    state: () => ({
      dialog: shown('cleanup-dialog'),
      counts: [...document.querySelectorAll('#cleanup-rules .cleanup-count')].map((node) => node.textContent),
      labels: [...document.querySelectorAll('#cleanup-rules strong')].map((node) => node.textContent),
      status: element('cleanup-status').textContent,
      stopShown: !element('cleanup-stop').hidden,
      showEnabled: !element('cleanup-show').disabled,
      cleanEnabled: !element('cleanup-apply').disabled,
      paneOpen: shown('search-pane'),
      cleanHead: shown('clean-head'),
      cleanRules: element('clean-rules').textContent,
      optionsShown: !element('search-pane').querySelector('.searchopts').hidden,
      replaceInputShown: !element('replace-input').hidden,
      summary: element('replace-summary').textContent,
      applyLabel: element('replace-apply').textContent,
      applyEnabled: !element('replace-apply').disabled,
      footer: element('search-count').textContent,
      applying: shown('apply-dialog'),
      applyTitle: element('apply-title').textContent,
      undo: shown('undo-bar') ? element('undo-text').textContent : '',
      toast: shown('toast') ? element('toast').textContent : '',
      menu: shown('reader-menu'),
      mounted: element('book-host').shadowRoot?.textContent ?? '',
      errors: window.__errors,
      contents: [...document.querySelectorAll('#toc-spacer .row:not([hidden])')].map((row) => ({
        entry: Number(row.dataset.entry), title: row.querySelector('.t').textContent,
      })),
      rows: [...list.querySelectorAll('.result')].filter(inView).map((row) => ({
        index: Number(row.dataset.index),
        mark: row.querySelector('.mark').textContent,
        inserted: row.querySelector('ins').textContent,
        note: row.querySelector('.note').textContent,
        struck: getComputedStyle(row.querySelector('.mark')).textDecorationLine,
        ...centre(row),
      })),
      ticks: [...list.querySelectorAll('.row-tick')].filter(inView).map((tick) => ({
        index: Number(tick.dataset.index), pressed: tick.getAttribute('aria-pressed'), ...centre(tick),
      })),
    }),
    spot: (id) => centre(element(id)),
    click: (id) => { element(id).click(); return true; },
    type: (id, text) => {
      const input = element(id);
      input.value = text;
      input.dispatchEvent(new Event('input', { bubbles: true }));
      return true;
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
    timeOpen: () => new Promise((done) => {
      const counts = element('cleanup-rules');
      const started = performance.now();
      const watch = new MutationObserver(() => {
        const texts = [...counts.querySelectorAll('.cleanup-count')].map((node) => node.textContent);
        if (texts.length === 0 || texts.some((text) => text === 'Counting…' || text === '')) return;
        watch.disconnect();
        done({ ms: performance.now() - started, texts });
      });
      watch.observe(counts, { subtree: true, childList: true, characterData: true });
      element('reader-more').click();
      element('reader-clean').click();
    }),
  };
  return true;
})()";

fn dir_for(label: &str) -> PathBuf {
    let dir = scratch_dir().join(format!("{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_into(source: &Path, dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::copy(source, &path).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn install(app: &mut App) {
    app.wait_first_text();
    app.assert_on_headless_output();
    assert_eq!(app.js(INSTALL), json!(true));
}

fn state(app: &mut App) -> Value {
    app.js("window.__c.state()")
}

fn wait_for(app: &mut App, what: &str, test: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let now = state(app);
        assert_eq!(now["errors"], json!([]), "{what}: the page threw");
        if test(&now) {
            return now;
        }
        assert!(started.elapsed() < WAIT, "{what}: never came, last {now}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn click_id(app: &mut App, id: &str) {
    let spot = app.js(&format!("window.__c.spot('{id}')"));
    app.click(spot["x"].as_f64().unwrap(), spot["y"].as_f64().unwrap());
}

fn click_at(app: &mut App, spot: &Value) {
    app.click(spot["x"].as_f64().unwrap(), spot["y"].as_f64().unwrap());
}

fn palette_run(app: &mut App, label: &str) {
    click_id(app, "open-palette");
    let started = Instant::now();
    while app.js("!document.getElementById('palette').hidden") != json!(true) {
        assert!(started.elapsed() < WAIT, "the palette never opened");
        std::thread::sleep(Duration::from_millis(10));
    }
    app.js(&format!("window.__c.type('palette-input', {})", json!(label)));
    app.key(KEY_ENTER);
}

fn truth() -> Value {
    let path = generated_book(FIXTURE).with_extension("truth.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn expected_counts(truth: &Value) -> Vec<String> {
    RULES
        .iter()
        .map(|rule| {
            let matches = truth["clean"]["matches"][rule].as_u64().unwrap();
            let chapters = truth["clean"]["chapters"][rule].as_u64().unwrap();
            if *rule == "titles" {
                return plural(matches, "chapter title", "chapter titles");
            }
            format!(
                "{} in {}",
                plural(matches, "paragraph", "paragraphs"),
                plural(chapters, "chapter", "chapters")
            )
        })
        .collect()
}

fn counted(app: &mut App) -> Value {
    wait_for(app, "the counts", |now| {
        now["dialog"] == true && now["cleanEnabled"] == true
    })
}

struct Chapter {
    ix: u32,
    text: String,
    bytes: Vec<u8>,
}

fn chapters(path: &Path) -> Vec<Chapter> {
    let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
    reader
        .rows()
        .iter()
        .filter(|row| !row.is_nav)
        .map(|row| {
            let Extracted::Document(bytes) = reader.extract(row.ix).unwrap() else {
                panic!("a text chapter");
            };
            Chapter {
                ix: row.ix.0,
                text: reader.visible_text(row.ix).unwrap().as_str().to_owned(),
                bytes,
            }
        })
        .collect()
}

fn source_of(chapter: &Chapter) -> String {
    String::from_utf8(chapter.bytes.clone()).unwrap()
}

fn br_lines(source: &str) -> Option<Vec<String>> {
    let start = source.find("<div>")? + "<div>".len();
    let end = source.find("</div>")?;
    Some(
        source[start..end]
            .split("<br/><br/>")
            .map(|line| line.trim().to_owned())
            .collect(),
    )
}

fn without_one(mut lines: Vec<String>, gone: &[&str]) -> Vec<String> {
    for text in gone {
        if let Some(at) = lines.iter().position(|line| line == text) {
            lines.remove(at);
        }
    }
    lines
}

fn retitled_away(fix: Option<&Value>, text: &str) -> usize {
    fix.map_or(0, |fix| {
        let from = fix["from"].as_str().unwrap().matches(text).count();
        let pulled = fix["pulled"].as_str().map_or(0, |line| line.matches(text).count());
        let to = fix["to"].as_str().unwrap().matches(text).count();
        from + pulled - to
    })
}

fn contents(path: &Path) -> Vec<(Option<u32>, String)> {
    let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
    reader
        .toc()
        .unwrap()
        .entries
        .iter()
        .map(|entry| (entry.target.map(|ix| ix.0), entry.label.clone()))
        .collect()
}

fn label_at(contents: &[(Option<u32>, String)], ix: u32) -> (usize, String) {
    contents
        .iter()
        .enumerate()
        .find(|(_, (target, _))| *target == Some(ix))
        .map(|(at, (_, label))| (at, label.clone()))
        .unwrap_or_else(|| panic!("an entry for spine row {ix}"))
}

fn shown_label(now: &Value, entry: usize) -> Option<String> {
    now["contents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["entry"] == entry)
        .map(|row| row["title"].as_str().unwrap().to_owned())
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
}

fn backups_in(data: &Path) -> Vec<PathBuf> {
    let listed = |dir: PathBuf| -> Vec<PathBuf> {
        fs::read_dir(dir)
            .map(|entries| entries.filter_map(Result::ok).map(|entry| entry.path()).collect())
            .unwrap_or_default()
    };
    listed(data.join("books"))
        .into_iter()
        .flat_map(|book| listed(book.join("backups")))
        .map(|slot| slot.join("book.epub"))
        .filter(|path| path.exists())
        .collect()
}

fn shelf_copy(data: &Path) -> PathBuf {
    let library: Value = serde_json::from_slice(&fs::read(data.join("library.json")).unwrap()).unwrap();
    let books = library["books"].as_object().unwrap();
    assert_eq!(books.len(), 1, "one book on the shelf");
    PathBuf::from(books.values().next().unwrap()["path"].as_str().unwrap())
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn counts_match_the_truth_and_one_save_removes_the_rest_and_undo_brings_it_all_back() {
    let truth = truth();
    let dir = dir_for("flow");
    let book = copy_into(&generated_book(FIXTURE), &dir, "book.epub");
    let original = fs::read(&book).unwrap();
    let before = chapters(&book);
    let contents_before = contents(&book);
    let data = dir.join("data");
    let mut app = App::launch_with_data(&book, &data);
    install(&mut app);

    palette_run(&mut app, "Clean up book");
    let ready = counted(&mut app);
    assert_eq!(ready["counts"], json!(expected_counts(&truth)));
    assert_eq!(
        ready["labels"],
        json!([
            "Watermark lines",
            "Translator notes and support requests",
            "Repeated chapter titles",
            "Fix chapter titles"
        ])
    );
    let planted = truth["clean"]["planted"].as_array().unwrap();
    let fixes = truth["clean"]["titles"].as_array().unwrap();
    let touched: std::collections::BTreeSet<u64> = planted
        .iter()
        .chain(fixes)
        .map(|line| line["chapter"].as_u64().unwrap())
        .collect();
    assert_eq!(
        ready["status"],
        format!(
            "{} paragraphs will be removed and {} chapter titles fixed, in {} chapters.",
            planted.len(),
            fixes.len(),
            touched.len()
        )
    );

    click_id(&mut app, "cleanup-show");
    let review = wait_for(&mut app, "the clean-up preview", |now| {
        now["paneOpen"] == true && now["cleanHead"] == true && !now["rows"].as_array().unwrap().is_empty()
    });
    assert_eq!(review["dialog"], false);
    assert_eq!(review["optionsShown"], false);
    assert_eq!(review["replaceInputShown"], false);
    assert_eq!(review["applyLabel"], "Clean up");
    assert_eq!(
        review["footer"],
        format!("{} changes.", planted.len() + fixes.len())
    );
    assert!(
        review["cleanRules"]
            .as_str()
            .unwrap()
            .contains("Repeated chapter titles, Fix chapter titles")
    );
    let kept_fix = fixes[0].clone();
    let retitled = &review["rows"][0];
    assert_eq!(retitled["mark"], kept_fix["from"]);
    assert_eq!(retitled["inserted"], kept_fix["to"]);
    assert_eq!(retitled["note"], "Fix chapter titles");
    assert_eq!(retitled["struck"], "line-through");
    let first = &review["rows"][1];
    assert_eq!(first["mark"], planted[0]["text"]);
    assert_eq!(first["inserted"], "");
    assert_eq!(first["note"], "Repeated chapter titles");
    assert_eq!(first["struck"], "line-through");

    let spared = planted[1].clone();
    assert_eq!(review["rows"][2]["mark"], spared["text"]);
    for row in [0, 2] {
        let index = review["rows"][row]["index"].clone();
        let tick = review["ticks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tick| tick["index"] == index)
            .unwrap()
            .clone();
        click_at(&mut app, &tick);
        wait_for(&mut app, "the row to untick", |now| {
            now["ticks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tick| tick["index"] == index && tick["pressed"] == "false")
        });
    }
    let unticked = state(&mut app);
    let removed = planted.len() - 1;
    let fixed = fixes.len() - 1;
    assert_eq!(
        unticked["summary"],
        format!(
            "{removed} paragraphs will be removed and {fixed} chapter titles fixed, in {} chapters.",
            touched.len()
        )
    );
    assert_eq!(fs::read(&book).unwrap(), original, "nothing is written before Clean up");

    click_id(&mut app, "replace-apply");
    let done = wait_for(&mut app, "the undo bar", |now| !now["undo"].as_str().unwrap().is_empty());
    assert_eq!(
        done["undo"],
        format!(
            "Removed {removed} paragraphs and fixed {fixed} chapter titles, in {} chapters.",
            touched.len()
        )
    );
    assert_eq!(done["cleanHead"], false, "the pane leaves the clean-up preview");

    let after = chapters(&book);
    let contents_after = contents(&book);
    assert_eq!(after.len(), before.len());
    assert_eq!(contents_after.len(), contents_before.len());
    let fix_of = |ix: usize| {
        fixes
            .iter()
            .find(|fix| fix["chapter"] == ix && **fix != kept_fix)
    };
    for (ix, (old, new)) in before.iter().zip(&after).enumerate() {
        let fix = fix_of(ix);
        let shift = |text: &str| retitled_away(fix, text);
        let gone: Vec<&str> = planted
            .iter()
            .filter(|line| line["chapter"] == ix && **line != spared)
            .map(|line| line["text"].as_str().unwrap())
            .collect();
        for text in &gone {
            assert_eq!(
                new.text.matches(text).count() + 1 + shift(text),
                old.text.matches(text).count(),
                "chapter {ix} lost exactly one {text:?}"
            );
        }
        if spared["chapter"] == ix {
            let text = spared["text"].as_str().unwrap();
            assert_eq!(
                new.text.matches(text).count() + shift(text),
                old.text.matches(text).count(),
                "the unticked line stays"
            );
        }
        for kept in truth["clean"]["kept"].as_array().unwrap() {
            let text = kept.as_str().unwrap();
            assert_eq!(
                new.text.matches(text).count() + shift(text),
                old.text.matches(text).count(),
                "chapter {ix} keeps {text:?}"
            );
        }
        let (_, label) = label_at(&contents_after, new.ix);
        let (_, label_before) = label_at(&contents_before, old.ix);
        match fix {
            Some(fix) => {
                assert_eq!(
                    label,
                    fix["to"].as_str().unwrap(),
                    "chapter {ix} has its new title in the contents"
                );
                let source = source_of(new);
                let title = escaped(fix["to"].as_str().unwrap());
                assert!(
                    source.contains(&format!("<title>{title}</title>")),
                    "chapter {ix} has its new <title>"
                );
                assert!(
                    source.contains(&format!("<h1>{title}</h1>")),
                    "chapter {ix} has its new heading"
                );
                if let Some(pulled) = fix["pulled"].as_str() {
                    assert!(
                        !new.text.contains(pulled),
                        "chapter {ix} lost the line pulled into its title"
                    );
                }
            }
            None => assert_eq!(label, label_before, "chapter {ix} keeps its contents label"),
        }
        if let Some(lines) = br_lines(&source_of(old)) {
            let gone: Vec<String> = gone.iter().map(|text| escaped(text)).collect();
            let gone: Vec<&str> = gone.iter().map(String::as_str).collect();
            assert_eq!(
                br_lines(&source_of(new)),
                Some(without_one(lines, &gone)),
                "chapter {ix} keeps one blank line between the lines left"
            );
        }
    }

    for line in planted.iter().filter(|line| {
        line["text"]
            .as_str()
            .unwrap()
            .contains("< report chapter >")
    }) {
        let ix = line["chapter"].as_u64().unwrap() as usize;
        assert!(
            !after[ix].text.contains("report chapter"),
            "chapter {ix} lost its report-chapter line"
        );
    }
    let (entry, _) = label_at(&contents_after, after[1].ix);
    let renamed = fix_of(1).unwrap()["to"].as_str().unwrap().to_owned();
    wait_for(&mut app, "the new title in the contents", |now| {
        shown_label(now, entry).as_deref() == Some(renamed.as_str())
    });

    let backups = backups_in(&data);
    assert_eq!(backups.len(), 1, "one backup record for the in-place book");
    assert_eq!(fs::read(&backups[0]).unwrap(), original, "the in-place book was backed up before the clean-up");

    click_id(&mut app, "undo-action");
    wait_for(&mut app, "the undo", |now| now["toast"] == "Clean-up undone.");
    let undone = chapters(&book);
    for (ix, (old, new)) in before.iter().zip(&undone).enumerate() {
        assert!(old.bytes == new.bytes, "chapter {ix} is byte-equal to the original after Undo");
    }
    assert_eq!(
        contents(&book),
        contents_before,
        "Undo brings the contents labels back"
    );
    let (_, original_label) = label_at(&contents_before, before[1].ix);
    wait_for(&mut app, "the old title in the contents", |now| {
        shown_label(now, entry).as_deref() == Some(original_label.as_str())
    });
    app.close();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn stop_during_counting_leaves_the_book_unchanged() {
    let dir = dir_for("stop");
    let book = copy_into(&generated_book(FIXTURE), &dir, "book.epub");
    let original = fs::read(&book).unwrap();
    let data = dir.join("data");
    let mut app = App::launch_with_data_env(
        Some(&book),
        &data,
        &[("JADESLIP_E2E_SEARCH_DELAY", Path::new("40:1500"))],
    );
    install(&mut app);

    click_id(&mut app, "reader-more");
    wait_for(&mut app, "the book menu", |now| now["menu"] == true);
    click_id(&mut app, "reader-clean");
    let counting = wait_for(&mut app, "the count to start", |now| {
        now["dialog"] == true && now["stopShown"] == true
    });
    assert_eq!(counting["counts"], json!(["Counting…", "Counting…", "Counting…", "Counting…"]));
    assert_eq!(counting["cleanEnabled"], false);
    assert_eq!(counting["menu"], false);

    click_id(&mut app, "cleanup-stop");
    let stopped = wait_for(&mut app, "the stop", |now| now["stopShown"] == false);
    assert_eq!(
        stopped["status"],
        "Stopped before every chapter was checked. Nothing was changed."
    );
    assert_eq!(stopped["cleanEnabled"], false);
    assert_eq!(stopped["showEnabled"], false);
    click_id(&mut app, "cleanup-cancel");
    wait_for(&mut app, "the dialog to close", |now| now["dialog"] == false);
    app.close();
    assert_eq!(fs::read(&book).unwrap(), original, "a stopped count changes nothing");
    assert!(backups_in(&data).is_empty(), "a count makes no backup");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_shelf_book_is_cleaned_in_its_own_copy_and_the_added_file_stays_as_it_was() {
    let truth = truth();
    let dir = dir_for("shelf");
    let source = copy_into(&generated_book(FIXTURE), &dir, "source.epub");
    let original = fs::read(&source).unwrap();
    let data = dir.join("data");
    let mut app = App::launch_with_data_opt(None, &data);
    app.wait_first_text();
    assert_eq!(app.js(INSTALL), json!(true));
    app.op(&json!({"op": "drop_paths", "paths": [&source]}));
    let started = Instant::now();
    while app.js("document.querySelectorAll('#shelf-grid [aria-haspopup=menu]').length") != json!(1) {
        assert!(started.elapsed() < WAIT, "the book never reached the shelf");
        std::thread::sleep(Duration::from_millis(20));
    }
    let copy = shelf_copy(&data);
    let before = chapters(&copy);
    app.js("document.querySelector('#shelf-grid [aria-haspopup=menu]').click()");
    app.js("document.getElementById('menu-clean').click()");
    let ready = counted(&mut app);
    assert_eq!(ready["counts"], json!(expected_counts(&truth)));
    click_id(&mut app, "cleanup-apply");
    let done = wait_for(&mut app, "the undo bar", |now| !now["undo"].as_str().unwrap().is_empty());
    let planted = truth["clean"]["planted"].as_array().unwrap();
    let fixes = truth["clean"]["titles"].as_array().unwrap();
    let removing = format!("Removed {} paragraphs and fixed {} chapter titles", planted.len(), fixes.len());
    assert!(done["undo"].as_str().unwrap().starts_with(&removing));
    app.close();

    assert_eq!(fs::read(&source).unwrap(), original, "the added file stays as it was");
    let after = chapters(&copy);
    let removed: usize = planted
        .iter()
        .map(|line| {
            let ix = line["chapter"].as_u64().unwrap() as usize;
            let text = line["text"].as_str().unwrap();
            let fix = fixes.iter().find(|fix| fix["chapter"] == ix);
            before[ix].text.matches(text).count() - after[ix].text.matches(text).count() - retitled_away(fix, text)
        })
        .sum();
    assert_eq!(removed, planted.len(), "the shelf copy lost every planted line");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn counting_the_largest_book_answers_fast_and_keeps_the_window_drawing() {
    let dir = dir_for("timing");
    let book = copy_into(&largest_real_book(), &dir, "large.epub");
    let original = fs::read(&book).unwrap();
    let rows = Reader::open(File::open(&book).unwrap(), DEFAULT_LIMITS)
        .unwrap()
        .rows()
        .len();
    let data = dir.join("data");
    let mut app = App::launch_with_data(&book, &data);
    install(&mut app);
    let mut times = Vec::new();
    let mut worst_p99: f64 = 0.0;
    for run in 0..TIMED_RUNS {
        app.js("window.__c.startFrames()");
        let opened = app.js("window.__c.timeOpen()");
        let frames = app.js("window.__c.stopFrames()");
        let ms = opened["ms"].as_f64().unwrap();
        println!("run {run}: counts shown after {ms:.0} ms, {}, frames {frames}", opened["texts"]);
        times.push(ms);
        worst_p99 = worst_p99.max(frames["p99"].as_f64().unwrap());
        click_id(&mut app, "cleanup-cancel");
        wait_for(&mut app, "the dialog to close", |now| now["dialog"] == false);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "cleanup counts on {rows} spine rows: median {:.0} ms, max {:.0} ms; worst frame p99 {worst_p99:.1} ms",
        times[times.len() / 2],
        times[times.len() - 1]
    );
    assert!(times[times.len() / 2] < 2_000.0, "{times:?}");
    assert!(worst_p99 < 3.0 * FRAME_MS, "the window keeps drawing while counting: {worst_p99}");
    app.close();
    assert_eq!(fs::read(&book).unwrap(), original);
    let _ = fs::remove_dir_all(&dir);
}
