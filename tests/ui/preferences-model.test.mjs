import test from 'node:test';
import assert from 'node:assert/strict';
import {
  KEEP_PERIODS, backupAsk, cleanedText, isShorter, keepAsk, lastCleanupText, storageRows, wordsText,
} from '../../app/ui/preferences-model.js';

test('keep periods run from shortest to forever', () => {
  assert.deepEqual(KEEP_PERIODS.map((period) => period.value), ['days7', 'days30', 'days90', 'year', 'forever']);
  assert.deepEqual(KEEP_PERIODS.map((period) => period.label), ['7 days', '30 days', '90 days', '1 year', 'Forever']);
});

test('only a shorter period counts as shorter', () => {
  assert.equal(isShorter('days7', 'days30'), true);
  assert.equal(isShorter('year', 'forever'), true);
  assert.equal(isShorter('days30', 'days30'), false);
  assert.equal(isShorter('days90', 'days30'), false);
  assert.equal(isShorter('forever', 'days7'), false);
});

test('an unknown saved period is treated as forever, so any choice asks first', () => {
  assert.equal(isShorter('year', 'weekly'), true);
  assert.equal(isShorter('forever', 'weekly'), false);
});

test('a cleanup reports what it freed', () => {
  assert.equal(cleanedText({ points: 214, books: 3, bytes: 12_400_000 }), 'Freed 12.4 MB from 3 books');
  assert.equal(cleanedText({ points: 2, books: 1, bytes: 2_100 }), 'Freed 2 KB from 1 book');
  assert.equal(cleanedText({ points: 0, books: 0, bytes: 0 }), 'Nothing to clean up');
  assert.equal(cleanedText({ points: 0, books: 2, bytes: 5_000_000 }), 'Freed 5.0 MB from 2 books');
});

test('the shorter period confirm names the points, the period, the books and the size', () => {
  const ask = keepAsk({ points: 214, books: 5, bytes: 48_000_000 }, 'days30');
  assert.equal(ask.text, 'This removes 214 automatic history points older than 30 days from 5 books and frees 48.0 MB. Named snapshots and original backups stay.');
  assert.equal(ask.ok, 'Remove');
  assert.match(keepAsk({ points: 1, books: 1, bytes: 1_500_000_000 }, 'year').text, /^This removes 1 automatic history point older than 1 year from 1 book and frees 1\.5 GB\./);
});

test('the backup confirm names the book, the size and what Revert loses', () => {
  const ask = backupAsk({ title: 'Monsoon', backup: 9_800_000 });
  assert.match(ask.text, /Monsoon \(9\.8 MB\)/);
  assert.match(ask.text, /Revert will no longer be able to go back to the file as it was before the first edit, and chapters deleted since then can no longer be restored\./);
});

test('the last automatic cleanup reads as a sentence', () => {
  const now = new Date(2026, 10, 7, 12, 0).getTime();
  assert.equal(lastCleanupText(null, null, now), 'No automatic cleanup has run yet.');
  assert.equal(lastCleanupText(new Date(2026, 10, 7, 9, 5).getTime(), 3_100_000, now), 'Last automatic cleanup today 09:05 freed 3.1 MB.');
  assert.equal(lastCleanupText(new Date(2026, 9, 28).getTime(), 0, now), 'Last automatic cleanup Oct 28 found nothing to remove.');
});

test('storage rows add up each book and list the largest first', () => {
  const rows = storageRows([
    { key: 'a', title: 'Small', book: 0, history: 10, backup: 0, other: 5 },
    { key: 'b', title: 'Big', book: 0, history: 900, backup: 100, other: 0 },
    { key: 'c', title: 'Copied', book: 2000, history: 0, backup: 0, other: 0 },
  ]);
  assert.deepEqual(rows.map((row) => [row.key, row.total]), [['c', 2000], ['b', 1000], ['a', 15]]);
});

test('added words count reads plainly', () => {
  assert.equal(wordsText(0), 'No added words');
  assert.equal(wordsText(1), '1 added word');
  assert.equal(wordsText(1204), '1,204 added words');
});
