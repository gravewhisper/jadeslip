use super::*;

const TEXT: &str = "The ferry came out of the fog without a sound, its lantern low.\nHe set three coins on the rail and waited for the ferryman to speak.\nNobody answered, and the river kept moving under the boards.\n";

fn range_of(text: &str, part: &str) -> VisRange {
    let start = text.find(part).unwrap();
    VisRange {
        start: start as u32,
        end: (start + part.len()) as u32,
    }
}

fn anchor(part: &str) -> Anchor {
    anchor_of(TEXT, "ch.xhtml".to_owned(), range_of(TEXT, part))
}

fn at(text: &str, range: VisRange) -> String {
    text[range.start as usize..range.end as usize].to_owned()
}

const CHAPTER: ChapterIx = ChapterIx(3);

fn found_text(text: &str, resolution: AnchorResolution) -> Option<String> {
    match resolution {
        AnchorResolution::Exact { range, .. } | AnchorResolution::Moved { range, .. } => {
            Some(at(text, range))
        }
        AnchorResolution::Detached => None,
    }
}

#[test]
fn a_range_keeps_its_text_and_context() {
    let made = anchor("three coins");
    assert_eq!(made.exact, "three coins");
    assert!(made.prefix.ends_with("He set "));
    assert!(made.suffix.starts_with(" on the rail"));
    assert_eq!(made.vis_hint, range_of(TEXT, "three coins").start);
}

#[test]
fn context_stops_at_the_edges_of_the_chapter_and_on_char_boundaries() {
    let text = "éééé end";
    let made = anchor_of(text, "c".to_owned(), range_of(text, "end"));
    assert_eq!(made.prefix, "éééé ");
    assert_eq!(made.suffix, "");
    let split = anchor_of(text, "c".to_owned(), VisRange { start: 1, end: 3 });
    assert_eq!(split.exact, "é");
}

#[test]
fn a_collapsed_range_takes_the_following_words() {
    let start = range_of(TEXT, "He set").start;
    let made = anchor_of(TEXT, "c".to_owned(), VisRange { start, end: start });
    assert!(made.exact.starts_with("He set three coins"));
    assert!(!made.exact.contains('\n'));
    assert_eq!(made.vis_hint, start);
}

#[test]
fn a_collapsed_range_at_the_end_takes_the_words_before_it() {
    let end = TEXT.len() as u32;
    let made = anchor_of(TEXT, "c".to_owned(), VisRange { start: end, end });
    assert!(made.exact.ends_with("the boards."));
    assert!(!made.exact.contains('\n'));
}

#[test]
fn an_empty_chapter_gives_an_empty_anchor_that_resolves_to_its_top() {
    let made = anchor_of("", "c".to_owned(), VisRange { start: 0, end: 0 });
    assert_eq!(made.exact, "");
    assert_eq!(
        resolve_in(&made, CHAPTER, "now has text"),
        AnchorResolution::Exact {
            chapter: CHAPTER,
            range: VisRange { start: 0, end: 0 }
        }
    );
}

#[test]
fn a_long_selection_is_capped_on_a_char_boundary() {
    let text = "é".repeat(5_000);
    let made = anchor_of(
        &text,
        "c".to_owned(),
        VisRange {
            start: 0,
            end: text.len() as u32,
        },
    );
    assert!(made.exact.len() <= MAX_EXACT_BYTES);
    assert!(made.exact.len() >= MAX_EXACT_BYTES - 1);
}

#[test]
fn unchanged_text_resolves_exactly_in_place() {
    let made = anchor("three coins");
    assert_eq!(
        resolve_in(&made, CHAPTER, TEXT),
        AnchorResolution::Exact {
            chapter: CHAPTER,
            range: range_of(TEXT, "three coins")
        }
    );
}

#[test]
fn an_edit_before_the_mark_shifts_it_and_reports_moved() {
    let made = anchor("three coins");
    let edited = format!("A new first line.\n{TEXT}");
    assert_eq!(
        resolve_in(&made, CHAPTER, &edited),
        AnchorResolution::Moved {
            chapter: CHAPTER,
            range: range_of(&edited, "three coins")
        }
    );
}

