import { chapterNumberOf } from './chapter-number.js';

export const ROW_HEIGHT = 40;
const OVERSCAN = 8;
const fold = (text) => text.toLowerCase();

const keyOf = (entry) => (entry.href ? `${entry.href}#${entry.fragment ?? ''}` : `#${entry.depth}#${entry.label}`);

function addFamily(entries) {
  const open = [];
  entries.forEach((entry, at) => {
    while (open.length > 0 && entries[open[open.length - 1]].depth >= entry.depth) open.pop();
    entry.parent = open.length > 0 ? open[open.length - 1] : null;
    entry.isParent = false;
    entry.first = entry.number;
    entry.last = entry.number;
    entry.chapters = entry.chapter === null ? null : { first: entry.chapter, last: entry.chapter };
    for (const ancestor of open) {
      const above = entries[ancestor];
      above.isParent = true;
      if (entry.number !== null) {
        above.first = above.first === null ? entry.number : Math.min(above.first, entry.number);
        above.last = above.last === null ? entry.number : Math.max(above.last, entry.number);
      }
      if (entry.chapter !== null) {
        above.chapters = above.chapters === null
          ? { first: entry.chapter, last: entry.chapter }
          : { first: Math.min(above.chapters.first, entry.chapter), last: Math.max(above.chapters.last, entry.chapter) };
      }
    }
    open.push(at);
  });
}

function buildChapterIndex(entries) {
  const chosen = new Map();
  const first = new Map();
  entries.forEach((entry, at) => {
    if (entry.ix === null) return;
    if (!first.has(entry.ix)) first.set(entry.ix, at);
    const held = chosen.get(entry.ix);
    const better = held === undefined
      || (entries[held].fragment && !entry.fragment)
      || (entries[held].isParent && !entries[held].fragment && !entry.isParent && !entry.fragment);
    if (better) chosen.set(entry.ix, at);
  });
  const chapters = [...chosen.keys()].sort((a, b) => a - b);
  return { chosen, chapters, first };
}

export function buildToc({ source, entries, linear }) {
  const rows = entries.map((entry) => ({ ...entry, key: keyOf(entry), folded: fold(entry.label), chapter: chapterNumberOf(entry.label) }));
  addFamily(rows);
  const toc = { source, entries: rows, linear, numbers: new Map(linear.map((ix, at) => [ix, at + 1])) };
  toc.index = buildChapterIndex(rows);
  return toc;
}

export function entryOfChapter(toc, ix) {
  const { chosen, chapters } = toc.index;
  if (chosen.has(ix)) return chosen.get(ix);
  let low = 0;
  let high = chapters.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (chapters[middle] <= ix) low = middle + 1;
    else high = middle;
  }
  return low === 0 ? null : chosen.get(chapters[low - 1]);
}

export const queryOf = (text) => fold(text.trim());
const withoutCommas = (text) => text.replace(/,/g, '');

export const matches = (entry, query) => withoutCommas(entry.folded).includes(withoutCommas(query));

export function visibleRows(toc, closed, query) {
  const { entries } = toc;
  const shown = [];
  if (query) {
    entries.forEach((entry, at) => {
      if (matches(entry, query)) shown.push(at);
    });
    return shown;
  }
  for (let at = 0; at < entries.length; at += 1) {
    shown.push(at);
    if (entries[at].isParent && closed.has(entries[at].key)) {
      const { depth } = entries[at];
      while (at + 1 < entries.length && entries[at + 1].depth > depth) at += 1;
    }
  }
  return shown;
}

export function rowOfChapter(toc, shown, ix) {
  let entry = entryOfChapter(toc, ix);
  while (entry !== null) {
    const row = shown.indexOf(entry);
    if (row >= 0) return row;
    entry = toc.entries[entry].parent;
  }
  return -1;
}

export function windowOf(scrollTop, viewport, rows) {
  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const end = Math.min(rows, Math.ceil((scrollTop + viewport) / ROW_HEIGHT) + OVERSCAN);
  return { first, end: Math.max(first, end) };
}

export function scrollToKeep(row, scrollTop, viewport) {
  const top = row * ROW_HEIGHT;
  const bottom = top + ROW_HEIGHT;
  if (top >= scrollTop && bottom <= scrollTop + viewport) return scrollTop;
  const margin = Math.max(0, Math.floor(viewport / 3));
  return Math.max(0, top - margin);
}

export function setHeading(toc, ix, title) {
  const at = toc.index.chosen.get(ix);
  if (at === undefined) return false;
  const entry = toc.entries[at];
  entry.label = title;
  entry.folded = fold(title);
  entry.chapter = chapterNumberOf(title);
  return true;
}

export function rowLabel(entry) {
  if (!entry.isParent || entry.chapters === null) return '';
  const { first, last } = entry.chapters;
  return first === last ? first.toLocaleString('en-US') : `${first.toLocaleString('en-US')}-${last.toLocaleString('en-US')}`;
}

export function countText(toc, shownCount, query) {
  const total = toc.entries.length.toLocaleString('en-US');
  if (query) return `${shownCount.toLocaleString('en-US')} of ${total} entries`;
  return `${total} entries`;
}
