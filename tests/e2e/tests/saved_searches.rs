use jadeslip_e2e::{
    App, KEY_DOWN, KEY_ENTER, KEY_ESCAPE, KEY_UP, MASK_CTRL, generated_book, repo_root, scratch_dir,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const KEY_F: u32 = 0x66;
const KEY_K: u32 = 0x6b;
const KEY_F3: u32 = 0xffc0;
const BOOK: &str = "large-h1-p";
const PATTERN: &str = r"zephyr(?:quill)\b";

const INSTALL: &str = r"(()=>{
  const element = (id) => document.getElementById(id);
  const input = element('search-input');
  window.__errors = [];
  window.addEventListener('error', (event) => window.__errors.push(event.message));
  window.addEventListener('unhandledrejection', (event) => window.__errors.push(String(event.reason?.message ?? event.reason)));
  const text = (node) => node?.textContent ?? null;
  window.__m = {
    state: () => {
      const menu = element('search-memory');
      const rows = [...menu.querySelectorAll('.ssrow')].map((row) => ({
        name: text(row.querySelector('.ssname')),
        text: text(row.querySelector('.sstext')),
        tag: text(row.querySelector('.sstag')),
        on: row.classList.contains('on'),
        current: row.classList.contains('current'),
        renaming: Boolean(row.querySelector('.ssrename')),
      }));
      return {
        open: !element('search-pane').hidden,
        focusMode: document.querySelector('.app').classList.contains('focus'),
        value: input.value,
        focused: document.activeElement === input,
        regex: element('search-regex').classList.contains('on'),
        matchCase: element('search-case').getAttribute('aria-pressed'),
        action: element('search-action').textContent,
        footer: element('search-count').textContent,
        message: element('search-message').hidden ? '' : element('search-message').textContent,
        menu: !menu.hidden,
        heads: [...menu.querySelectorAll('.sstitle')].map(text),
        rows,
        form: !element('search-save-form').hidden,
        formName: element('search-save-name').value,
        formError: element('search-save-error').hidden ? '' : element('search-save-error').textContent,
        saveOn: element('search-save').classList.contains('on'),
        saveLabel: element('search-save').getAttribute('aria-label'),
        toast: element('toast').hidden ? '' : element('toast').textContent,
        undo: element('undo-bar').hidden ? '' : element('undo-text').textContent,
        palette: !element('palette').hidden,
        paletteRows: [...document.querySelectorAll('#palette-rows .palrow')].map((row) => [text(row.querySelector('.pallabel')), text(row.querySelector('.palhint'))]),
        errors: window.__errors,
      };
    },
    set: (id, value) => {
      const field = element(id);
      field.value = value;
      field.dispatchEvent(new Event('input', { bubbles: true }));
      return true;
    },
    click: (id) => { element(id).click(); return true; },
    clickRow: (at, selector) => { element('search-memory').querySelectorAll('.ssrow')[at].querySelector(selector).click(); return true; },
    clickPalette: (label) => { [...document.querySelectorAll('#palette-rows .palrow')].find((row) => row.querySelector('.pallabel').textContent === label).click(); return true; },
    keyIn: (id, key) => { element(id).dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })); return true; },
    strangers: () => {
      const allowed = new Set(['div.sshead', 'span.sstitle', 'button.btn', 'div.ssrow', 'button.ssrun', 'span.ssname', 'span.sstext', 'span.sstag', 'button.mini', 'svg.icon', 'use.', 'input.field']);
      return [...element('search-memory').querySelectorAll('*')].map((node) => `${node.localName}.${node.classList[0] ?? ''}`).filter((name) => !allowed.has(name));
    },
  };
  return true;
})()";

