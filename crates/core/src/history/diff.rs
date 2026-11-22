use serde::Serialize;

const MAX_CELLS: usize = 4_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffKind {
    Same,
    Gone,
    New,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct DiffPart {
    pub kind: DiffKind,
    pub text: String,
}

fn push(parts: &mut Vec<DiffPart>, kind: DiffKind, text: &str) {
    if text.is_empty() {
        return;
    }
    match parts.last_mut() {
        Some(last) if last.kind == kind => last.text.push_str(text),
        _ => parts.push(DiffPart {
            kind,
            text: text.to_owned(),
        }),
    }
}

fn align<'t>(old: &[&'t str], new: &[&'t str]) -> Vec<(DiffKind, &'t str)> {
    let lead = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[lead..]
        .iter()
        .rev()
        .zip(new[lead..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (a, b) = (&old[lead..old.len() - tail], &new[lead..new.len() - tail]);
    let mut out: Vec<(DiffKind, &str)> = old[..lead].iter().map(|t| (DiffKind::Same, *t)).collect();
    if (a.len() + 1) * (b.len() + 1) > MAX_CELLS {
        out.extend(a.iter().map(|t| (DiffKind::Gone, *t)));
        out.extend(b.iter().map(|t| (DiffKind::New, *t)));
    } else {
        let width = b.len() + 1;
        let mut table = vec![0u32; (a.len() + 1) * width];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                table[i * width + j] = if a[i] == b[j] {
                    table[(i + 1) * width + j + 1] + 1
                } else {
                    table[(i + 1) * width + j].max(table[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < a.len() || j < b.len() {
            if i < a.len() && j < b.len() && a[i] == b[j] {
                out.push((DiffKind::Same, a[i]));
                i += 1;
                j += 1;
            } else if j < b.len()
                && (i == a.len() || table[i * width + j + 1] >= table[(i + 1) * width + j])
            {
                out.push((DiffKind::New, b[j]));
                j += 1;
            } else {
                out.push((DiffKind::Gone, a[i]));
                i += 1;
            }
        }
    }
    out.extend(old[old.len() - tail..].iter().map(|t| (DiffKind::Same, *t)));
    out
}

fn words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut last_space = None;
    for (at, ch) in text.char_indices() {
        let space = ch.is_whitespace();
        if last_space.is_some_and(|was| was != space) {
            out.push(&text[start..at]);
            start = at;
        }
        last_space = Some(space);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn flush_run(parts: &mut Vec<DiffPart>, gone: &[&str], new: &[&str]) {
    let paired = gone.len().min(new.len());
    for (old_line, new_line) in gone[..paired].iter().zip(&new[..paired]) {
        for (kind, word) in align(&words(old_line), &words(new_line)) {
            push(parts, kind, word);
        }
    }
    for line in &gone[paired..] {
        push(parts, DiffKind::Gone, line);
    }
    for line in &new[paired..] {
        push(parts, DiffKind::New, line);
    }
}

pub(crate) fn diff_text(old: &str, new: &str) -> Vec<DiffPart> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let mut parts = Vec::new();
    let (mut gone, mut added): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for (kind, line) in align(&old_lines, &new_lines) {
        match kind {
            DiffKind::Gone => gone.push(line),
            DiffKind::New => added.push(line),
            DiffKind::Same => {
                flush_run(&mut parts, &gone, &added);
                gone.clear();
                added.clear();
                push(&mut parts, DiffKind::Same, line);
            }
        }
    }
    flush_run(&mut parts, &gone, &added);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(parts: &[DiffPart], keep: DiffKind) -> String {
        parts
            .iter()
            .filter(|part| part.kind == DiffKind::Same || part.kind == keep)
            .map(|part| part.text.as_str())
            .collect()
    }

    #[test]
    fn both_sides_rebuild_exactly_and_a_one_word_change_marks_only_that_word() {
        let old = "First line stays.\nThe cat sat here.\nLast line.";
        let new = "First line stays.\nThe dog sat here.\nLast line.\nAdded line.";
        let parts = diff_text(old, new);
        assert_eq!(side(&parts, DiffKind::Gone), old);
        assert_eq!(side(&parts, DiffKind::New), new);
        let gone: Vec<&str> = parts
            .iter()
            .filter(|p| p.kind == DiffKind::Gone)
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(gone, ["cat"]);
        let added: Vec<&str> = parts
            .iter()
            .filter(|p| p.kind == DiffKind::New)
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(added, ["dog", "\nAdded line."]);
    }

    #[test]
    fn equal_texts_are_one_same_part_and_empty_sides_are_whole_lines() {
        assert_eq!(
            diff_text("a\nb", "a\nb"),
            vec![DiffPart {
                kind: DiffKind::Same,
                text: "a\nb".to_owned()
            }]
        );
        assert_eq!(side(&diff_text("", "x\ny"), DiffKind::New), "x\ny");
        assert_eq!(side(&diff_text("x\ny", ""), DiffKind::Gone), "x\ny");
    }
}
