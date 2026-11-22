import test from 'node:test';
import assert from 'node:assert/strict';
import { chapterNumberOf, placeName, shortName } from '../../app/ui/chapter-number.js';

const READS = [
  ['Chapter 1818: The Gate', 1818],
  ['Chapter 1 : Three Lamp Keepers 1', 1],
  ['Chapter 1131 – Home Again', 1131],
  ['Chapter1131 – Home Again', 1131],
  ['Chapter 546 Thousand Buddha Palm', 546],
  ['Chapter 1,818: The Gate', 1818],
  ['chapter 7', 7],
  ['Ch. 1818', 1818],
  ['Ch.1818: The Gate', 1818],
  ['Ep 12 - Return', 12],
  ['Imc 2309 - Crossing the Pass', 2309],
  ['​Imc 2019 - Tamed', 2019],
  ['730 – A Duel Under Two Stars', 730],
  ['1818 - The Gate', 1818],
  ['1. Starting Small', 1],
  ['42', 42],
  ['28 Red Lantern Fruit', 28],
  ['2,908 Old Friends', 2908],
  ['1222 10 Ginseng Roots', 1222],
  ['3725 “A Favour”', 3725],
  ['41 《Sword》', 41],
  ['第1818章 城门', 1818],
];

const REFUSES = [
  'Volume 3',
  'Vol. 2: The Return',
  'Side Story 7: The Lamp Keeper Wen',
  'Part 4 - Ending',
  'Book 2: Ashes',
  'Day 12: Harvest',
  'Chapter 12.5: Interlude',
  '1.1.2. Loosely Coupled',
  '69 with friends',
  '12.5 Interlude',
  '300 years later',
  'Synopsis',
  'Chapter One',
  '',
  null,
];

test('a title reads as the chapter number it carries', () => {
  for (const [title, number] of READS) assert.equal(chapterNumberOf(title), number, title);
});

test('volumes, parts, sections, decimals and bare leading numbers are not chapter numbers', () => {
  for (const title of REFUSES) assert.equal(chapterNumberOf(title), null, String(title));
});

test('a place is named by its title, then by its position', () => {
  assert.equal(placeName('Chapter 1818: The Gate', 1839), 'Chapter 1818: The Gate');
  assert.equal(placeName('', 1839), 'Position 1,839');
  assert.equal(placeName(null, null), 'Outside the reading order');
  assert.equal(shortName('Chapter 1818: The Gate', 1839), 'Chapter 1818');
  assert.equal(shortName('Synopsis', 2), 'Synopsis');
  assert.equal(shortName('28 Red Lantern Fruit', 30), 'Chapter 28');
});
