import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyChoices, toggleChapter, toggleHit } from '../../app/ui/replace-model.js';
import {
  canClean, countingState, doneText, removingText, exclusionsOf, failState, failedText, STALE, headerText, previewFooter, remaining, ruleCountText, statusText, summaryText, takeBatch, toggleRule, viewOf,
} from '../../app/ui/cleanup-model.js';

const SPECS = [
  { rule: 'watermark', label: 'Watermark lines', detail: 'Site lines.' },
  { rule: 'notes', label: 'Translator notes and support requests', detail: 'Credits.' },
  { rule: 'heading', label: 'Repeated chapter titles', detail: 'Titles.' },
];

const hit = (rule, text = 'junk') => ({ range: { start: 0, end: text.length }, before: '', matched: text, after: '', replaced: '', spans_tags: false, crosses_block: false, same: false, rule });
const group = (ix, hits) => ({ ix, rev: { crc32: ix, inflated_len: 1 }, number: ix + 1, title: null, hits });
const base = { done: 2, total: 3, limited: 0, unreadable: 0, capped: false, preview: null, end: null };
const preview = (extra = {}) => ({
  matches: 5, changes: 5, chapters: 3, skipped: 0, spanning: 0, limited: 0, unreadable: 0, touched: [],
  rules: [{ rule: 'watermark', matches: 2, chapters: 2 }, { rule: 'notes', matches: 2, chapters: 1 }, { rule: 'heading', matches: 1, chapters: 1 }],
  ...extra,
});

function counted(extra = {}) {
  const state = countingState(SPECS, 7);
  takeBatch(state, { ...base, groups: [group(0, [hit('heading'), hit('watermark'), hit('notes')]), group(1, [hit('notes')])] });
  takeBatch(state, { ...base, done: 3, groups: [group(2, [hit('watermark')])], preview: preview(extra), end: { kind: 'done' } });
  return state;
}

test('counting shows progress and no counts until the preview ends', () => {
  const state = countingState(SPECS, 3);
  takeBatch(state, { ...base, groups: [group(0, [hit('notes')])] });
  assert.equal(state.phase, 'counting');
  assert.equal(statusText({ ...state, done: 2400, total: 6191 }), '2,400 of 6,191 chapters checked');
  assert.equal(ruleCountText(state, state.rules[0]), 'Counting…');
  assert.equal(canClean(state, remaining(state, emptyChoices())), false);
});

test('the finished preview fills every rule from its tally', () => {
  const state = counted();
  assert.equal(state.phase, 'ready');
  assert.deepEqual(state.rules.map((entry) => ruleCountText(state, entry)), ['2 paragraphs in 2 chapters', '2 paragraphs in 1 chapter', '1 paragraph in 1 chapter']);
  assert.deepEqual(remaining(state, emptyChoices()), { paragraphs: 5, titles: 0, chapters: 3 });
  assert.equal(summaryText(remaining(state, emptyChoices())), '5 paragraphs will be removed from 3 chapters.');
  assert.equal(canClean(state, remaining(state, emptyChoices())), true);
});

test('large counts read with separators and a rule with nothing says so', () => {
  const state = counted({ rules: [{ rule: 'watermark', matches: 1234, chapters: 1200 }, { rule: 'notes', matches: 0, chapters: 0 }, { rule: 'heading', matches: 1, chapters: 1 }] });
  assert.equal(ruleCountText(state, state.rules[0]), '1,234 paragraphs in 1,200 chapters');
  assert.equal(ruleCountText(state, state.rules[1]), 'None found');
});

test('an unchecked rule leaves its rows out of the count, the view and the save', () => {
  const state = toggleRule(counted(), 'notes');
  const choices = emptyChoices();
  assert.deepEqual(remaining(state, choices), { paragraphs: 3, titles: 0, chapters: 2 });
  assert.deepEqual(exclusionsOf(state, choices), [{ kind: 'rule', rule: 'notes' }]);
  const view = viewOf(state);
  assert.deepEqual(view.rows.map((row) => row.hit.rule), ['heading', 'watermark', 'watermark']);
  assert.equal(view.byChapter.has(1), false);
  assert.equal(headerText(state), 'Watermark lines, Repeated chapter titles');
});

