import { chapterNumberOf } from './chapter-number.js';

const FINISHED_AT = 0.995;
const RECENT = 3;
const LOOKS = 9;
const ARTS = 5;
const DAY_MS = 86_400_000;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

const count = (value) => value.toLocaleString('en-US');
const plural = (value, one, many) => `${count(value)} ${value === 1 ? one : many}`;
const collator = new Intl.Collator('en', { sensitivity: 'base', numeric: true });
const digit = (key, from, to) => Number.parseInt(key.slice(from, to), 16) || 0;

export function stateOf(book) {
  if (book.own) return 'mine';
  if (!book.read) return 'new';
  return book.read.progress >= FINISHED_AT ? 'finished' : 'reading';
}

export const TABS = [
  { id: 'all', label: 'All', has: () => true },
  { id: 'reading', label: 'Reading', has: (book) => stateOf(book) === 'reading' },
  { id: 'new', label: 'Not started', has: (book) => stateOf(book) === 'new' },
  { id: 'finished', label: 'Finished', has: (book) => stateOf(book) === 'finished' },
  { id: 'mine', label: 'Mine', has: (book) => stateOf(book) === 'mine' },
];

export const tabCounts = (books) => TABS.map((tab) => ({ id: tab.id, label: tab.label, count: books.filter(tab.has).length }));

const progressOf = (book) => (book.own ? -1 : book.read?.progress ?? 0);
const byTitle = (a, b) => collator.compare(a.title, b.title) || a.key.localeCompare(b.key);

export const SORTS = {
  opened: { label: 'Last opened', compare: (a, b) => b.touchedAt - a.touchedAt || byTitle(a, b) },
  title: { label: 'Title', compare: byTitle },
  author: { label: 'Author', compare: (a, b) => (a.author ? 0 : 1) - (b.author ? 0 : 1) || collator.compare(a.author ?? '', b.author ?? '') || byTitle(a, b) },
  progress: { label: 'Progress', compare: (a, b) => progressOf(b) - progressOf(a) || byTitle(a, b) },
  chapters: { label: 'Chapters', compare: (a, b) => b.chapters - a.chapters || byTitle(a, b) },
  size: { label: 'Size', compare: (a, b) => b.bytes - a.bytes || byTitle(a, b) },
};

export const sortOf = (name) => (SORTS[name] ? name : 'opened');

export function matches(book, text) {
  const wanted = text.trim().toLowerCase();
  return !wanted || book.title.toLowerCase().includes(wanted) || (book.author ?? '').toLowerCase().includes(wanted);
}

export function shelfView(books, { tab = 'all', query = '', sort = 'opened' } = {}) {
  const has = (TABS.find((entry) => entry.id === tab) ?? TABS[0]).has;
  return books.filter((book) => has(book) && matches(book, query)).sort(SORTS[sortOf(sort)].compare);
}

export const recentOf = (books) => [...books].sort(SORTS.opened.compare).slice(0, RECENT);

export function lookOf(key) {
  return { cls: `b${1 + digit(key, 0, 4) % LOOKS}`, art: `a${1 + digit(key, 4, 8) % ARTS}` };
}

const pointOf = (book) => (book.own ? book.write ?? book.read : book.read);

export function chapterTag(label) {
  const number = chapterNumberOf(label);
  return number === null ? null : `Ch ${count(number)}`;
}

export const wordsText = (book) => (book.words == null ? plural(book.chapters, 'chapter', 'chapters') : plural(book.words, 'word', 'words'));

export const percentText = (progress) => `${Math.round(progress * 100)}%`;

const GO = {
  mine: { label: 'Continue writing', writing: true, edit: true },
  reading: { label: 'Resume', writing: false, edit: false },
  new: { label: 'Start reading', writing: false, edit: false },
  finished: { label: 'Read again', writing: false, edit: false },
};

const READ_IT = { label: 'Read it', writing: false, edit: false };
const OPEN_IN_EDIT = { label: 'Open in Edit', writing: false, edit: true };