fn data_dir(name: &str) -> PathBuf {
    let dir = scratch_dir().join(format!("saved-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn open(data: &Path) -> App {
    let mut app = App::launch_with_data(&generated_book(BOOK), data);
    app.wait_first_text();
    app.assert_on_headless_output();
    assert_eq!(app.js(INSTALL), json!(true));
    app
}

fn state(app: &mut App) -> Value {
    app.js("window.__m.state()")
}

fn wait_for(app: &mut App, what: &str, test: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let now = state(app);
        if test(&now) {
            return now;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "timed out waiting for {what}: {}",
            json!({"value": now["value"], "footer": now["footer"], "action": now["action"], "menu": now["menu"], "rows": now["rows"], "form": now["form"], "formError": now["formError"], "errors": now["errors"]})
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn finished(now: &Value) -> bool {
    now["action"] == "Rerun" || !now["message"].as_str().unwrap_or_default().is_empty()
}

fn count(now: &Value) -> u64 {
    now["footer"]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .find_map(|word| word.trim_end_matches('.').replace(',', "").parse().ok())
        .unwrap_or(0)
}

fn exact_and_ignoring() -> (u64, u64) {
    let path = repo_root().join(format!("fixtures/generated/{BOOK}.truth.json"));
    let truth: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let facts = &truth["terms"]["zephyrquill"];
    (
        facts["exact_total"].as_u64().unwrap(),
        facts["insensitive_total"].as_u64().unwrap(),
    )
}

fn open_pane(app: &mut App) -> Value {
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(app, "the pane", |now| {
        now["open"] == true && now["focused"] == true
    })
}

fn set_mode(app: &mut App, regex: bool, match_case: bool) {
    if state(app)["regex"] != json!(regex) {
        app.js("window.__m.click('search-regex')");
    }
    if state(app)["matchCase"] != json!(match_case.to_string()) {
        app.js("window.__m.click('search-case')");
    }
    wait_for(app, "the mode and case", |now| {
        now["regex"] == regex && now["matchCase"] == json!(match_case.to_string())
    });
}

fn search(app: &mut App, text: &str) -> Value {
    app.js(&format!("window.__m.set('search-input', {})", json!(text)));
    std::thread::sleep(Duration::from_millis(200));
    wait_for(app, "the search to finish", |now| {
        now["value"] == text && finished(now)
    })
}

fn stored(data: &Path) -> Value {
    fs::read_to_string(data.join("searches.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

fn wait_stored(data: &Path, what: &str, test: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let now = stored(data);
        if test(&now) {
            return now;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "searches.json never had {what}: {now}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn recent_texts(file: &Value) -> Vec<String> {
    file["recent"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| row["text"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

fn no_errors(app: &mut App) {
    assert_eq!(state(app)["errors"], json!([]));
}

fn save_as(app: &mut App, name: &str) -> Value {
    app.js("window.__m.click('search-save')");
    wait_for(app, "the name form", |now| now["form"] == true);
    app.js(&format!(
        "window.__m.set('search-save-name', {})",
        json!(name)
    ));
    app.js("window.__m.click('search-save-ok')");
    wait_for(app, "the save to finish", |now| {
        now["form"] == false || !now["formError"].as_str().unwrap_or_default().is_empty()
    })
}

fn show_list(app: &mut App) -> Value {
    if state(app)["menu"] == false {
        app.js("window.__m.click('search-saved')");
    }
    wait_for(app, "the list", |now| now["menu"] == true)
}

fn clear_the_query(app: &mut App) {
    set_mode(app, false, false);
    app.js("window.__m.set('search-input', '')");
    wait_for(app, "an empty field", |now| {
        now["value"] == "" && now["footer"] == ""
    });
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn a_saved_regex_with_match_case_survives_a_relaunch_and_runs_from_the_list_and_the_palette() {
    let (exact, ignoring) = exact_and_ignoring();
    assert!(exact < ignoring, "the fixture plants capitals");
    let data = data_dir("relaunch");
    let mut app = open(&data);
    open_pane(&mut app);
    set_mode(&mut app, true, true);
    let typed = search(&mut app, PATTERN);
    let manual = count(&typed);
    assert_eq!(manual, exact, "typed by hand: {}", typed["footer"]);
    assert_eq!(typed["saveOn"], false);

    app.js("window.__m.click('search-save')");
    let asked = wait_for(&mut app, "the name form", |now| now["form"] == true);
    assert_eq!(asked["formName"], PATTERN, "the name starts as the query");
    app.js("window.__m.set('search-save-name', 'Zephyr exact')");
    app.js("window.__m.keyIn('search-save-name', 'Enter')");
    let saved = wait_for(&mut app, "the saved state", |now| now["saveOn"] == true);
    assert_eq!(saved["form"], false);
    assert_eq!(saved["saveLabel"], "Saved as \"Zephyr exact\"");
    assert_eq!(saved["toast"], "Saved \"Zephyr exact\".");
    let file = wait_stored(&data, "the saved search", |file| {
        file["saved"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    });
    assert_eq!(
        file["saved"],
        json!([{"name": "Zephyr exact", "text": PATTERN, "regex": true, "matchCase": true}])
    );
    no_errors(&mut app);
    app.close();

    let mut again = open(&data);
    open_pane(&mut again);
    clear_the_query(&mut again);
    let listed = show_list(&mut again);
    assert_eq!(listed["heads"][0], "Saved");
    assert_eq!(listed["rows"][0]["name"], "Zephyr exact");
    assert_eq!(listed["rows"][0]["text"], PATTERN);
    assert_eq!(listed["rows"][0]["tag"], "Regex · Match case");
    again.js("window.__m.clickRow(0, '.ssrun')");
    let ran = wait_for(&mut again, "the saved search to run", |now| {
        now["value"] == PATTERN && finished(now)
    });
    assert_eq!(ran["regex"], true);
    assert_eq!(ran["matchCase"], "true");
    assert_eq!(ran["menu"], false, "running a row closes the list");
    assert_eq!(
        count(&ran),
        manual,
        "the saved run finds what typing it found: {}",
        ran["footer"]
    );
    assert_eq!(ran["saveOn"], true, "the field shows it is a saved search");

    clear_the_query(&mut again);
    again.key(KEY_ESCAPE);
    let list_closed = wait_for(&mut again, "Esc to close the list first", |now| {
        now["menu"] == false
    });
    assert_eq!(list_closed["open"], true);
    again.key(KEY_ESCAPE);
    wait_for(&mut again, "the pane to close", |now| now["open"] == false);
    again.key_with(KEY_K, MASK_CTRL);
    wait_for(&mut again, "the palette", |now| now["palette"] == true);
    again.js("window.__m.set('palette-input', 'zephyr')");
    let offered = wait_for(&mut again, "the palette row", |now| {
        now["paletteRows"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row[0] == "Search: Zephyr exact"))
    });
    assert!(
        offered["paletteRows"]
            .as_array()
            .unwrap()
            .contains(&json!(["Search: Zephyr exact", "Saved search"]))
    );
    again.js("window.__m.clickPalette('Search: Zephyr exact')");
    let from_palette = wait_for(&mut again, "the palette search to run", |now| {
        now["open"] == true && now["value"] == PATTERN && finished(now)
    });
    assert_eq!(from_palette["matchCase"], "true");
    assert_eq!(from_palette["regex"], true);
    assert_eq!(count(&from_palette), manual, "{}", from_palette["footer"]);
    no_errors(&mut again);
    again.close();
    let _ = fs::remove_dir_all(&data);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn an_invalid_regex_is_refused_with_the_engine_message_and_nothing_is_stored() {
    let data = data_dir("invalid");
    let mut app = open(&data);
    open_pane(&mut app);
    set_mode(&mut app, true, false);
    app.js("window.__m.set('search-input', 'zephyr(quill')");
    let refused = save_as(&mut app, "Broken");
    assert_eq!(refused["form"], true);
    let error = refused["formError"].as_str().unwrap();
    assert!(
        error.starts_with("This regex has an error, so it was not saved."),
        "{error}"
    );
    assert!(
        error.len() > "This regex has an error, so it was not saved.".len() + 3,
        "the engine says why: {error}"
    );
    assert_eq!(refused["saveOn"], false);
    app.js("window.__m.set('search-input', 'zephyr(quill)')");
    let fixed = wait_for(&mut app, "the fixed pattern", |now| {
        finished(now) && now["message"] == ""
    });
    assert!(count(&fixed) > 0);
    app.js("window.__m.click('search-save-ok')");
    let kept = wait_for(&mut app, "the fixed save", |now| now["saveOn"] == true);
    assert_eq!(kept["form"], false);
    let file = wait_stored(&data, "the fixed pattern", |file| !file["saved"].is_null());
    assert_eq!(file["saved"][0]["text"], "zephyr(quill)");
    assert!(
        !fs::read_to_string(data.join("searches.json"))
            .unwrap()
            .contains("zephyr(quill\"")
    );
    no_errors(&mut app);
    app.close();
    let _ = fs::remove_dir_all(&data);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn recent_searches_come_back_newest_first_and_enter_and_f3_still_step() {
    let (exact, ignoring) = exact_and_ignoring();
    let data = data_dir("recent");
    let mut app = open(&data);
    open_pane(&mut app);
    search(&mut app, "zeph");
    search(&mut app, "zephyrq");
    let plain = search(&mut app, "zephyrquill");
    assert_eq!(stored(&data), Value::Null, "typing alone records nothing");
    app.key(KEY_ENTER);
    wait_for(&mut app, "the first match", |now| {
        now["footer"].as_str().unwrap().starts_with("Match 1 of")
    });
    set_mode(&mut app, true, false);
    let regex = search(&mut app, "zephyr\\w+");
    app.key(KEY_F3);
    wait_for(&mut app, "the first regex match", |now| {
        now["footer"].as_str().unwrap().starts_with("Match 1 of")
    });
    set_mode(&mut app, false, true);
    let cased = search(&mut app, "zephyrquill");
    assert_eq!(count(&cased), exact);
    app.key(KEY_ENTER);
    wait_for(&mut app, "the first cased match", |now| {
        now["footer"].as_str().unwrap().starts_with("Match 1 of")
    });
    let file = wait_stored(&data, "three searches", |file| {
        recent_texts(file).len() == 3
    });
    assert_eq!(
        file["recent"],
        json!([
            {"text": "zephyrquill", "regex": false, "matchCase": true},
            {"text": "zephyr\\w+", "regex": true, "matchCase": false},
            {"text": "zephyrquill", "regex": false, "matchCase": false},
        ]),
        "newest first, and the prefixes typed on the way are not kept"
    );
    no_errors(&mut app);
    app.close();

    let mut again = open(&data);
    let opened = open_pane(&mut again);
    assert_eq!(opened["menu"], true, "Ctrl+F shows the list");
    assert_eq!(opened["heads"], json!(["Recent"]));
    let tags: Vec<(Value, Value)> = opened["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["text"].clone(), row["tag"].clone()))
        .collect();
    assert_eq!(
        tags,
        [
            (json!("zephyrquill"), json!("Plain · Match case")),
            (json!("zephyr\\w+"), json!("Regex")),
            (json!("zephyrquill"), json!("Plain")),
        ]
    );
    again.key(KEY_UP);
    let up = wait_for(&mut again, "Up to pick the newest", |now| {
        now["rows"][0]["on"] == true
    });
    assert_eq!(
        up["value"], "",
        "the field keeps what was typed until Enter"
    );
    again.key(KEY_DOWN);
    wait_for(&mut again, "Down to the next older", |now| {
        now["rows"][1]["on"] == true
    });
    again.key(KEY_ENTER);
    let rerun = wait_for(&mut again, "the recalled search", |now| {
        now["value"] == "zephyr\\w+" && finished(now)
    });
    assert_eq!(rerun["regex"], true);
    assert_eq!(rerun["matchCase"], "false");
    assert_eq!(rerun["menu"], false);
    assert_eq!(count(&rerun), count(&regex), "{}", rerun["footer"]);
    assert!(
        !rerun["footer"].as_str().unwrap().starts_with("Match"),
        "running a row does not also step"
    );

    again.key(KEY_ENTER);
    wait_for(&mut again, "Enter steps", |now| {
        now["footer"].as_str().unwrap().starts_with("Match 1 of")
    });
    again.key(KEY_F3);
    wait_for(&mut again, "F3 steps", |now| {
        now["footer"].as_str().unwrap().starts_with("Match 2 of")
    });
    let file = wait_stored(&data, "the rerun moving up", |file| {
        recent_texts(file)[0] == "zephyr\\w+"
    });
    assert_eq!(
        recent_texts(&file).len(),
        3,
        "a rerun moves, it does not add"
    );

    again.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut again, "the list again", |now| {
        now["menu"] == true && now["focused"] == true
    });
    set_mode(&mut again, false, false);
    again.js("window.__m.set('search-input', 'zephyrq')");
    let filtered = wait_for(&mut again, "the filter", |now| {
        now["menu"] == true && now["rows"].as_array().unwrap().len() == 2
    });
    assert!(
        filtered["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["text"] == "zephyrquill")
    );
    again.key(KEY_ENTER);
    let stepped = wait_for(&mut again, "Enter with no row picked to step", |now| {
        now["footer"].as_str().unwrap().starts_with("Match ") && finished(now)
    });
    assert_eq!(stepped["value"], "zephyrq");
    assert_eq!(stepped["menu"], false);
    assert_eq!(count(&plain), ignoring);

    again.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut again, "the list", |now| now["menu"] == true);
    again.key(KEY_ESCAPE);
    let closed = wait_for(&mut again, "Esc to close only the list", |now| {
        now["menu"] == false
    });
    assert_eq!(closed["open"], true);
    assert_eq!(closed["value"], "zephyrq");
    again.key(KEY_ESCAPE);
    wait_for(&mut again, "the next Esc to close the pane", |now| {
        now["open"] == false
    });
    no_errors(&mut again);
    again.close();
    let _ = fs::remove_dir_all(&data);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn saved_and_recent_searches_work_inside_focus_mode() {
    let data = data_dir("focus");
    let mut app = open(&data);
    open_pane(&mut app);
    let first = search(&mut app, "zephyrquill");
    app.key(KEY_ENTER);
    wait_for(&mut app, "the first match", |now| {
        now["footer"].as_str().unwrap().starts_with("Match 1 of")
    });
    save_as(&mut app, "Zephyr");
    app.key(KEY_ESCAPE);
    wait_for(&mut app, "the pane to close", |now| now["open"] == false);
    app.js("document.getElementById('toggle-focus').click()");
    wait_for(&mut app, "focus mode", |now| now["focusMode"] == true);
    let opened = open_pane(&mut app);
    assert_eq!(opened["focusMode"], true);
    assert_eq!(opened["menu"], true);
    assert_eq!(opened["rows"][0]["name"], "Zephyr");
    app.js("window.__m.set('search-input', '')");
    app.key(KEY_DOWN);
    wait_for(&mut app, "the saved row", |now| {
        now["rows"][0]["on"] == true
    });
    app.key(KEY_ENTER);
    let ran = wait_for(&mut app, "the saved run in focus", |now| {
        now["value"] == "zephyrquill" && finished(now)
    });
    assert_eq!(count(&ran), count(&first));
    assert_eq!(ran["focusMode"], true);
    app.key_with(KEY_F, MASK_CTRL);
    wait_for(&mut app, "the list in focus", |now| now["menu"] == true);
    app.key(KEY_ESCAPE);
    let list_closed = wait_for(&mut app, "the list to close", |now| now["menu"] == false);
    assert_eq!(list_closed["open"], true);
    assert_eq!(list_closed["focusMode"], true);
    app.key(KEY_ESCAPE);
    let pane_closed = wait_for(&mut app, "the pane to close", |now| now["open"] == false);
    assert_eq!(
        pane_closed["focusMode"], true,
        "focus stays until the next Esc"
    );
    no_errors(&mut app);
    app.close();
    let _ = fs::remove_dir_all(&data);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh"]
fn rename_delete_undo_and_clear_history_keep_names_as_plain_text() {
    let data = data_dir("manage");
    let hostile = "<img src=x onerror=alert(1)>";
    let mut app = open(&data);
    open_pane(&mut app);
    search(&mut app, "zephyrquill");
    app.key(KEY_ENTER);
    wait_for(&mut app, "a step", |now| {
        now["footer"].as_str().unwrap().starts_with("Match 1 of")
    });
    search(&mut app, "quillmarble");
    save_as(&mut app, hostile);
    let taken = save_as(&mut app, hostile);
    assert_eq!(
        taken["form"], false,
        "saving the same query again keeps one entry"
    );
    search(&mut app, "zephyr");
    let twin = save_as(&mut app, "<IMG src=x onerror=alert(1)>");
    assert_eq!(
        twin["formError"],
        format!("A saved search is already called \"{hostile}\".").as_str()
    );
    app.js("window.__m.click('search-save-cancel')");

    let listed = show_list(&mut app);
    assert_eq!(
        listed["rows"][0]["name"], hostile,
        "the name is shown as typed"
    );
    assert_eq!(app.js("window.__m.strangers()"), json!([]));
    assert_eq!(
        app.js("document.querySelectorAll('#search-memory img').length"),
        json!(0)
    );
    app.key_with(KEY_K, MASK_CTRL);
    wait_for(&mut app, "the palette", |now| now["palette"] == true);
    app.js("window.__m.set('palette-input', 'onerror')");
    let offered = wait_for(&mut app, "the hostile palette row", |now| {
        now["paletteRows"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    });
    assert_eq!(
        offered["paletteRows"][0],
        json!([format!("Search: {hostile}"), "Saved search"])
    );
    assert_eq!(
        app.js("document.querySelectorAll('#palette-rows img').length"),
        json!(0)
    );
    app.key(KEY_ESCAPE);
    wait_for(&mut app, "the palette to close", |now| {
        now["palette"] == false
    });
    show_list(&mut app);

    app.js("window.__m.clickRow(0, '.mini[aria-label=\"Rename saved search\"]')");
    wait_for(&mut app, "the rename field", |now| {
        now["rows"][0]["renaming"] == true
    });
    app.js("(()=>{const field=document.querySelector('#search-memory .ssrename');field.value='Marble';field.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));return true})()");
    wait_for(&mut app, "the new name", |now| {
        now["rows"][0]["name"] == "Marble"
    });
    wait_stored(&data, "the rename", |file| {
        file["saved"][0]["name"] == "Marble"
    });

    app.js("window.__m.clickRow(0, '.mini[aria-label=\"Delete saved search\"]')");
    let deleted = wait_for(&mut app, "the delete", |now| {
        now["undo"] == "Deleted saved search \"Marble\"."
    });
    assert!(
        deleted["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["name"] != "Marble")
    );
    wait_stored(&data, "the delete", |file| file["saved"] == json!([]));
    app.js("document.getElementById('undo-action').click()");
    wait_stored(&data, "the undo", |file| {
        file["saved"][0]["name"] == "Marble"
    });

    let reopened = show_list(&mut app);
    let recent_at = reopened["rows"]
        .as_array()
        .unwrap()
        .iter()
        .position(|row| row["text"] == "zephyrquill")
        .unwrap();
    app.js(&format!(
        "window.__m.clickRow({recent_at}, '.mini[aria-label=\"Remove from recent searches\"]')"
    ));
    wait_stored(&data, "the removal", |file| {
        !recent_texts(file).contains(&"zephyrquill".to_owned())
    });
    app.js("window.__m.set('search-input', 'zephyrq')");
    std::thread::sleep(Duration::from_millis(300));
    app.key(KEY_ENTER);
    let file = wait_stored(&data, "a new recent search", |file| {
        recent_texts(file).contains(&"zephyrq".to_owned())
    });
    let before_clear = recent_texts(&file);
    show_list(&mut app);
    app.js("[...document.querySelectorAll('#search-memory .sshead .btn')].find((button)=>button.textContent==='Clear history').click()");
    wait_for(&mut app, "the clear", |now| {
        now["undo"] == "Cleared recent searches."
    });
    wait_stored(&data, "no recent searches", |file| {
        file["recent"] == json!([])
    });
    assert_eq!(
        stored(&data)["saved"][0]["name"],
        "Marble",
        "clearing history keeps saved searches"
    );
    app.js("document.getElementById('undo-action').click()");
    wait_stored(&data, "the history back", |file| {
        recent_texts(file) == before_clear
    });
    no_errors(&mut app);
    app.close();
    let _ = fs::remove_dir_all(&data);
}
