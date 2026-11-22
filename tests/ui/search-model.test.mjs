import test from 'node:test';
import assert from 'node:assert/strict';
import {
  MAX_RESULTS, NO_PLACE, ROW_HEIGHT, WHOLE_BOOK, WHOLE_RANGE, applyBatch, applyLabel, barText, emptyResults, failResults, filterTo, footerText, inRange, indexFrom, rangeOf, rangeStrip, reanchored, toggled,
  rerunFor, sameRev, snippetBefore, statusOf, stepIndex, whereOf, windowOf, wireOf,
} from '../../app/ui/search-model.js';

const hit = (start) => ({ range: { start, end: start + 3 }, before: 'b', matched: 'the', after: 'a', replaced: null, spans_tags: false });
const group = (ix, count, base = 0) => ({
  ix, rev: { crc32: ix, inflated_len: 10 }, number: ix + 1, title: null,
  hits: Array.from({ length: count }, (_, at) => hit(base + at * 10)),
});
const batch = (groups, extra = {}) => ({ groups, done: 1, total: 4, limited: 0, unreadable: 0, capped: false, end: null, ...extra });

test('rows keep arrival order and a chapter split over batches stays one list', () => {
  const results = emptyResults('running');
  applyBatch(results, batch([group(3, 2)]));
  applyBatch(results, batch([group(3, 1, 100), group(7, 1)]));
  assert.deepEqual(results.rows.map((row) => row.chapter.ix), [3, 3, 3, 7]);
  assert.equal(results.byChapter.get(3).length, 3);
  assert.equal(results.byChapter.get(3)[2].hit.range.start, 100);
});

test('each ending maps to its phase and message', () => {
  const end = (kind, message) => applyBatch(emptyResults('running'), batch([], { end: { kind, message } }));
  assert.equal(end('done').phase, 'done');
  assert.equal(end('tooSlow').phase, 'done');
  assert.equal(end('cancelled').phase, 'stopped');
  assert.equal(end('badPattern', 'nope').phase, 'bad');
  assert.equal(end('badPattern', 'nope').message, 'nope');
  assert.equal(end('failed', 'gone').phase, 'bad');
});

test('the footer says what the search found in plain words', () => {
  const results = emptyResults('running');
  assert.equal(footerText(emptyResults()), '');
  applyBatch(results, batch([group(0, 1)]));
  assert.equal(footerText(results), `1 match so far. Results stop at ${MAX_RESULTS.toLocaleString('en-US')}.`);
  applyBatch(results, batch([group(1, 1233)], { end: { kind: 'done' } }));
  assert.equal(footerText(results), '1,234 matches.');
  assert.equal(footerText(results, 3), 'Match 4 of 1,234.');
  assert.equal(footerText(applyBatch(emptyResults('running'), batch([], { end: { kind: 'done' } }))), 'No matches.');
  const capped = applyBatch(emptyResults('running'), batch([group(0, 2)], { capped: true, end: { kind: 'done' } }));
  assert.equal(footerText(capped), '2 matches shown. Results stop at 100,000.');
  const slow = applyBatch(emptyResults('running'), batch([group(0, 1)], { limited: 2, unreadable: 1, end: { kind: 'tooSlow', limited: 2 } }));
  assert.match(footerText(slow), /too slow, so 2 chapters were not fully searched/);
  assert.match(footerText(slow), /1 chapter could not be read/);
});

test('the find bar counts matches in a few words', () => {
  assert.equal(barText(emptyResults()), '');
  const results = emptyResults('running');
  assert.equal(barText(results), '');
  applyBatch(results, batch([group(0, 1)]));
  assert.equal(barText(results), '1 match');
  applyBatch(results, batch([group(1, 1233)], { end: { kind: 'done' } }));
  assert.equal(barText(results), '1,234 matches');
  assert.equal(barText(results, 0), '1 of 1,234');
  assert.equal(barText(applyBatch(emptyResults('running'), batch([], { end: { kind: 'done' } }))), 'No matches');
});

