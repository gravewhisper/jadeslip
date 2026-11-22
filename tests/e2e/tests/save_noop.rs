use serde_json::Value;
use jadeslip_core::proof::{CanonToken, tokens_of};
use jadeslip_core::{
    BlockChange, BlockEdits, BlockIx, Book, BookKey, ChapterIx, DEFAULT_LIMITS, Extracted,
    SaveOutcome,
};
use jadeslip_e2e::{
    App, book_label, generated_book, largest_real_book, real_books, scratch_dir,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const INSTALL: &str = r"(async()=>{
  const main = await import('/main.js');
  const textmap = await import('/textmap.js');
  const host = document.getElementById('book-host');
  const notice = document.getElementById('notice');
  const serializer = new XMLSerializer();
  const textNodes = (block) => {
    const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT);
    const nodes = [];
    while (walker.nextNode()) if (walker.currentNode.data.trim().length > 0) nodes.push(walker.currentNode);
    return nodes;
  };
  window.__mount = async (ix) => {
    await main.goTo(ix);
    if (notice.classList.contains('error') && !notice.hidden) return { failed: true };
    return { xml: [...textmap.blocksOf(host.shadowRoot)].map((block) => serializer.serializeToString(block)) };
  };
  const place = (node, offset) => {
    const selection = document.getSelection();
    selection.setBaseAndExtent(node, offset, node, offset);
    return selection;
  };
  window.__edit = (blockIx, op) => {
    const root = host.shadowRoot;
    const editingHost = root.querySelector('.ss-body');
    const block = root.querySelector(`[data-ss-b='${blockIx}']`);
    const parent = block.parentNode;
    const snapshot = block.cloneNode(true);
    const after = block.nextSibling;
    const others = () => [...textmap.blocksOf(root)]
      .filter((item) => item.getAttribute('data-ss-b') !== String(blockIx))
      .map((item) => serializer.serializeToString(item));
    const othersBefore = others();
    editingHost.setAttribute('contenteditable', 'true');
    editingHost.focus();
    const nodes = textNodes(block);
    const node = nodes[Math.floor(nodes.length / 2)];
    const middle = Math.floor(node.data.length / 2);
    const selection = place(node, middle);
    let applied = true;
    if (op === 'typeThenErase') {
      applied = document.execCommand('insertText', false, 'x');
      applied = document.execCommand('delete') && applied;
    } else if (op === 'typeWords') {
      applied = document.execCommand('insertText', false, 'ab  cd ');
    } else if (op === 'typeAtEndThenSpace') {
      const last = nodes[nodes.length - 1];
      place(last, last.data.length);
      applied = document.execCommand('insertText', false, ' ');
    } else if (op === 'backspace') {
      applied = document.execCommand('delete');
    } else if (op === 'selectAllDelete') {
      const last = nodes[nodes.length - 1];
      selection.setBaseAndExtent(nodes[0], 0, last, last.data.length);
      applied = document.execCommand('delete');
    } else if (op === 'enterSplit') {
      applied = document.execCommand('insertParagraph');
    } else if (op === 'lineBreak') {
      applied = document.execCommand('insertLineBreak');
    }
    editingHost.removeAttribute('contenteditable');
    const shown = [...root.querySelectorAll(`[data-ss-b='${blockIx}']`)];
    const xml = shown.map((item) => serializer.serializeToString(item));
    const othersAfter = others();
    const othersChanged = othersAfter.length !== othersBefore.length
      || othersAfter.some((one, at) => one !== othersBefore[at]);
    const loose = [];
    if (shown.length === 1 && shown[0].nextElementSibling && !shown[0].nextElementSibling.hasAttribute('data-ss-b') && !shown[0].nextElementSibling.hasAttribute('data-ss-w')) {
      loose.push(shown[0].nextElementSibling.localName);
    }
    shown.forEach((item) => item.remove());
    parent.insertBefore(snapshot, after && after.parentNode === parent ? after : null);
    document.getSelection().removeAllRanges();
    return { applied, xml, loose, othersChanged };
  };
  return true;
})()";

fn sample(count: u32, per_book: usize) -> Vec<u32> {
    let step = (count as usize / per_book).max(1);
    (0..count).step_by(step).collect()
}

fn with_host(xml: &str, session: u64) -> String {
    let prefix = "jadeslip-book://";
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(at) = rest.find(prefix) {
        let end = at + prefix.len();
        out.push_str(&rest[..end]);
        out.push_str(&format!("{session:016x}"));
        rest = &rest[end + 16..];
    }
    out.push_str(rest);
    out
}

struct Copy {
    dir: PathBuf,
    book: Book,
    path: PathBuf,
}

