use super::*;
use crate::DEFAULT_LIMITS;
use jadeslip_fixtures::spelling::{self, PLANTED};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn scratch(name: &str) -> PathBuf {
    jadeslip_fixtures::scratch::scratch(&format!("spell-{name}"))
}

fn speller() -> Speller {
    Speller::new(None)
}

fn flagged(speller: &Speller, text: &str) -> Vec<String> {
    misspelled_spans(&speller.dictionary(), text)
        .into_iter()
        .map(|span| text[span].to_owned())
        .collect()
}

fn book() -> (PathBuf, Reader<File>) {
    let path = scratch("book").join("spelling.epub");
    spelling::write_spelling_book(&path).expect("book");
    let reader = Reader::open(File::open(&path).expect("file"), DEFAULT_LIMITS).expect("open");
    (path, reader)
}

fn chapter_words(speller: &Speller, reader: &Reader<File>, ordinal: u32) -> Vec<String> {
    let visible = reader.visible_text(ChapterIx(ordinal)).expect("text");
    speller
        .chapter(reader, ChapterIx(ordinal))
        .expect("marks")
        .iter()
        .map(|mark| {
            let range = visible.from_dom(mark.range);
            visible.as_str()[range.start as usize..range.end as usize].to_owned()
        })
        .collect()
}

#[test]
fn the_bundled_dictionary_parses_and_knows_ordinary_words() {
    let speller = speller();
    assert_eq!(
        flagged(&speller, "the quick brown fox jumps over a lazy dog"),
        Vec::<String>::new()
    );
    assert_eq!(flagged(&speller, "the qiuck fox"), ["qiuck"]);
}

#[test]
fn apostrophes_and_quotes_do_not_make_a_word_wrong() {
    let speller = speller();
    for text in [
        "don't",
        "don\u{2019}t",
        "the dogs' bowls",
        "the dogs\u{2019} bowls",
        "'hello'",
        "\u{2018}hello\u{2019}",
        "\"hello\"",
        "\u{201C}hello\u{201D}",
    ] {
        assert_eq!(flagged(&speller, text), Vec::<String>::new(), "{text}");
    }
}

#[test]
fn the_mark_covers_only_the_wrong_word_without_its_apostrophe() {
    assert_eq!(
        flagged(&speller(), "'recieve' and the qiucks'"),
        ["recieve", "qiucks"]
    );
}

#[test]
fn a_hyphen_splits_a_compound_and_marks_only_the_wrong_part() {
    let speller = speller();
    assert_eq!(
        flagged(&speller, "a well-known mother-in-law"),
        Vec::<String>::new()
    );
    assert_eq!(flagged(&speller, "e-mail and x-ray"), Vec::<String>::new());
    assert_eq!(flagged(&speller, "a well-knwon man"), ["knwon"]);
    assert_eq!(
        flagged(&speller, "the end- of it - and -more"),
        Vec::<String>::new()
    );
}

#[test]
fn numbers_capitals_and_addresses_are_left_alone() {
    let speller = speller();
    for text in [
        "room 101b",
        "3rd 1st A4",
        "NASA UNESCO QWRTZ",
        "https://example.invalid/longpathz",
        "www.exampelz.invalid",
        "writer@exampelz.invalid",
    ] {
        assert_eq!(flagged(&speller, text), Vec::<String>::new(), "{text}");
    }
}

#[test]
fn a_capital_run_at_a_sentence_start_is_left_alone() {
    assert_eq!(flagged(&speller(), "QWRTZ was here."), Vec::<String>::new());
}

#[test]
fn only_latin_words_are_checked() {
    let speller = speller();
    for text in [
        "\u{5929}\u{5730}\u{4EBA}",
        "\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}",
        "\u{C548}\u{B155}",
        "\u{41F}\u{440}\u{438}\u{432}\u{435}\u{442}",
        "\u{393}\u{3B5}\u{3B9}\u{3B1}",
        "\u{645}\u{631}\u{62D}\u{628}\u{627}",
        "\u{5E9}\u{5DC}\u{5D5}\u{5DD}",
        "\u{E2A}\u{E27}\u{E31}\u{E2A}\u{E14}\u{E35}",
        "\u{928}\u{92E}\u{938}\u{94D}\u{924}\u{947}",
    ] {
        assert_eq!(flagged(&speller, text), Vec::<String>::new(), "{text}");
    }
}

