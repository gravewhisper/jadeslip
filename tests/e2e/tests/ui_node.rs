use jadeslip_e2e::repo_root;
use std::process::Command;

#[test]
fn the_edit_machine_and_key_map_pass_in_node() {
    let root = repo_root();
    let done = Command::new("node")
        .current_dir(&root)
        .args([
            "--test",
            "tests/ui/edit-machine.test.mjs",
            "tests/ui/keys.test.mjs",
            "tests/ui/search-model.test.mjs",
            "tests/ui/replace-model.test.mjs",
            "tests/ui/toc-model.test.mjs",
            "tests/ui/chapters-model.test.mjs",
            "tests/ui/shelf-model.test.mjs",
            "tests/ui/tokens.test.mjs",
            "tests/ui/word-count.test.mjs",
            "tests/ui/pages.test.mjs",
            "tests/ui/search-memory-model.test.mjs",
            "tests/ui/cleanup-model.test.mjs",
        ])
        .output()
        .expect("node runs");
    let out = String::from_utf8_lossy(&done.stdout);
    assert!(done.status.success(), "{out}");
}
