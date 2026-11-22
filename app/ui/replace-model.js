const count = (value) => value.toLocaleString('en-US');
const plural = (value, one, many) => `${count(value)} ${value === 1 ? one : many}`;

export const MAX_PREVIEW_ROWS = 5000;
export const UNDO_TEXT = 'To undo it, use Revert to backup, which returns the whole book to how it was before your first edit.';

export const emptyChoices = () => ({ hits: new Set(), chapters: new Set() });

const hitKey = (ix, ordinal) => `${ix}:${ordinal}`;

export const canChange = (hit) => !hit.crosses_block && !hit.same;

export const isHitOut = (choices, row) => choices.hits.has(hitKey(row.chapter.ix, row.ordinal));
export const isChapterOut = (choices, row) => choices.chapters.has(row.chapter.ix);

export function toggleHit(choices, row) {
  const key = hitKey(row.chapter.ix, row.ordinal);
  if (!choices.hits.delete(key)) choices.hits.add(key);
}

export function toggleChapter(choices, row) {
  const { ix } = row.chapter;
  if (!choices.chapters.delete(ix)) choices.chapters.add(ix);
}

export function exclusionsOf(choices) {
  const hits = [...choices.hits].map((key) => {
    const [chapter, ordinal] = key.split(':').map(Number);
    return { kind: 'hit', chapter, ordinal };
  });
  const chapters = [...choices.chapters].map((chapter) => ({ kind: 'chapter', chapter }));
  return [...hits, ...chapters];
}

export function remaining(preview, results, choices) {
  const left = new Map(preview.touched.map(({ chapter, changes }) => [chapter, changes]));
  for (const ix of choices.chapters) left.delete(ix);
  for (const key of choices.hits) {
    const [ix, ordinal] = key.split(':').map(Number);
    const row = results.byChapter.get(ix)?.[ordinal];
    if (row && canChange(row.hit) && left.has(ix)) left.set(ix, left.get(ix) - 1);
  }
  let changes = 0;
  let chapters = 0;
  for (const value of left.values()) {
    changes += value;
    if (value > 0) chapters += 1;
  }
  return { changes, chapters };
}

export function summaryText(preview, left) {
  if (preview.limited > 0) {
    return `This pattern was too slow or matched too often on ${plural(preview.limited, 'chapter', 'chapters')}, so it cannot be applied.`;
  }
  const parts = [`Preview: ${plural(left.changes, 'change', 'changes')} in ${plural(left.chapters, 'chapter', 'chapters')}.`];
  if (preview.skipped > 0) parts.push(`${count(preview.skipped)} skipped because ${preview.skipped === 1 ? 'it crosses' : 'they cross'} a paragraph.`);
  if (preview.spanning > 0) parts.push(`${count(preview.spanning)} ${preview.spanning === 1 ? 'crosses' : 'cross'} formatting.`);
  if (preview.unreadable > 0) parts.push(`${plural(preview.unreadable, 'chapter', 'chapters')} could not be read and will stay as they are.`);
  parts.push('Applied as one atomic save.', UNDO_TEXT);
  return parts.join(' ');
}

export function noteOf(hit) {
  if (hit.crosses_block) return 'Crosses a paragraph, so it is skipped.';
  if (hit.same) return 'Already reads this way.';
  if (hit.spans_tags) return 'Crosses formatting. The new text takes the style of the first part.';
  return '';
}

export function previewFooter(preview, shown) {
  if (preview.matches > shown) {
    return `Showing ${count(shown)} of ${plural(preview.matches, 'match', 'matches')}. Only shown rows can be unticked.`;
  }
  return `${plural(shown, 'match', 'matches')}.`;
}

export const canApply = (preview, left, busy) => !busy && preview !== null && preview.limited === 0 && left.changes > 0;

export function appliedText(end) {
  return `${plural(end.changes, 'change', 'changes')} saved. Your original is backed up.`;
}

export function progressText({ done, total }) {
  if (total === 0) return 'Backing up your original.';
  if (done >= total) return 'Finishing the save.';
  return `${count(done)} of ${plural(total, 'chapter', 'chapters')} changed.`;
}

export function offerText(preview) {
  if (preview === null) return 'Fixed here. Looking for other copies.';
  return `Fixed here. ${plural(preview.changes, 'more copy', 'more copies')} in this book.`;
}

export function offerDialog(offer, preview) {
  return {
    title: `Fix ${plural(preview.changes, 'more copy', 'more copies')}?`,
    text: `Sample preview: "${offer.from}" becomes "${offer.to}" in ${plural(preview.chapters, 'chapter', 'chapters')}. Whole words only. Capitalised and all-capitals copies follow their own case. Every change is saved at once, and your original stays backed up. ${UNDO_TEXT}`,
  };
}
