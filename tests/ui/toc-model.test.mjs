import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ROW_HEIGHT, buildToc, countText, entryOfChapter, queryOf, rowLabel, rowOfChapter, scrollToKeep, setHeading, visibleRows, windowOf,
} from '../../app/ui/toc-model.js';
import { footerHint, paletteRows } from '../../app/ui/palette-model.js';

const entry = (label, ix, depth = 0, fragment = null) => ({
  label, ix, depth, fragment, number: ix === null ? null : ix,
});

const nested = () => buildToc({
  source: 'nav',
  linear: [1, 2, 3, 4, 5, 6],
  entries: [
    entry('Part One', null, 0),
    entry('Chapter One', 1, 1),
    entry('Chapter Two', 2, 1),
    entry('Scene', 2, 2, 's2'),
    entry('Part Two', null, 0),
    entry('Book Three', null, 1),
    entry('Chapter Three', 3, 2),
    entry('Chapter Four', 4, 2),
    entry('Chapter Five', 5, 1),
    entry('Chapter Six', 6, 0),
  ],
});

test('parents know they are parents and show the chapter numbers their titles carry, leaves show none', () => {
  const toc = nested();
  assert.equal(toc.entries[0].isParent, true);
  assert.equal(toc.entries[1].isParent, false);
  assert.equal(rowLabel(toc.entries[0]), '');
  assert.equal(rowLabel(toc.entries[9]), '');
  const numbered = buildToc({
    source: 'nav',
    linear: [1, 2, 3, 4],
    entries: [entry('Volume 1', null, 0), entry('Chapter 1: A', 1, 1), entry('Synopsis', 2, 1), entry('Chapter 1,203: B', 3, 1), entry('Volume 2', null, 0), entry('Ch. 1204', 4, 1)],
  });
  assert.equal(rowLabel(numbered.entries[0]), '1-1,203');
  assert.equal(rowLabel(numbered.entries[4]), '1,204');
  assert.equal(rowLabel(numbered.entries[1]), '');
  assert.equal(toc.entries[3].parent, 2);
  assert.equal(toc.entries[6].parent, 5);
});

