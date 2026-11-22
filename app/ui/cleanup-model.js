import { applyBatch, emptyResults, filterTo } from './search-model.js';
import { exclusionsOf as choiceExclusions, isChapterOut, isHitOut } from './replace-model.js';

const count = (value) => value.toLocaleString('en-US');
const plural = (value, one, many) => `${count(value)} ${value === 1 ? one : many}`;

export const INTRO = 'Removes whole paragraphs and fixes chapter titles. The story text is left alone, and the one save can be undone.';

const TITLES = 'titles';
const titles = (value) => plural(value, 'chapter title', 'chapter titles');

export function countingState(specs, previewId) {
  return {
    phase: 'counting',
    rules: specs.map(({ rule, label, detail }) => ({ rule, label, detail, matches: 0, chapters: 0, on: true })),
    done: 0,
    total: 0,
    previewId,
    results: emptyResults('running'),
    message: '',
  };
}

function phaseAtEnd(end, preview) {
  if (end.kind === 'cancelled') return 'stopped';
  if (end.kind === 'done' && preview) return 'ready';
  return 'failed';
}

export function takeBatch(state, batch) {
  applyBatch(state.results, batch);
  state.done = batch.done;
  state.total = batch.total;
  if (!batch.end) return state;
  const { preview } = state.results;
  state.phase = phaseAtEnd(batch.end, preview);
  state.message = batch.end.message ?? '';
  if (state.phase === 'ready') {
    const tallies = new Map(preview.rules.map((tally) => [tally.rule, tally]));
    for (const entry of state.rules) {
      entry.matches = tallies.get(entry.rule)?.matches ?? 0;
      entry.chapters = tallies.get(entry.rule)?.chapters ?? 0;
    }
  }
  return state;
}

export function failState(state, failure) {
  state.phase = 'failed';
  state.message = String(failure?.message ?? failure);
  return state;
}

export function toggleRule(state, rule) {
  const entry = state.rules.find((found) => found.rule === rule);
  if (entry) entry.on = !entry.on;
  return state;
}

export const onRules = (state) => state.rules.filter((entry) => entry.on);

const isOn = (state, row) => state.rules.some((entry) => entry.on && entry.rule === row.hit.rule);

export function exclusionsOf(state, choices) {
  const rules = state.rules.filter((entry) => !entry.on).map(({ rule }) => ({ kind: 'rule', rule }));
  return [...rules, ...choiceExclusions(choices)];
}

const allShown = (state) => state.results.rows.length >= (state.results.preview?.matches ?? 0);

const isTitle = (row) => row.hit.rule === TITLES;

export function remaining(state, choices) {
  const shown = state.results.rows.filter((row) => isOn(state, row));
  const kept = shown.filter((row) => !isChapterOut(choices, row) && !isHitOut(choices, row));
  const keptTitles = kept.filter(isTitle).length;
  if (allShown(state)) {
    return { paragraphs: kept.length - keptTitles, titles: keptTitles, chapters: new Set(kept.map((row) => row.chapter.ix)).size };
  }
  const on = onRules(state);
  const dropped = (wanted) => shown.filter((row) => isTitle(row) === wanted).length - kept.filter((row) => isTitle(row) === wanted).length;
  const sum = (wanted) => on.filter((entry) => (entry.rule === TITLES) === wanted).reduce((total, entry) => total + entry.matches, 0);
  const chapters = on.reduce((most, entry) => Math.max(most, entry.chapters), 0);
  return { paragraphs: sum(false) - dropped(false), titles: sum(true) - dropped(true), chapters };
}

export function viewOf(state) {
  const view = emptyResults(state.results.phase);
  view.rows = state.results.rows.filter((row) => isOn(state, row));
  for (const row of view.rows) {
    const rows = view.byChapter.get(row.chapter.ix) ?? [];
    rows.push(row);
    view.byChapter.set(row.chapter.ix, rows);
  }
  return Object.assign(filterTo(view, view.range), {
    done: state.results.done,
    total: state.results.total,
    unreadable: state.results.unreadable,
    preview: state.results.preview,
  });
}

