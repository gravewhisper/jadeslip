use crate::clean::{self, CleanRule};
use crate::commit::FileStamp;
use crate::replace::preview_on;
use crate::search::{ByteBudget, CHAPTER_BYTE_BUDGET, SearchEvent, SearchOptions, SearchScope};
use crate::{
    CancelToken, ChapterIx, DEFAULT_LIMITS, Error, Extracted, RULES, Reader, ReplaceRequest,
};
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufWriter, ErrorKind, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

const USAGE: &str = "Usage: jadeslip info <book.epub>\n       jadeslip extract <book.epub> <index>\n       jadeslip clean-report <book.epub> [--samples <count>] [--all] [--titles]\n";
const DEFAULT_SAMPLES: usize = 10;
const TITLE_SAMPLES: usize = 15;

fn open(path: &Path) -> Result<Reader, Error> {
    Reader::open(File::open(path)?, DEFAULT_LIMITS)
}

fn write_info(reader: &Reader, out: &mut dyn Write) -> io::Result<()> {
    let mut out = BufWriter::new(out);
    let summary = reader.summary();
    writeln!(out, "Title: {}", summary.title)?;
    writeln!(
        out,
        "Author: {}",
        summary.author.as_deref().unwrap_or("Unknown")
    )?;
    writeln!(out, "Chapters: {}", summary.linear_chapters)?;
    writeln!(out, "Spine items: {}", reader.rows().len())?;
    writeln!(
        out,
        "Inflated spine bytes: {}",
        summary.inflated_spine_bytes
    )?;
    writeln!(out, "index\tchapter\thref")?;
    for row in reader.rows() {
        let chapter = row
            .linear_ordinal
            .map_or_else(|| "-".to_owned(), |ordinal| ordinal.to_string());
        let href = row.href.as_ref().map_or("-", |href| href.as_str());
        writeln!(out, "{}\t{chapter}\t{href}", row.ix.0)?;
    }
    out.flush()
}

pub fn info(path: &Path, out: &mut dyn Write) -> Result<(), Error> {
    let reader = open(path)?;
    write_info(&reader, out)?;
    Ok(())
}

pub fn extract(
    path: &Path,
    index: u32,
    out: &mut dyn Write,
    note: &mut dyn Write,
    out_is_terminal: bool,
) -> Result<(), Error> {
    let reader = open(path)?;
    match reader.extract(ChapterIx(index))? {
        Extracted::Document(bytes) => out.write_all(&bytes)?,
        Extracted::Image { media_type, bytes } if out_is_terminal => {
            writeln!(
                note,
                "Chapter {index} is an image ({media_type}, {} bytes). Redirect the output to save it.",
                bytes.len()
            )?;
        }
        Extracted::Image { bytes, .. } => out.write_all(&bytes)?,
    }
    out.flush()?;
    Ok(())
}

pub enum CleanListing {
    Samples(usize),
    All,
    Titles,
}

