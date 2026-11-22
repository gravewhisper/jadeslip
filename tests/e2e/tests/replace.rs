use serde_json::{Value, json};
use jadeslip_core::{DEFAULT_LIMITS, Reader};
use jadeslip_e2e::{App, KEY_ESCAPE, MASK_CTRL, generated_book, scratch_dir};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const KEY_F: u32 = 0x66;
const KEY_E: u32 = 0x65;
const KEY_S: u32 = 0x73;
const QUADRATIC: &str = "(?s)(?=.*QQQ)x";
const FIXTURE: &str = "large-h1-p";
const WAIT: Duration = Duration::from_secs(60);

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
  window.__r = {
    state: () => ({
      paneOpen: shown('search-pane'),
      replaceOpen: shown('replace-section'),
      summary: element('replace-summary').textContent,
      applyEnabled: !element('replace-apply').disabled,
      applyLabel: element('replace-apply').textContent,
      range: { before: element('search-before').classList.contains('on'), after: element('search-after').classList.contains('on') },
      footer: element('search-count').textContent,
      matchCase: element('search-case')?.getAttribute('aria-pressed') ?? 'missing',
      scanned: element('search-status').hidden ? '' : element('search-scanned').textContent,
      action: element('search-action').textContent,
      message: shown('search-message') ? element('search-message').textContent : '',
      offer: shown('fix-offer') ? element('fix-offer-text').textContent : '',
      offerReady: !element('fix-offer-fix').disabled,
      toolbar: shown('toolbar'),
      fixDialog: shown('fix-dialog'),
      fixTitle: element('fix-dialog-title').textContent,
      fixText: element('fix-dialog-text').textContent,
      applying: shown('apply-dialog'),
      unsavedDialog: shown('dialog'),
      unsaved: shown('unsaved'),
      toast: shown('toast') ? element('toast').textContent : '',
      readOn: element('mode-read').classList.contains('on'),
      editOn: element('mode-edit').classList.contains('on'),
      where: element('where-title').title,
      errors: window.__errors,
      rows: [...list.querySelectorAll('.result')].filter(inView).map((row) => ({
        index: Number(row.dataset.index),
        mark: row.querySelector('.mark').textContent,
        inserted: row.querySelector('ins').textContent,
        note: row.querySelector('.note').textContent,
        ...centre(row),
      })),
      ticks: [...list.querySelectorAll('.row-tick')].filter(inView).map((tick) => ({
        index: Number(tick.dataset.index), pressed: tick.getAttribute('aria-pressed'), ...centre(tick),
      })),
      skips: [...list.querySelectorAll('.row-skip')].filter(inView).map((skip) => ({
        index: Number(skip.dataset.index), text: skip.textContent, ...centre(skip),
      })),
    }),
    spot: (id) => centre(element(id)),
    setValue: (id, text) => {
      const input = element(id);
      input.value = text;
      input.dispatchEvent(new Event('input', { bubbles: true }));
      return true;
    },
    click: (id) => { element(id).click(); return true; },
    tick: (side, on) => { const button = element(`search-${side}`); if (button.classList.contains('on') !== on) button.click(); return true; },
    mode: (regex) => { if (element('search-regex').classList.contains('on') !== regex) element('search-regex').click(); return true; },
    selectWord: (word) => {
      const root = element('book-host').shadowRoot;
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const at = node.data.search(new RegExp(`\\b${word}\\b`));
        if (at >= 0) {
          document.getSelection().setBaseAndExtent(node, at, node, at + word.length);
          return true;
        }
      }
      return false;
    },
    mountedHas: (word) => (element('book-host').shadowRoot?.textContent ?? '').includes(word),
    editable: () => element('book-host').shadowRoot?.querySelector('[contenteditable=true]') !== null,
    insertAtCaret: (text) => document.execCommand('insertText', false, text),
    blur: () => { document.activeElement?.blur(); return true; },
  };
  return true;
})()";

fn copy_of(label: &str) -> PathBuf {
    let path = scratch_dir().join(format!("replace-{label}-{}.epub", std::process::id()));
    let _ = fs::remove_file(&path);
    fs::copy(generated_book(FIXTURE), &path).unwrap();
    path
}

