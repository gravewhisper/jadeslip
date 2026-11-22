import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  SORTS, addedText, cardOf, chapterTag, lookOf, menuOf, movingText, openedText, recentOf, removalText, shelfView, sizeText, sortOf, stateOf, statusText, stepIndex, tabCounts,
} from '../../app/ui/shelf-model.js';

const book = (patch = {}) => ({
  key: '0123456789abcdef0123456789abcdef',
  title: 'A Long Road',
  author: 'Yan Bi Xiao Sheng',
  chapters: 6193,
  bytes: 45_600_000,
  own: false,
  read: null,
  write: null,
  touchedAt: 1,
  backedUp: false,
  cover: null,
  words: null,
  ...patch,
});
const point = (patch = {}) => ({ label: 'Chapter 1818: The Gate', ordinal: 1973, progress: 0.3186, passage: 'A passage.', ...patch });

const shelf = [
  book({ key: 'a', title: 'Monsoon', read: point(), touchedAt: 50 }),
  book({ key: 'b', title: 'amber Tide', author: 'A. Writer', read: point({ progress: 0.55 }), touchedAt: 40, chapters: 4352, bytes: 31_000_000 }),
  book({ key: 'c', title: 'Bells', author: null, touchedAt: 10, chapters: 212, bytes: 1_800_000 }),
  book({ key: 'd', title: 'Quiet Hills', read: point({ progress: 1 }), touchedAt: 20, chapters: 2334 }),
  book({ key: 'e', title: 'The Ninth Lantern', author: 'M. Reyes', own: true, write: point({ label: 'Chapter 38: The Keeper Wakes' }), words: 92_410, touchedAt: 45, chapters: 38, bytes: 300_000 }),
];

test('each book has one state, and the tabs count them', () => {
  assert.deepEqual(shelf.map(stateOf), ['reading', 'reading', 'new', 'finished', 'mine']);
  assert.equal(stateOf(book({ read: point({ progress: 0.995 }) })), 'finished');
  assert.equal(stateOf(book({ own: true, read: point({ progress: 1 }) })), 'mine');
  assert.deepEqual(tabCounts(shelf).map((tab) => [tab.id, tab.count]), [['all', 5], ['reading', 2], ['new', 1], ['finished', 1], ['mine', 1]]);
});

test('a tab and a search narrow the shelf by title or author in any case', () => {
  const keys = (options) => shelfView(shelf, options).map((entry) => entry.key);
  assert.deepEqual(keys({ tab: 'reading' }), ['a', 'b']);
  assert.deepEqual(keys({ tab: 'mine' }), ['e']);
  assert.deepEqual(keys({ query: ' AMBER ' }), ['b']);
  assert.deepEqual(keys({ query: 'reyes' }), ['e']);
  assert.deepEqual(keys({ query: 'writer', tab: 'finished' }), []);
  assert.deepEqual(keys({ tab: 'nonsense' }).length, 5);
});

test('sorts put the last opened first and order titles without case', () => {
  const keys = (sort) => shelfView(shelf, { sort }).map((entry) => entry.key);
  assert.deepEqual(keys('opened'), ['a', 'e', 'b', 'd', 'c']);
  assert.deepEqual(keys('title'), ['b', 'c', 'a', 'd', 'e']);
  assert.deepEqual(keys('author').slice(0, 3), ['b', 'e', 'a']);
  assert.equal(keys('author').at(-1), 'c');
  assert.deepEqual(keys('progress'), ['d', 'b', 'a', 'c', 'e']);
  assert.deepEqual(keys('chapters'), ['a', 'b', 'd', 'c', 'e']);
  assert.deepEqual(keys('size'), ['a', 'd', 'b', 'c', 'e']);
  assert.equal(sortOf('<b>'), 'opened');
  assert.deepEqual(Object.keys(SORTS), ['opened', 'title', 'author', 'progress', 'chapters', 'size']);
});

test('the strip holds the last three books opened', () => {
  assert.deepEqual(recentOf(shelf).map((entry) => entry.key), ['a', 'e', 'b']);
  assert.deepEqual(recentOf(shelf.slice(2, 3)).map((entry) => entry.key), ['c']);
  assert.deepEqual(recentOf([]), []);
});

test('the place reads by the chapter title, never by position', () => {
  const going = cardOf(shelf[0]);
  assert.equal(going.line, 'Chapter 1818: The Gate.');
  assert.deepEqual(going.meta, ['32%', 'Ch 1,818']);
  assert.ok(!JSON.stringify(going).includes('1,973'));
  assert.ok(!JSON.stringify(going).includes('6,193'));
  assert.equal(chapterTag('Chapter 1818: The Gate'), 'Ch 1,818');
  assert.equal(chapterTag('第1818章 The Gate'), 'Ch 1,818');
  assert.equal(chapterTag('Prologue'), null);
  assert.deepEqual(cardOf(book({ read: point({ label: 'Prologue' }) })).meta, ['32%']);
});

test('progress, words and wording follow the state', () => {
  const going = cardOf(shelf[0]);
  assert.equal(going.kicker, 'Continue reading');
  assert.equal(going.progress, 0.3186);
  assert.equal(going.stat, '32%');
  assert.deepEqual(going.go, { label: 'Resume', writing: false, edit: false });
  assert.deepEqual(going.alt, { label: 'Open in Edit', writing: false, edit: true });
  assert.equal(going.passage, 'A passage.');
  assert.equal(cardOf(book({ read: point({ passage: 'Chapter 1818: The Gate The sovereign fell.' }) })).passage, 'The sovereign fell.');
  assert.equal(cardOf(book({ read: point({ passage: 'Chapter 1818: The Gate' }) })).passage, 'Chapter 1818: The Gate');
  const fresh = cardOf(shelf[2]);
  assert.equal(fresh.go.label, 'Start reading');
  assert.equal(fresh.line, 'Not started. 212 chapters.');
  assert.deepEqual(fresh.meta, ['New', '212 ch']);
  const done = cardOf(shelf[3]);
  assert.equal(done.go.label, 'Read again');
  assert.equal(done.stat, 'Finished');
});