pub fn clean_report(path: &Path, listing: &CleanListing, out: &mut dyn Write) -> Result<(), Error> {
    let file = File::open(path)?;
    let stamp = FileStamp::of(&file.metadata()?);
    let reader = Arc::new(Reader::open(file, DEFAULT_LIMITS)?);
    let request = ReplaceRequest::clean(RULES.iter().map(|spec| spec.rule).collect());
    let mut found = Vec::new();
    let started = Instant::now();
    let preview = preview_on(
        (stamp, reader, ByteBudget::new(CHAPTER_BYTE_BUDGET)),
        &request,
        SearchScope::Whole,
        u32::MAX,
        &SearchOptions::default(),
        &CancelToken::default(),
        &mut |event| {
            if let SearchEvent::Hits { chapter, hits, .. } = event {
                found.extend(hits.into_iter().map(|hit| (chapter, hit)));
            }
        },
    )?;
    let elapsed = started.elapsed();
    let mut out = BufWriter::new(out);
    let flat = |text: &str| text.replace(['\t', '\n'], " ");
    if let CleanListing::All = listing {
        for (chapter, hit) in &found {
            let rule = hit
                .rule
                .and_then(|rule| RULES.iter().find(|spec| spec.rule == rule));
            let label = rule.map_or("-", |spec| spec.label);
            let matched = flat(&hit.matched);
            match (hit.rule, &hit.replaced) {
                (Some(CleanRule::Titles), Some(new)) => {
                    writeln!(out, "{label}\t{}\t{matched} -> {}", chapter.0, flat(new))?;
                }
                _ => writeln!(out, "{label}\t{}\t{matched}", chapter.0)?,
            }
        }
        out.flush()?;
        return Ok(());
    }
    if let CleanListing::Titles = listing {
        let titles: Vec<(ChapterIx, &str, &str)> = found
            .iter()
            .filter(|(_, hit)| hit.rule == Some(CleanRule::Titles))
            .filter_map(|(chapter, hit)| {
                Some((*chapter, hit.matched.as_str(), hit.replaced.as_deref()?))
            })
            .collect();
        writeln!(out, "Titles fixed: {}", titles.len())?;
        writeln!(out, "Samples:")?;
        for at in spread(titles.len(), TITLE_SAMPLES) {
            let (chapter, old, new) = titles[at];
            writeln!(out, "{}\t{} -> {}", chapter.0, flat(old), flat(new))?;
        }
        let fixed: Vec<ChapterIx> = titles.iter().map(|(chapter, ..)| *chapter).collect();
        write_unfixed(path, &fixed, &mut out)?;
        out.flush()?;
        return Ok(());
    }
    writeln!(out, "Preview: {} ms", elapsed.as_millis())?;
    writeln!(
        out,
        "Matches: {} in {} chapters",
        preview.matches, preview.chapters
    )?;
    writeln!(out, "Unreadable chapters: {}", preview.unreadable)?;
    for tally in &preview.rules {
        let label = RULES
            .iter()
            .find(|spec| spec.rule == tally.rule)
            .map_or("-", |spec| spec.label);
        let unit = if tally.rule == CleanRule::Titles {
            "titles"
        } else {
            "paragraphs"
        };
        writeln!(
            out,
            "{label}: {} {unit} in {} chapters",
            tally.matches, tally.chapters
        )?;
    }
    if let CleanListing::Samples(count) = *listing
        && count > 0
        && !found.is_empty()
    {
        writeln!(out, "Samples:")?;
        let mut picked: Vec<usize> = (0..count).map(|ix| ix * found.len() / count).collect();
        picked.dedup();
        for ix in picked {
            let (chapter, hit) = &found[ix];
            let label = RULES
                .iter()
                .find(|spec| Some(spec.rule) == hit.rule)
                .map_or("-", |spec| spec.label);
            writeln!(out, "{}\t{label}\t{}", chapter.0, hit.matched)?;
        }
    }
    out.flush()?;
    Ok(())
}

fn spread(len: usize, wanted: usize) -> Vec<usize> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut picked: Vec<usize> = Vec::new();
    while picked.len() < wanted.min(len) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let at = (state % len as u64) as usize;
        if !picked.contains(&at) {
            picked.push(at);
        }
    }
    picked.sort_unstable();
    picked
}

fn shape_of(text: &str) -> String {
    let mut shape = String::new();
    let mut first = true;
    for word in text.split_whitespace() {
        if !shape.is_empty() {
            shape.push(' ');
        }
        let core = word.trim_matches(|ch: char| !ch.is_alphanumeric());
        let token = if core.is_empty() {
            word.to_owned()
        } else if core.chars().all(|ch| ch.is_ascii_digit()) {
            word.replace(core, "N")
        } else if first {
            word.to_owned()
        } else {
            word.replace(core, "W")
        };
        shape.push_str(&token);
        first = false;
    }
    let mut squeezed = String::new();
    for part in shape.split(' ') {
        if part == "W" && squeezed.ends_with("W") {
            continue;
        }
        if !squeezed.is_empty() {
            squeezed.push(' ');
        }
        squeezed.push_str(part);
    }
    squeezed
}

