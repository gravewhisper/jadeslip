use std::path::PathBuf;
use std::process::{Command, Stdio};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../fixtures/generated/{name}.epub"))
}

fn headless_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jadeslip"));
    command
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET");
    command
}

#[test]
fn info_prints_a_summary_without_a_display() {
    let output = headless_command()
        .args(["info".as_ref(), fixture("images-mixed").as_os_str()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Chapters: "));
    assert!(text.contains("index\tchapter\thref"));
}

#[test]
fn extract_prints_chapter_bytes_without_a_display() {
    let output = headless_command()
        .args([
            "extract".as_ref(),
            fixture("large-h1-p").as_os_str(),
            "3".as_ref(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.windows(5).any(|window| window == b"<html"));
}

#[test]
fn a_closed_pipe_is_success() {
    let mut child = headless_command()
        .args(["info".as_ref(), fixture("large-h1-p").as_os_str()])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    assert!(child.wait().unwrap().success());
}

#[test]
fn bad_usage_exits_two_without_a_display() {
    let output = headless_command().args(["--help"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Usage"));
}

#[test]
fn a_book_that_will_not_open_is_a_readable_error() {
    let output = headless_command()
        .args(["info", "/nonexistent/book.epub"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("jadeslip: "));
}