test('the status row shows the scan and the right action', () => {
  const running = applyBatch(emptyResults('running'), batch([], { done: 4210, total: 6193 }));
  assert.deepEqual(statusOf(running), { visible: true, scanned: '4,210 of 6,193 chapters searched', fill: '68%', action: 'Stop' });
  assert.equal(statusOf(applyBatch(emptyResults('running'), batch([], { done: 1, total: 1 }))).scanned, '1 of 1 chapter searched');
  const done = applyBatch(emptyResults('running'), batch([], { done: 6193, total: 6193, end: { kind: 'done' } }));
  assert.equal(statusOf(done).action, 'Rerun');
  assert.doesNotMatch(statusOf(done).scanned, /match/);
  assert.doesNotMatch(footerText(done), /chapter/);
  assert.equal(statusOf(emptyResults()).visible, false);
});

test('where names the chapter by its reading number', () => {
  assert.equal(whereOf({ number: 1968, title: 'Ferry' }), 'Ferry');
  assert.equal(whereOf({ number: 3, title: null }), 'Position 3');
  assert.equal(whereOf({ number: 1839, title: 'Chapter 1818: The Gate' }), 'Chapter 1818: The Gate');
  assert.equal(whereOf({ number: null, title: null }), 'Outside the reading order');
});

test('the virtual window covers the view at any scroll and never leaves the list', () => {
  const rows = 100000;
  for (const top of [0, 1, ROW_HEIGHT * 50 + 7, ROW_HEIGHT * (rows - 5)]) {
    const { first, end } = windowOf(top, 600, rows);
    assert.ok(first * ROW_HEIGHT <= top);
    assert.ok(Math.min(rows, Math.ceil((top + 600) / ROW_HEIGHT)) <= end);
    assert.ok(end <= rows);
    assert.ok(end - first < 20);
  }
  assert.deepEqual(windowOf(0, 600, 0), { first: 0, end: 0 });
  assert.deepEqual(windowOf(9e9, 600, 3), { first: 107142853, end: 107142853 });
});

test('snippets cut on characters, never inside a surrogate pair', () => {
  const emoji = '😀'.repeat(40);
  assert.equal([...snippetBefore(emoji)].length, 36);
  assert.ok(!snippetBefore(emoji).startsWith('\ude00'));
});

test('sameRev compares both fields', () => {
  assert.ok(sameRev({ crc32: 1, inflated_len: 2 }, { crc32: 1, inflated_len: 2 }));
  assert.ok(!sameRev({ crc32: 1, inflated_len: 2 }, { crc32: 1, inflated_len: 3 }));
});

test('a failed command shows its message and keeps the counts', () => {
  const results = applyBatch(emptyResults('running'), batch([], { done: 3, total: 9 }));
  failResults(results, new Error('The book is gone.'));
  assert.equal(results.phase, 'bad');
  assert.equal(results.message, 'The book is gone.');
  assert.equal(results.done, 3);
});

test('stepping walks the matches and wraps at both ends', () => {
  assert.equal(stepIndex(null, 1, 0), null);
  assert.equal(stepIndex(null, -1, 0), null);
  assert.equal(stepIndex(null, 1, 5), 0);
  assert.equal(stepIndex(null, -1, 5), 4);
  assert.equal(stepIndex(0, 1, 5), 1);
  assert.equal(stepIndex(4, 1, 5), 0);
  assert.equal(stepIndex(0, -1, 5), 4);
  assert.equal(stepIndex(3, -1, 5), 2);
});