export const changesOf = (left) => left.paragraphs + left.titles;

export const canClean = (state, left) => state.phase === 'ready' && changesOf(left) > 0;

export const labelOf = (state, rule) => state.rules.find((entry) => entry.rule === rule)?.label ?? '';

export function ruleCountText(state, entry) {
  if (state.phase === 'counting') return 'Counting…';
  if (state.phase !== 'ready') return '';
  if (entry.matches === 0) return 'None found';
  if (entry.rule === TITLES) return titles(entry.matches);
  return `${plural(entry.matches, 'paragraph', 'paragraphs')} in ${plural(entry.chapters, 'chapter', 'chapters')}`;
}

export function statusText(state) {
  if (state.phase === 'counting') {
    return state.total === 0 ? 'Opening the book.' : `${count(state.done)} of ${plural(state.total, 'chapter', 'chapters')} checked`;
  }
  if (state.phase === 'stopped') return 'Stopped before every chapter was checked. Nothing was changed.';
  if (state.phase === 'failed') return state.message || 'The book could not be checked. Nothing was changed.';
  const unreadable = state.results.unreadable;
  if (unreadable > 0) return `${plural(unreadable, 'chapter', 'chapters')} could not be read, so ${unreadable === 1 ? 'it is' : 'they are'} left alone.`;
  return '';
}

export function summaryText(left) {
  if (changesOf(left) === 0) return 'Nothing to change.';
  if (left.titles === 0) return `${plural(left.paragraphs, 'paragraph', 'paragraphs')} will be removed from ${plural(left.chapters, 'chapter', 'chapters')}.`;
  if (left.paragraphs === 0) return `${titles(left.titles)} will be fixed.`;
  return `${plural(left.paragraphs, 'paragraph', 'paragraphs')} will be removed and ${titles(left.titles)} fixed, in ${plural(left.chapters, 'chapter', 'chapters')}.`;
}

export const headerText = (state) => onRules(state).map((entry) => entry.label).join(', ');

export function previewFooter(state, shown) {
  const on = onRules(state).filter((entry) => entry.matches > 0);
  const [one, many] = on.some((entry) => entry.rule === TITLES) ? ['change', 'changes'] : ['paragraph', 'paragraphs'];
  const total = on.reduce((sum, entry) => sum + entry.matches, 0);
  if (total > shown) return `Showing ${count(shown)} of ${plural(total, one, many)}. Only shown rows can be unticked.`;
  return `${plural(shown, one, many)}.`;
}

export function removingText(left) {
  if (left.titles === 0) return `Removing ${plural(left.paragraphs, 'paragraph', 'paragraphs')}`;
  if (left.paragraphs === 0) return `Fixing ${titles(left.titles)}`;
  return `Removing ${plural(left.paragraphs, 'paragraph', 'paragraphs')} and fixing ${titles(left.titles)}`;
}

export function doneText(end) {
  const fixed = end.titles ?? 0;
  if (fixed === 0) return `Removed ${plural(end.changes, 'paragraph', 'paragraphs')} from ${plural(end.chapters, 'chapter', 'chapters')}.`;
  if (end.changes === 0) return `Fixed ${titles(fixed)}.`;
  return `Removed ${plural(end.changes, 'paragraph', 'paragraphs')} and fixed ${titles(fixed)}, in ${plural(end.chapters, 'chapter', 'chapters')}.`;
}

export const STALE = 'The book was saved after the count, so nothing was removed. Open Clean up book again.';

export const failedText = (end) => (end.reason === 'stale' ? STALE : end.message);

export const NEWER_SAVE = 'The book was saved again after the clean-up, so it cannot be undone here. Open History and snapshots to go back further.';
export const UNDONE = 'Clean-up undone.';
export const ALREADY = 'The book already matches how it was before the clean-up.';
