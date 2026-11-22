const DIGITS = String.raw`(\d{1,3}(?:,\d{3})+|\d{1,6})`;
const NAMED = new RegExp(String.raw`^(chapter|chap|ch|c|episode|ep|[a-z]{2,5})\.?\s*${DIGITS}(?![.,]?\d)`, 'i');
const BARE = new RegExp(String.raw`^${DIGITS}(?![.,]?\d)\s*(?:[:.)\-–—]|$)`);
const SPACED = new RegExp(String.raw`^${DIGITS}(?![.,]?\d)\s+[\p{Lu}\d“"‘'《【(]`, 'u');
const HAN = new RegExp(String.raw`^第\s*${DIGITS}\s*[章回节節话話]`);
const NOT_CHAPTERS = new Set([
  'volume', 'vol', 'book', 'part', 'arc', 'act', 'side', 'season', 'day', 'week', 'year',
  'page', 'step', 'level', 'no', 'num', 'item', 'unit', 'case', 'floor', 'room',
]);

const count = (value) => value.toLocaleString('en-US');

export function chapterNumberOf(title) {
  const text = (title ?? '').replace(/^[\s​﻿]+/, '');
  const named = NAMED.exec(text);
  const found = named && !NOT_CHAPTERS.has(named[1].toLowerCase()) ? named[2] : (!named && (BARE.exec(text) ?? SPACED.exec(text) ?? HAN.exec(text))?.[1]);
  return found ? Number(found.replace(/,/g, '')) : null;
}

export const positionText = (number) => (number === null ? 'Outside the reading order' : `Position ${count(number)}`);

export const placeName = (title, number) => title || positionText(number);

export function shortName(title, number) {
  const chapter = chapterNumberOf(title);
  return chapter === null ? placeName(title, number) : `Chapter ${chapter}`;
}