fn copy_of(source: &Path, label: &str) -> Copy {
    let dir = scratch_dir().join(format!("save-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("data")).unwrap();
    let path = dir.join("book.epub");
    fs::write(&path, fs::read(source).unwrap()).unwrap();
    let book = Book::open(&path, &dir.join("data"), BookKey(11), DEFAULT_LIMITS).unwrap();
    Copy { dir, book, path }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Default)]
struct NoOp {
    chapters: u64,
    blocks: u64,
    wrote: u64,
    refused: u64,
    block_count_differs: u64,
}

fn no_op_sweep(source: &Path, label: &str, indexes: impl Fn(u32) -> Vec<u32>) -> NoOp {
    let copy = copy_of(source, label);
    let before = fs::read(&copy.path).unwrap();
    let session = copy.book.read().session();
    let reader = copy.book.read();
    let mut app = App::launch(Some(source), &[]);
    app.wait_first_text();
    app.assert_on_headless_output();
    app.js(INSTALL);
    let mut tally = NoOp::default();
    for ix in indexes(reader.rows().len() as u32) {
        let chapter = ChapterIx(ix);
        if !matches!(reader.extract(chapter), Ok(Extracted::Document(_))) {
            continue;
        }
        let Ok(mounted) = app.try_js(&format!("window.__mount({ix})"), 60_000) else {
            tally.refused += 1;
            continue;
        };
        let xml = strings(&mounted["xml"]);
        let rendered = reader.render(chapter).unwrap();
        tally.chapters += 1;
        tally.blocks += xml.len() as u64;
        let expected = jadeslip_core::proof::block_xml_of(&rendered.xhtml)
            .unwrap()
            .len();
        if expected != xml.len() {
            tally.block_count_differs += 1;
        }
        let changes = xml
            .iter()
            .enumerate()
            .map(|(block, xml)| BlockChange::Replace {
                of: BlockIx(block as u32),
                xml: with_host(xml, session),
            })
            .collect();
        match copy.book.save_chapter(
            chapter,
            &BlockEdits {
                base: rendered.rev,
                changes,
            },
        ) {
            Ok(saved) if matches!(saved.outcome, SaveOutcome::Unchanged) => {}
            Ok(_) => tally.wrote += 1,
            Err(_) => tally.refused += 1,
        }
    }
    app.close();
    let same = fs::read(&copy.path).unwrap() == before;
    let no_backup = fs::read_dir(copy.dir.join("data"))
        .unwrap()
        .next()
        .is_none();
    println!(
        "{label:<20} chapters {:>5} blocks {:>7} wrote {:>3} refused {:>3} block_count_differs {:>3} file_same {same} no_backup {no_backup}",
        tally.chapters, tally.blocks, tally.wrote, tally.refused, tally.block_count_differs
    );
    assert!(same && no_backup, "{label}: a no-op touched the book");
    let _ = fs::remove_dir_all(&copy.dir);
    tally
}

#[test]
#[ignore = "GUI and real books: SS_E2E_TEST=save_noop tests/e2e/run.sh"]
fn every_chapter_of_the_largest_real_book_is_a_no_op_through_webkit() {
    let source = largest_real_book();
    let tally = no_op_sweep(&source, &book_label(&source), |count| (0..count).collect());
    assert!(tally.chapters > 6000);
    assert_eq!(tally.wrote + tally.refused + tally.block_count_differs, 0);
}

#[test]
#[ignore = "GUI and real books: SS_E2E_TEST=save_noop tests/e2e/run.sh"]
fn samples_of_every_other_book_are_no_ops_through_webkit() {
    let largest = largest_real_book();
    let mut bad = 0;
    for source in real_books().iter().filter(|book| **book != largest) {
        let tally = no_op_sweep(source, &book_label(source), |count| sample(count, 150));
        assert!(tally.chapters > 0);
        bad += tally.wrote + tally.refused + tally.block_count_differs;
    }
    for name in [
        "gbk-xml-decl",
        "gbk-meta-charset",
        "malformed-markup",
        "images-mixed",
        "attack",
    ] {
        let tally = no_op_sweep(&generated_book(name), name, |count| (0..count).collect());
        bad += tally.wrote + tally.refused + tally.block_count_differs;
    }
    assert_eq!(bad, 0);
}

fn tag_shapes(xml: &str) -> Vec<String> {
    tokens_of(xml)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|token| match token {
            CanonToken::Open { name, attrs } => {
                let names: Vec<String> = attrs.into_iter().map(|(name, _)| name).collect();
                Some(format!("{name}[{}]", names.join(" ")))
            }
            _ => None,
        })
        .collect()
}

fn text_length(xml: &str) -> usize {
    tokens_of(xml)
        .unwrap_or_default()
        .iter()
        .map(|token| match token {
            CanonToken::Text(text) => text.trim().chars().count(),
            _ => 0,
        })
        .sum()
}

