use serde_json::{Value, json};
use jadeslip_e2e::{App, generated_book, repo_root};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const INSTALL: &str = r"(async()=>{
  const ipc = await import('/ipc.js');
  window.__regex = async (id, text) => {
    await ipc.searchStart(id, { text, regex: true, caseSensitive: true, wholeWord: false }, { kind: 'whole' });
    let hits = 0;
    for (;;) {
      const batch = await ipc.searchNext(id);
      for (const group of batch.groups) hits += group.hits.length;
      if (batch.end) return { hits, end: batch.end, done: batch.done, total: batch.total };
    }
  };
  return true;
})()";

const LOOKAROUND: &str = "(?<=t)h(?=e)";

struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn install_to(label: &str, extra: &[&str], target: &str) -> Folder {
    let dest = jadeslip_fixtures::scratch::root().join(format!("install-{label}"));
    let _ = fs::remove_dir_all(&dest);
    let root = repo_root();
    assert!(
        !dest.starts_with(&root),
        "the installed copy must sit outside the repository"
    );
    let done = Command::new(root.join("scripts/install-local.sh"))
        .arg(&dest)
        .args(extra)
        .env("CARGO_TARGET_DIR", root.join(target))
        .output()
        .expect("the install script runs");
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    Folder(dest)
}

fn launch_from(folder: &Path, book: &Path) -> App {
    let mut command = Command::new(folder.join("bin/jadeslip"));
    command.arg(book).current_dir(folder).env_remove("PATH");
    let mut app = App::spawn(command, &[]);
    app.wait_first_text();
    assert_eq!(app.js(INSTALL), json!(true));
    let exe = fs::read_link(format!("/proc/{}/exe", app.pid)).expect("exe link");
    assert!(exe.starts_with(folder), "the app ran from {exe:?}");
    app
}

fn regex(app: &mut App, id: u32) -> Value {
    app.js(&format!("window.__regex({id}, {})", json!(LOOKAROUND)))
}

#[test]
#[ignore = "GUI: installs into a temp folder and runs from there"]
fn regex_search_runs_from_an_installed_copy_outside_target() {
    let folder = install_to("gui", &["--features", "e2e"], "target/e2e");
    let bin = folder.0.join("bin");
    assert!(bin.join("jadeslip").is_file());
    assert!(bin.join("jadeslip-worker").is_file());
    assert!(
        folder
            .0
            .join("share/doc/jadeslip/README_en_US.txt")
            .is_file()
    );
    let book = generated_book("toc-none");
    let mut app = launch_from(&folder.0, &book);
    let found = regex(&mut app, 1);
    println!("installed copy, lookaround regex: {found}");
    assert_eq!(found["end"]["kind"], json!("done"), "{found}");
    assert!(found["hits"].as_u64().unwrap_or(0) > 0, "{found}");
    assert_eq!(found["done"], found["total"]);
    app.assert_on_headless_output();
    app.close();

    fs::remove_file(bin.join("jadeslip-worker")).expect("worker removed");
    let mut app = launch_from(&folder.0, &book);
    let refused = regex(&mut app, 1);
    println!("same copy without the worker: {refused}");
    assert_eq!(refused["hits"], json!(0), "{refused}");
    assert_eq!(refused["end"]["kind"], json!("failed"), "{refused}");
    app.close();
}

#[test]
#[ignore = "installs the plain build into a temp folder"]
fn the_plain_installed_copy_has_both_binaries_and_runs_outside_target() {
    let folder = install_to("plain", &[], "target");
    let bin = folder.0.join("bin");
    assert!(bin.join("jadeslip-worker").is_file());
    let done = Command::new(bin.join("jadeslip"))
        .args(["info"])
        .arg(generated_book("toc-none"))
        .current_dir(&folder.0)
        .env_remove("PATH")
        .output()
        .expect("runs");
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    assert!(!done.stdout.is_empty());
    let probe = Command::new(bin.join("jadeslip-worker"))
        .current_dir(&folder.0)
        .env_remove("PATH")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the worker starts");
    println!(
        "plain worker with empty input exits with {:?}",
        probe.status.code()
    );
    assert!(
        probe.status.code().is_some(),
        "the worker was killed by a signal"
    );
}
