import { chapterNumberOf, placeName, positionText } from './chapter-number.js';

export const MAX_RESULTS = 100000;
export const ROW_HEIGHT = 84;
const OVERSCAN = 4;
const BEFORE_CHARS = 36;
const NO_MATCHES = 'No matches.';
const SHORT_TITLE_CHARS = 18;

export const WHOLE_BOOK = Object.freeze({ kind: 'whole' });
export const WHOLE_RANGE = Object.freeze({ before: true, after: true, here: null, name: null });
export const NO_PLACE = 'Open a chapter first';

export const emptyResults = (phase = 'idle', scope = WHOLE_BOOK, range = WHOLE_RANGE) => ({
  phase,
  scope,
  range,
  rows: [],
  shown: [],
  byChapter: new Map(),
  done: 0,
  total: 0,
  limited: 0,
  unreadable: 0,
  capped: false,
  message: '',
  preview: null,
});

const PHASE_OF_END = {
  done: 'done',
  tooSlow: 'done',
  cancelled: 'stopped',
  badPattern: 'bad',
  failed: 'bad',
};

export function applyBatch(results, batch) {
  const keep = keeper(results.range);
  for (const group of batch.groups) {
    const chapter = { ix: group.ix, rev: group.rev, number: group.number, title: group.title };
    const kept = keep(group.ix);
    let hits = results.byChapter.get(group.ix);
    if (!hits) {
      hits = [];
      results.byChapter.set(group.ix, hits);
    }
    for (const hit of group.hits) {
      const row = { chapter, hit, ordinal: hits.length };
      results.rows.push(row);
      if (kept) results.shown.push(row);
      hits.push(row);
    }
  }
  results.done = batch.done;
  results.total = batch.total;
  results.limited = batch.limited;
  results.unreadable = batch.unreadable;
  results.capped = batch.capped;
  if (batch.preview) results.preview = batch.preview;
  if (batch.end) {
    results.phase = PHASE_OF_END[batch.end.kind];
    results.message = batch.end.message ?? '';
  }
  return results;
}

export function failResults(results, failure) {
  results.phase = 'bad';
  results.message = String(failure?.message ?? failure);
  return results;
}

const count = (value) => value.toLocaleString('en-US');
const plural = (value, one, many) => `${count(value)} ${value === 1 ? one : many}`;

const sentence = (text) => text.charAt(0).toUpperCase() + text.slice(1);

const SCOPE_COPY = {
  whole: {
    search: () => 'Search the whole book',
    apply: () => 'Apply to whole book',
  },
  upTo: {
    search: (name) => `Search ${name.short} and before`,
    apply: (name) => `Apply to ${name.short} and before`,
  },
  from: {
    search: (name) => `Search ${name.short} and after`,
    apply: (name) => `Apply to ${name.short} and after`,
  },
  only: {
    search: (name) => `Search ${name.short} only`,
    apply: (name) => `Apply to ${name.short} only`,
  },
};
const SIDE_HINT = {
  before: (name, on) => `${on ? 'Hide' : 'Show'} matches before ${name.long}`,
  after: (name, on) => `${on ? 'Hide' : 'Show'} matches after ${name.long}`,
};
const SPAN = {
  whole: () => [0, Infinity],
  upTo: (here) => [0, here],
  from: (here) => [here, Infinity],
  only: (here) => [here, here],
};

function clipped(title) {
  const chars = [...title];
  return chars.length > SHORT_TITLE_CHARS ? `${chars.slice(0, SHORT_TITLE_CHARS - 1).join('')}…` : title;
}

function hereName({ title, number }) {
  const chapter = chapterNumberOf(title);
  if (chapter !== null) return { short: `ch. ${chapter}`, long: `Chapter ${chapter}` };
  if (title) return { short: clipped(title), long: title };
  if (number !== null && number !== undefined) return { short: `position ${count(number)}`, long: positionText(number) };
  return { short: 'here', long: 'this page' };
}

const kindOf = (range) => (range.before ? (range.after ? 'whole' : 'upTo') : (range.after ? 'from' : 'only'));

export function rangeOf(before, after, here) {
  if (before && after) return WHOLE_RANGE;
  return { before, after, here: here.ix, name: hereName(here) };
}

export function wireOf(range) {
  const kind = kindOf(range);
  return kind === 'whole' ? WHOLE_BOOK : { kind, here: range.here };
}

const spanOf = (wire) => SPAN[wire.kind](wire.here);

function keeper(range) {
  const [low, high] = spanOf(wireOf(range));
  return (ix) => ix >= low && ix <= high;
}

export function filterTo(results, range) {
  const keep = keeper(range);
  results.range = range;
  results.shown = results.rows.filter((row) => keep(row.chapter.ix));
  return results;
}

export const inRange = (range, ix) => keeper(range)(ix);

function covers(wire, range) {
  const [low, high] = spanOf(wire);
  const [from, to] = spanOf(wireOf(range));
  return low <= from && to <= high;
}