#[test]
fn accents_and_invisible_joiners_are_folded_before_the_check() {
    let speller = speller();
    assert_eq!(
        flagged(
            &speller,
            "caf\u{E9} na\u{EF}ve cafe\u{301} inter\u{AD}esting"
        ),
        Vec::<String>::new()
    );
}

#[test]
fn a_long_run_is_refused_rather_than_checked_and_never_suggested() {
    let speller = speller();
    let long = "z".repeat(100_000);
    assert_eq!(flagged(&speller, &long), Vec::<String>::new());
    assert_eq!(speller.suggest(&long), Vec::<String>::new());
}

#[test]
fn raw_block_offsets_count_utf16_units_and_breaks() {
    let speller = speller();
    let text = "\u{1F680} qiuck\nfox caf\u{E9} recieve";
    let marks = speller.raw_block(BlockIx(7), text);
    let units: Vec<u16> = text.encode_utf16().collect();
    let words: Vec<String> = marks
        .iter()
        .map(|mark| {
            assert_eq!(mark.range.start.block, BlockIx(7));
            String::from_utf16(
                &units[mark.range.start.offset_u16 as usize..mark.range.end.offset_u16 as usize],
            )
            .expect("units")
        })
        .collect();
    assert_eq!(words, ["qiuck", "recieve"]);
}

#[test]
fn every_planted_typo_is_marked_in_its_chapter_and_each_has_a_good_suggestion() {
    let (_path, reader) = book();
    let speller = speller();
    for ordinal in 0..spelling::CHAPTERS {
        let mut wanted: Vec<&str> = PLANTED
            .iter()
            .filter(|plant| plant.chapter == ordinal)
            .map(|plant| plant.wrong)
            .collect();
        if wanted.is_empty() {
            continue;
        }
        if ordinal == spelling::PROSE {
            wanted.extend(spelling::BRITISH_IN_PROSE);
        }
        let mut found = chapter_words(&speller, &reader, ordinal);
        wanted.sort_unstable();
        found.sort_unstable();
        assert_eq!(found, wanted, "chapter {ordinal}");
    }
    for plant in PLANTED {
        assert!(
            speller
                .suggest(plant.wrong)
                .iter()
                .any(|word| word == plant.right),
            "{} gave {:?}",
            plant.wrong,
            speller.suggest(plant.wrong)
        );
    }
}

