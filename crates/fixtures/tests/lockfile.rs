use serde_json::Value;
use jadeslip_fixtures::{CATALOG, Options, generate, lock_path, output_files, sha256_file};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn generated() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fixtures-default-port");
        generate(CATALOG, &dir, Options::default()).expect("generation succeeds");
        dir
    })
}

fn verifier() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/verify_fixtures.py")
}

fn run_verifier(dir: &Path, books: &[&str]) {
    let output = Command::new("python3")
        .arg("-I")
        .arg(verifier())
        .arg(dir)
        .args(books)
        .output()
        .expect("python3 is required to run the independent verifier");
    assert!(
        output.status.success(),
        "verifier failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn read_truth(dir: &Path, book: &str) -> Value {
    let text = std::fs::read_to_string(dir.join(format!("{book}.truth.json"))).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn every_generated_file_matches_the_committed_lockfile() {
    let dir = generated();
    let lock = std::fs::read_to_string(lock_path()).expect("lockfile is committed");
    let locked: BTreeMap<&str, &str> = lock
        .lines()
        .map(|line| {
            let (hash, file) = line.split_once("  ").expect("sha256sum line format");
            (file, hash)
        })
        .collect();
    let mut drifted = Vec::new();
    for file in output_files() {
        let actual = sha256_file(&dir.join(&file)).unwrap();
        if locked.get(file.as_str()) != Some(&actual.as_str()) {
            drifted.push(file);
        }
    }
    assert!(
        drifted.is_empty(),
        "fixtures drifted from the lockfile: {drifted:?}"
    );
    assert_eq!(
        locked.len(),
        output_files().len(),
        "lockfile lists stale files"
    );
}

#[test]
fn independent_reader_agrees_with_every_truth_sidecar() {
    run_verifier(generated(), &[]);
}

#[test]
fn probe_port_moves_only_the_attack_book_urls() {
    let port = 40_123;
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fixtures-other-port");
    let attack: Vec<_> = CATALOG
        .iter()
        .filter(|def| def.name == "attack")
        .copied()
        .collect();
    generate(&attack, &dir, Options { probe_port: port }).unwrap();
    run_verifier(&dir, &["attack"]);

    let moved = read_truth(&dir, "attack");
    assert_eq!(moved["attack"]["probe_port"], port);
    let original = read_truth(generated(), "attack");
    let renumbered = original.to_string().replace(
        &Options::default().probe_port.to_string(),
        &port.to_string(),
    );
    assert_eq!(renumbered, moved.to_string());
    assert_ne!(
        sha256_file(&dir.join("attack.epub")).unwrap(),
        sha256_file(&generated().join("attack.epub")).unwrap()
    );
}

#[test]
fn ebook_meta_reads_the_opf_title_of_every_book_meant_to_be_valid() {
    if Command::new("ebook-meta")
        .arg("--version")
        .output()
        .is_err()
    {
        println!("skipped: ebook-meta is not installed");
        return;
    }
    let dir = generated();
    let mut failures = Vec::new();
    let mut opened = 0;
    for def in CATALOG {
        let truth = read_truth(dir, def.name);
        if truth["meant_valid"] != true {
            continue;
        }
        let output = Command::new("ebook-meta")
            .arg(dir.join(format!("{}.epub", def.name)))
            .env("QT_QPA_PLATFORM", "offscreen")
            .output()
            .expect("ebook-meta is installed");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let title = truth["title"].as_str().unwrap();
        let title_seen = stdout
            .lines()
            .any(|line| line.starts_with("Title") && line.ends_with(title));
        println!(
            "ebook-meta {} exit={:?} title_seen={title_seen}",
            def.name,
            output.status.code()
        );
        opened += 1;
        if !output.status.success() || !title_seen {
            failures.push(def.name);
        }
    }
    assert!(
        opened >= 12,
        "expected at least 12 valid books, found {opened}"
    );
    assert!(
        failures.is_empty(),
        "ebook-meta did not report the OPF title for {failures:?}. It exits 0 on random bytes, so the title is the real check"
    );
}
