import test from 'node:test';
import assert from 'node:assert/strict';
import { applyBatch, emptyResults } from '../../app/ui/search-model.js';
import {
  canApply, canChange, emptyChoices, exclusionsOf, noteOf, offerDialog, offerText, previewFooter, progressText, remaining, summaryText, toggleChapter, toggleHit,
} from '../../app/ui/replace-model.js';

const hit = (extra = {}) => ({ range: { start: 0, end: 3 }, before: '', matched: 'the', after: '', replaced: 'new', spans_tags: false, crosses_block: false, same: false, ...extra });
const group = (ix, hits) => ({ ix, rev: { crc32: ix, inflated_len: 1 }, number: ix + 1, title: null, hits });
const preview = (extra = {}) => ({
  matches: 5, changes: 4, chapters: 2, skipped: 1, spanning: 0, same: 0, limited: 0, unreadable: 0,
  touched: [{ chapter: 0, matches: 3, changes: 3 }, { chapter: 1, matches: 2, changes: 1 }],
  ...extra,
});

function loaded() {
  const results = emptyResults('running');
  applyBatch(results, { groups: [group(0, [hit(), hit(), hit()]), group(1, [hit({ crosses_block: true, spans_tags: true }), hit()])], done: 2, total: 2, limited: 0, unreadable: 0, capped: false, preview: preview(), end: { kind: 'done' } });
  return results;
}

test('rows know their place in their chapter, across a split batch', () => {
  const results = emptyResults('running');
  const base = { done: 0, total: 1, limited: 0, unreadable: 0, capped: false, end: null };
  applyBatch(results, { ...base, groups: [group(0, [hit(), hit()])] });
  applyBatch(results, { ...base, groups: [group(0, [hit()])] });
  assert.deepEqual(results.rows.map((row) => row.ordinal), [0, 1, 2]);
});

test('unticking and skipping lower the counts and name exactly those exclusions', () => {
  const results = loaded();
  const choices = emptyChoices();
  assert.deepEqual(remaining(results.preview, results, choices), { changes: 4, chapters: 2 });
  toggleHit(choices, results.rows[1]);
  assert.deepEqual(remaining(results.preview, results, choices), { changes: 3, chapters: 2 });
  toggleChapter(choices, results.rows[4]);
  assert.deepEqual(remaining(results.preview, results, choices), { changes: 2, chapters: 1 });
  assert.deepEqual(exclusionsOf(choices), [{ kind: 'hit', chapter: 0, ordinal: 1 }, { kind: 'chapter', chapter: 1 }]);
  toggleHit(choices, results.rows[1]);
  toggleChapter(choices, results.rows[4]);
  assert.deepEqual(exclusionsOf(choices), []);
});

test('a hit unticked inside a skipped chapter is not counted twice', () => {
  const results = loaded();
  const choices = emptyChoices();
  toggleHit(choices, results.rows[0]);
  toggleChapter(choices, results.rows[0]);
  assert.deepEqual(remaining(results.preview, results, choices), { changes: 1, chapters: 1 });
});

test('only rows that would change can be unticked', () => {
  assert.equal(canChange(hit()), true);
  assert.equal(canChange(hit({ crosses_block: true })), false);
  assert.equal(canChange(hit({ same: true })), false);
});

test('the summary names the count, the skipped, the atomic save and how to undo', () => {
  const text = summaryText(preview({ spanning: 2, unreadable: 1 }), { changes: 4, chapters: 2 });
  assert.match(text, /^Preview: 4 changes in 2 chapters\./);
  assert.match(text, /1 skipped because it crosses a paragraph\./);
  assert.match(text, /2 cross formatting\./);
  assert.match(text, /1 chapter could not be read/);
  assert.match(text, /Applied as one atomic save\./);
  assert.match(text, /Revert to backup/);
  assert.match(summaryText(preview({ limited: 3 }), { changes: 4, chapters: 2 }), /too slow.*3 chapters.*cannot be applied/);
});

test('each kind of row gets its own note', () => {
  assert.equal(noteOf(hit()), '');
  assert.match(noteOf(hit({ spans_tags: true })), /formatting/);
  assert.match(noteOf(hit({ spans_tags: true, crosses_block: true })), /paragraph/);
  assert.match(noteOf(hit({ same: true })), /Already/);
});

test('apply needs a finished, unlimited preview with something left to change', () => {
  const left = { changes: 1, chapters: 1 };
  assert.equal(canApply(preview(), left, false), true);
  assert.equal(canApply(preview(), left, true), false);
  assert.equal(canApply(null, left, false), false);
  assert.equal(canApply(preview({ limited: 1 }), left, false), false);
  assert.equal(canApply(preview(), { changes: 0, chapters: 0 }, false), false);
});

test('copy reads right for one and for many', () => {
  assert.equal(previewFooter(preview({ matches: 9000 }), 5000), 'Showing 5,000 of 9,000 matches. Only shown rows can be unticked.');
  assert.equal(previewFooter(preview({ matches: 1 }), 1), '1 match.');
  const offer = { from: 'a"b', to: '<c>' };
  assert.equal(offerText(null), 'Fixed here. Looking for other copies.');
  assert.equal(offerText(preview({ changes: 1 })), 'Fixed here. 1 more copy in this book.');
  assert.equal(offerDialog(offer, preview({ changes: 37, chapters: 31 })).title, 'Fix 37 more copies?');
  assert.match(offerDialog(offer, preview({ changes: 37, chapters: 31 })).text, /"a"b" becomes "<c>" in 31 chapters/);
  assert.equal(progressText({ done: 0, total: 0 }), 'Backing up your original.');
  assert.equal(progressText({ done: 2, total: 5 }), '2 of 5 chapters changed.');
  assert.equal(progressText({ done: 5, total: 5 }), 'Finishing the save.');
});