#[test]
fn a_capital_word_inside_a_sentence_is_a_name_everywhere_in_the_text() {
    let speller = speller();
    assert_eq!(
        flagged(&speller, "We met Captain Wenzhiq at the gate."),
        Vec::<String>::new()
    );
    assert_eq!(
        flagged(
            &speller,
            "Wenzhiq nodded. We met Wenzhiq and Wenzhiq\u{2019}s guard. Wenzhiq's sword."
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        flagged(&speller, "Teh lamp turned. Teh wick burned."),
        ["Teh", "Teh"]
    );
    assert_eq!(
        flagged(&speller, "\u{201C}Teh lamp,\u{201D} he said."),
        ["Teh"]
    );
    assert_eq!(flagged(&speller, "first line\nTeh second"), ["Teh"]);
}

#[test]
fn a_book_that_says_it_is_not_english_gets_no_marks() {
    let dir = scratch("french");
    let path = dir.join("french.epub");
    spelling::write_french_book(&path).expect("book");
    let reader = Reader::open(File::open(&path).expect("file"), DEFAULT_LIMITS).expect("open");
    assert_eq!(reader.language(), Some("fr-FR"));
    assert_eq!(
        dictionaries::resolve(&dictionaries::SpellChoice::Auto, reader.language(), &[]),
        None
    );
    for (language, covered) in [
        (None, true),
        (Some("en"), true),
        (Some("EN-gb"), true),
        (Some("en_US"), true),
        (Some("und"), true),
        (Some(""), true),
        (Some("fr"), false),
        (Some("zh-CN"), false),
        (Some("eng"), false),
    ] {
        assert_eq!(
            dictionaries::english_applies(language),
            covered,
            "{language:?}"
        );
    }
}

#[test]
fn an_added_word_passes_now_and_after_a_restart_and_is_not_duplicated_in_meaning() {
    let dir = scratch("words");
    let path = dir.join("nested/spelling-words.txt");
    let first = Speller::new(Some(path.clone()));
    assert_eq!(flagged(&first, "Zorblatt"), ["Zorblatt"]);
    first.add_word("Zorblatt").expect("adds");
    assert_eq!(flagged(&first, "Zorblatt"), Vec::<String>::new());
    let second = Speller::new(Some(path.clone()));
    assert_eq!(flagged(&second, "Zorblatt and qiuck"), ["qiuck"]);
    assert_eq!(fs::read_to_string(&path).expect("list"), "Zorblatt\n");
}

#[test]
fn a_curly_added_word_is_stored_plain_and_matches_both_apostrophes() {
    let dir = scratch("curly");
    let path = dir.join("words.txt");
    let speller = Speller::new(Some(path.clone()));
    speller.add_word("Zor\u{2019}blatt").expect("adds");
    assert_eq!(fs::read_to_string(&path).expect("list"), "Zor'blatt\n");
    assert_eq!(
        flagged(&speller, "Zor'blatt Zor\u{2019}blatt"),
        Vec::<String>::new()
    );
}

#[test]
fn text_that_is_not_one_word_is_refused_and_the_list_stays_clean() {
    let dir = scratch("refused");
    let path = dir.join("words.txt");
    let speller = Speller::new(Some(path.clone()));
    for bad in [
        "",
        " ",
        "two words",
        "line\nbreak",
        "slash/flag",
        "1234",
        "'",
        &"z".repeat(41),
    ] {
        assert!(speller.add_word(bad).is_err(), "{bad:?}");
    }
    assert!(!path.exists());
}

#[test]
fn a_damaged_or_huge_word_list_does_not_stop_the_speller() {
    let dir = scratch("damaged");
    let path = dir.join("words.txt");
    fs::write(&path, b"Zorblatt\n\xFF\xFE broken\ntwo words\n\nGrelmish\n").expect("list");
    let speller = Speller::new(Some(path.clone()));
    assert_eq!(flagged(&speller, "Zorblatt Grelmish qiuck"), ["qiuck"]);
    fs::write(&path, "Zorblatt\n".repeat(200_000)).expect("list");
    let speller = Speller::new(Some(path));
    assert_eq!(flagged(&speller, "Zorblatt"), ["Zorblatt"]);
}

#[test]
fn hostile_text_never_panics_and_ranges_stay_on_char_boundaries() {
    let speller = speller();
    for text in [
        "\u{AD}\u{AD}\u{AD}",
        "'''",
        "---",
        "a-",
        "-a",
        "\u{300}\u{301}",
        "\u{200D}qiuck\u{200D}",
        "\u{1F680}-\u{1F680}",
        "qiuck\u{2010}\u{2011}-",
        "\u{5929}qiuck\u{5730}",
    ] {
        let _ = flagged(&speller, text);
        let _ = speller.raw_block(BlockIx(0), text);
        let _ = speller.suggest(text);
    }
}

#[test]
fn marks_stop_at_the_cap() {
    let dir = scratch("cap");
    let path = dir.join("worst.epub");
    spelling::write_worst_chapter(&path, MAX_MARKS + 50).expect("book");
    let reader = Reader::open(File::open(&path).expect("file"), DEFAULT_LIMITS).expect("open");
    assert_eq!(
        speller()
            .chapter(&reader, ChapterIx(0))
            .expect("marks")
            .len(),
        MAX_MARKS
    );
}

fn word_count(text: &str) -> usize {
    words(text).len()
}

fn pct(part: usize, whole: usize) -> f64 {
    100.0 * part as f64 / whole.max(1) as f64
}

#[test]
#[ignore = "prints false positive rates for the generated spelling book"]
fn report_fixture_false_positive_rates() {
    let (_path, reader) = book();
    let speller = speller();
    let rows = [
        ("prose, correct words", spelling::PROSE),
        ("line shape, correct words", spelling::LINES),
        ("ordinary forms", spelling::FORMS),
        ("non-Latin scripts", spelling::SCRIPTS),
        ("untagged French", spelling::FRENCH_UNTAGGED),
        (
            "tagged passages and code before the UI filter",
            spelling::TAGGED,
        ),
    ];
    for name in ["gbk-xml-decl", "gbk-meta-charset"] {
        let reader = Reader::open(
            File::open(crate::common::fixture(name)).expect("file"),
            DEFAULT_LIMITS,
        )
        .expect("open");
        let (mut total, mut marked, mut not_latin) = (0, 0, 0);
        for ix in 0..reader.rows().len() as u32 {
            let visible = reader.visible_text(ChapterIx(ix)).expect("text");
            total += word_count(visible.as_str());
            let found = chapter_words(&speller, &reader, ix);
            marked += found.len();
            not_latin += found.iter().filter(|word| !word.is_ascii()).count();
        }
        println!(
            "{name}: tokens {total}, marks {marked}, of those not ASCII {not_latin}, rate {:.2}%",
            pct(marked, total)
        );
    }
    for (label, ordinal) in rows {
        let visible = reader.visible_text(ChapterIx(ordinal)).expect("text");
        let words = word_count(visible.as_str());
        let planted = PLANTED
            .iter()
            .filter(|plant| plant.chapter == ordinal)
            .count();
        let marks = chapter_words(&speller, &reader, ordinal).len();
        let wrong = marks.saturating_sub(planted);
        println!(
            "{label}: words {words}, planted {planted}, false positives {wrong}, rate {:.2}%",
            pct(wrong, words - planted)
        );
    }
}

fn real_books() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real");
    let mut books: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    books.retain(|path| path.extension().is_some_and(|ext| ext == "epub"));
    books.sort();
    books
}