test('folding hides only the descendants', () => {
  const toc = nested();
  assert.deepEqual(visibleRows(toc, new Set(), ''), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
  const keys = (...rows) => new Set(rows.map((at) => toc.entries[at].key));
  assert.deepEqual(visibleRows(toc, keys(4), ''), [0, 1, 2, 3, 4, 9]);
  assert.deepEqual(visibleRows(toc, keys(5), ''), [0, 1, 2, 3, 4, 5, 8, 9]);
  assert.deepEqual(visibleRows(toc, keys(0, 4), ''), [0, 4, 9]);
  assert.deepEqual(visibleRows(toc, keys(1), ''), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
});

test('a fold follows its group when the rows around it change', () => {
  const row = (label, ix, href, depth) => ({ label, ix, href, depth, fragment: null, number: ix });
  const before = buildToc({
    source: 'nav',
    linear: [0, 1, 2],
    entries: [row('Volume', 0, 'v.xhtml', 0), row('One', 1, 'a.xhtml', 1), row('Two', 2, 'b.xhtml', 0)],
  });
  const folded = new Set([before.entries[0].key]);
  assert.deepEqual(visibleRows(before, folded, ''), [0, 2]);
  const after = buildToc({
    source: 'nav',
    linear: [0, 1, 2],
    entries: [row('Two', 0, 'b.xhtml', 0), row('Volume renamed', 1, 'v.xhtml', 0), row('One', 2, 'a.xhtml', 1)],
  });
  assert.deepEqual(visibleRows(after, folded, ''), [0, 1]);
});

test('a filter ignores folding, case and commas in numbers', () => {
  const toc = nested();
  assert.deepEqual(visibleRows(toc, new Set([toc.entries[4].key]), queryOf('  CHAPTER four ')), [7]);
  assert.deepEqual(visibleRows(toc, new Set(), queryOf('nothing like this')), []);
  const big = buildToc({ source: 'nav', linear: [], entries: [entry('Chapter 1234: x', 1), entry('Chapter 9: y', 1234)] });
  assert.deepEqual(visibleRows(big, new Set(), queryOf('1,234')), [0]);
  assert.deepEqual(visibleRows(big, new Set(), queryOf('234')), [0]);
  assert.equal(countText(toc, 1, 'x'), '1 of 10 entries');
  assert.equal(countText(toc, 10, ''), '10 entries');
});

test('the current row is the plain entry of the chapter, or its nearest shown ancestor', () => {
  const toc = nested();
  assert.equal(entryOfChapter(toc, 2), 2, 'the fragment entry does not own the chapter');
  const all = visibleRows(toc, new Set(), '');
  assert.equal(rowOfChapter(toc, all, 3), 6);
  const folded = visibleRows(toc, new Set([toc.entries[5].key]), '');
  assert.equal(folded[rowOfChapter(toc, folded, 3)], 5);
  const filtered = visibleRows(toc, new Set(), 'six');
  assert.equal(rowOfChapter(toc, filtered, 3), -1);
});

test('a chapter missing from the contents belongs to the entry before it', () => {
  const toc = buildToc({ source: 'nav', linear: [0, 1, 2, 3, 4], entries: [entry('A', 1), entry('B', 3)] });
  assert.equal(entryOfChapter(toc, 0), null);
  assert.equal(entryOfChapter(toc, 2), 0);
  assert.equal(entryOfChapter(toc, 4), 1);
});

test('a parent with a link gives way to a child at the same chapter', () => {
  const toc = buildToc({ source: 'nav', linear: [0], entries: [entry('Volume', 0, 0), entry('First', 0, 1)] });
  assert.equal(entryOfChapter(toc, 0), 1);
});

test('the window covers the view with overscan and never passes the rows', () => {
  const { first, end } = windowOf(ROW_HEIGHT * 1000, ROW_HEIGHT * 20, 6189);
  assert.ok(first <= 1000 && end >= 1020);
  assert.ok(end - first < 50, 'only a small slice is rendered');
  assert.deepEqual(windowOf(0, 600, 3), { first: 0, end: 3 });
  assert.equal(windowOf(ROW_HEIGHT * 6188, 600, 6189).end, 6189);
});

test('a row in view does not move the list and a far row lands a third of the way down', () => {
  assert.equal(scrollToKeep(10, 0, 600), 0);
  assert.equal(scrollToKeep(1000, 0, 600), 1000 * ROW_HEIGHT - 200);
  assert.equal(scrollToKeep(0, 5000, 600), 0);
});

test('a heading replaces a placeholder and the filter sees it', () => {
  const toc = buildToc({ source: 'headings', linear: [0, 1], entries: [entry('Chapter 1', 0), entry('Chapter 2', 1)] });
  assert.equal(setHeading(toc, 1, 'The Long Road'), true);
  assert.equal(setHeading(toc, 9, 'nothing'), false);
  assert.deepEqual(visibleRows(toc, new Set(), queryOf('long road')), [1]);
});

const COMMANDS = [{ label: 'Switch theme', run() {} }, { label: 'Focus mode', run() {} }];

test('a number with no title carrying it falls back to the reading order position, not the spine index', () => {
  const toc = buildToc({ source: 'nav', linear: [4, 5, 7], entries: [entry('Skipped', 7)] });
  const [row] = paletteRows('3', toc, COMMANDS);
  assert.equal(row.ix, 7);
  assert.equal(row.hint, 'Position 3');
  assert.equal(row.label, 'Skipped');
  assert.equal(paletteRows('chapter 2', toc, COMMANDS)[0].ix, 5);
  assert.equal(paletteRows('2', toc, COMMANDS)[0].label, 'Go to position 2');
  assert.deepEqual(paletteRows('999', toc, COMMANDS), []);
  assert.deepEqual(paletteRows('0', toc, COMMANDS), []);
});

test('a number offers only the chapter whose title carries it, with the position as the hint', () => {
  const titles = ['Synopsis', 'Volume 1', ...Array.from({ length: 200 }, (_, at) => `Chapter ${at + 1}: Title ${at + 1}`)];
  const entries = titles.map((label, at) => entry(label, label.startsWith('Volume') ? null : at));
  const linear = entries.filter((row) => row.ix !== null).map((row) => row.ix);
  entries.forEach((row) => { row.number = row.ix === null ? null : linear.indexOf(row.ix) + 1; });
  const toc = buildToc({ source: 'nav', linear, entries });
  const rows = paletteRows('184', toc, COMMANDS);
  assert.equal(rows[0].label, 'Chapter 184: Title 184');
  assert.equal(rows[0].hint, 'Position 185');
  assert.deepEqual(rows.map((row) => row.label), ['Chapter 184: Title 184']);
  assert.equal(paletteRows('Ch. 184', toc, COMMANDS)[0].label, 'Chapter 184: Title 184');
  assert.equal(paletteRows('chapter 200', toc, COMMANDS)[0].label, 'Chapter 200: Title 200');
  assert.equal(paletteRows('201', toc, COMMANDS)[0].label, 'Chapter 200: Title 200');
  assert.equal(paletteRows('201', toc, COMMANDS)[0].hint, 'Position 201');
});

test('a number offers the chapter whose title starts with it, not the chapter at that position', () => {
  const titles = ['Cover', ...Array.from({ length: 800 }, (_, at) => `${at + 1} Plan ${at + 1}`)];
  const entries = titles.map((label, at) => entry(label, at));
  entries.forEach((row) => { row.number = row.ix + 1; });
  const toc = buildToc({ source: 'nav', linear: entries.map((row) => row.ix), entries });
  const rows = paletteRows('710', toc, COMMANDS);
  assert.equal(rows[0].label, '710 Plan 710');
  assert.equal(rows[0].hint, 'Position 711');
  assert.ok(!rows.some((row) => row.label.startsWith('709 ')), rows.map((row) => row.label).join(' | '));
  assert.equal(paletteRows('1', toc, COMMANDS)[0].label, '1 Plan 1');
});

test('numbers with commas are understood', () => {
  const linear = Array.from({ length: 2000 }, (_, at) => at);
  const toc = buildToc({ source: 'nav', linear, entries: [] });
  assert.equal(paletteRows('1,500', toc, COMMANDS)[0].ix, 1499);
});

test('commands come before titles, so a title cannot bury them', () => {
  const entries = Array.from({ length: 20 }, (_, at) => entry(`The theme ${at}`, at));
  const toc = buildToc({ source: 'nav', linear: entries.map((_, at) => at), entries });
  const rows = paletteRows('theme', toc, COMMANDS);
  assert.equal(rows[0].kind, 'command');
  assert.equal(rows.length, 7);
  assert.equal(rows[1].kind, 'chapter');
});

test('an empty query offers commands, a parent with no link is never offered, and commands work with no contents yet', () => {
  const toc = nested();
  assert.deepEqual(paletteRows('', toc, COMMANDS).map((row) => row.label), ['Switch theme', 'Focus mode']);
  assert.deepEqual(paletteRows('part', toc, COMMANDS), []);
  assert.equal(paletteRows('focus', null, COMMANDS)[0].label, 'Focus mode');
  assert.deepEqual(paletteRows('12', null, COMMANDS), []);
  assert.equal(footerHint(6000), 'Try 2,000, a word from a title or theme');
});

test('a saved search shows in the palette under its own hint and a typed word finds it', () => {
  const commands = [...COMMANDS, { label: 'Search this book', run() {} }, { label: 'Search: Romance <b>words</b>', hint: 'Saved search', run() {} }];
  const rows = paletteRows('search', null, commands);
  assert.deepEqual(rows.map((row) => [row.label, row.hint]), [['Search this book', 'Command'], ['Search: Romance <b>words</b>', 'Saved search']]);
  assert.deepEqual(paletteRows('romance', null, commands).map((row) => row.label), ['Search: Romance <b>words</b>']);
});