test('stepping from the reading place finds the first match at or after it and wraps once the search is done', () => {
  const results = applyBatch(emptyResults('running'), batch([group(1, 2), group(4, 3), group(9, 1)]));
  const rows = results.rows;
  const at = (index) => index === null ? null : [rows[index].chapter.ix, rows[index].ordinal];
  assert.deepEqual(at(indexFrom(rows, { ix: 4, passed: 0 }, 1, false)), [4, 0]);
  assert.deepEqual(at(indexFrom(rows, { ix: 4, passed: 2 }, 1, false)), [4, 2]);
  assert.deepEqual(at(indexFrom(rows, { ix: 4, passed: 3 }, 1, false)), [9, 0]);
  assert.deepEqual(at(indexFrom(rows, { ix: 5, passed: 0 }, 1, false)), [9, 0]);
  assert.deepEqual(at(indexFrom(rows, { ix: 0, passed: 0 }, 1, false)), [1, 0]);
  assert.equal(indexFrom(rows, { ix: 12, passed: 0 }, 1, false), null);
  assert.deepEqual(at(indexFrom(rows, { ix: 12, passed: 0 }, 1, true)), [1, 0]);
  assert.deepEqual(at(indexFrom(rows, { ix: 4, passed: 2 }, -1, false)), [4, 1]);
  assert.deepEqual(at(indexFrom(rows, { ix: 4, passed: 0 }, -1, false)), [1, 1]);
  assert.deepEqual(at(indexFrom(rows, { ix: 12, passed: 0 }, -1, false)), [9, 0]);
  assert.equal(indexFrom(rows, { ix: 1, passed: 0 }, -1, false), null);
  assert.deepEqual(at(indexFrom(rows, { ix: 1, passed: 0 }, -1, true)), [9, 0]);
  assert.equal(indexFrom([], { ix: 1, passed: 0 }, 1, true), null);
});

test('the footer names the current match and its total', () => {
  const results = applyBatch(emptyResults('running'), batch([group(1, 1233), group(2, 1)], { end: { kind: 'done' } }));
  assert.equal(footerText(results, null), '1,234 matches.');
  assert.equal(footerText(results, 0), 'Match 1 of 1,234.');
  assert.equal(footerText(results, 1233), 'Match 1,234 of 1,234.');
  const running = applyBatch(emptyResults('running'), batch([group(1, 3)]));
  assert.equal(footerText(running, 1), 'Match 2 of 3 so far.');
  const slow = applyBatch(emptyResults('running'), batch([group(1, 2)], { limited: 1, end: { kind: 'tooSlow' } }));
  assert.match(footerText(slow, 0), /^Match 1 of 2\. This pattern is too slow/);
});

const place = (ix, title, number = ix + 1) => ({ ix, title, number });
const at12 = place(11, 'Chapter 12');
const ranges = {
  whole: WHOLE_RANGE,
  upTo: rangeOf(true, false, at12),
  from: rangeOf(false, true, at12),
  only: rangeOf(false, false, at12),
};
const chaptersOf = (rows) => [...new Set(rows.map((row) => row.chapter.ix))];
const book = () => batch([group(3, 2), group(11, 3), group(12, 1), group(20, 4)]);

test('a place is named by the chapter number in its title, then its title, then its position', () => {
  assert.equal(applyLabel(rangeOf(false, true, place(40, 'Chapter 1818: The Gate'))), 'Apply to ch. 1818 and after');
  assert.equal(applyLabel(rangeOf(true, false, place(3, 'Prologue'))), 'Apply to Prologue and before');
  assert.equal(applyLabel(rangeOf(false, true, place(3, 'A Very Long Chapter Title Indeed'))), 'Apply to A Very Long Chapt… and after');
  assert.equal(applyLabel(rangeOf(false, true, place(1838, '', 1839))), 'Apply to position 1,839 and after');
  assert.equal(applyLabel(rangeOf(true, false, place(0, '', null))), 'Apply to here and before');
});

test('both boxes ticked is the whole book whatever the place', () => {
  assert.equal(rangeOf(true, true, at12), WHOLE_RANGE);
  assert.equal(rangeOf(true, true, null), WHOLE_RANGE);
  assert.deepEqual(wireOf(WHOLE_RANGE), WHOLE_BOOK);
});