fn write_unfixed(path: &Path, fixed: &[ChapterIx], out: &mut dyn Write) -> Result<(), Error> {
    let reader = open(path)?;
    let limits = reader.limits();
    let mut groups: std::collections::BTreeMap<(&str, String), Vec<(ChapterIx, String)>> =
        std::collections::BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for row in reader.rows() {
        if row.is_nav
            || fixed.contains(&row.ix)
            || !row
                .href
                .as_ref()
                .is_some_and(|href| seen.insert(href.clone()))
        {
            continue;
        }
        let Ok(Some(model)) = reader.model(row.ix) else {
            continue;
        };
        let Ok(chapter) = clean::Chapter::of(&model, &limits) else {
            continue;
        };
        let mut headings = chapter.paras.iter().filter(|para| para.heading);
        let Some(last) = headings.clone().next_back() else {
            continue;
        };
        let heading = headings
            .find(|para| clean::chapter_title(&para.text).is_some())
            .unwrap_or(last);
        let reason = match clean::chapter_title(&heading.text) {
            None => "no chapter number, or a second one",
            Some(found) if found.name.is_empty() => {
                "no name, and no line below with the same number"
            }
            Some(_) => continue,
        };
        groups
            .entry((reason, shape_of(&heading.text)))
            .or_default()
            .push((row.ix, heading.text.clone()));
    }
    let total: usize = groups.values().map(Vec::len).sum();
    writeln!(out, "Titles left alone: {total}")?;
    for ((reason, shape), members) in &groups {
        writeln!(out, "{reason}: {shape} ({})", members.len())?;
        for (chapter, text) in members {
            writeln!(out, "\t{}\t{text}", chapter.0)?;
        }
    }
    Ok(())
}

fn clean_listing(flags: &[OsString]) -> Option<CleanListing> {
    match flags {
        [] => Some(CleanListing::Samples(DEFAULT_SAMPLES)),
        [flag] if flag == "--all" => Some(CleanListing::All),
        [flag] if flag == "--titles" => Some(CleanListing::Titles),
        [flag, count] if flag == "--samples" => count
            .to_str()
            .and_then(|text| text.parse::<usize>().ok())
            .map(CleanListing::Samples),
        _ => None,
    }
}

fn exit_code(result: Result<(), Error>, err: &mut dyn Write) -> i32 {
    match result {
        Ok(()) => 0,
        Err(Error::Io(error)) if error.kind() == ErrorKind::BrokenPipe => 0,
        Err(error) => {
            let _ = writeln!(err, "jadeslip: {}", error.user_message());
            1
        }
    }
}

fn usage(err: &mut dyn Write) -> i32 {
    let _ = err.write_all(USAGE.as_bytes());
    2
}

pub fn run(
    args: &[OsString],
    out: &mut dyn Write,
    err: &mut dyn Write,
    out_is_terminal: bool,
) -> i32 {
    match args {
        [command, path] if command == "info" => exit_code(info(Path::new(path), out), err),
        [command, path, index] if command == "extract" => {
            let Some(index) = index.to_str().and_then(|text| text.parse::<u32>().ok()) else {
                let _ = writeln!(err, "jadeslip: The chapter index must be a whole number.");
                return 2;
            };
            let mut note = Vec::new();
            let result = extract(Path::new(path), index, out, &mut note, out_is_terminal);
            let _ = err.write_all(&note);
            exit_code(result, err)
        }
        [command, path, flags @ ..] if command == "clean-report" => match clean_listing(flags) {
            Some(listing) => exit_code(clean_report(Path::new(path), &listing, out), err),
            None => usage(err),
        },
        _ => usage(err),
    }
}