#[test]
#[ignore = "real books, counts only"]
fn report_real_book_counts() {
    let speller = speller();
    let loaded = Instant::now();
    drop(Speller::new(None));
    println!(
        "dictionary load {:.2} ms",
        loaded.elapsed().as_secs_f64() * 1e3
    );
    for (at, path) in real_books().iter().enumerate() {
        let reader = Reader::open(File::open(path).expect("file"), DEFAULT_LIMITS).expect("open");
        let mut words = 0;
        let mut marks = 0;
        let mut unique: HashSet<String> = HashSet::new();
        let mut check_ms: Vec<f64> = Vec::new();
        let mut whole_ms: Vec<f64> = Vec::new();
        let mut per_chapter: Vec<usize> = Vec::new();
        let mut largest = 0;
        for ix in 0..reader.rows().len() as u32 {
            let whole = Instant::now();
            let Ok(found) = speller.chapter(&reader, ChapterIx(ix)) else {
                continue;
            };
            whole_ms.push(whole.elapsed().as_secs_f64() * 1e3);
            let visible = reader.visible_text(ChapterIx(ix)).expect("text");
            let text = visible.as_str();
            largest = largest.max(text.len());
            let started = Instant::now();
            let spans = misspelled_spans(&speller.dictionary(), text);
            check_ms.push(started.elapsed().as_secs_f64() * 1e3);
            words += word_count(text);
            marks += spans.len();
            per_chapter.push(found.len());
            unique.extend(spans.into_iter().map(|span| text[span].to_owned()));
        }
        check_ms.sort_by(f64::total_cmp);
        whole_ms.sort_by(f64::total_cmp);
        per_chapter.sort_unstable();
        println!(
            "book {at}: chapters {}, words {words}, marks {marks} ({:.2}%), unique marked {}, marks per chapter median {} max {}, word check median {:.3} ms max {:.3} ms, whole call median {:.3} ms max {:.3} ms, largest text {largest} bytes",
            check_ms.len(),
            pct(marks, words),
            unique.len(),
            per_chapter[per_chapter.len() / 2],
            per_chapter[per_chapter.len() - 1],
            check_ms[check_ms.len() / 2],
            check_ms[check_ms.len() - 1],
            whole_ms[whole_ms.len() / 2],
            whole_ms[whole_ms.len() - 1],
        );
    }
}