test('each range shows exactly its chapters, the anchor chapter always included', () => {
  const want = { whole: [3, 11, 12, 20], upTo: [3, 11], from: [11, 12, 20], only: [11] };
  for (const [kind, range] of Object.entries(ranges)) {
    const all = applyBatch(emptyResults('running'), book());
    assert.deepEqual(chaptersOf(filterTo(all, range).shown), want[kind], kind);
    assert.equal(all.rows.length, 10, `${kind} keeps every searched row`);
    const streamed = applyBatch(emptyResults('running', WHOLE_BOOK, range), book());
    assert.deepEqual(chaptersOf(streamed.shown), want[kind], `${kind} filters rows as they arrive`);
    assert.deepEqual([3, 11, 12, 20].filter((ix) => inRange(range, ix)), want[kind], kind);
  }
});

test('ticking a box back on brings the hidden rows back in reading order', () => {
  const results = filterTo(applyBatch(emptyResults('running'), book()), ranges.only);
  assert.equal(results.shown.length, 3);
  filterTo(results, WHOLE_RANGE);
  assert.deepEqual(results.shown, results.rows);
  assert.deepEqual(results.shown.map((row) => row.chapter.ix), [3, 3, 11, 11, 11, 12, 20, 20, 20, 20]);
});

test('counts and stepping go by the rows on show', () => {
  const results = filterTo(applyBatch(emptyResults('running'), book()), ranges.from);
  applyBatch(results, batch([], { end: { kind: 'done' } }));
  assert.equal(footerText(results), '8 matches.');
  assert.equal(barText(results, 0), '1 of 8');
  const first = indexFrom(results.shown, { ix: 3, passed: 0 }, 1, true);
  assert.equal(results.shown[first].chapter.ix, 11);
});

test('the wire form carries kind and here only', () => {
  assert.deepEqual(wireOf(ranges.from), { kind: 'from', here: 11 });
  assert.deepEqual(wireOf(ranges.upTo), { kind: 'upTo', here: 11 });
  assert.deepEqual(wireOf(ranges.only), { kind: 'only', here: 11 });
  assert.deepEqual(wireOf(rangeOf(false, true, place(0, 'Chapter 1'))), { kind: 'from', here: 0 });
});

const ended = (kind, extra = {}, scope = WHOLE_BOOK) => applyBatch(emptyResults('running', scope), batch([group(3, 1)], { end: { kind }, ...extra }));

test('a whole-book search running or done answers any range without a new run', () => {
  const running = applyBatch(emptyResults('running'), batch([group(3, 1)]));
  for (const range of Object.values(ranges)) {
    assert.equal(rerunFor(running, range, false), null);
    assert.equal(rerunFor(ended('done'), range, false), null);
    assert.equal(rerunFor(ended('tooSlow'), range, false), null);
  }
});

test('a stopped search runs the whole book again', () => {
  for (const range of Object.values(ranges)) assert.deepEqual(rerunFor(ended('cancelled'), range, false), WHOLE_BOOK);
});

test('a capped search runs the narrower range on its own, since the whole book would stop at the same cap', () => {
  const capped = ended('done', { capped: true });
  assert.equal(rerunFor(capped, WHOLE_RANGE, false), null);
  assert.deepEqual(rerunFor(capped, ranges.from, false), { kind: 'from', here: 11 });
  assert.deepEqual(rerunFor(capped, ranges.only, false), { kind: 'only', here: 11 });
  const cappedFrom = ended('done', { capped: true }, wireOf(ranges.from));
  assert.equal(rerunFor(cappedFrom, ranges.from, false), null);
  assert.deepEqual(rerunFor(cappedFrom, WHOLE_RANGE, false), WHOLE_BOOK);
  assert.deepEqual(rerunFor(cappedFrom, ranges.only, false), { kind: 'only', here: 11 });
});

test('a search over part of the book filters what it covers and reruns the whole book for the rest', () => {
  const from = ended('done', {}, wireOf(ranges.from));
  assert.equal(rerunFor(from, ranges.only, false), null);
  assert.equal(rerunFor(from, rangeOf(false, true, place(15, 'Chapter 16')), false), null);
  assert.deepEqual(rerunFor(from, ranges.upTo, false), WHOLE_BOOK);
  assert.deepEqual(rerunFor(from, WHOLE_RANGE, false), WHOLE_BOOK);
});