test('an unticked row keeps its place in the whole chapter after a rule is unchecked', () => {
  const state = toggleRule(counted(), 'heading');
  const view = viewOf(state);
  const watermark = view.rows.find((row) => row.chapter.ix === 0 && row.hit.rule === 'watermark');
  const choices = emptyChoices();
  toggleHit(choices, watermark);
  assert.deepEqual(exclusionsOf(state, choices), [{ kind: 'rule', rule: 'heading' }, { kind: 'hit', chapter: 0, ordinal: 1 }]);
  assert.deepEqual(remaining(state, choices), { paragraphs: 3, titles: 0, chapters: 3 });
});

test('a skipped chapter drops all of its checked rows', () => {
  const state = counted();
  const choices = emptyChoices();
  toggleChapter(choices, state.results.rows[0]);
  assert.deepEqual(remaining(state, choices), { paragraphs: 2, titles: 0, chapters: 2 });
  assert.deepEqual(exclusionsOf(state, choices), [{ kind: 'chapter', chapter: 0 }]);
});

test('nothing left to remove cannot be applied', () => {
  const state = counted();
  for (const entry of state.rules) toggleRule(state, entry.rule);
  const left = remaining(state, emptyChoices());
  assert.deepEqual(left, { paragraphs: 0, titles: 0, chapters: 0 });
  assert.equal(summaryText(left), 'Nothing to change.');
  assert.equal(canClean(state, left), false);
});

test('past the row cap the count comes from the tallies minus the unticked rows', () => {
  const state = counted({ matches: 40000, rules: [{ rule: 'watermark', matches: 30000, chapters: 5000 }, { rule: 'notes', matches: 9999, chapters: 6000 }, { rule: 'heading', matches: 1, chapters: 1 }] });
  const choices = emptyChoices();
  toggleHit(choices, state.results.rows[1]);
  assert.deepEqual(remaining(state, choices), { paragraphs: 39999, titles: 0, chapters: 6000 });
  assert.equal(previewFooter(state, 5), 'Showing 5 of 40,000 paragraphs. Only shown rows can be unticked.');
});

test('a stop, a failure and an unreadable chapter each say what happened', () => {
  const stopped = countingState(SPECS, 1);
  takeBatch(stopped, { ...base, groups: [], end: { kind: 'cancelled' } });
  assert.equal(stopped.phase, 'stopped');
  assert.equal(statusText(stopped), 'Stopped before every chapter was checked. Nothing was changed.');
  assert.equal(ruleCountText(stopped, stopped.rules[0]), '');
  const failed = failState(countingState(SPECS, 2), new Error('No book is open.'));
  assert.equal(statusText(failed), 'No book is open.');
  const unreadable = counted();
  unreadable.results.unreadable = 1;
  assert.equal(statusText(unreadable), '1 chapter could not be read, so it is left alone.');
});

test('the result names paragraphs and chapters in the singular when there is one', () => {
  assert.equal(doneText({ changes: 1, chapters: 1 }), 'Removed 1 paragraph from 1 chapter.');
  assert.equal(doneText({ changes: 4342, chapters: 4116 }), 'Removed 4,342 paragraphs from 4,116 chapters.');
  assert.equal(removingText({ paragraphs: 1, titles: 0 }), 'Removing 1 paragraph');
  assert.equal(removingText({ paragraphs: 4342, titles: 0 }), 'Removing 4,342 paragraphs');
});

test('a stale preview asks for a new count, and any other failure keeps its own words', () => {
  assert.equal(failedText({ kind: 'failed', reason: 'stale', message: 'Check the new preview and apply again.' }), STALE);
  assert.equal(failedText({ kind: 'failed', reason: 'other', message: 'The disk is full.' }), 'The disk is full.');
});

const WITH_TITLES = [...SPECS, { rule: 'titles', label: 'Fix chapter titles', detail: 'Numbers and names.' }];
const fix = (from, to) => ({ ...hit('titles', from), replaced: to });