const sameWire = (a, b) => a.kind === b.kind && a.here === b.here;

export function rerunFor(results, range, replacing) {
  const wire = wireOf(range);
  if (replacing) return wire;
  if (results.phase === 'idle' || results.phase === 'bad') return null;
  if (results.capped) return sameWire(results.scope, wire) ? null : wire;
  if (results.phase === 'stopped' || !covers(results.scope, range)) return WHOLE_BOOK;
  return null;
}

export function toggled(range, side, place) {
  const before = side === 'before' ? !range.before : range.before;
  const after = side === 'after' ? !range.after : range.after;
  if (before && after) return WHOLE_RANGE;
  if (range.here === null) return rangeOf(before, after, place);
  return { ...range, before, after };
}

export const reanchored = (range, place) => (range.here === null || place === null ? range : rangeOf(range.before, range.after, place));

export function rangeStrip(range, place) {
  const placeholder = SCOPE_COPY[kindOf(range)].search(range.name);
  if (place === null && range.here === null) {
    const off = { on: true, hint: NO_PLACE, disabled: true };
    return { before: off, after: off, here: { label: 'This chapter', hint: NO_PLACE, disabled: true }, placeholder };
  }
  const name = range.name ?? hereName(place);
  const side = (key) => ({ on: range[key], hint: SIDE_HINT[key](name, range[key]), disabled: false });
  const moved = range.here !== null && place !== null && place.ix !== range.here;
  const here = moved
    ? { label: sentence(name.short), hint: `Count from ${hereName(place).long} instead`, disabled: false }
    : { label: sentence(name.short), hint: `Matches are counted from ${name.long}`, disabled: true };
  return { before: side('before'), after: side('after'), here, placeholder };
}

export const applyLabel = (range) => SCOPE_COPY[kindOf(range)].apply(range.name);

export function stepIndex(current, by, total) {
  if (total === 0) return null;
  if (current === null) return by > 0 ? 0 : total - 1;
  return (current + by + total) % total;
}

function firstFrom(rows, here) {
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    const { chapter, ordinal } = rows[middle];
    if (chapter.ix < here.ix || (chapter.ix === here.ix && ordinal < here.passed)) low = middle + 1;
    else high = middle;
  }
  return low;
}

export function indexFrom(rows, here, by, complete) {
  if (rows.length === 0) return null;
  const at = firstFrom(rows, here);
  if (by > 0) return at < rows.length ? at : (complete ? 0 : null);
  return at > 0 ? at - 1 : (complete ? rows.length - 1 : null);
}

export function barText(results, current = null) {
  const shown = results.shown.length;
  if (current !== null && current < shown) return `${count(current + 1)} of ${count(shown)}`;
  if (results.phase === 'idle' || results.phase === 'bad') return '';
  if (shown === 0) return results.phase === 'done' ? 'No matches' : '';
  return plural(shown, 'match', 'matches');
}

export function footerText(results, current = null) {
  const shown = results.shown.length;
  const parts = [];
  if (current !== null && current < shown) {
    parts.push(`Match ${count(current + 1)} of ${count(shown)}${results.phase === 'running' ? ' so far' : ''}.`);
  } else if (results.phase === 'running') {
    parts.push(`${plural(shown, 'match', 'matches')} so far. Results stop at ${count(MAX_RESULTS)}.`);
  } else if (results.phase !== 'idle' && results.phase !== 'bad') {
    if (results.capped) parts.push(`${count(shown)} matches shown. Results stop at ${count(MAX_RESULTS)}.`);
    else if (shown === 0 && results.phase === 'done') parts.push(NO_MATCHES);
    else parts.push(`${plural(shown, 'match', 'matches')}.`);
  }
  if (results.limited > 0) parts.push(`This pattern is too slow, so ${plural(results.limited, 'chapter was', 'chapters were')} not fully searched.`);
  if (results.unreadable > 0) parts.push(`${plural(results.unreadable, 'chapter', 'chapters')} could not be read.`);
  return parts.join(' ');
}

export function statusOf(results) {
  const share = results.total === 0 ? 0 : results.done / results.total;
  return {
    visible: results.phase !== 'idle' && results.phase !== 'bad',
    scanned: `${count(results.done)} of ${plural(results.total, 'chapter', 'chapters')} searched`,
    fill: `${Math.round(share * 100)}%`,
    action: results.phase === 'running' ? 'Stop' : 'Rerun',
  };
}

export const whereOf = (chapter) => placeName(chapter.title, chapter.number);

export const snippetBefore = (before, chars = BEFORE_CHARS) => [...before].slice(-chars).join('');

export function windowOf(scrollTop, viewport, rows) {
  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const end = Math.min(rows, Math.ceil((scrollTop + viewport) / ROW_HEIGHT) + OVERSCAN);
  return { first, end: Math.max(first, end) };
}

export const sameRev = (a, b) => a.crc32 === b.crc32 && a.inflated_len === b.inflated_len;
