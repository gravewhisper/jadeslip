import { positionText } from './chapter-number.js';
import { matches } from './toc-model.js';

const MAX_ROWS = 7;
const NUMBER = /^(?:chapter\s*|ch\.?\s*)?([0-9][0-9,]*)$/;

const LEADING = /^[\s\u200b\ufeff]*(\d{1,3}(?:,\d{3})+|\d+)(?![.,]?\d)/;

const numberText = (number) => number.toLocaleString('en-US');

function carries(entry, number) {
  if (entry.chapter !== null) return entry.chapter === number;
  const found = LEADING.exec(entry.label ?? '');
  return found !== null && Number(found[1].replace(/,/g, '')) === number;
}

const chapterRow = (entry) => ({
  kind: 'chapter',
  ix: entry.ix,
  fragment: entry.fragment,
  label: entry.label,
  hint: positionText(entry.number),
});

function numberedRows(number, toc) {
  const titled = toc.entries.filter((entry) => entry.ix !== null && carries(entry, number)).map(chapterRow);
  if (titled.length || number < 1 || number > toc.linear.length) return titled;
  const ix = toc.linear[number - 1];
  const at = toc.index.chosen.get(ix);
  const entry = at === undefined ? null : toc.entries[at];
  return [{ kind: 'chapter', ix, fragment: null, label: entry?.label ?? `Go to position ${numberText(number)}`, hint: positionText(number) }];
}

export function paletteRows(rawQuery, toc, commands) {
  const query = rawQuery.trim().toLowerCase();
  const found = NUMBER.exec(query);
  const rows = found && toc ? numberedRows(Number(found[1].replace(/,/g, '')), toc) : [];
  const offered = new Set(rows.map((row) => `${row.ix}#${row.fragment ?? ''}`));
  for (const command of commands) {
    if (!query || command.label.toLowerCase().includes(query)) {
      rows.push({ kind: 'command', label: command.label, hint: command.hint ?? 'Command', run: command.run });
    }
  }
  if (!query || !toc) return rows.slice(0, MAX_ROWS);
  for (const entry of toc.entries) {
    if (rows.length >= MAX_ROWS) break;
    if (entry.ix === null || offered.has(`${entry.ix}#${entry.fragment ?? ''}`)) continue;
    if (matches(entry, query)) rows.push(chapterRow(entry));
  }
  return rows.slice(0, MAX_ROWS);
}

export const footerHint = (chapters) => `Try ${numberText(Math.max(1, Math.floor(chapters / 3)))}, a word from a title or theme`;
