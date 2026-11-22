import test from 'node:test';
import assert from 'node:assert/strict';
import {
  LIMITS, clearRecent, emptyMemory, forget, moveHighlight, nameFrom, record, remove, rename, restore, rowsOf, save, savedOf, tagOf,
} from '../../app/ui/search-memory-model.js';

const ROMANCE = String.raw`\b(?:kiss(?:ed|es)?|blush(?:ed)?)\b`;
const query = (text, regex = false, matchCase = false) => ({ text, regex, matchCase });

test('saving keeps the mode and Match case and puts the newest first', () => {
  let memory = emptyMemory();
  ({ memory } = save(memory, 'Names', query('Han Mo')));
  const saved = save(memory, 'Romance', query(ROMANCE, true, true));
  assert.equal(saved.name, 'Romance');
  assert.deepEqual(saved.memory.saved, [
    { name: 'Romance', text: ROMANCE, regex: true, matchCase: true },
    { name: 'Names', text: 'Han Mo', regex: false, matchCase: false },
  ]);
  assert.deepEqual(emptyMemory(), { saved: [], recent: [] }, 'the input memory is not changed');
});

test('an empty name falls back to the start of the query, truncated', () => {
  assert.equal(save(emptyMemory(), '   ', query('Han   Mo\n')).name, 'Han Mo');
  const long = nameFrom(ROMANCE + ROMANCE);
  assert.equal([...long].length, 40);
  assert.ok(long.endsWith('…'));
  assert.equal(nameFrom('short'), 'short');
});

test('saving the same query again renames it instead of adding a twin', () => {
  let { memory } = save(emptyMemory(), 'Old', query(ROMANCE, true));
  ({ memory } = save(memory, 'New', query(ROMANCE, true)));
  assert.deepEqual(memory.saved.map((entry) => entry.name), ['New']);
  ({ memory } = save(memory, 'Cased', query(ROMANCE, true, true)));
  ({ memory } = save(memory, 'Plain', query(ROMANCE, false)));
  assert.equal(memory.saved.length, 3, 'mode and case make a different search');
});

test('names are unique ignoring case and bad names are refused', () => {
  const { memory } = save(emptyMemory(), 'Romance', query('a'));
  assert.equal(save(memory, 'ROMANCE', query('b')).error, 'A saved search is already called "Romance".');
  assert.match(save(memory, 'x'.repeat(LIMITS.nameChars + 1), query('b')).error, /80 characters/);
  assert.equal(save(memory, 'x'.repeat(LIMITS.nameChars), query('b')).name.length, LIMITS.nameChars);
  assert.equal(save(memory, 'tab\u0007', query('b')).error, 'A saved search name cannot hold line breaks.');
});

test('empty and overlong queries are refused', () => {
  assert.equal(save(emptyMemory(), 'x', query('')).error, 'Type a search first, then save it.');
  assert.match(save(emptyMemory(), 'x', query('é'.repeat(LIMITS.textBytes / 2 + 1))).error, /longer than 16 KB/);
  assert.ok(save(emptyMemory(), 'x', query('a'.repeat(LIMITS.textBytes))).memory);
});

test('the saved list stops at its limit', () => {
  let memory = emptyMemory();
  for (let at = 0; at < LIMITS.saved; at += 1) ({ memory } = save(memory, `s${at}`, query(`t${at}`)));
  assert.equal(memory.saved.length, LIMITS.saved);
  assert.equal(save(memory, 'one more', query('more')).error, 'You have 200 saved searches. Delete one to save another.');
  assert.equal(save(memory, 'renamed', query('t3')).name, 'renamed', 'renaming still works when full');
});

test('rename refuses a taken name and keeps the query', () => {
  let { memory } = save(emptyMemory(), 'A', query('a', true));
  ({ memory } = save(memory, 'B', query('b')));
  assert.equal(rename(memory, 'A', 'b').error, 'A saved search is already called "B".');
  assert.equal(rename(memory, 'A', '  ').error, 'Give the saved search a name.');
  assert.equal(rename(memory, 'gone', 'C').error, 'That saved search is gone.');
  const renamed = rename(memory, 'A', ' a ');
  assert.equal(renamed.name, 'a', 'a change of case alone is allowed');
  assert.deepEqual(renamed.memory.saved.find((entry) => entry.name === 'a'), { name: 'a', text: 'a', regex: true, matchCase: false });
});