fn launch_with(book: &Path, env: &[(&str, &str)]) -> App {
    let mut command = Command::new(jadeslip_e2e::e2e_binary());
    command.arg(book);
    for (name, value) in env {
        command.env(name, value);
    }
    let mut app = App::spawn(command, &[]);
    app.wait_first_text();
    app.assert_on_headless_output();
    assert_eq!(app.js(INSTALL), json!(true));
    app
}

fn launch(book: &Path) -> App {
    launch_with(book, &[])
}

fn state(app: &mut App) -> Value {
    app.js("window.__r.state()")
}

fn wait_for(app: &mut App, what: &str, test: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let now = state(app);
        assert_eq!(now["errors"], json!([]), "{what}: the page threw");
        if test(&now) {
            return now;
        }
        assert!(
            started.elapsed() < WAIT,
            "{what}: never came, last footer {}",
            now["footer"]
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn click_id(app: &mut App, id: &str) {
    let spot = app.js(&format!("window.__r.spot('{id}')"));
    app.click(spot["x"].as_f64().unwrap(), spot["y"].as_f64().unwrap());
}

fn click_at(app: &mut App, spot: &Value) {
    app.click(spot["x"].as_f64().unwrap(), spot["y"].as_f64().unwrap());
}

fn set_value(app: &mut App, id: &str, text: &str) {
    app.js(&format!("window.__r.setValue('{id}', {})", json!(text)));
}

fn preview_ready(now: &Value) -> bool {
    now["summary"].as_str().is_some_and(|text| !text.is_empty()) && now["action"] == "Rerun"
}

fn open_preview(app: &mut App, query: &str, replacement: &str) -> Value {
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(app, "the pane", |now| now["paneOpen"] == true);
    if state(app)["replaceOpen"] != true {
        click_id(app, "replace-toggle");
        wait_for(app, "the replace field", |now| now["replaceOpen"] == true);
    }
    set_value(app, "search-input", query);
    set_value(app, "replace-input", replacement);
    wait_for(app, "the preview", preview_ready)
}

fn number_after(text: &str, marker: &str) -> u64 {
    let start = text.find(marker).map_or(0, |at| at + marker.len());
    let digits: String = text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == ',')
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().unwrap_or(0)
}

fn changes_in(summary: &str) -> u64 {
    number_after(summary, "Preview: ")
}

fn occurrences(path: &Path, term: &str) -> u64 {
    let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
    reader
        .rows()
        .iter()
        .filter(|row| !row.is_nav)
        .filter_map(|row| reader.visible_text(row.ix).ok())
        .map(|text| text.as_str().matches(term).count() as u64)
        .sum()
}

fn truth_exact(term: &str) -> u64 {
    let path = generated_book(FIXTURE).with_extension("truth.json");
    let truth: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    truth["terms"][term]["exact"]
        .as_object()
        .unwrap()
        .values()
        .map(|count| count.as_u64().unwrap())
        .sum()
}

fn enter_edit(app: &mut App) {
    app.js("window.__r.blur()");
    app.key(KEY_E);
    wait_for(app, "Edit", |now| now["editOn"] == true);
}

fn type_a_change(app: &mut App) {
    app.type_text("zz");
    wait_for(app, "the unsaved mark", |now| now["unsaved"] == true);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn preview_exclude_apply_and_search_again_match_the_truth() {
    let book = copy_of("flow");
    let term = "quillmarble";
    let planted = truth_exact(term);
    let mut app = launch(&book);
    let shown = open_preview(&mut app, term, "<new&term>");
    let summary = shown["summary"].as_str().unwrap().to_owned();
    assert_eq!(
        changes_in(&summary),
        planted,
        "the count before any exclusion"
    );
    assert!(summary.contains("Applied as one atomic save."));
    assert!(summary.contains("Revert to backup"));
    assert_eq!(shown["rows"][0]["mark"], term);
    assert_eq!(shown["rows"][0]["inserted"], "<new&term>");
    let on_disk_before = fs::read(&book).unwrap();

    click_at(&mut app, &shown["ticks"][0].clone());
    let unticked = wait_for(&mut app, "the row to untick", |now| {
        now["ticks"][0]["pressed"] == "false"
    });
    assert_eq!(
        changes_in(unticked["summary"].as_str().unwrap()),
        planted - 1
    );

    let skip_row = unticked["skips"]
        .as_array()
        .unwrap()
        .last()
        .cloned()
        .expect("a row to skip");
    click_at(&mut app, &skip_row);
    let skipped = wait_for(&mut app, "the chapter to skip", |now| {
        changes_in(now["summary"].as_str().unwrap()) < planted - 1
    });
    let left = changes_in(skipped["summary"].as_str().unwrap());
    assert_eq!(
        fs::read(&book).unwrap(),
        on_disk_before,
        "preview never writes"
    );

    click_id(&mut app, "replace-apply");
    let done = wait_for(&mut app, "the save", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
    });
    assert_eq!(number_after(done["toast"].as_str().unwrap(), ""), left);
    let again = wait_for(&mut app, "the search to rerun", |now| {
        now["replaceOpen"] == false && now["action"] == "Rerun"
    });
    assert_eq!(
        number_after(again["footer"].as_str().unwrap(), ""),
        planted - left,
        "the excluded copies are what a new search finds"
    );
    assert_eq!(occurrences(&book, term), planted - left);
    assert_eq!(occurrences(&book, "<new&term>"), left);
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn fix_everywhere_from_a_real_typed_correction() {
    let book = copy_of("fix");
    let lower = truth_exact("zephyrquill");
    let upper = truth_exact("ZEPHYRQUILL");
    let mixed = truth_exact("ZephyrQuill");
    let mut app = launch(&book);
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", |now| now["paneOpen"] == true);
    set_value(&mut app, "search-input", "zephyrquill");
    let found = wait_for(&mut app, "the search", |now| {
        now["action"] == "Rerun" && now["rows"].as_array().is_some_and(|rows| !rows.is_empty())
    });
    let lower_row = found["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["mark"] == "zephyrquill")
        .cloned()
        .expect("a lower case row on screen");
    click_at(&mut app, &lower_row);
    std::thread::sleep(Duration::from_millis(300));
    app.key(KEY_ESCAPE);
    wait_for(&mut app, "the pane to close", |now| {
        now["paneOpen"] == false
    });
    enter_edit(&mut app);
    assert_eq!(app.js("window.__r.selectWord('zephyrquill')"), json!(true));
    app.type_text("zephyrquilt");
    wait_for(&mut app, "the unsaved mark", |now| now["unsaved"] == true);
    app.key(KEY_ESCAPE);
    let offered = wait_for(&mut app, "the counted offer", |now| {
        now["offerReady"] == true
    });
    assert_eq!(
        offered["readOn"], true,
        "Esc returned to Read and the offer stayed"
    );
    let text = offered["offer"].as_str().unwrap();
    assert_eq!(
        number_after(text, "Fixed here. "),
        lower + upper - 1,
        "lower and all capitals follow, mixed case does not, the saved copy is gone: {text}"
    );
    let saved_copy = fs::read(&book).unwrap();
    click_id(&mut app, "fix-offer-fix");
    let asked = wait_for(&mut app, "the dialog", |now| now["fixDialog"] == true);
    assert!(asked["fixTitle"].as_str().unwrap().starts_with("Fix "));
    assert!(
        asked["fixText"]
            .as_str()
            .unwrap()
            .contains("\"zephyrquill\" becomes \"zephyrquilt\"")
    );
    assert!(
        asked["fixText"]
            .as_str()
            .unwrap()
            .contains("Revert to backup")
    );
    assert_eq!(
        fs::read(&book).unwrap(),
        saved_copy,
        "asking writes nothing"
    );
    click_id(&mut app, "fix-dialog-apply");
    wait_for(&mut app, "the fix", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
            && now["offer"] == ""
    });
    assert_eq!(occurrences(&book, "zephyrquill"), 0);
    assert_eq!(occurrences(&book, "ZEPHYRQUILL"), 0);
    assert_eq!(occurrences(&book, "ZephyrQuill"), mixed);
    assert_eq!(occurrences(&book, "zephyrquilt"), lower);
    assert_eq!(occurrences(&book, "ZEPHYRQUILT"), upper);
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn the_offer_stays_in_edit_enables_the_toolbar_button_and_can_be_dismissed() {
    let book = copy_of("fix-edit");
    let mut app = launch(&book);
    enter_edit(&mut app);
    let word = app.js(
        "(()=>{const root=document.getElementById('book-host').shadowRoot;const words=root.textContent.match(/\\b[a-z]{7,}\\b/g);return words?words[0]:null})()",
    );
    let word = word.as_str().expect("a long word").to_owned();
    assert_eq!(
        app.js(&format!("window.__r.selectWord('{word}')")),
        json!(true)
    );
    let corrected = format!("{word}q");
    app.type_text(&corrected);
    wait_for(&mut app, "the unsaved mark", |now| now["unsaved"] == true);
    app.key_with(KEY_S, MASK_CTRL);
    let offered = wait_for(&mut app, "the counted offer", |now| {
        now["unsaved"] == false && now["offerReady"] == true
    });
    assert_eq!(offered["editOn"], true);
    assert_eq!(offered["offerReady"], true, "the word has other copies");
    app.js("(()=>{document.getElementById('open-palette').click();const input=document.getElementById('palette-input');input.value='Fix everywhere';input.dispatchEvent(new Event('input'));document.querySelector('#palette-rows .palrow').click();return true})()");
    wait_for(&mut app, "the dialog", |now| now["fixDialog"] == true);
    app.key(KEY_ESCAPE);
    let closed = wait_for(&mut app, "the dialog to close", |now| {
        now["fixDialog"] == false
    });
    assert_eq!(closed["editOn"], true, "Esc closed the dialog and not Edit");
    click_id(&mut app, "fix-offer-dismiss");
    wait_for(&mut app, "the offer to go", |now| now["offer"] == "");
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_stale_preview_is_refused_with_a_message_and_replaced_by_a_fresh_one() {
    let book = copy_of("stale");
    let mut app = launch(&book);
    let shown = open_preview(&mut app, "quillmarble", "stone");
    let before = changes_in(shown["summary"].as_str().unwrap());
    app.js("window.__r.blur()");
    enter_edit(&mut app);
    type_a_change(&mut app);
    app.key_with(KEY_S, MASK_CTRL);
    wait_for(&mut app, "the save", |now| now["unsaved"] == false);
    app.js("window.__r.click('mode-read')");
    wait_for(&mut app, "Read", |now| now["readOn"] == true);
    let after_save = fs::read(&book).unwrap();
    click_id(&mut app, "replace-apply");
    let refused = wait_for(&mut app, "the refusal", |now| {
        now["message"]
            .as_str()
            .is_some_and(|text| text.contains("saved after this preview"))
            && preview_ready(now)
    });
    assert_eq!(
        fs::read(&book).unwrap(),
        after_save,
        "a stale apply writes nothing"
    );
    assert_eq!(refused["applying"], false);
    assert_eq!(changes_in(refused["summary"].as_str().unwrap()), before);
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("Check the new preview")
    );
    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the second apply", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
    });
    assert_eq!(occurrences(&book, "quillmarble"), 0);
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn stop_during_apply_leaves_the_file_as_it_was() {
    let book = copy_of("cancel");
    let mut app = launch_with(&book, &[("JADESLIP_E2E_SEARCH_DELAY", "7000:6")]);
    let shown = open_preview(&mut app, "the", "THE");
    assert!(changes_in(shown["summary"].as_str().unwrap()) > 1000);
    let before = fs::read(&book).unwrap();
    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the progress dialog", |now| {
        now["applying"] == true
    });
    std::thread::sleep(Duration::from_millis(150));
    click_id(&mut app, "apply-stop");
    let stopped = wait_for(&mut app, "the stop", |now| {
        now["applying"] == false
            && now["toast"]
                .as_str()
                .is_some_and(|text| text.contains("Stopped"))
    });
    assert_eq!(fs::read(&book).unwrap(), before);
    assert!(
        stopped["summary"]
            .as_str()
            .unwrap()
            .starts_with("Preview: ")
    );
    assert_eq!(
        stopped["applyEnabled"], true,
        "the same preview can still run"
    );
    let temps = fs::read_dir(book.parent().unwrap())
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            let name = name.to_string_lossy().into_owned();
            name.contains("replace-cancel") && name.ends_with(".tmp")
        })
        .count();
    assert_eq!(temps, 0);
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn esc_during_apply_stops_it_and_never_reaches_the_chapter() {
    let book = copy_of("cancel-esc");
    let mut app = launch_with(&book, &[("JADESLIP_E2E_SEARCH_DELAY", "7000:6")]);
    open_preview(&mut app, "the", "THE");
    let before = fs::read(&book).unwrap();
    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the progress dialog", |now| {
        now["applying"] == true
    });
    app.key(KEY_E);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        state(&mut app)["editOn"],
        false,
        "E went through the dialog"
    );
    app.key(KEY_ESCAPE);
    wait_for(&mut app, "the stop", |now| now["applying"] == false);
    assert_eq!(fs::read(&book).unwrap(), before);
    assert_eq!(state(&mut app)["paneOpen"], true);
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn an_injected_save_failure_shows_its_message_and_changes_nothing() {
    let book = copy_of("inject");
    let mut app = launch(&book);
    open_preview(&mut app, "quillmarble", "stone");
    let before = fs::read(&book).unwrap();
    app.fail_save(Some("wouldBreak"));
    click_id(&mut app, "replace-apply");
    let failed = wait_for(&mut app, "the failure", |now| {
        now["message"]
            .as_str()
            .is_some_and(|text| text.contains("final check"))
    });
    assert_eq!(failed["applying"], false);
    assert_eq!(fs::read(&book).unwrap(), before);
    app.fail_save(None);
    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the retry", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
    });
    assert_eq!(occurrences(&book, "quillmarble"), 0);
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn apply_in_a_clean_edit_updates_the_mounted_chapter_and_stays_in_edit() {
    let book = copy_of("clean-edit");
    let mut app = launch(&book);
    let word = app.js(
        "(()=>{const root=document.getElementById('book-host').shadowRoot;const words=root.textContent.match(/\\b[a-z]{7,}\\b/g);return words?words[0]:null})()",
    );
    let word = word.as_str().expect("a long word").to_owned();
    enter_edit(&mut app);
    open_preview(&mut app, &word, "QXZQXZ");
    click_id(&mut app, "replace-apply");
    let done = wait_for(&mut app, "the save", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
            && now["applying"] == false
    });
    assert_eq!(done["editOn"], true, "still in Edit");
    wait_for(&mut app, "the search again", |now| now["action"] == "Rerun");
    assert_eq!(app.js("window.__r.mountedHas('QXZQXZ')"), json!(true));
    assert_eq!(
        app.js(&format!("window.__r.mountedHas(' {word} ')")),
        json!(false)
    );
    assert_eq!(app.js("window.__r.editable()"), json!(true));
    app.js("document.getElementById('book-host').shadowRoot.querySelector('[contenteditable=true]').focus(), true");
    assert_eq!(app.js("window.__r.selectWord('QXZQXZ')"), json!(true));
    app.type_text("after");
    wait_for(&mut app, "the unsaved mark", |now| now["unsaved"] == true);
    app.key_with(KEY_S, MASK_CTRL);
    wait_for(&mut app, "a save after the remount", |now| {
        now["unsaved"] == false
    });
    assert!(occurrences(&book, "after") > 0);
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn apply_over_unsaved_text_asks_and_each_answer_behaves() {
    let book = copy_of("dirty");
    let mut app = launch(&book);
    enter_edit(&mut app);
    type_a_change(&mut app);
    let shown = open_preview(&mut app, "quillmarble", "stone");
    let planted = changes_in(shown["summary"].as_str().unwrap());
    let before = fs::read(&book).unwrap();

    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the prompt", |now| now["unsavedDialog"] == true);
    click_id(&mut app, "dialog-keep");
    let kept = wait_for(&mut app, "Keep", |now| now["unsavedDialog"] == false);
    assert_eq!(kept["editOn"], true);
    assert_eq!(fs::read(&book).unwrap(), before);

    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the prompt", |now| now["unsavedDialog"] == true);
    click_id(&mut app, "dialog-save");
    let refused = wait_for(&mut app, "the saved chapter to stale the preview", |now| {
        now["message"]
            .as_str()
            .is_some_and(|text| text.contains("saved after this preview"))
            && preview_ready(now)
    });
    assert_eq!(refused["readOn"], true);
    assert!(occurrences(&book, "zz") > 0, "the typed text was saved");
    assert_eq!(occurrences(&book, "quillmarble"), planted);
    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the apply", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
    });
    assert_eq!(occurrences(&book, "quillmarble"), 0);
    app.close();
    let _ = fs::remove_file(&book);

    let book = copy_of("discard");
    let mut app = launch(&book);
    enter_edit(&mut app);
    type_a_change(&mut app);
    open_preview(&mut app, "quillmarble", "stone");
    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the prompt", |now| now["unsavedDialog"] == true);
    click_id(&mut app, "dialog-discard");
    wait_for(&mut app, "the apply", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
    });
    assert_eq!(occurrences(&book, "quillmarble"), 0);
    let started = Instant::now();
    while app.js("window.__r.mountedHas('zz')") != json!(false) {
        assert!(
            started.elapsed() < WAIT,
            "the discarded text stayed on screen"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_mounted_chapter_in_read_shows_the_new_text_at_once() {
    let book = copy_of("mounted");
    let mut app = launch(&book);
    let word = app.js(
        "(()=>{const root=document.getElementById('book-host').shadowRoot;const words=root.textContent.match(/\\b[a-z]{7,}\\b/g);return words?words[0]:null})()",
    );
    let word = word.as_str().expect("a long word").to_owned();
    open_preview(&mut app, &word, "QXZQXZ");
    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the save", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
    });
    let started = Instant::now();
    while app.js("window.__r.mountedHas('QXZQXZ')") != json!(true) {
        assert!(started.elapsed() < WAIT, "the chapter was not refreshed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(state(&mut app)["readOn"], true);
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn the_replace_pane_leaves_room_for_rows_at_the_test_size() {
    let book = copy_of("look");
    let mut app = launch(&book);
    open_preview(&mut app, "quillmarble", "stone");
    let shot = scratch_dir().join("replace-pane.png");
    app.snapshot_png(&shot);
    assert!(shot.exists());
    let layout = app.js("(()=>{const list=document.getElementById('search-results').getBoundingClientRect();const head=document.querySelector('#search-pane .railhead').getBoundingClientRect();return {list:list.height,head:head.height}})()");
    assert!(
        layout["list"].as_f64().unwrap() >= 168.0,
        "room for two rows: {layout}"
    );
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_too_slow_pattern_cannot_be_applied_and_a_bad_one_says_why() {
    let book = jadeslip_e2e::largest_real_book();
    let copy = scratch_dir().join(format!("replace-slow-{}.epub", std::process::id()));
    fs::copy(&book, &copy).unwrap();
    let before = fs::read(&copy).unwrap();
    let mut app = launch(&copy);
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", |now| now["paneOpen"] == true);
    app.js("window.__r.mode(true)");
    click_id(&mut app, "replace-toggle");
    wait_for(&mut app, "the replace field", |now| now["replaceOpen"] == true);
    set_value(&mut app, "search-input", QUADRATIC);
    set_value(&mut app, "replace-input", "y");
    let slow = wait_for(&mut app, "the slow preview", preview_ready);
    assert!(slow["summary"].as_str().unwrap().contains("too slow"));
    assert_eq!(slow["applyEnabled"], false);
    set_value(&mut app, "search-input", "(");
    let bad = wait_for(&mut app, "the bad pattern", |now| now["message"] != "");
    assert_eq!(bad["applyEnabled"], false);
    set_value(&mut app, "search-input", "(a)");
    set_value(&mut app, "replace-input", "$9");
    let bad_template = wait_for(&mut app, "the bad template", |now| {
        now["message"]
            .as_str()
            .is_some_and(|text| text.contains("group"))
    });
    assert_eq!(bad_template["applyEnabled"], false);
    assert_eq!(fs::read(&copy).unwrap(), before);
    app.close();
    let _ = fs::remove_file(&copy);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn book_wide_replace_follows_match_case_in_its_preview_and_its_save() {
    let book = copy_of("case");
    let lower = truth_exact("zephyrquill");
    let upper = truth_exact("ZEPHYRQUILL");
    let mixed = truth_exact("ZephyrQuill");
    let mut app = launch(&book);
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", |now| now["paneOpen"] == true);
    app.js("window.__r.mode(true)");
    let ignoring = open_preview(&mut app, "zephyrquill", "<swapped>");
    assert_eq!(
        changes_in(ignoring["summary"].as_str().unwrap()),
        lower + upper + mixed,
        "a regex replace ignores case by default: {}",
        ignoring["summary"]
    );
    assert_eq!(ignoring["matchCase"], "false");

    click_id(&mut app, "search-case");
    let exact = wait_for(&mut app, "the preview with Match case", |now| {
        now["matchCase"] == "true"
            && preview_ready(now)
            && changes_in(now["summary"].as_str().unwrap()) != lower + upper + mixed
    });
    assert_eq!(
        changes_in(exact["summary"].as_str().unwrap()),
        lower,
        "Match case narrows the preview: {}",
        exact["summary"]
    );
    let rows = exact["rows"].as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        assert_eq!(row["mark"], "zephyrquill", "the preview shows only what will change");
        assert_eq!(row["inserted"], "<swapped>");
    }

    click_id(&mut app, "replace-apply");
    wait_for(&mut app, "the save", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
    });
    wait_for(&mut app, "the search to rerun", |now| {
        now["replaceOpen"] == false && now["action"] == "Rerun"
    });
    assert_eq!(occurrences(&book, "zephyrquill"), 0);
    assert_eq!(occurrences(&book, "<swapped>"), lower);
    assert_eq!(occurrences(&book, "ZEPHYRQUILL"), upper, "capitals are left alone");
    assert_eq!(occurrences(&book, "ZephyrQuill"), mixed, "mixed case is left alone");
    app.close();
    let _ = fs::remove_file(&book);
}
fn truth_exact_by_chapter(term: &str) -> Vec<(u32, u64)> {
    let path = generated_book(FIXTURE).with_extension("truth.json");
    let truth: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    truth["terms"][term]["exact"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(ix, count)| (ix.parse().unwrap(), count.as_u64().unwrap()))
        .collect()
}

fn occurrences_by_side(path: &Path, term: &str, here: u32) -> (u64, u64) {
    let reader = Reader::open(File::open(path).unwrap(), DEFAULT_LIMITS).unwrap();
    let mut sides = (0, 0);
    for row in reader.rows().iter().filter(|row| !row.is_nav) {
        let found = reader
            .visible_text(row.ix)
            .map_or(0, |text| text.as_str().matches(term).count() as u64);
        if row.ix.0 < here {
            sides.0 += found;
        } else {
            sides.1 += found;
        }
    }
    sides
}

fn go_to(app: &mut App, ix: u32) {
    app.js(&format!(
        "import('/main.js').then((main)=>main.goTo({ix})).then(()=>true)"
    ));
    let want = format!("Position {} of", ix + 1);
    wait_for(app, "the chapter to show", |now| {
        now["where"]
            .as_str()
            .is_some_and(|place| place.replace(',', "").starts_with(&want))
    });
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_scoped_replace_previews_and_saves_only_its_side() {
    let book = copy_of("scoped");
    let term = "quillmarble";
    let here = 3000;
    let by_chapter = truth_exact_by_chapter(term);
    let before: u64 = by_chapter
        .iter()
        .filter(|(ix, _)| *ix < here)
        .map(|(_, count)| count)
        .sum();
    let after: u64 = by_chapter
        .iter()
        .filter(|(ix, _)| *ix >= here)
        .map(|(_, count)| count)
        .sum();
    assert!(before > 0 && after > 0);
    let mut app = launch(&book);
    go_to(&mut app, here);
    let whole = open_preview(&mut app, term, "zqxstonez");
    assert_eq!(
        changes_in(whole["summary"].as_str().unwrap()),
        before + after
    );
    assert_eq!(whole["applyLabel"], "Apply to whole book");
    app.js("window.__r.tick('before', false)");
    let scoped = wait_for(&mut app, "the scoped preview", |now| {
        preview_ready(now) && now["applyLabel"] == "Apply to ch. 3001 and after"
    });
    assert_eq!(changes_in(scoped["summary"].as_str().unwrap()), after);
    assert_eq!(scoped["applyLabel"], "Apply to ch. 3001 and after");
    go_to(&mut app, 10);
    assert_eq!(
        state(&mut app)["applyLabel"],
        "Apply to ch. 3001 and after",
        "moving keeps the previewed side"
    );
    click_id(&mut app, "replace-apply");
    let done = wait_for(&mut app, "the save", |now| {
        now["toast"]
            .as_str()
            .is_some_and(|text| text.contains("saved"))
    });
    assert_eq!(number_after(done["toast"].as_str().unwrap(), ""), after);
    let again = wait_for(&mut app, "the search to rerun", |now| {
        now["replaceOpen"] == false && now["action"] == "Rerun"
    });
    assert_eq!(
        again["applyLabel"], "Apply to ch. 3001 and after",
        "the refresh keeps the side"
    );
    assert_eq!(occurrences_by_side(&book, term, here), (before, 0));
    assert_eq!(occurrences_by_side(&book, "zqxstonez", here), (0, after));
    app.close();
    let _ = fs::remove_file(&book);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_fix_review_covers_the_whole_book_even_from_a_scoped_pane() {
    let book = copy_of("scoped-fix");
    let lower = truth_exact("zephyrquill");
    let upper = truth_exact("ZEPHYRQUILL");
    let mut app = launch(&book);
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the pane", |now| now["paneOpen"] == true);
    set_value(&mut app, "search-input", "zephyrquill");
    let found = wait_for(&mut app, "the search", |now| {
        now["action"] == "Rerun" && now["rows"].as_array().is_some_and(|rows| !rows.is_empty())
    });
    let lower_row = found["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["mark"] == "zephyrquill")
        .cloned()
        .expect("a lower case row on screen");
    click_at(&mut app, &lower_row);
    std::thread::sleep(Duration::from_millis(300));
    app.js("window.__r.tick('before', false)");
    wait_for(&mut app, "the scoped search", |now| {
        now["action"] == "Rerun"
            && now["applyLabel"]
                .as_str()
                .is_some_and(|text| text.ends_with(" and after"))
    });
    enter_edit(&mut app);
    assert_eq!(app.js("window.__r.selectWord('zephyrquill')"), json!(true));
    app.type_text("zephyrquilt");
    wait_for(&mut app, "the unsaved mark", |now| now["unsaved"] == true);
    app.key(KEY_ESCAPE);
    let offered = wait_for(&mut app, "the counted offer", |now| {
        now["offerReady"] == true
    });
    assert_eq!(
        number_after(offered["offer"].as_str().unwrap(), "Fixed here. "),
        lower + upper - 1,
        "the offer counts the whole book"
    );
    click_id(&mut app, "fix-offer-fix");
    wait_for(&mut app, "the dialog", |now| now["fixDialog"] == true);
    click_id(&mut app, "fix-dialog-review");
    let reviewed = wait_for(&mut app, "the review", |now| {
        now["paneOpen"] == true && preview_ready(now)
    });
    assert_eq!(
        reviewed["scanned"], "6,000 of 6,000 chapters searched",
        "the review searches the whole book"
    );
    assert_eq!(reviewed["range"], json!({ "before": true, "after": true }));
    assert_eq!(reviewed["applyLabel"], "Apply to whole book");
    assert_eq!(
        changes_in(reviewed["summary"].as_str().unwrap()),
        lower + upper - 1
    );
    app.close();
    let _ = fs::remove_file(&book);
}