#[test]
#[ignore = "prints the worst chapter time"]
fn report_worst_chapter_time() {
    let dir = scratch("worst");
    let path = dir.join("worst.epub");
    spelling::write_worst_chapter(&path, 500_000).expect("book");
    let reader = Reader::open(File::open(&path).expect("file"), DEFAULT_LIMITS).expect("open");
    let speller = speller();
    let mut times = Vec::new();
    let mut marks = 0;
    let mut text_bytes = 0;
    for _ in 0..5 {
        let started = Instant::now();
        marks = speller.chapter(&reader, ChapterIx(0)).expect("marks").len();
        times.push(started.elapsed().as_secs_f64() * 1e3);
        text_bytes = reader
            .visible_text(ChapterIx(0))
            .expect("text")
            .as_str()
            .len();
    }
    times.sort_by(f64::total_cmp);
    println!(
        "worst chapter: text {text_bytes} bytes, every word unknown, marks kept {marks}, median {:.1} ms, range {:.1} to {:.1}, 5 runs",
        times[2], times[0], times[4]
    );
}

fn write_pair(dir: &Path, code: &str, aff: &[u8], dic: &[u8]) -> (PathBuf, PathBuf) {
    let aff_path = dir.join(format!("{code}.aff"));
    let dic_path = dir.join(format!("{code}.dic"));
    fs::write(&aff_path, aff).unwrap();
    fs::write(&dic_path, dic).unwrap();
    (aff_path, dic_path)
}

const TINY_AFF: &[u8] = b"SET UTF-8\nTRY aeiou\n\nSFX S Y 1\nSFX S 0 s .\n";
const TINY_DIC: &[u8] = b"3\nbalay/S\nsuba\ndalan\n";

#[test]
fn a_dictionary_from_files_checks_its_own_words_and_affixes() {
    let dir = scratch("load-good");
    let (aff, dic) = write_pair(&dir, "xx", TINY_AFF, TINY_DIC);
    let speller = Speller::load(&aff, &dic, None).expect("loads");
    assert_eq!(
        flagged(&speller, "balay balays suba dalan sibaw"),
        ["sibaw"]
    );
    assert!(speller.suggest("dalam").contains(&"dalan".to_owned()));
}

#[test]
fn a_latin1_dictionary_is_read_through_its_set_line() {
    let dir = scratch("load-latin1");
    let (aff, dic) = write_pair(
        &dir,
        "xx",
        b"SET ISO8859-1\r\nTRY ae\r\n",
        b"2\r\ncaf\xE9\r\nna\xEFf\r\n",
    );
    let speller = Speller::load(&aff, &dic, None).expect("loads");
    assert_eq!(flagged(&speller, "caf\u{E9} na\u{EF}f cafx"), ["cafx"]);
}

#[test]
fn a_broken_dictionary_is_an_error_and_never_a_panic() {
    let dir = scratch("load-broken");
    let cases: [(&str, &[u8], &[u8]); 5] = [
        ("flag", b"FLAG nonsense\n", TINY_DIC),
        ("bytes", b"TRY \xFF\xFE\x00\x81\n", TINY_DIC),
        ("charset", b"SET KLINGON-9\n", TINY_DIC),
        ("dic", TINY_AFF, b"\xFF\xFE\xFD"),
        ("rule", b"SFX S Y 9\nSFX S\n", TINY_DIC),
    ];
    for (code, aff, dic) in cases {
        let (aff, dic) = write_pair(&dir, code, aff, dic);
        let error = Speller::load(&aff, &dic, None)
            .err()
            .unwrap_or_else(|| panic!("{code} loaded"));
        assert!(error.user_message().contains("dictionary"), "{code}");
    }
    let missing = dir.join("absent.aff");
    assert!(Speller::load(&missing, &dir.join("absent.dic"), None).is_err());
}

#[test]
fn an_added_word_goes_to_the_list_the_speller_was_given() {
    let dir = scratch("load-words");
    let (aff, dic) = write_pair(&dir, "xx", TINY_AFF, TINY_DIC);
    let list = dir.join("spelling-words.xx.txt");
    let speller = Speller::load(&aff, &dic, Some(list.clone())).expect("loads");
    speller.add_word("sibaw").expect("added");
    assert_eq!(fs::read_to_string(&list).unwrap(), "sibaw\n");
    let again = Speller::load(&aff, &dic, Some(list)).expect("loads");
    assert_eq!(flagged(&again, "sibaw suba"), Vec::<String>::new());
}