#[test]
fn the_right_copy_wins_by_context_even_when_another_sits_on_the_hint() {
    let made = anchor("the ferryman");
    let hint = made.vis_hint as usize;
    let decoy = format!("{}the ferryman stands alone\n", "x".repeat(hint));
    let edited = format!("{decoy}and much later waited for the ferryman to speak.\n");
    assert_eq!(edited.find("the ferryman"), Some(hint));
    let found = resolve_in(&made, CHAPTER, &edited);
    let AnchorResolution::Moved { range, .. } = found else {
        panic!("expected moved");
    };
    assert!(edited[range.end as usize..].starts_with(" to speak."));
}

#[test]
fn nearest_to_the_hint_breaks_a_context_tie() {
    let text = "aaaa a phrase of some length bbbb\ncccc a phrase of some length dddd\neeee a phrase of some length ffff\n";
    let made = Anchor {
        href: "c".to_owned(),
        vis_hint: text.rfind("a phrase").unwrap() as u32 - 2,
        exact: "a phrase of some length".to_owned(),
        prefix: "zz ".to_owned(),
        suffix: " ".to_owned(),
    };
    let found = resolve_in(&made, CHAPTER, text);
    let AnchorResolution::Moved { range, .. } = found else {
        panic!("expected moved");
    };
    assert_eq!(range.start as usize, text.rfind("a phrase").unwrap());
}

#[test]
fn a_short_word_with_no_context_left_is_detached_not_guessed() {
    let made = Anchor {
        href: "c".to_owned(),
        vis_hint: 0,
        exact: "the".to_owned(),
        prefix: "gone before ".to_owned(),
        suffix: " gone after".to_owned(),
    };
    assert_eq!(
        resolve_in(&made, CHAPTER, "xx the xx"),
        AnchorResolution::Detached
    );
}

#[test]
fn a_long_passage_is_found_even_when_all_its_context_changed() {
    let made = anchor("three coins on the rail");
    let edited = "Completely new. three coins on the rail Also new.";
    assert_eq!(
        found_text(edited, resolve_in(&made, CHAPTER, edited)).unwrap(),
        "three coins on the rail"
    );
}

#[test]
fn a_typo_fixed_inside_the_mark_is_found_through_its_context() {
    let made = anchor("three coins");
    let edited = TEXT.replace("three coins", "three coinz");
    assert_eq!(
        found_text(&edited, resolve_in(&made, CHAPTER, &edited)).unwrap(),
        "three coinz"
    );
}

#[test]
fn a_mark_at_the_top_of_a_chapter_stays_with_the_chapter_when_its_first_line_changes() {
    let top = anchor_of(TEXT, "c".to_owned(), VisRange { start: 0, end: 0 });
    assert_eq!(
        resolve_in(&top, CHAPTER, "A renamed heading\nOther words entirely."),
        AnchorResolution::Moved {
            chapter: CHAPTER,
            range: VisRange { start: 0, end: 0 }
        }
    );
}

#[test]
fn text_that_is_really_gone_is_detached() {
    let made = anchor("three coins on the rail");
    assert_eq!(
        resolve_in(&made, CHAPTER, "An entirely different chapter."),
        AnchorResolution::Detached
    );
}

#[test]
fn hostile_hints_and_non_ascii_never_panic() {
    let made = Anchor {
        href: "c".to_owned(),
        vis_hint: u32::MAX,
        exact: "é".to_owned(),
        prefix: "é".repeat(100),
        suffix: "é".repeat(100),
    };
    let text = "ééééééééééééééé".repeat(40);
    let _ = resolve_in(&made, CHAPTER, &text);
    let _ = bridged(&made, &text);
    let wild = anchor_of(
        "é",
        "c".to_owned(),
        VisRange {
            start: 9_000,
            end: 1,
        },
    );
    assert!(wild.exact.len() <= 2);
}
