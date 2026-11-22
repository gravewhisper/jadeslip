mod common;

use common::fixture;
use jadeslip_core::cli::{extract, info, run};
use std::ffi::OsString;
use std::io::Read;
use std::process::{Command, Stdio};

fn cli_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jadeslip_cli")
}

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

fn path_text(book: &str) -> String {
    fixture(book).to_str().unwrap().to_owned()
}

fn close_after_one_byte(arguments: &[&str]) -> (std::process::ExitStatus, Vec<u8>) {
    let mut child = Command::new(cli_binary())
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cargo builds the binary for this test");
    let mut stdout = child.stdout.take().unwrap();
    let mut first = [0u8; 1];
    stdout.read_exact(&mut first).unwrap();
    drop(stdout);
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    (child.wait().unwrap(), stderr)
}

#[test]
fn a_closed_pipe_exits_cleanly_with_nothing_on_stderr() {
    let huge = path_text("bomb-ratio");
    let long_info = path_text("large-h1-p");
    let raw_image = path_text("images-only");
    for arguments in [
        vec!["extract", huge.as_str(), "0"],
        vec!["info", long_info.as_str()],
        vec!["extract", raw_image.as_str(), "4"],
    ] {
        let (status, stderr) = close_after_one_byte(&arguments);
        assert_eq!(status.code(), Some(0), "{arguments:?}");
        assert!(
            stderr.is_empty(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&stderr)
        );
    }
}

#[test]
fn a_shell_pipeline_into_head_reports_success_and_no_panic() {
    let script = format!(
        "\"{}\" extract \"{}\" 0 | head -c1 >/dev/null; echo ${{PIPESTATUS[0]}}",
        cli_binary(),
        path_text("bomb-ratio")
    );
    let output = Command::new("bash")
        .args(["-c", &script])
        .output()
        .expect("bash runs");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "0");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn a_raw_image_chapter_writes_its_bytes_to_a_pipe_and_a_note_on_a_terminal() {
    let path = fixture("images-only");
    let (mut out, mut note) = (Vec::new(), Vec::new());
    extract(&path, 4, &mut out, &mut note, false).unwrap();
    assert!(out.starts_with(b"\x89PNG"), "the image bytes are written");
    assert!(note.is_empty());

    let (mut out, mut note) = (Vec::new(), Vec::new());
    extract(&path, 4, &mut out, &mut note, true).unwrap();
    assert!(out.is_empty(), "no binary reaches a terminal");
    let note = String::from_utf8(note).unwrap();
    assert!(note.contains("image/png"), "{note}");
}

#[test]
fn an_image_page_document_is_extracted_as_its_document() {
    let path = fixture("images-only");
    let (mut out, mut note) = (Vec::new(), Vec::new());
    extract(&path, 0, &mut out, &mut note, false).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("<img"));
    assert!(note.is_empty());
}

#[test]
fn info_lists_the_summary_and_every_spine_row() {
    let mut out = Vec::new();
    info(&fixture("toc-nested-nav"), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Title: Toc Nested Nav"), "{text}");
    assert!(text.contains("Chapters: 6"), "{text}");
    assert!(text.contains("\n0\t1\tOEBPS/Text/c00.xhtml\n"), "{text}");
    assert_eq!(
        text.lines().filter(|line| line.contains(".xhtml")).count(),
        6
    );
}

#[test]
fn failures_are_readable_lines_with_a_failing_code() {
    let cases: Vec<(Vec<OsString>, i32, &str)> = vec![
        (
            args(&["info", "/nonexistent/book.epub"]),
            1,
            "could not be read",
        ),
        (
            args(&["extract", &path_text("toc-nested-nav"), "999"]),
            1,
            "no chapter",
        ),
        (
            args(&["extract", &path_text("toc-nested-nav"), "abc"]),
            2,
            "whole number",
        ),
        (
            args(&["extract", &path_text("bomb-oversize-entry"), "0"]),
            1,
            "limit",
        ),
        (args(&["frobnicate"]), 2, "Usage"),
        (args(&[]), 2, "Usage"),
    ];
    for (arguments, code, message) in cases {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(
            run(&arguments, &mut out, &mut err, false),
            code,
            "{arguments:?}"
        );
        let err = String::from_utf8(err).unwrap();
        assert!(err.contains(message), "{arguments:?}: {err}");
        assert!(out.is_empty(), "{arguments:?}");
    }
}

#[test]
fn clean_report_counts_each_rule_and_lists_samples_or_every_match() {
    let book = path_text("large-h1-div-br");
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(run(&args(&["clean-report", &book, "--samples", "3"]), &mut out, &mut err, false), 0);
    let text = String::from_utf8(out).unwrap();
    for label in ["Watermark lines:", "Translator notes and support requests:", "Repeated chapter titles:"] {
        assert!(text.contains(label), "{text}");
    }
    let mut all = Vec::new();
    assert_eq!(run(&args(&["clean-report", &book, "--all"]), &mut all, &mut err, false), 0);
    assert!(err.is_empty());
    assert_eq!(run(&args(&["clean-report", &book, "--samples", "x"]), &mut Vec::new(), &mut err, false), 2);
}