test('with nothing to filter, a range change runs nothing', () => {
  assert.equal(rerunFor(emptyResults(), ranges.from, false), null);
  assert.equal(rerunFor(ended('badPattern'), ranges.from, false), null);
});

test('a replace preview always runs again on the range itself', () => {
  for (const range of Object.values(ranges)) {
    assert.deepEqual(rerunFor(ended('done'), range, true), wireOf(range));
    assert.deepEqual(rerunFor(emptyResults(), range, true), wireOf(range));
  }
});

test('switching a side off anchors to the place being read, and the anchor then stays', () => {
  const at30 = place(29, 'Chapter 30');
  const from = toggled(WHOLE_RANGE, 'before', at12);
  assert.deepEqual([from.before, from.after, from.here], [false, true, 11]);
  const only = toggled(from, 'after', at30);
  assert.deepEqual([only.before, only.after, only.here], [false, false, 11], 'the shown anchor is kept, not the reader');
  const upTo = toggled(only, 'before', at30);
  assert.deepEqual([upTo.before, upTo.after, upTo.here], [true, false, 11]);
  assert.equal(toggled(upTo, 'after', at30), WHOLE_RANGE);
});

test('the middle chip moves the anchor to the place being read', () => {
  const at30 = place(29, 'Chapter 30');
  const moved = reanchored(ranges.from, at30);
  assert.deepEqual([moved.before, moved.after, moved.here], [false, true, 29]);
  assert.equal(applyLabel(moved), 'Apply to ch. 30 and after');
  assert.equal(reanchored(WHOLE_RANGE, at30), WHOLE_RANGE);
  assert.equal(reanchored(ranges.from, null), ranges.from);
});

test('the strip shows each side, the anchor, and offers to move it when the reader has moved', () => {
  const whole = rangeStrip(WHOLE_RANGE, at12);
  assert.deepEqual([whole.before.on, whole.after.on], [true, true]);
  assert.equal(whole.before.hint, 'Hide matches before Chapter 12');
  assert.equal(whole.here.label, 'Ch. 12');
  assert.equal(whole.here.disabled, true);
  assert.equal(whole.placeholder, 'Search the whole book');
  const away = rangeStrip(ranges.from, place(29, 'Chapter 30'));
  assert.deepEqual([away.before.on, away.after.on], [false, true]);
  assert.equal(away.before.hint, 'Show matches before Chapter 12');
  assert.equal(away.after.hint, 'Hide matches after Chapter 12');
  assert.equal(away.here.label, 'Ch. 12');
  assert.equal(away.here.disabled, false);
  assert.equal(away.here.hint, 'Count from Chapter 30 instead');
  assert.equal(away.placeholder, 'Search ch. 12 and after');
  assert.equal(rangeStrip(ranges.from, at12).here.disabled, true);
});

test('with no place the strip is disabled and says why', () => {
  const shown = rangeStrip(WHOLE_RANGE, null);
  for (const part of [shown.before, shown.after, shown.here]) {
    assert.equal(part.disabled, true);
    assert.equal(part.hint, NO_PLACE);
  }
  assert.equal(shown.placeholder, 'Search the whole book');
});

test('placeholder and apply label name each range, and the status counts the whole search', () => {
  const copy = {
    whole: ['Search the whole book', 'Apply to whole book'],
    upTo: ['Search ch. 12 and before', 'Apply to ch. 12 and before'],
    from: ['Search ch. 12 and after', 'Apply to ch. 12 and after'],
    only: ['Search ch. 12 only', 'Apply to ch. 12 only'],
  };
  for (const [kind, [placeholder, apply]] of Object.entries(copy)) {
    const range = ranges[kind];
    const results = applyBatch(emptyResults('running', WHOLE_BOOK, range), batch([], { done: 1204, total: 2008 }));
    assert.equal(rangeStrip(range, at12).placeholder, placeholder, kind);
    assert.equal(statusOf(results).scanned, '1,204 of 2,008 chapters searched', kind);
    assert.equal(applyLabel(range), apply, kind);
  }
});