test('your own book shows its chapter and word count, not a percent', () => {
  const mine = cardOf(shelf[4]);
  assert.equal(mine.kicker, 'Continue writing');
  assert.equal(mine.line, 'Chapter 38: The Keeper Wakes. 38 chapters.');
  assert.equal(mine.stat, '92,410 words');
  assert.equal(mine.progress, null);
  assert.deepEqual(mine.go, { label: 'Continue writing', writing: true, edit: true });
  assert.deepEqual(mine.alt, { label: 'Read it', writing: false, edit: false });
  assert.equal(cardOf(book({ own: true, chapters: 1 })).stat, '1 chapter');
  assert.equal(cardOf(book({ own: true, words: 1 })).stat, '1 word');
  assert.equal(cardOf(book({ own: true, read: point({ label: 'Reading' }) })).line, 'Reading. 6,193 chapters.');
});

test('the book menu holds the rare actions with the right words', () => {
  assert.deepEqual(menuOf(shelf[0]).map((item) => item.label), ['Resume', 'Open in Edit', 'This is my own book', 'Show file in folder', 'Export a copy', 'Clean up book…', 'Remove from shelf']);
  assert.deepEqual(menuOf(shelf[4]).slice(0, 3).map((item) => item.label), ['Continue writing', 'Read it', 'Not my own book']);
});

test('a book keeps one generated look from its key', () => {
  assert.deepEqual(lookOf('ffffffffffffffffffffffffffffffff'), lookOf('ffffffffffffffffffffffffffffffff'));
  assert.match(lookOf('0123456789abcdef0123456789abcdef').cls, /^b[1-9]$/);
  assert.match(lookOf('0123456789abcdef0123456789abcdef').art, /^a[1-5]$/);
});

test('sizes, dates and the status line read plainly', () => {
  assert.equal(sizeText(512), '512 B');
  assert.equal(sizeText(2_100), '2 KB');
  assert.equal(sizeText(48_210_000), '48.2 MB');
  assert.equal(sizeText(3_500_000_000), '3.5 GB');
  const now = new Date(2026, 10, 7, 15, 0).getTime();
  assert.equal(openedText(new Date(2026, 10, 7, 9, 5).getTime(), now), 'Today 09:05');
  assert.equal(openedText(new Date(2026, 10, 6, 23, 0).getTime(), now), 'Yesterday');
  assert.equal(openedText(new Date(2026, 9, 28).getTime(), now), 'Oct 28');
  assert.equal(openedText(new Date(2025, 0, 2).getTime(), now), 'Jan 2, 2025');
  assert.deepEqual(statusText(shelf), ['5 books', '2 reading', '1 finished', '1 yours']);
});

test('the remove dialog says what stays and what can go', () => {
  const text = removalText({ title: 'Monsoon', dataBytes: 2_400_000, fileBytes: 48_000_000 });
  assert.equal(text.title, 'Remove Monsoon from the shelf?');
  assert.match(text.body, /stay, and adding the book again brings them back/);
  assert.match(text.body, /The file you added it from is never touched/);
  assert.match(text.data, /\(2\.4 MB\)/);
  assert.equal(text.file, "Also delete Jadeslip's copy of the EPUB (48.0 MB)");
  assert.equal(removalText({ title: 'Gone', dataBytes: 0, fileBytes: null }).file, null);
});

test('the move notice counts books and names what changed', () => {
  assert.equal(movingText({ finished: false, total: 7, done: 2, copied: 2, failed: [] }), "Copying your books into Jadeslip's library, 3 of 7. The files on your disk are not changed.");
  assert.equal(movingText({ finished: false, total: 7, done: 7, copied: 7, failed: [] }).includes('7 of 7'), true);
  assert.match(movingText({ finished: true, total: 7, done: 7, copied: 6, failed: ['Gone: missing.'] }), /own copy of 6 books on your shelf.*Gone: missing\.$/);
  assert.equal(movingText({ finished: true, total: 1, done: 1, copied: 0, failed: [] }), '');
});

test('adding names one book, counts several and lists the ones already there', () => {
  assert.equal(addedText([{ state: 'added', title: 'Monsoon' }]), 'Added Monsoon to your shelf.');
  assert.equal(addedText([{ state: 'added', title: 'A' }, { state: 'added', title: 'B' }, { state: 'alreadyOnShelf', title: 'C' }]), 'Added 2 books to your shelf. Already on your shelf: C.');
  assert.equal(addedText([]), '');
});

test('arrows step through the grid by one and by a row, and stop at the edges', () => {
  assert.equal(stepIndex(-1, 10, 4, 'right'), 0);
  assert.equal(stepIndex(0, 10, 4, 'left'), 0);
  assert.equal(stepIndex(3, 10, 4, 'right'), 4);
  assert.equal(stepIndex(1, 10, 4, 'down'), 5);
  assert.equal(stepIndex(7, 10, 4, 'down'), 7);
  assert.equal(stepIndex(5, 10, 4, 'up'), 1);
  assert.equal(stepIndex(1, 10, 4, 'up'), 1);
  assert.equal(stepIndex(9, 10, 4, 'right'), 9);
  assert.equal(stepIndex(0, 0, 4, 'right'), -1);
});
