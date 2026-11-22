use serde_json::{Value, json};
use jadeslip_e2e::{App, Listener, scratch_dir};
use jadeslip_fixtures::{CATALOG, Options, generate_book};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

const INSTALL: &str = r#"(async()=>{
  const main = await import('/main.js');
  const host = document.getElementById('book-host');
  const notice = document.getElementById('notice');
  const violations = [];
  document.addEventListener('securitypolicyviolation', (event) => violations.push(event.effectiveDirective));
  const FORBIDDEN = new Set(['script','iframe','frame','frameset','object','embed','applet','form','base','meta','link','portal','noscript','template']);
  const RESOURCE_ATTRS = new Set(['src','srcset','poster','action','formaction','data','ping','background','manifest','code','codebase','lowsrc','dynsrc']);
  const EVIL_CSS = /@import|expression\(|url\(\s*(?!["']?(?:jadeslip-book:|#))/i;
  const audit = (root) => {
    const findings = new Set();
    for (const element of root.querySelectorAll('*')) {
      const name = element.localName;
      if (FORBIDDEN.has(name)) findings.add('element:' + name);
      for (const attr of element.attributes) {
        const key = attr.name.toLowerCase();
        const local = attr.localName.toLowerCase();
        if (key.startsWith('on')) findings.add('handler:' + key);
        if (RESOURCE_ATTRS.has(local) && !attr.value.startsWith('jadeslip-book://')) findings.add('resource:' + key);
        if (local === 'href' && name !== 'a' && name !== 'area' && !attr.value.startsWith('jadeslip-book://') && !attr.value.startsWith('#')) findings.add('href:' + name);
        if (local === 'href' && /^\s*(javascript|vbscript|data|file|ipc|tauri|jadeslip):/i.test(attr.value)) findings.add('scheme:' + name);
        if (key === 'style' && EVIL_CSS.test(attr.value)) findings.add('style-attr');
        if (key === 'target') findings.add('target');
      }
      if (name === 'style' && EVIL_CSS.test(element.textContent)) findings.add('style-sheet');
    }
    return [...findings];
  };
  const frames = () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
  const targets = (root) => {
    const seen = [];
    for (const element of root.querySelectorAll('a,area,button,input,summary,label,select,textarea,[onclick],[href]')) {
      element.scrollIntoView?.();
      const box = element.getBoundingClientRect();
      if (box.width > 0 && box.height > 0) seen.push({ x: box.x + box.width / 2, y: box.y + box.height / 2 });
    }
    return seen;
  };
  window.__visit = async (ix, id) => {
    await main.goTo(ix);
    await frames();
    const root = host.shadowRoot;
    return {
      mounted: (root?.textContent ?? '').includes('attack row ' + id),
      error: !notice.hidden && notice.classList.contains('error'),
      findings: root ? audit(root) : [],
      targets: root ? targets(root) : [],
    };
  };
  window.__after = () => ({
    violations: violations.length,
    tauriGlobal: typeof window.__TAURI__,
    internals: typeof window.__TAURI_INTERNALS__?.invoke,
    ran: [...document.documentElement.attributes].filter((a) => a.name.startsWith('data-ran-')).map((a) => a.name.slice(9)),
    namedAccess: ['ss-root', 'x'].filter((name) => name in window),
  });
  window.__visitEdit = async (ix, id) => {
    await main.goTo(ix);
    const refused = !host.shadowRoot?.querySelector('.ss-body');
    if (!refused) {
      main.dispatch({ type: 'enter', editable: true });
      await frames();
    }
    const root = host.shadowRoot;
    return {
      mounted: (root?.textContent ?? '').includes('attack row ' + id),
      editable: root?.querySelector('.ss-body')?.getAttribute('contenteditable') === 'true',
      error: !notice.hidden && notice.classList.contains('error'),
      findings: root ? audit(root) : [],
      targets: root ? targets(root) : [],
    };
  };
  window.__visitUnprotected = async (ix, id) => {
    const ipc = await import('/ipc.js');
    const chapter = await ipc.renderChapter(ix);
    if (!window.__handlerBypassed) {
      window.addEventListener('click', (event) => event.stopImmediatePropagation(), true);
      window.__handlerBypassed = true;
    }
    const shadow = host.shadowRoot ?? host.attachShadow({ mode: 'open' });
    const range = document.createRange();
    range.selectNodeContents(host);
    const fragment = range.createContextualFragment(chapter.xhtml);
    shadow.replaceChildren(fragment);
    await frames();
    return {
      mounted: shadow.textContent.includes('attack row ' + id),
      error: false,
      findings: audit(shadow),
      targets: targets(shadow),
    };
  };
  window.__frames = frames;
  window.__goTo = main.goTo;
  window.__control = () => {
    const root = host.shadowRoot;
    const links = [...root.querySelectorAll('a')].map((a) => {
      const box = a.getBoundingClientRect();
      return { href: a.getAttribute('href'), x: box.x + box.width / 2, y: box.y + box.height / 2 };
    });
    const image = root.querySelector('img');
    return { links, imageWidth: image?.naturalWidth ?? 0, imageSrc: image?.getAttribute('src') ?? '', chapterText: root.textContent.length };
  };
  return true;
})()"#;

#[derive(Default)]
struct RowResult {
    id: String,
    family: String,
    mounted: bool,
    mount_error: bool,
    hits: Vec<String>,
    ran: Vec<String>,
    pwned: Vec<String>,
    findings: Vec<String>,
    violations: u64,
    clicks: u64,
    script_failed: Option<String>,
    probes: BTreeSet<String>,
    probe_hit: bool,
    ran_here: bool,
    pwned_here: bool,
}

impl RowResult {
    fn fired(&self) -> bool {
        self.probe_hit || self.ran_here || self.pwned_here
    }
}

#[derive(Default)]
struct Run {
    rows: Vec<RowResult>,
    all_hits: Vec<String>,
    pwned_total: Vec<String>,
    external: Vec<String>,
    listener_alive: bool,
    canary_hits: Vec<String>,
    inline_script_ran: bool,
    denied_commands: BTreeMap<String, bool>,
    tauri_global: String,
    internals: String,
    named_access: Vec<String>,
    control: Value,
    control_external_before_trusted: u64,
    control_external_after_trusted: bool,
    control_probe_before_trusted: bool,
    control_probe_after_trusted: bool,
    control_other_chapter_followed: bool,
    url_after_nav: String,
    ui_url: String,
    nav_blocked: Vec<String>,
    nav_allowed_foreign: Vec<String>,
    windows: u64,
}

fn attack_book(listener: &Listener) -> PathBuf {
    let dir = scratch_dir().join(format!("attack-{}", listener.port));
    let definition = CATALOG
        .iter()
        .find(|definition| definition.name == "attack")
        .expect("the catalog has the attack book");
    std::fs::create_dir_all(&dir).expect("attack directory");
    generate_book(
        definition,
        &dir,
        Options {
            probe_port: listener.port,
        },
    )
    .expect("attack book builds");
    dir.join("attack.epub")
}

fn attack_rows(listener: &Listener) -> (PathBuf, Vec<Value>) {
    let book = attack_book(listener);
    let truth_path = book.with_file_name("attack.truth.json");
    let truth: Value = serde_json::from_slice(&std::fs::read(truth_path).expect("truth exists"))
        .expect("truth parses");
    (
        book,
        truth["attack"]["rows"].as_array().expect("rows").clone(),
    )
}

fn settle(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn relaunch(book: &std::path::Path, off: &[&str]) -> App {
    let mut app = App::launch(Some(book), off);
    app.wait_first_text();
    app.assert_on_headless_output();
    app.js(INSTALL);
    app
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mount {
    Real,
    Edit,
    Unprotected,
}

fn visit_row(
    app: &mut App,
    row: &Value,
    listener: &Listener,
    seen_pwned: &mut usize,
    mount: Mount,
) -> RowResult {
    let id = row["id"].as_str().expect("row id");
    let ix = row["chapter"].as_u64().expect("row chapter");
    let trigger = row["trigger"].as_str().unwrap_or_default();
    let mut result = RowResult {
        id: id.to_owned(),
        family: row["family"].as_str().unwrap_or_default().to_owned(),
        probes: string_list(&row["probes"]).into_iter().collect(),
        ..RowResult::default()
    };
    let hits_before = listener.hits().len();
    let violations_before = app
        .try_js("window.__after().violations", 3_000)
        .ok()
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let visit = match mount {
        Mount::Real => "__visit",
        Mount::Edit => "__visitEdit",
        Mount::Unprotected => "__visitUnprotected",
    };
    let visited = match app.try_js(&format!("window.{visit}({ix},{})", json!(id)), 8_000) {
        Ok(visited) => visited,
        Err(error) => {
            result.script_failed = Some(error);
            return result;
        }
    };
    result.mounted = visited["mounted"].as_bool().unwrap_or(false);
    result.mount_error = visited["error"].as_bool().unwrap_or(false);
    result.findings = string_list(&visited["findings"]);
    if trigger == "click" {
        for target in visited["targets"].as_array().cloned().unwrap_or_default() {
            app.click(
                target["x"].as_f64().unwrap_or(0.0),
                target["y"].as_f64().unwrap_or(0.0),
            );
            result.clicks += 1;
        }
        app.click(748.0, 400.0);
        result.clicks += 1;
    }
    settle(if trigger == "click" { 60 } else { 25 });
    let _ = app.try_js("window.__frames().then(()=>1)", 3_000);
    if let Ok(after) = app.try_js("window.__after()", 3_000) {
        result.ran = string_list(&after["ran"]);
        result.violations = after["violations"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(violations_before);
    }
    let _ = app.try_js(
        "(()=>{for(const a of [...document.documentElement.attributes]){if(a.name.startsWith('data-ran-'))document.documentElement.removeAttribute(a.name)}return 1})()",
        3_000,
    );
    let state = app.state();
    let pwned = string_list(&state["pwned"]);
    result.pwned = pwned.get(*seen_pwned..).unwrap_or_default().to_vec();
    *seen_pwned = pwned.len();
    result.hits = listener
        .hits()
        .get(hits_before..)
        .unwrap_or_default()
        .to_vec();
    result
}

fn control_row_checks(app: &mut App, listener: &Listener, run: &mut Run) {
    app.js("window.__visit(0,'control_plain')");
    settle(100);
    let control = app.js("window.__control()");
    run.control_probe_before_trusted = listener.hits().iter().any(|hit| hit.starts_with("/C/"));
    app.js(
        "(()=>{for(const a of document.getElementById('book-host').shadowRoot.querySelectorAll('a')){a.click()}return 1})()",
    );
    settle(100);
    run.control_external_before_trusted = string_list(&app.state()["external"]).len() as u64;
    let links = control["links"].as_array().cloned().unwrap_or_default();
    let by_href = |needle: &str| {
        links
            .iter()
            .find(|link| {
                link["href"]
                    .as_str()
                    .is_some_and(|href| href.contains(needle))
            })
            .cloned()
    };
    if let Some(link) = by_href("/C/") {
        app.click(
            link["x"].as_f64().unwrap_or(0.0),
            link["y"].as_f64().unwrap_or(0.0),
        );
        settle(200);
    }
    run.control_external_after_trusted = !string_list(&app.state()["external"]).is_empty();
    run.control_probe_after_trusted = listener.hits().iter().any(|hit| hit.starts_with("/C/"));
    if let Some(link) = by_href(".xhtml") {
        let before = app.js("document.getElementById('where-title').title");
        app.click(
            link["x"].as_f64().unwrap_or(0.0),
            link["y"].as_f64().unwrap_or(0.0),
        );
        settle(200);
        run.control_other_chapter_followed =
            app.js("document.getElementById('where-title').title") != before;
    }
    run.control = control;
}

fn canaries(app: &mut App, listener: &Listener, run: &mut Run) {
    let origin = listener.origin();
    let hits_before = listener.hits().len();
    run.listener_alive = listener.self_test();
    let commands = [
        "plugin:window|close",
        "plugin:event|listen",
        "plugin:path|resolve_directory",
        "plugin:fs|read_file",
        "plugin:shell|execute",
        "plugin:webview|create_webview",
        "no_such_command",
    ];
    for command in commands {
        let refused = app.js(&format!(
            "window.__TAURI_INTERNALS__.invoke({}, {{}}).then(()=>false,()=>true)",
            json!(command)
        ));
        run.denied_commands
            .insert(command.to_owned(), refused.as_bool().unwrap_or(false));
    }
    app.js(&format!(
        "(async()=>{{await fetch('{origin}/K/csp_fetch',{{mode:'no-cors'}}).catch(()=>0);new Image().src='{origin}/K/csp_img';const s=document.createElement('script');s.textContent='window.__inlineRan=1';document.body.append(s);return 1}})()"
    ));
    settle(150);
    run.inline_script_ran = app.js("window.__inlineRan===1").as_bool().unwrap_or(false);
    let ui_url = app.state()["ui_url"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    run.ui_url.clone_from(&ui_url);
    let after = app.js("window.__after()");
    run.tauri_global = after["tauriGlobal"].as_str().unwrap_or_default().to_owned();
    run.internals = after["internals"].as_str().unwrap_or_default().to_owned();
    run.named_access = string_list(&after["namedAccess"]);
    let book_url = run.control["imageSrc"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let navigations = [
        format!("location.assign('{origin}/N/location')"),
        format!(
            "(()=>{{const a=document.createElement('a');a.href='{origin}/N/anchor';document.body.append(a);a.click();a.remove()}})()"
        ),
        format!(
            "(()=>{{const a=document.createElementNS('http://www.w3.org/1999/xhtml','a');a.href='{origin}/N/blank';a.target='_blank';document.body.append(a);a.click();a.remove()}})()"
        ),
        format!("window.open('{origin}/N/open')"),
        format!("location.assign('{book_url}')"),
        "location.assign('jadeslip://localhost/app.css')".to_owned(),
        "location.assign('jadeslip://localhost/index.html?x=1')".to_owned(),
    ];
    for script in navigations {
        let _ = app.try_js(&format!("(()=>{{{script};return 1}})()"), 1_500);
        settle(150);
    }
    settle(200);
    let state = app.state();
    run.url_after_nav = state["url"].as_str().unwrap_or_default().to_owned();
    run.windows = state["windows"].as_u64().unwrap_or(0);
    run.external = string_list(&state["external"]);
    for entry in state["navigations"].as_array().cloned().unwrap_or_default() {
        let url = entry[0].as_str().unwrap_or_default().to_owned();
        if entry[1].as_bool() == Some(true) {
            if url != ui_url {
                run.nav_allowed_foreign.push(url);
            }
        } else {
            run.nav_blocked.push(url);
        }
    }
    run.canary_hits = listener
        .hits()
        .get(hits_before..)
        .unwrap_or_default()
        .to_vec();
    run.canary_hits.retain(|hit| hit != "/K/listener_self_test");
    run.pwned_total = string_list(&state["pwned"]);
}

fn run_attack(off: &[&str]) -> Run {
    run_attack_with(off, Mount::Real)
}

fn run_attack_with(off: &[&str], mount: Mount) -> Run {
    let listener = Listener::start();
    let (book, rows) = attack_rows(&listener);
    let mut run = Run::default();
    let mut app = relaunch(&book, off);
    control_row_checks(&mut app, &listener, &mut run);
    let hits_before_rows = listener.hits().len();
    let mut seen_pwned = 0;
    for row in &rows {
        let result = visit_row(&mut app, row, &listener, &mut seen_pwned, mount);
        let state = app.state();
        let lost_page = result.script_failed.is_some() || state["url"] != state["ui_url"];
        run.rows.push(result);
        if lost_page {
            app.close();
            app = relaunch(&book, off);
            seen_pwned = 0;
        }
    }
    settle(500);
    run.all_hits = listener
        .hits()
        .get(hits_before_rows..)
        .unwrap_or_default()
        .to_vec();
    let ran: Vec<String> = run.rows.iter().flat_map(|row| row.ran.clone()).collect();
    let pwned: Vec<String> = run.rows.iter().flat_map(|row| row.pwned.clone()).collect();
    for row in &mut run.rows {
        row.probe_hit = run.all_hits.iter().any(|hit| row.probes.contains(hit));
        row.ran_here = ran.contains(&row.id);
        let prefix = format!("{}-", row.id);
        row.pwned_here = pwned.iter().any(|from| from.starts_with(&prefix));
    }
    canaries(&mut app, &listener, &mut run);
    run.pwned_total.extend(pwned);
    run.pwned_total.sort();
    run.pwned_total.dedup();
    app.close();
    let _ = std::fs::remove_dir_all(book.parent().expect("book has a directory"));
    run
}

fn csp_failures(run: &Run) -> Vec<String> {
    let mut failures = Vec::new();
    if run.canary_hits.iter().any(|hit| hit.starts_with("/K/")) {
        failures.push(format!(
            "csp: page requests reached the listener: {:?}",
            run.canary_hits
        ));
    }
    if run.inline_script_ran {
        failures.push("csp: an inline script ran".to_owned());
    }
    failures
}

fn sanitizer_failures(run: &Run) -> Vec<String> {
    let mut failures = Vec::new();
    for row in &run.rows {
        if !row.findings.is_empty() {
            failures.push(format!("sanitizer: {} mounted {:?}", row.id, row.findings));
        }
        if row.violations > 0 {
            failures.push(format!(
                "sanitizer: {} tripped the CSP {} times",
                row.id, row.violations
            ));
        }
    }
    failures
}

fn navigation_failures(run: &Run) -> Vec<String> {
    let mut failures = Vec::new();
    if run.url_after_nav != run.ui_url {
        failures.push(format!(
            "navigation: the main window left the UI for {}",
            run.url_after_nav
        ));
    }
    if run.canary_hits.iter().any(|hit| hit.starts_with("/N/")) {
        failures.push(format!(
            "navigation: a navigation reached the listener: {:?}",
            run.canary_hits
        ));
    }
    if !run.nav_allowed_foreign.is_empty() {
        failures.push(format!(
            "navigation: the guard allowed {:?}",
            run.nav_allowed_foreign
        ));
    }
    if run.windows != 1 {
        failures.push(format!("navigation: {} windows are open", run.windows));
    }
    failures
}

fn payload_failures(run: &Run) -> Vec<String> {
    let mut failures = Vec::new();
    for row in run.rows.iter().filter(|row| row.family != "control") {
        if row.fired() || !row.hits.is_empty() || !row.ran.is_empty() || !row.pwned.is_empty() {
            failures.push(format!(
                "payload: {} fired, hits {:?}, ran {:?}, pwned {:?}",
                row.id, row.hits, row.ran, row.pwned
            ));
        }
        if !row.mounted {
            failures.push(format!("not exercised: {} did not mount its text", row.id));
        }
        if let Some(error) = &row.script_failed {
            failures.push(format!("not exercised: {} lost the page: {error}", row.id));
        }
    }
    let unattributed: Vec<&String> = run
        .all_hits
        .iter()
        .filter(|hit| !hit.starts_with("/C/"))
        .collect();
    if !unattributed.is_empty() {
        failures.push(format!(
            "payload: probes reached the listener: {unattributed:?}"
        ));
    }
    if !run.pwned_total.is_empty() {
        failures.push(format!(
            "payload: commands were invoked: {:?}",
            run.pwned_total
        ));
    }
    failures
}

fn exposure_failures(run: &Run) -> Vec<String> {
    let mut failures = Vec::new();
    if run.tauri_global != "undefined" {
        failures.push(format!(
            "exposure: window.__TAURI__ is {}",
            run.tauri_global
        ));
    }
    if run.internals != "function" {
        failures.push(format!(
            "exposure: a book replaced the internals, typeof {}",
            run.internals
        ));
    }
    if !run.named_access.is_empty() {
        failures.push(format!(
            "exposure: shadow ids reached window: {:?}",
            run.named_access
        ));
    }
    for (command, refused) in &run.denied_commands {
        if !refused {
            failures.push(format!("exposure: {command} was not refused"));
        }
    }
    failures
}

fn control_failures(run: &Run) -> Vec<String> {
    let mut failures = Vec::new();
    if !run.listener_alive {
        failures.push("control: the listener did not answer".to_owned());
    }
    if run.control["imageWidth"].as_u64().unwrap_or(0) != 32 {
        failures.push(format!(
            "control: the local image width is {}",
            run.control["imageWidth"]
        ));
    }
    if run.control_probe_before_trusted {
        failures.push("control: the external link probe hit before any click".to_owned());
    }
    if run.control_external_before_trusted != 0 {
        failures.push("control: an untrusted click opened an external link".to_owned());
    }
    if !run.control_external_after_trusted || !run.control_probe_after_trusted {
        failures.push("control: a trusted click did not open the external link".to_owned());
    }
    if !run.control_other_chapter_followed {
        failures.push("control: a trusted click did not follow the chapter link".to_owned());
    }
    failures
}

fn all_failures(run: &Run) -> Vec<String> {
    let mut failures = control_failures(run);
    failures.extend(payload_failures(run));
    failures.extend(csp_failures(run));
    failures.extend(sanitizer_failures(run));
    failures.extend(navigation_failures(run));
    failures.extend(exposure_failures(run));
    failures
}

fn summary(label: &str, run: &Run) {
    let mounted = run.rows.iter().filter(|row| row.mounted).count();
    let fired = run
        .rows
        .iter()
        .filter(|row| row.family != "control" && row.fired())
        .count();
    let clicks: u64 = run.rows.iter().map(|row| row.clicks).sum();
    let findings = run
        .rows
        .iter()
        .filter(|row| !row.findings.is_empty())
        .count();
    let violations: u64 = run.rows.iter().map(|row| row.violations).sum();
    println!(
        "[{label}] rows {} mounted {mounted} fired {fired} synthesised clicks {clicks} rows with DOM findings {findings} CSP violations {violations} listener hits {} commands invoked {} canary hits {:?} inline script ran {} url kept {} blocked navigations {} externals {}",
        run.rows.len(),
        run.all_hits.len(),
        run.pwned_total.len(),
        run.canary_hits,
        run.inline_script_ran,
        run.url_after_nav == run.ui_url,
        run.nav_blocked.len(),
        run.external.len(),
    );
}

fn print_failures(failures: &[String]) {
    let mut categories: BTreeMap<&str, u64> = BTreeMap::new();
    for failure in failures {
        let category = failure.split(':').next().unwrap_or_default();
        *categories.entry(category).or_default() += 1;
    }
    println!("detected by category: {categories:?}");
}

fn silent_rows(run: &Run) -> (BTreeMap<&str, (u32, u32)>, BTreeSet<String>) {
    let mut families: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    let mut silent = BTreeSet::new();
    for row in run.rows.iter().filter(|row| row.family != "control") {
        let entry = families.entry(row.family.as_str()).or_default();
        entry.1 += 1;
        if row.fired() {
            entry.0 += 1;
        } else {
            silent.insert(row.id.clone());
        }
    }
    (families, silent)
}

fn assert_silent_rows_match(silent: &BTreeSet<String>, pinned: &[(&str, &[&str])]) {
    let expected: BTreeSet<String> = pinned
        .iter()
        .flat_map(|(_, ids)| ids.iter().map(|id| (*id).to_owned()))
        .collect();
    let new_silent: Vec<_> = silent.difference(&expected).collect();
    let new_firing: Vec<_> = expected.difference(silent).collect();
    assert!(
        new_silent.is_empty(),
        "dead rows, whose zero proves nothing: {new_silent:?}"
    );
    assert!(new_firing.is_empty(), "stale silent list: {new_firing:?}");
    for (reason, ids) in pinned {
        println!("silent, {} rows: {reason}", ids.len());
    }
}

const FIRES_ONLY_IF_THE_POINTER_IS_ALREADY_THERE: &[&str] = &["soup_attr_breakout"];

const SILENT_WITH_SANITIZER_AND_CSP_OFF: &[(&str, &[&str])] = &[
    (
        "The row has no probe. The DOM audit and exposure checks cover it.",
        &[
            "click_a_app",
            "click_a_file",
            "click_a_ipc",
            "click_a_svg_document",
            "click_form_ipc",
            "click_vbscript",
            "css_host_escape",
            "css_javascript_url",
            "css_overlay_fixed",
            "doc_entity_expansion",
            "doc_entity_external_file",
            "doc_meta_refresh_javascript",
            "doc_xinclude",
            "forge_data_ss_block",
            "forge_id_clobber",
            "forge_scheme_url",
            "frame_embed_javascript",
            "frame_iframe_data_base64",
            "frame_object_javascript",
            "scheme_iframe_app",
            "scheme_iframe_file",
            "scheme_iframe_ipc",
            "scheme_iframe_tauri",
            "scheme_img_book_traversal",
            "scheme_img_chapter_as_image",
            "scheme_img_file",
            "scheme_img_ipc",
            "scheme_img_relative_traversal",
            "scheme_img_unlisted",
        ],
    ),
    (
        "A click still meets the link handler and the navigation guard, which stay on in this run.",
        &[
            "click_a_data",
            "click_a_download",
            "click_a_ping",
            "click_a_remote",
            "click_a_target_blank",
            "click_a_target_parent",
            "click_a_target_top",
            "click_area",
            "click_button_formaction",
            "click_form_get",
            "click_form_post",
            "click_form_top",
            "click_js_entity_decimal",
            "click_js_entity_hex",
            "click_js_entity_newline",
            "click_js_entity_tab",
            "click_js_leading_space",
            "click_js_mixed_case",
            "click_js_percent",
            "click_js_plain",
            "click_svg_a_href",
            "click_svg_a_xlink",
            "click_svg_set_href",
        ],
    ),
    (
        "WebKit does not act on this markup when it is adopted into a shadow root.",
        &[
            "css_comment_split",
            "css_filter",
            "doc_base",
            "doc_doctype_external",
            "doc_entity_external_remote",
            "doc_meta_csp_relax",
            "doc_meta_refresh_remote",
            "doc_xml_stylesheet",
            "encoding_utf7",
            "event_body_attr",
            "event_img_onerror_upper",
            "event_input_onfocus",
            "event_marquee_onstart",
            "event_svg_onload",
            "mxss_noscript_title",
            "mxss_svg_foreign_script",
            "mxss_svg_title",
            "mxss_template",
            "resource_link_icon",
            "resource_link_prefetch",
            "resource_svg_use",
            "resource_track",
            "resource_video_source",
            "search_text_breakout",
            "search_text_img",
            "search_text_in_match",
            "search_text_script",
            "search_title_markup",
            "text_cdata",
            "text_comment",
            "text_double_decode",
        ],
    ),
    (
        "The payload sits in a book file or a relative URL, which only the sanitizer maps to the book scheme, so the raw markup cannot reach it.",
        &[
            "css_file_import",
            "css_svg_background",
            "forge_data_ss_orig",
            "svg_local_embed",
            "svg_local_iframe",
            "svg_local_img",
            "svg_local_object",
        ],
    ),
    (
        "The navigation guard refuses subframe loads, and it stays on in this run.",
        &[
            "frame_embed_remote",
            "frame_iframe_data",
            "frame_iframe_remote",
            "frame_iframe_srcdoc",
            "frame_object_remote",
        ],
    ),
    (
        "DOMParser marks scripts as already started, so the real mount never runs them.",
        &[
            "script_cdata",
            "script_inline_body",
            "script_inline_head",
            "script_namespaced",
            "script_src_local",
            "script_src_remote",
            "script_svg_inline",
            "script_svg_xlink",
        ],
    ),
    (
        "XML parsing refuses the raw markup, so the real mount shows an error.",
        &[
            "soup_attr_breakout",
            "soup_comment_break",
            "soup_javascript_unquoted",
            "soup_math_mglyph",
            "soup_nested_script",
            "soup_noscript_close",
            "soup_nul_in_tag",
            "soup_script_mixed",
            "soup_script_slash",
            "soup_script_unclosed",
            "soup_script_upper",
            "soup_slash_separators",
            "soup_svg_style_img",
            "soup_unclosed_img",
            "soup_unquoted_onerror",
        ],
    ),
];
const SILENT_WITH_EVERY_DEFENCE_REMOVED: &[(&str, &[&str])] = &[
    (
        "The row has no probe. The DOM audit and exposure checks cover it.",
        &[
            "click_a_app",
            "click_a_file",
            "click_a_ipc",
            "click_a_svg_document",
            "click_form_ipc",
            "click_vbscript",
            "css_host_escape",
            "css_javascript_url",
            "css_overlay_fixed",
            "doc_entity_expansion",
            "doc_entity_external_file",
            "doc_meta_refresh_javascript",
            "doc_xinclude",
            "forge_data_ss_block",
            "forge_id_clobber",
            "forge_scheme_url",
            "frame_embed_javascript",
            "frame_iframe_data_base64",
            "frame_object_javascript",
            "scheme_iframe_app",
            "scheme_iframe_file",
            "scheme_iframe_ipc",
            "scheme_iframe_tauri",
            "scheme_img_book_traversal",
            "scheme_img_chapter_as_image",
            "scheme_img_file",
            "scheme_img_ipc",
            "scheme_img_relative_traversal",
            "scheme_img_unlisted",
        ],
    ),
    (
        "WebKit refuses this navigation from a click even with the guard off, or opens no document for it.",
        &["click_a_data", "click_a_target_blank", "click_js_percent"],
    ),
    (
        "The test's capture listener that bypasses the link handler also stops the row's own handler.",
        &["click_onclick"],
    ),
    (
        "An HTML fragment mount in a shadow root ignores this markup in WebKit.",
        &[
            "css_comment_split",
            "css_filter",
            "doc_base",
            "doc_doctype_external",
            "doc_entity_external_remote",
            "doc_meta_refresh_remote",
            "doc_xml_stylesheet",
            "event_body_attr",
            "event_input_onfocus",
            "event_marquee_onstart",
            "event_svg_onload",
            "mxss_noscript_title",
            "mxss_svg_title",
            "mxss_template",
            "resource_link_icon",
            "resource_link_prefetch",
            "resource_svg_use",
            "resource_track",
            "script_namespaced",
            "script_svg_inline",
            "script_svg_xlink",
            "soup_javascript_unquoted",
            "soup_math_mglyph",
            "soup_nested_script",
            "soup_nul_in_tag",
            "soup_script_unclosed",
            "soup_slash_separators",
        ],
    ),
    (
        "The payload sits in a book file or a relative URL, which only the sanitizer maps to the book scheme, so the raw markup cannot reach it.",
        &[
            "css_file_import",
            "css_svg_background",
            "forge_data_ss_orig",
            "script_src_local",
            "svg_local_embed",
            "svg_local_iframe",
            "svg_local_img",
            "svg_local_object",
        ],
    ),
    (
        "The payload is escaped text or a refused encoding, so it stays inert even in an HTML mount.",
        &[
            "encoding_utf7",
            "search_text_breakout",
            "search_text_img",
            "search_text_in_match",
            "search_text_script",
            "search_title_markup",
            "text_double_decode",
        ],
    ),
];

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn every_payload_stays_silent_with_every_layer_on() {
    let run = run_attack(&[]);
    summary("all layers on", &run);
    let failures = all_failures(&run);
    for failure in &failures {
        println!("  {failure}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert_eq!(run.rows.len(), 158);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn every_payload_stays_silent_while_its_chapter_is_in_edit() {
    let run = run_attack_with(&[], Mount::Edit);
    summary("all layers on, in Edit", &run);
    let failures = all_failures(&run);
    for failure in &failures {
        println!("  {failure}");
    }
    assert!(failures.is_empty(), "{} failures", failures.len());
    assert_eq!(run.rows.len(), 158);
}

const INSTALL_SAVE: &str = r#"(async()=>{
  const main = await import('/main.js');
  const host = document.getElementById('book-host');
  const element = (id) => document.getElementById(id);
  const root = () => host.shadowRoot.querySelector('.ss-body');
  window.__payload = (port) => {
    const block = root().querySelector('[data-ss-b]') ?? root();
    const remote = `http://127.0.0.1:${port}`;
    const make = (name, attrs, text) => {
      const made = document.createElementNS('http://www.w3.org/1999/xhtml', name);
      for (const [key, value] of Object.entries(attrs)) made.setAttribute(key, value);
      if (text) made.textContent = text;
      return made;
    };
    block.append(
      make('b', { onclick: 'window.__pwned=1' }, 'EDITMARK'),
      make('a', { href: 'javascript:window.__pwned=1' }, 'link'),
      make('a', { href: `${remote}/K/edit_link` }, 'away'),
      make('span', { style: `background:url(${remote}/K/edit_style)` }, 'styled'),
      document.createTextNode('<script>window.__pwned=1</script>'),
    );
    const inert = new DOMParser().parseFromString(`<x xmlns='http://www.w3.org/1999/xhtml'><img src='${remote}/K/edit_img' onerror='window.__pwned=1'/><script>window.__pwned=1</script><iframe src='${remote}/K/edit_frame'/><form action='${remote}/K/edit_form'><input/></form><object data='${remote}/K/edit_object'/><svg xmlns='http://www.w3.org/2000/svg' onload='window.__pwned=1'><use href='${remote}/K/edit_use'/></svg></x>`, 'application/xhtml+xml').documentElement;
    for (const child of [...inert.childNodes]) block.append(document.adoptNode(child));
    return true;
  };
  window.__open = async (ix) => {
    await main.goTo(ix);
    if (!root()) return false;
    main.dispatch({ type: 'enter', editable: true });
    return true;
  };
  window.__state = () => ({
    read: element('mode-read').classList.contains('on'),
    unsaved: !element('unsaved').hidden,
    fail: element('failbar').hidden ? '' : element('fail-text').textContent,
  });
  window.__leave = async () => {
    return main.dispatch({ type: 'dirtyChanged', dirty: true }).then(() => main.dispatch({ type: 'esc' })).then(() => true);
  };
  window.__audit = async (ix) => {
    await main.goTo(ix);
    const body = root();
    if (!body) return { live: 0, mark: false };
    const live = [...body.querySelectorAll('*')].filter((node) => {
      if (['script', 'iframe', 'object', 'embed', 'form'].includes(node.localName)) return true;
      return [...node.attributes].some((attr) => attr.name.toLowerCase().startsWith('on')
        || /^\s*javascript:/i.test(attr.value)
        || /127\.0\.0\.1/.test(node.localName === 'a' ? '' : attr.value));
    }).length;
    return { live, mark: body.textContent.includes('EDITMARK') };
  };
  window.__pwnedFlag = () => window.__pwned ?? 0;
  return true;
})()"#;

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn saving_a_hostile_edit_stores_no_live_payload() {
    let listener = Listener::start();
    let (book, rows) = attack_rows(&listener);
    let mut app = relaunch(&book, &[]);
    app.js(INSTALL_SAVE);
    let port = listener.port;
    let (mut saved, mut not_mounted, mut failed) = (0u32, 0u32, 0u32);
    let mut stored_live: Vec<String> = Vec::new();
    let mut shown_live: Vec<String> = Vec::new();
    let mut not_well_formed: Vec<String> = Vec::new();
    for row in &rows {
        let id = row["id"].as_str().expect("row id");
        let ix = row["chapter"].as_u64().expect("row chapter") as u32;
        let old = chapter_of(&book, ix);
        if app.js(&format!("window.__open({ix})")) != json!(true) {
            not_mounted += 1;
            continue;
        }
        app.js(&format!("window.__payload({port})"));
        app.key(u32::from('q'));
        settle(30);
        app.js("window.__leave().then(()=>true)");
        let state = wait_settled(&mut app);
        if state["fail"] != json!("") {
            failed += 1;
            println!("  {id}: {}", state["fail"]);
            app = relaunch(&book, &[]);
            app.js(INSTALL_SAVE);
            continue;
        }
        saved += 1;
        let new = chapter_of(&book, ix);
        if !parses(&new) {
            not_well_formed.push(id.to_owned());
            assert!(!parses(&old), "{id}: the save broke a well-formed chapter");
        }
        for needle in LIVE_NEEDLES {
            if occurrences(&new, needle) > occurrences(&old, needle) {
                stored_live.push(format!("{id}:{needle}"));
            }
        }
        if occurrences(&new, "editmark") != occurrences(&old, "editmark") + 1 {
            stored_live.push(format!("{id}:the harmless mark was not stored"));
        }
        let audit = app.js(&format!("window.__audit({ix})"));
        if audit["live"].as_u64() != Some(0) {
            shown_live.push(format!("{id}:{}", audit["live"]));
        }
    }
    settle(500);
    let pwned = app.js("window.__pwnedFlag()");
    let state = app.state();
    let hits = listener.hits();
    let edit_hits: Vec<&String> = hits
        .iter()
        .filter(|hit| hit.starts_with("/K/edit_"))
        .collect();
    println!(
        "hostile saves: rows {} saved {saved} not_mounted {not_mounted} failed {failed} stored_live {} shown_live {} listener_hits {} pwned {pwned} ipc_pwned {}",
        rows.len(),
        stored_live.len(),
        shown_live.len(),
        edit_hits.len(),
        string_list(&state["pwned"]).len(),
    );
    println!("  not well-formed after the save: {not_well_formed:?}");
    println!("  stored {stored_live:?}");
    println!("  shown {shown_live:?}");
    app.close();
    assert!(saved > 100);
    assert!(stored_live.is_empty(), "{stored_live:?}");
    assert!(shown_live.is_empty(), "{shown_live:?}");
    assert!(edit_hits.is_empty(), "{edit_hits:?}");
    assert_eq!(pwned, json!(0));
    assert_eq!(failed, 0);
    let _ = std::fs::remove_dir_all(book.parent().expect("book has a directory"));
}

const LIVE_NEEDLES: [&str; 11] = [
    "<script",
    "onerror",
    "onclick",
    "onload",
    "javascript:",
    "<iframe",
    "<object",
    "<form",
    "/k/edit_img",
    "/k/edit_style",
    "/k/edit_use",
];

fn wait_settled(app: &mut App) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = app.js("window.__state()");
        let done = state["fail"] != json!("")
            || (state["read"] == json!(true) && state["unsaved"] == json!(false));
        if done || std::time::Instant::now() > deadline {
            return state;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn chapter_of(book: &std::path::Path, ix: u32) -> Vec<u8> {
    let file = std::fs::File::open(book).expect("book opens");
    let reader = jadeslip_core::Reader::open(file, jadeslip_core::DEFAULT_LIMITS)
        .expect("book parses");
    match reader.extract(jadeslip_core::ChapterIx(ix)) {
        Ok(jadeslip_core::Extracted::Document(bytes)) => bytes,
        _ => Vec::new(),
    }
}

fn occurrences(bytes: &[u8], needle: &str) -> usize {
    String::from_utf8_lossy(bytes)
        .to_ascii_lowercase()
        .replace(REPAIRED_SOUP_CLOSE, "")
        .matches(needle)
        .count()
}

const REPAIRED_SOUP_CLOSE: &str = "</scr<script>";

fn parses(bytes: &[u8]) -> bool {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    std::str::from_utf8(bytes)
        .is_ok_and(|text| roxmltree::Document::parse_with_options(text, options).is_ok())
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn payloads_fire_with_the_sanitizer_and_csp_off() {
    let run = run_attack(&["csp", "sanitizer"]);
    summary("control, sanitizer and CSP off", &run);
    assert!(run.listener_alive, "the listener answered its own probe");
    assert!(
        run.rows.iter().any(|row| !row.hits.is_empty()),
        "a book probe reached the listener"
    );
    assert!(
        !run.pwned_total.is_empty(),
        "a book payload reached the command probe"
    );
    let (families, silent) = silent_rows(&run);
    println!("fired per family (fired, rows): {families:?}");
    println!("commands invoked by payloads: {}", run.pwned_total.len());
    assert_silent_rows_match(&silent, SILENT_WITH_SANITIZER_AND_CSP_OFF);
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn payloads_fire_with_every_defence_removed() {
    let run = run_attack_with(&["csp", "sanitizer", "nav_guard"], Mount::Unprotected);
    summary("control, every defence removed", &run);
    assert!(
        run.rows.iter().any(|row| !row.hits.is_empty()),
        "a book probe reached the listener"
    );
    assert!(
        !run.pwned_total.is_empty(),
        "a book payload reached the command probe"
    );
    let (families, mut silent) = silent_rows(&run);
    println!("fired per family (fired, rows): {families:?}");
    println!("commands invoked by payloads: {}", run.pwned_total.len());
    for id in FIRES_ONLY_IF_THE_POINTER_IS_ALREADY_THERE {
        silent.remove(*id);
    }
    assert_silent_rows_match(&silent, SILENT_WITH_EVERY_DEFENCE_REMOVED);
}

fn assert_mutation_detected(off: &[&str], expected_category: &str) {
    let run = run_attack(off);
    summary(&format!("mutation {off:?} off"), &run);
    let failures = all_failures(&run);
    print_failures(&failures);
    assert!(
        failures
            .iter()
            .any(|failure| failure.starts_with(expected_category)),
        "turning {off:?} off was not detected: {failures:?}"
    );
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn turning_the_csp_off_is_detected() {
    assert_mutation_detected(&["csp"], "csp");
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn turning_the_sanitizer_off_is_detected() {
    assert_mutation_detected(&["sanitizer"], "sanitizer");
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn turning_the_navigation_guard_off_is_detected() {
    assert_mutation_detected(&["nav_guard"], "navigation");
}

const PANE_AUDIT: &str = r"(()=>{
  const pane = document.getElementById('search-pane');
  const list = document.getElementById('search-results');
  const allowed = new Set(['button.result', 'span.where', 'span.snip', 'span.mark', 'span.note', 'ins.', 'button.row-tick', 'button.row-skip']);
  const rowStates = new Set(['result', 'preview', 'noted', 'out', 'current']);
  const strangers = [];
  for (const node of document.getElementById('search-spacer').querySelectorAll('*')) {
    const name = `${node.localName}.${node.classList[0] ?? ''}`;
    if (!allowed.has(name)) strangers.push(name);
    if (node.classList[0] === 'result' && [...node.classList].some((token) => !rowStates.has(token))) strangers.push(`${name}#class`);
    if (node.classList[0] !== 'result' && node.classList.length > 1) strangers.push(`${name}#class`);
    for (const attribute of node.attributes) {
      if (!['class', 'style', 'data-index', 'hidden', 'aria-label', 'aria-pressed'].includes(attribute.name)) strangers.push(`${name}@${attribute.name}`);
    }
  }
  const rows = [...list.querySelectorAll('.result')].filter((row) => !row.hidden);
  const shapes = rows.map((row) => {
    const snip = row.querySelector('.snip');
    return [...snip.childNodes].map((child) => child.nodeType === 3 ? 'text' : child.localName).join(',');
  });
  return {
    open: !pane.hidden,
    finished: document.getElementById('search-action').textContent === 'Rerun',
    footer: document.getElementById('search-count').textContent,
    strangers,
    shapes,
    rows: rows.length,
    wheres: rows.map((row) => row.querySelector('.where').textContent),
    marks: rows.map((row) => row.querySelector('.mark').textContent),
    inserted: rows.map((row) => row.querySelector('ins').textContent),
    notes: rows.map((row) => row.querySelector('.note').textContent),
    summaryChildren: document.getElementById('replace-summary').children.length,
    messageChildren: document.getElementById('search-message').children.length,
    message: document.getElementById('search-message').textContent,
    toast: document.getElementById('toast').hidden ? '' : document.getElementById('toast').textContent,
    texts: rows.map((row) => row.querySelector('.snip').textContent),
    positions: rows.map((row) => { const box = row.getBoundingClientRect(); return [box.left + box.width / 2, box.top + box.height / 2]; }),
  };
})()";

fn query_pane(app: &mut App, text: &str, regex: bool) -> Value {
    app.js(&format!(
        "(()=>{{const mode=document.getElementById('search-regex');if(mode.classList.contains('on')!=={})mode.click();const input=document.getElementById('search-input');input.value={};input.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()",
        regex,
        json!(text),
    ));
    settle(250);
    for _ in 0..200 {
        let seen = app.js(PANE_AUDIT);
        if seen["finished"] == json!(true) {
            return seen;
        }
        settle(20);
    }
    panic!("the search never finished");
}

fn assert_text_only(seen: &Value, what: &str) {
    assert_eq!(seen["strangers"], json!([]), "{what}: {seen}");
    assert!(
        seen["rows"].as_u64().unwrap() > 0,
        "{what}: a search with no rows proves nothing"
    );
    for shape in seen["shapes"].as_array().unwrap() {
        assert_eq!(shape, "text,span,ins,text", "{what}");
    }
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn search_results_render_hostile_chapter_text_as_text_only() {
    let listener = Listener::start();
    let (book, rows) = attack_rows(&listener);
    let search_rows: Vec<&Value> = rows
        .iter()
        .filter(|row| row["family"] == "search-text")
        .collect();
    assert_eq!(search_rows.len(), 5);
    let mut app = relaunch(&book, &[]);
    let hits_before = listener.hits().len();
    app.op(&json!({"op": "key", "keyval": 0x66, "state": 4}));
    settle(100);
    let plain = query_pane(&mut app, "searchprobe", false);
    assert_text_only(&plain, "plain");
    let texts = plain["texts"].to_string();
    assert!(
        texts.contains("<img"),
        "the payload must reach the results: {texts}"
    );
    assert!(texts.contains("<script"), "{texts}");
    assert!(texts.contains("</span>"), "{texts}");
    assert!(texts.contains("<iframe"), "{texts}");
    let wheres = plain["wheres"].to_string();
    assert!(
        wheres.contains("<img"),
        "the hostile label must reach the results: {wheres}"
    );

    let in_match = query_pane(&mut app, "searchprobe<img[^>]*>", true);
    assert_text_only(&in_match, "regex");
    assert!(in_match["marks"].to_string().contains("onerror"));

    let clicked = plain_click_every_row(&mut app, &listener);
    println!("{clicked} hostile results clicked");
    settle(500);
    let state = app.state();
    assert_eq!(state["pwned"], json!([]), "a result called the app");
    assert_eq!(
        listener.hits().get(hits_before..).unwrap_or_default().len(),
        0,
        "a result reached the network"
    );
    assert_eq!(state["url"], state["ui_url"]);
    app.close();
    let _ = std::fs::remove_dir_all(book.parent().expect("book has a directory"));
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn replace_rows_render_hostile_patterns_replacements_and_chapters_as_text_only() {
    const ROWS: [(&str, &str, bool, &str); 8] = [
        (
            "searchprobe",
            "<img src=x onerror=\"window.__TAURI_INTERNALS__.invoke('pwned')\">",
            false,
            "img",
        ),
        (
            "searchprobe",
            "</ins><script>window.__TAURI_INTERNALS__.invoke('pwned')</script>",
            false,
            "script",
        ),
        (
            "searchprobe",
            "</span><iframe srcdoc='<script>1</script>'></iframe>",
            false,
            "iframe",
        ),
        (
            "searchprobe",
            "&lt;b&gt;&#x3C;svg onload=x&#x3E;",
            false,
            "svg",
        ),
        (
            "searchprobe",
            "<![CDATA[<a href=javascript:1>]]>",
            false,
            "CDATA",
        ),
        (
            "searchprobe",
            "<?xml-stylesheet href=x?><!--",
            false,
            "xml-stylesheet",
        ),
        (
            "(searchprobe)",
            "<a href='javascript:x'>${1}</a>",
            true,
            "javascript",
        ),
        (
            "(searchprobe)(?=.)",
            "<style>@import 'http://x'</style>$1",
            true,
            "@import",
        ),
    ];
    let listener = Listener::start();
    let (book, _) = attack_rows(&listener);
    let mut app = relaunch(&book, &[]);
    let hits_before = listener.hits().len();
    app.op(&json!({"op": "key", "keyval": 0x66, "state": 4}));
    settle(100);
    app.js("document.getElementById('replace-toggle').click(), true");
    for (pattern, replacement, regex, expected) in ROWS {
        app.js(&format!(
            "(()=>{{const input=document.getElementById('replace-input');input.value={};input.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()",
            json!(replacement)
        ));
        let seen = query_pane(&mut app, pattern, regex);
        assert_text_only(&seen, replacement);
        assert_eq!(seen["summaryChildren"], json!(0));
        assert!(
            seen["inserted"].to_string().contains(expected),
            "the hostile replacement must reach the row: {replacement}"
        );
        assert_eq!(app.state()["pwned"], json!([]), "{replacement}");
    }

    for hostile in ["<img src=x onerror=1>", "(<img", "[<script>"] {
        let seen = query_pane(&mut app, hostile, true);
        assert_eq!(seen["strangers"], json!([]), "{hostile}");
        assert_eq!(seen["messageChildren"], json!(0), "{hostile}");
    }
    app.js("(()=>{const input=document.getElementById('replace-input');input.value='${<img src=x onerror=1>}';input.dispatchEvent(new Event('input',{bubbles:true}));return true})()");
    let bad_template = query_pane(&mut app, "(searchprobe)", true);
    assert_eq!(bad_template["messageChildren"], json!(0));
    assert!(bad_template["message"].to_string().contains("group"));

    let on_disk = std::fs::read(&book).unwrap();
    app.js(&format!(
        "(()=>{{const input=document.getElementById('replace-input');input.value={};input.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()",
        json!("</p><script>window.__TAURI_INTERNALS__.invoke('pwned')</script><img src=x onerror=1>")
    ));
    let seen = query_pane(&mut app, "searchprobe", false);
    assert_text_only(&seen, "before apply");
    let positions = seen["positions"].as_array().unwrap().clone();
    for position in positions.iter().take(3) {
        app.click(position[0].as_f64().unwrap(), position[1].as_f64().unwrap());
        settle(120);
        assert_eq!(app.js(PANE_AUDIT)["strangers"], json!([]));
    }
    app.js("document.getElementById('replace-apply').click(), true");
    for _ in 0..400 {
        if app.js(PANE_AUDIT)["toast"].to_string().contains("saved") {
            break;
        }
        settle(25);
    }
    assert!(app.js(PANE_AUDIT)["toast"].to_string().contains("saved"));
    assert_ne!(
        std::fs::read(&book).unwrap(),
        on_disk,
        "the replacement was applied"
    );
    settle(300);
    let after = query_pane(&mut app, "onerror=1", false);
    assert_eq!(after["strangers"], json!([]));
    assert!(
        after["rows"].as_u64().unwrap() > 0,
        "the replacement is text in the book"
    );
    let chapter_audit = app.js(
        "(()=>{const root=document.getElementById('book-host').shadowRoot;return {scripts:root.querySelectorAll('script').length,images:root.querySelectorAll('img[src=x]').length}})()",
    );
    assert_eq!(chapter_audit["scripts"], json!(0));
    assert_eq!(chapter_audit["images"], json!(0));
    settle(300);
    let state = app.state();
    assert_eq!(state["pwned"], json!([]), "a preview row called the app");
    assert_eq!(
        listener.hits().get(hits_before..).unwrap_or_default().len(),
        0,
        "a row reached the network"
    );
    assert_eq!(state["url"], state["ui_url"]);
    let reader = jadeslip_core::Reader::open(
        std::fs::File::open(&book).unwrap(),
        jadeslip_core::DEFAULT_LIMITS,
    )
    .unwrap();
    assert!(reader.rows().len() > 10);
    app.close();
    let _ = std::fs::remove_dir_all(book.parent().expect("book has a directory"));
}

fn plain_click_every_row(app: &mut App, listener: &Listener) -> usize {
    let hits_before = listener.hits().len();
    let plain = query_pane(app, "searchprobe", false);
    let positions = plain["positions"].as_array().unwrap().clone();
    let mut clicked = 0;
    for position in positions {
        let (x, y) = (position[0].as_f64().unwrap(), position[1].as_f64().unwrap());
        if !(0.0..800.0).contains(&y) {
            continue;
        }
        app.click(x, y);
        clicked += 1;
        settle(120);
        let audit = app.js(PANE_AUDIT);
        assert_eq!(audit["strangers"], json!([]));
        let current = app.js("CSS.highlights.get('ss-search-current')?.size ?? 0");
        assert_eq!(current, json!(1), "the hit is highlighted after the click");
    }
    assert_eq!(listener.hits().len(), hits_before);
    assert!(clicked >= 4, "only {clicked} rows clicked");
    clicked
}

const LABEL_AUDIT: &str = r"(()=>{
  const allowedRail = new Set(['button', 'span']);
  const railNodes = [...document.getElementById('toc-spacer').querySelectorAll('.row, .row *')];
  const paletteNodes = [...document.getElementById('palette-rows').querySelectorAll('*')];
  const rows = [...document.querySelectorAll('#toc-spacer .row:not([hidden])')];
  return {
    railStrangers: railNodes.filter((node) => !allowedRail.has(node.localName)).map((node) => node.localName),
    paletteStrangers: paletteNodes.filter((node) => !allowedRail.has(node.localName)).map((node) => node.localName),
    attrs: [...railNodes, ...paletteNodes].flatMap((node) => [...node.attributes].map((attr) => attr.name)).filter((name) => name.startsWith('on') || name === 'src' || name === 'href'),
    rowCount: rows.length,
    labels: rows.map((row) => row.querySelector('.t').textContent),
    longest: Math.max(0, ...rows.map((row) => row.querySelector('.t').textContent.length)),
    positions: rows.map((row) => { const box = row.getBoundingClientRect(); return [box.left + 150, box.top + 17]; }),
    title: document.getElementById('where-title').textContent,
    paletteLabels: [...document.querySelectorAll('#palette-rows .pallabel')].map((node) => node.textContent),
    overflow: document.getElementById('toc-list').scrollWidth - document.getElementById('toc-list').clientWidth,
  };
})()";

fn hostile_book(listener: &Listener, name: &str) -> PathBuf {
    let dir = scratch_dir().join(format!("{name}-{}", listener.port));
    let definition = CATALOG
        .iter()
        .find(|definition| definition.name == name)
        .expect("the catalog has the book");
    std::fs::create_dir_all(&dir).expect("book directory");
    generate_book(
        definition,
        &dir,
        Options {
            probe_port: listener.port,
        },
    )
    .expect("the book builds");
    dir.join(format!("{name}.epub"))
}

fn wait_contents(app: &mut App) {
    for _ in 0..500 {
        let count = app.js("document.getElementById('count').textContent");
        if count.as_str().unwrap_or_default().contains("entries")
            && !count.as_str().unwrap_or_default().contains("Titling")
        {
            return;
        }
        settle(10);
    }
    panic!("the contents never loaded");
}

fn assert_only_text(seen: &Value, what: &str) {
    assert_eq!(seen["railStrangers"], json!([]), "{what}: {seen}");
    assert_eq!(seen["paletteStrangers"], json!([]), "{what}: {seen}");
    assert_eq!(seen["attrs"], json!([]), "{what}: {seen}");
}

fn click_all_rows(app: &mut App, seen: &Value) -> usize {
    let positions = seen["positions"].as_array().unwrap();
    for position in positions {
        app.click(position[0].as_f64().unwrap(), position[1].as_f64().unwrap());
        settle(40);
    }
    positions.len()
}

fn palette_query(app: &mut App, text: &str) -> Value {
    app.js(&format!(
        "(()=>{{const i=document.getElementById('palette-input');i.value={};i.dispatchEvent(new Event('input',{{bubbles:true}}));return true}})()",
        json!(text)
    ));
    settle(30);
    app.js(LABEL_AUDIT)
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn hostile_contents_labels_and_targets_stay_text_in_the_rail_and_palette() {
    let listener = Listener::start();
    let book = hostile_book(&listener, "toc-hostile");
    let mut app = relaunch(&book, &[]);
    app.wait_first_text();
    wait_contents(&mut app);
    let seen = app.js(LABEL_AUDIT);
    assert_only_text(&seen, "rail");
    let labels = seen["labels"].to_string();
    assert!(
        labels.contains("<img"),
        "the payload must reach the rail: {}",
        labels.len()
    );
    assert!(labels.contains("<script"), "{}", labels.len());
    assert!(labels.contains("</span>"), "{}", labels.len());
    assert!(
        labels.contains("&lt;b&gt;"),
        "an entity stays visible as text"
    );
    assert!(labels.contains("Untitled"), "an empty label gets a name");
    assert!(
        seen["longest"].as_u64().unwrap() <= 300,
        "a huge label is cut: {}",
        seen["longest"]
    );
    assert!(
        seen["overflow"].as_i64().unwrap() <= 0,
        "a deep or huge label stays inside the rail"
    );
    let clicked = click_all_rows(&mut app, &seen);
    println!("{clicked} hostile rows clicked");
    assert!(clicked >= 12);

    app.js("(()=>{const i=document.getElementById('filter');i.value='tocprobe';i.dispatchEvent(new Event('input',{bubbles:true}));return true})()");
    assert_only_text(&app.js(LABEL_AUDIT), "filtered rail");
    app.js("(()=>{const i=document.getElementById('filter');i.value='';i.dispatchEvent(new Event('input',{bubbles:true}));return true})()");

    app.key_with(0x6b, 4);
    settle(60);
    for text in ["tocprobe", "<img", "onerror", "1"] {
        let palette = palette_query(&mut app, text);
        assert_only_text(&palette, text);
        assert!(
            !palette["paletteLabels"].as_array().unwrap().is_empty(),
            "{text} found rows"
        );
    }
    let dead = palette_query(&mut app, "tocprobe target");
    assert_eq!(
        dead["paletteLabels"],
        json!([]),
        "a target outside the book is never offered"
    );
    let palette = palette_query(&mut app, "<img");
    assert!(
        palette["paletteLabels"].to_string().contains("<iframe"),
        "the payload must reach the palette"
    );
    let hint_rows = palette["paletteLabels"].as_array().unwrap().len();
    for row in 0..hint_rows {
        for _ in 0..row {
            app.key(0xff54);
        }
        app.key(0xff0d);
        settle(60);
        app.key_with(0x6b, 4);
        settle(40);
        palette_query(&mut app, "<img");
    }
    app.key(0xff1b);
    settle(40);
    let state = app.state();
    assert_eq!(state["pwned"], json!([]), "a label called the app");
    assert_eq!(
        listener.hits().len(),
        0,
        "a label or target reached the network: {:?}",
        listener.hits()
    );
    assert_eq!(
        state["external"],
        json!([]),
        "a hostile target opened a browser"
    );
    assert_eq!(state["url"], state["ui_url"]);
    app.close();
    let _ = std::fs::remove_dir_all(book.parent().expect("book has a directory"));
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn hostile_headings_stay_text_when_they_title_the_contents() {
    let listener = Listener::start();
    let book = hostile_book(&listener, "headings-hostile");
    let mut app = relaunch(&book, &[]);
    app.wait_first_text();
    wait_contents(&mut app);
    let seen = app.js(LABEL_AUDIT);
    assert_only_text(&seen, "rail");
    assert!(
        seen["labels"].to_string().contains("<img"),
        "the fill reached the rail"
    );
    assert!(
        seen["title"].as_str().unwrap().contains("<img"),
        "the footer title is text"
    );
    assert_eq!(
        app.js("document.getElementById('where-title').children.length"),
        0
    );
    click_all_rows(&mut app, &seen);
    app.key_with(0x6b, 4);
    settle(60);
    let palette = palette_query(&mut app, "tocprobe");
    assert_only_text(&palette, "palette");
    assert!(palette["paletteLabels"].to_string().contains("<img"));
    settle(100);
    assert_eq!(app.state()["pwned"], json!([]));
    assert_eq!(listener.hits().len(), 0, "{:?}", listener.hits());
    app.close();
    let _ = std::fs::remove_dir_all(book.parent().expect("book has a directory"));
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn the_attack_book_label_stays_text_in_the_rail_and_palette() {
    let listener = Listener::start();
    let (book, _) = attack_rows(&listener);
    let mut app = relaunch(&book, &[]);
    app.wait_first_text();
    wait_contents(&mut app);
    app.js("(()=>{const i=document.getElementById('filter');i.value='onerror';i.dispatchEvent(new Event('input',{bubbles:true}));return true})()");
    let seen = app.js(LABEL_AUDIT);
    assert_only_text(&seen, "rail");
    assert!(
        seen["labels"].to_string().contains("<img"),
        "the attack label is in the rail"
    );
    click_all_rows(&mut app, &seen);
    settle(200);
    assert_eq!(app.state()["pwned"], json!([]));
    assert_eq!(listener.hits().len(), 0, "{:?}", listener.hits());
    app.close();
    let _ = std::fs::remove_dir_all(book.parent().expect("book has a directory"));
}

const CHAPTER_CONTROLS_AUDIT: &str = r#"(()=>{
  const icon = (node) => node.localName === 'svg' && node.getAttribute('class') === 'icon' && node.getAttribute('aria-hidden') === 'true' && node.attributes.length === 2 && node.parentElement.matches('#rail-actions > .mini') && node.children.length === 1;
  const iconUse = (node) => node.localName === 'use' && icon(node.parentElement) && node.attributes.length === 1 && /^#i-[a-z0-9-]+$/.test(node.getAttribute('href'));
  const stranger = (root) => [...document.querySelectorAll(`${root} *`)].filter((node) => !['DIV','BUTTON','SPAN','INPUT','P','H2','LABEL'].includes(node.tagName) && !icon(node) && !iconUse(node)).map((node) => node.tagName);
  return ['#toc-list', '#deleted-dialog', '#chapter-dialog', '#undo-bar', '#newbook-dialog'].flatMap(stranger);
})()"#;

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn hostile_titles_and_a_hostile_nav_survive_chapter_operations_as_text() {
    let listener = Listener::start();
    let book = hostile_book(&listener, "toc-hostile");
    let mut app = relaunch(&book, &[]);
    app.wait_first_text();
    wait_contents(&mut app);
    let before = app.js("(async()=>{const t=await (await import('/ipc.js')).loadToc();return t.entries.map((e)=>e.label)})()");
    const TITLES: &[&str] = &[
        "<img src=x onerror=window.__TAURI_INTERNALS__.invoke('pwned')>",
        "</a></li><li><a href=\"evil.xhtml\">hostile</a></li>",
        "]]><script>window.__TAURI_INTERNALS__.invoke('pwned')</script>",
        "&lt;b&gt; &amp; \" ' -->",
    ];
    for title in TITLES {
        let landed = app.js(&format!(
            "(async()=>{{const ipc=await import('/ipc.js');const r=await ipc.manageChapters({{kind:'add',after:0,title:{0}}},null);await ipc.manageChapters({{kind:'rename',row:{{chapter:r.landing}},title:{0}+'!'}},null);return r.landing}})()",
            json!(title)
        ));
        assert_eq!(landed, json!(1), "{title}");
    }
    let moved = app.js("(async()=>{const ipc=await import('/ipc.js');const t=await ipc.loadToc();const last=t.entries.length-1;await ipc.manageChapters({kind:'move',row:{chapter:1},to:{row:{chapter:4},after:true}},null);return last})()");
    assert!(moved.as_u64().unwrap() > 0);
    let after = app.js("(async()=>{const t=await (await import('/ipc.js')).loadToc();return t.entries.map((e)=>e.label)})()");
    let old = before.as_array().unwrap();
    let new = after.as_array().unwrap();
    assert_eq!(
        new.len(),
        old.len() + TITLES.len(),
        "the rows changed count"
    );
    for label in old {
        assert!(
            new.contains(label),
            "a hostile label did not survive as text"
        );
    }
    for title in TITLES {
        let wanted = format!(
            "{}!",
            title.split_whitespace().collect::<Vec<_>>().join(" ")
        );
        assert!(
            new.iter()
                .any(|label| label.as_str() == Some(wanted.as_str())),
            "a hostile title did not come back as text"
        );
    }
    app.js("(async()=>{const ipc=await import('/ipc.js');await ipc.manageChapters({kind:'delete',chapter:1},null);await ipc.reloadBook();await (await import('/main.js')).goTo(0)})()");
    app.key(0x65);
    settle(150);
    let edit_rows = app.js("!document.getElementById('add-chapter').hidden");
    assert_eq!(edit_rows, json!(true), "Edit shows the chapter actions");
    app.js("(()=>{const open=document.getElementById('deleted-open');open.hidden=false;open.click();return true})()");
    settle(150);
    let strangers = app.js(CHAPTER_CONTROLS_AUDIT);
    assert_eq!(
        strangers,
        json!([]),
        "markup was parsed in a chapter control"
    );
    let state = app.state();
    assert_eq!(state["pwned"], json!([]), "a title called the app");
    assert_eq!(listener.hits().len(), 0, "a title reached the network");
    assert_eq!(state["url"], state["ui_url"]);
    app.close();
    let _ = std::fs::remove_dir_all(book.parent().expect("book has a directory"));
}

#[test]
#[ignore = "GUI: tests/e2e/run.sh attack"]
fn the_page_cannot_reach_the_dialog_or_file_plugins() {
    let mut app = App::launch(None, &[]);
    app.wait_first_text();
    for command in [
        "plugin:dialog|save",
        "plugin:dialog|open",
        "plugin:dialog|message",
        "plugin:fs|read_file",
        "plugin:fs|write_file",
    ] {
        let answer = app.js(&format!(
            "window.__TAURI_INTERNALS__.invoke({}, {{ options: {{}}, path: '/etc/hostname' }}).then(() => 'answered', () => 'refused')",
            json!(command)
        ));
        assert_eq!(answer, json!("refused"), "{command} answered the page");
    }
    app.close();
}
