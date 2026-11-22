use jadeslip_fixtures::samples::{SAMPLES, generate};
use std::path::Path;

#[test]
fn samples_are_byte_identical_across_runs() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let first = generate(&base.join("samples-a")).unwrap();
    let second = generate(&base.join("samples-b")).unwrap();
    assert_eq!(first.len(), SAMPLES.len());
    for (a, b) in first.iter().zip(&second) {
        assert_eq!(
            std::fs::read(a).unwrap(),
            std::fs::read(b).unwrap(),
            "{}",
            a.display()
        );
    }
}

#[test]
fn samples_carry_what_the_screenshots_show() {
    let lantern = SAMPLES
        .iter()
        .find(|s| s.title == "The Lantern Keeper")
        .unwrap();
    let text: Vec<String> = lantern
        .chapters
        .iter()
        .map(|c| c.paragraphs.join(" "))
        .collect();
    assert!(
        text[1].contains("recieved"),
        "the edit shot needs a misspelling in chapter 2"
    );
    let hits = text
        .iter()
        .filter(|t| t.contains("lighthouse") || t.contains("logbook"))
        .count();
    assert!(
        hits >= 3,
        "the search shot needs lighthouse|logbook in several chapters"
    );
    for sample in SAMPLES {
        assert_eq!(sample.chapters.len(), 5, "{}", sample.title);
    }
}