export function cardOf(book) {
  const state = stateOf(book);
  const point = pointOf(book);
  const chapters = plural(book.chapters, 'chapter', 'chapters');
  const label = point?.label ?? '';
  const progress = book.read?.progress ?? 0;
  const passage = point?.passage ?? '';
  const after = label && passage.startsWith(label) ? passage.slice(label.length).trimStart() : passage;
  const base = { state, go: GO[state], alt: state === 'mine' ? READ_IT : OPEN_IN_EDIT, passage: after || passage };
  if (state === 'mine') {
    return { ...base, kicker: 'Continue writing', line: label ? `${label}. ${chapters}.` : `${chapters}.`, progress: null, stat: wordsText(book), meta: [wordsText(book)] };
  }
  if (state === 'new') {
    return { ...base, kicker: 'Not started', line: `Not started. ${chapters}.`, progress: 0, stat: 'Not started', meta: ['New', `${count(book.chapters)} ch`] };
  }
  if (state === 'finished') {
    return { ...base, kicker: 'Finished', line: `Finished. ${chapters}.`, progress: 1, stat: 'Finished', meta: ['Finished', `${count(book.chapters)} ch`] };
  }
  const tag = chapterTag(label);
  return {
    ...base,
    kicker: 'Continue reading',
    line: label ? `${label}.` : `${chapters}.`,
    progress,
    stat: percentText(progress),
    meta: tag ? [percentText(progress), tag] : [percentText(progress)],
  };
}

export function bookLabel(book) {
  const card = cardOf(book);
  return [book.title, book.author, card.state === 'mine' ? `Your book, ${card.stat}` : card.line.replace(/\.$/, '')].filter(Boolean).join(', ');
}

export function menuOf(book) {
  const card = cardOf(book);
  return [
    { id: 'go', label: card.go.label, key: 'Enter' },
    { id: 'alt', label: card.alt.label, key: 'E' },
    { id: 'own', label: book.own ? 'Not my own book' : 'This is my own book' },
    { id: 'folder', label: 'Show file in folder' },
    { id: 'export', label: 'Export a copy' },
    { id: 'clean', label: 'Clean up book…' },
    { id: 'remove', label: 'Remove from shelf' },
  ];
}

export function sizeText(bytes) {
  if (bytes < 1000) return `${count(bytes)} B`;
  if (bytes < 1e6) return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
  if (bytes < 1e9) return `${(bytes / 1e6).toFixed(1)} MB`;
  return `${(bytes / 1e9).toFixed(1)} GB`;
}

const pad = (value) => String(value).padStart(2, '0');

export function openedText(at, now) {
  const when = new Date(at);
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  if (at >= today.getTime()) return `Today ${pad(when.getHours())}:${pad(when.getMinutes())}`;
  if (at >= today.getTime() - DAY_MS) return 'Yesterday';
  const day = `${MONTHS[when.getMonth()]} ${when.getDate()}`;
  return when.getFullYear() === today.getFullYear() ? day : `${day}, ${when.getFullYear()}`;
}

export function statusText(books) {
  const counts = Object.fromEntries(tabCounts(books).map((tab) => [tab.id, tab.count]));
  return [plural(books.length, 'book', 'books'), `${count(counts.reading)} reading`, `${count(counts.finished)} finished`, `${count(counts.mine)} yours`];
}

export function removalText(info) {
  return {
    title: `Remove ${info.title} from the shelf?`,
    body: "Jadeslip's copy of the book, its backups, history, notes and reading place stay, and adding the book again brings them back. The file you added it from is never touched.",
    data: `Also delete Jadeslip's data for this book (${sizeText(info.dataBytes)}): backups, history, notes and highlights, drafts, reading place and cover`,
    file: info.fileBytes == null ? null : `Also delete Jadeslip's copy of the EPUB (${sizeText(info.fileBytes)})`,
  };
}

export function movingText(state) {
  if (!state.finished) {
    const at = Math.min(state.done + 1, state.total);
    return `Copying your books into Jadeslip's library, ${count(at)} of ${count(state.total)}. The files on your disk are not changed.`;
  }
  const parts = [];
  if (state.copied > 0) parts.push(`Jadeslip now keeps its own copy of ${plural(state.copied, 'book', 'books')} on your shelf, so edits change that copy. The files you added them from were not changed.`);
  parts.push(...state.failed);
  return parts.join(' ');
}

export function addedText(adds) {
  const added = adds.filter((add) => add.state === 'added');
  const already = adds.filter((add) => add.state === 'alreadyOnShelf');
  const parts = [];
  if (added.length === 1) parts.push(`Added ${added[0].title} to your shelf.`);
  else if (added.length > 1) parts.push(`Added ${plural(added.length, 'book', 'books')} to your shelf.`);
  if (already.length > 0) parts.push(`Already on your shelf: ${already.map((add) => add.title).join(', ')}.`);
  return parts.join(' ');
}

export function stepIndex(index, length, columns, direction) {
  if (length === 0) return -1;
  if (index < 0) return 0;
  const by = { left: -1, right: 1, up: -columns, down: columns }[direction];
  const next = index + by;
  if (next < 0 || next >= length) return direction === 'down' || direction === 'up' ? index : Math.min(length - 1, Math.max(0, next));
  return next;
}