fn changed_region<'a>(old: &'a [u8], new: &'a [u8]) -> (&'a [u8], &'a [u8]) {
    let shared = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[shared..]
        .iter()
        .rev()
        .zip(new[shared..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (
        &old[shared..old.len() - tail],
        &new[shared..new.len() - tail],
    )
}

#[test]
#[ignore = "GUI and real books: SS_E2E_TEST=save_noop tests/e2e/run.sh"]
fn real_webkit_editing_commands_stay_inside_the_closed_artefact_list() {
    const OPS: [&str; 7] = [
        "typeThenErase",
        "typeWords",
        "typeAtEndThenSpace",
        "backspace",
        "lineBreak",
        "selectAllDelete",
        "enterSplit",
    ];
    let mut books = real_books();
    books.push(generated_book("large-h1-p"));
    books.push(generated_book("large-h1-div-br"));
    let mut outcomes: BTreeMap<String, u64> = BTreeMap::new();
    let mut new_shapes: BTreeMap<String, u64> = BTreeMap::new();
    let mut largest_change: BTreeMap<&str, usize> = BTreeMap::new();
    for source in &books {
        let label = book_label(source);
        let copy = copy_of(source, &format!("edit-{label}"));
        let reader = copy.book.read();
        let session = reader.session();
        let mut app = App::launch(Some(source), &[]);
        app.wait_first_text();
        app.assert_on_headless_output();
        app.js(INSTALL);
        for ix in sample(reader.rows().len() as u32, 12) {
            let chapter = ChapterIx(ix);
            let Ok(Extracted::Document(old)) = reader.extract(chapter) else {
                continue;
            };
            let rendered = reader.render(chapter).unwrap();
            let Ok(mounted) = app.try_js(&format!("window.__mount({ix})"), 60_000) else {
                continue;
            };
            let xml = strings(&mounted["xml"]);
            let Some(block) = xml.iter().position(|one| text_length(one) > 30) else {
                continue;
            };
            let before: Vec<String> = tag_shapes(&xml[block]);
            for op in OPS {
                let Ok(edited) = app.try_js(&format!("window.__edit({block},'{op}')"), 30_000)
                else {
                    *outcomes.entry(format!("{op} page_error")).or_default() += 1;
                    continue;
                };
                if edited["applied"].as_bool() != Some(true) {
                    *outcomes.entry(format!("{op} not_applied")).or_default() += 1;
                    continue;
                }
                if edited["loose"]
                    .as_array()
                    .is_some_and(|list| !list.is_empty())
                {
                    *outcomes.entry(format!("{op} loose_sibling")).or_default() += 1;
                }
                let parts = strings(&edited["xml"]);
                let Some((first, rest)) = parts.split_first() else {
                    *outcomes.entry(format!("{op} block_gone")).or_default() += 1;
                    continue;
                };
                for shape in parts.iter().flat_map(|part| tag_shapes(part)) {
                    if !before.contains(&shape) {
                        *new_shapes.entry(format!("{op} {shape}")).or_default() += 1;
                    }
                }
                let mut changes = vec![BlockChange::Replace {
                    of: BlockIx(block as u32),
                    xml: with_host(first, session),
                }];
                for part in rest {
                    changes.push(BlockChange::InsertAfter {
                        after: Some(BlockIx(block as u32)),
                        xml: with_host(part, session),
                    });
                }
                let edits = BlockEdits {
                    base: rendered.rev,
                    changes,
                };
                let class = match reader.plan_preview(chapter, &edits) {
                    Ok(None) => "unchanged",
                    Ok(Some((new, warnings))) => {
                        let (was, now) = changed_region(&old, &new);
                        let size = was.len().max(now.len());
                        let top = largest_change.entry(op).or_default();
                        *top = (*top).max(size);
                        if warnings.is_empty() {
                            "written"
                        } else {
                            "written_with_warning"
                        }
                    }
                    Err(_) => "refused",
                };
                *outcomes.entry(format!("{op} {class}")).or_default() += 1;
            }
        }
        app.close();
        let _ = fs::remove_dir_all(&copy.dir);
    }
    for (name, count) in &outcomes {
        println!("{name:<40} {count}");
    }
    println!("largest changed span per op, bytes: {largest_change:?}");
    println!("tag shapes new to the block: {new_shapes:?}");
    let refused: u64 = outcomes
        .iter()
        .filter(|(name, _)| name.ends_with("refused"))
        .map(|(_, count)| count)
        .sum();
    assert_eq!(refused, 0);
    assert!(
        new_shapes.keys().all(|shape| shape.ends_with(" br[]")),
        "WebKit made a tag outside the closed artefact list"
    );
    let troubles = [
        "page_error",
        "not_applied",
        "block_gone",
        "other_block_changed",
    ];
    assert!(
        !outcomes
            .keys()
            .any(|name| troubles.iter().any(|trouble| name.ends_with(trouble))),
        "every command ran and touched only its block"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|(name, _)| name.starts_with("typeThenErase ") && !name.ends_with(" unchanged"))
            .map(|(_, count)| count)
            .sum::<u64>(),
        0,
        "typing then erasing must save nothing"
    );
}