function countedWithTitles(extra = {}) {
  const state = countingState(WITH_TITLES, 9);
  const groups = [group(0, [fix('Chapter 1 - Dawn', '1 Dawn'), hit('watermark')]), group(1, [fix('Chapter 2: Dusk', '2 Dusk')]), group(2, [hit('notes')])];
  takeBatch(state, {
    ...base, done: 3, groups, end: { kind: 'done' },
    preview: preview({ matches: 4, rules: [{ rule: 'watermark', matches: 1, chapters: 1 }, { rule: 'notes', matches: 1, chapters: 1 }, { rule: 'heading', matches: 0, chapters: 0 }, { rule: 'titles', matches: 2, chapters: 2 }], ...extra }),
  });
  return state;
}

test('title fixes are on by default and count as chapter titles, not paragraphs', () => {
  const state = countedWithTitles();
  const titles = state.rules.find((entry) => entry.rule === 'titles');
  assert.equal(titles.on, true);
  assert.equal(ruleCountText(state, titles), '2 chapter titles');
  const left = remaining(state, emptyChoices());
  assert.deepEqual(left, { paragraphs: 2, titles: 2, chapters: 3 });
  assert.equal(summaryText(left), '2 paragraphs will be removed and 2 chapter titles fixed, in 3 chapters.');
  assert.equal(removingText(left), 'Removing 2 paragraphs and fixing 2 chapter titles');
  assert.equal(previewFooter(state, 4), '4 changes.');
});

test('a title row shows the old title and the new one and can be unticked', () => {
  const state = countedWithTitles();
  const row = viewOf(state).rows.find((found) => found.hit.rule === 'titles' && found.chapter.ix === 1);
  assert.equal(row.hit.matched, 'Chapter 2: Dusk');
  assert.equal(row.hit.replaced, '2 Dusk');
  const choices = emptyChoices();
  toggleHit(choices, row);
  assert.deepEqual(exclusionsOf(state, choices), [{ kind: 'hit', chapter: 1, ordinal: 0 }]);
  assert.deepEqual(remaining(state, choices), { paragraphs: 2, titles: 1, chapters: 2 });
});

test('titles alone can be saved, and unchecking them leaves only paragraphs', () => {
  const only = countedWithTitles();
  for (const rule of ['watermark', 'notes', 'heading']) toggleRule(only, rule);
  const left = remaining(only, emptyChoices());
  assert.deepEqual(left, { paragraphs: 0, titles: 2, chapters: 2 });
  assert.equal(canClean(only, left), true);
  assert.equal(summaryText(left), '2 chapter titles will be fixed.');
  assert.equal(removingText(left), 'Fixing 2 chapter titles');
  const none = toggleRule(countedWithTitles(), 'titles');
  assert.deepEqual(remaining(none, emptyChoices()), { paragraphs: 2, titles: 0, chapters: 2 });
  assert.equal(previewFooter(none, 2), '2 paragraphs.');
});

test('past the row cap titles and paragraphs are counted apart', () => {
  const state = countedWithTitles({ matches: 9000, rules: [{ rule: 'watermark', matches: 5000, chapters: 5000 }, { rule: 'notes', matches: 0, chapters: 0 }, { rule: 'heading', matches: 0, chapters: 0 }, { rule: 'titles', matches: 4000, chapters: 4000 }] });
  const choices = emptyChoices();
  toggleHit(choices, state.results.rows[0]);
  assert.deepEqual(remaining(state, choices), { paragraphs: 5000, titles: 3999, chapters: 5000 });
});

test('the result names the titles it fixed', () => {
  assert.equal(doneText({ changes: 62, titles: 39, chapters: 45 }), 'Removed 62 paragraphs and fixed 39 chapter titles, in 45 chapters.');
  assert.equal(doneText({ changes: 0, titles: 1, chapters: 1 }), 'Fixed 1 chapter title.');
  assert.equal(doneText({ changes: 3, titles: 0, chapters: 2 }), 'Removed 3 paragraphs from 2 chapters.');
});