test('delete hands back what undo needs to put it back in place', () => {
  let { memory } = save(emptyMemory(), 'A', query('a'));
  ({ memory } = save(memory, 'B', query('b')));
  ({ memory } = save(memory, 'C', query('c')));
  const { memory: without, removed } = remove(memory, 'B');
  assert.deepEqual(without.saved.map((entry) => entry.name), ['C', 'A']);
  assert.deepEqual(restore(without, removed).saved.map((entry) => entry.name), ['C', 'B', 'A']);
  assert.equal(remove(memory, 'nope').removed, null);
  const retaken = save(without, 'B', query('other')).memory;
  assert.equal(restore(retaken, removed), retaken, 'undo does not make a twin name');
});

test('recent searches go newest first, move up when run again and stop at 50', () => {
  let memory = emptyMemory();
  memory = record(memory, query('a'));
  memory = record(memory, query('b', true));
  memory = record(memory, query('a'));
  assert.deepEqual(memory.recent, [query('a'), query('b', true)]);
  memory = record(memory, query('a', false, true));
  assert.equal(memory.recent.length, 3, 'Match case makes it a different search');
  assert.equal(record(memory, query('')), memory, 'an empty search is not kept');
  for (let at = 0; at < LIMITS.recent + 10; at += 1) memory = record(memory, query(`q${at}`));
  assert.equal(memory.recent.length, LIMITS.recent);
  assert.equal(memory.recent[0].text, `q${LIMITS.recent + 9}`);
});

test('forget and clear touch only the recent list', () => {
  let { memory } = save(emptyMemory(), 'A', query('a'));
  memory = record(record(memory, query('x')), query('y'));
  assert.deepEqual(forget(memory, query('x')).recent, [query('y')]);
  assert.deepEqual(clearRecent(memory), { saved: memory.saved, recent: [] });
});

test('rows put saved first, hide recent twins of saved ones and filter both', () => {
  let { memory } = save(emptyMemory(), 'Romance', query(ROMANCE, true));
  memory = record(memory, query('blush'));
  memory = record(memory, query(ROMANCE, true));
  memory = record(memory, query('Han Mo'));
  assert.deepEqual(rowsOf(memory).map((row) => [row.kind, row.name ?? row.query.text]), [['saved', 'Romance'], ['recent', 'Han Mo'], ['recent', 'blush']]);
  assert.deepEqual(rowsOf(memory, 'ROM').map((row) => row.name), ['Romance']);
  assert.deepEqual(rowsOf(memory, 'kiss').map((row) => row.name), ['Romance'], 'the pattern matches too');
  assert.deepEqual(rowsOf(memory, 'han').map((row) => row.query.text), ['Han Mo']);
  assert.deepEqual(rowsOf(memory, 'zzz'), []);
});

test('savedOf finds the saved search that matches the field', () => {
  const { memory } = save(emptyMemory(), 'Romance', query(ROMANCE, true, true));
  assert.equal(savedOf(memory, query(ROMANCE, true, true)).name, 'Romance');
  assert.equal(savedOf(memory, query(ROMANCE, true, false)), null);
});

test('Down starts at the top, Up starts at the newest recent search', () => {
  const rows = [{ kind: 'saved' }, { kind: 'saved' }, { kind: 'recent' }, { kind: 'recent' }];
  assert.equal(moveHighlight(rows, null, 1), 0);
  assert.equal(moveHighlight(rows, null, -1), 2);
  assert.equal(moveHighlight(rows, 2, 1), 3);
  assert.equal(moveHighlight(rows, 3, 1), 3);
  assert.equal(moveHighlight(rows, 0, -1), null, 'Up from the top leaves the list for the field');
  assert.equal(moveHighlight([{ kind: 'saved' }], null, -1), 0);
  assert.equal(moveHighlight([], null, 1), null);
});

test('the tag names the mode and Match case', () => {
  assert.equal(tagOf(query('a')), 'Plain');
  assert.equal(tagOf(query('a', true, true)), 'Regex · Match case');
});
