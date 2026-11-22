import { openedText, sizeText } from './shelf-model.js';

export const KEEP_PERIODS = [
  { value: 'days7', label: '7 days', days: 7 },
  { value: 'days30', label: '30 days', days: 30 },
  { value: 'days90', label: '90 days', days: 90 },
  { value: 'year', label: '1 year', days: 365 },
  { value: 'forever', label: 'Forever', days: Infinity },
];

const periodOf = (value) => KEEP_PERIODS.find((period) => period.value === value);

export const isShorter = (next, saved) => (periodOf(next)?.days ?? Infinity) < (periodOf(saved)?.days ?? Infinity);

const plural = (value, one, many) => `${value.toLocaleString('en-US')} ${value === 1 ? one : many}`;

export const cleanedText = (result) => (result.points === 0 && result.bytes === 0 ? 'Nothing to clean up' : `Freed ${sizeText(result.bytes)} from ${plural(result.books, 'book', 'books')}`);

export function keepAsk(preview, keep) {
  const removes = plural(preview.points, 'automatic history point', 'automatic history points');
  return {
    title: 'Remove older history?',
    text: `This removes ${removes} older than ${periodOf(keep).label.toLowerCase()} from ${plural(preview.books, 'book', 'books')} and frees ${sizeText(preview.bytes)}. Named snapshots and original backups stay.`,
    ok: 'Remove',
  };
}

export const backupAsk = (book) => ({
  title: 'Delete the original backup?',
  text: `This deletes the original backup of ${book.title} (${sizeText(book.backup)}). Revert will no longer be able to go back to the file as it was before the first edit, and chapters deleted since then can no longer be restored.`,
  ok: 'Delete backup',
});

export function lastCleanupText(lastCleanup, lastFreed, now) {
  if (lastCleanup == null) return 'No automatic cleanup has run yet.';
  const when = openedText(lastCleanup, now).replace(/^(Today|Yesterday)/, (word) => word.toLowerCase());
  return lastFreed ? `Last automatic cleanup ${when} freed ${sizeText(lastFreed)}.` : `Last automatic cleanup ${when} found nothing to remove.`;
}

export const storageRows = (books) => books
  .map((book) => ({ ...book, total: book.book + book.history + book.backup + book.other }))
  .sort((a, b) => b.total - a.total || a.title.localeCompare(b.title));

export const wordsText = (words) => (words === 0 ? 'No added words' : plural(words, 'added word', 'added words'));
