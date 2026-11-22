import test from 'node:test';
import assert from 'node:assert/strict';
import { buildToc } from '../../app/ui/toc-model.js';
import { dropSpot, isInside, rowActions, rowRef, stepSpot } from '../../app/ui/chapters-model.js';
import { actionForKey } from '../../app/ui/keys.js';

const entry = (label, ix, depth = 0, fragment = null) => ({
  label, ix, depth, fragment, number: ix === null ? null : ix + 1,
});

const nested = () => buildToc({
  source: 'nav',
  linear: [0, 1, 2, 3],
  entries: [
    entry('Volume', null),
    entry('One', 0, 1),
    entry('One part', 0, 2, 'part'),
    entry('Two', 1, 1),
    entry('Three', 2),
    entry('Four', 3),
  ],
});

test('a section offers rename only, a group moves but cannot be deleted', () => {
  const toc = nested();
  assert.deepEqual(rowActions(toc, 0), { move: true, rename: true, delete: false });
  assert.deepEqual(rowActions(toc, 1), { move: true, rename: true, delete: true });
  assert.deepEqual(rowActions(toc, 2), { move: false, rename: true, delete: false });
});

test('a row on a book with no contents is named by its chapter', () => {
  const toc = buildToc({ source: 'headings', linear: [0, 1], entries: [entry('A', 0), entry('B', 1)] });
  assert.deepEqual(rowRef(toc, 1), { chapter: 1 });
  assert.deepEqual(rowRef(nested(), 1), { entry: 1 });
});

test('a step moves past a sibling and leaves the group at its edge', () => {
  const toc = nested();
  assert.deepEqual(stepSpot(toc, 3, -1), { row: { entry: 1 }, after: false });
  assert.deepEqual(stepSpot(toc, 1, -1), { row: { entry: 0 }, after: false });
  assert.deepEqual(stepSpot(toc, 3, 1), { row: { entry: 0 }, after: true });
  assert.deepEqual(stepSpot(toc, 4, 1), { row: { entry: 5 }, after: true });
  assert.equal(stepSpot(toc, 0, -1), null);
  assert.equal(stepSpot(toc, 5, 1), null);
});

test('a drop lands between rows, inside an open group, and never inside itself', () => {
  const toc = nested();
  const shown = [0, 1, 2, 3, 4, 5];
  assert.deepEqual(dropSpot(toc, shown, 4, false, 5), { row: { entry: 4 }, after: false });
  assert.deepEqual(dropSpot(toc, shown, 4, true, 5), { row: { entry: 4 }, after: true });
  assert.deepEqual(dropSpot(toc, shown, 0, true, 5), { row: { entry: 1 }, after: false });
  assert.equal(dropSpot(toc, shown, 3, false, 0), null);
  assert.equal(isInside(toc, 0, 3), true);
  assert.equal(isInside(toc, 0, 4), false);
});

const key = (over) => ({ key: '', ctrl: false, shift: false, alt: false, meta: false, composing: false, inField: false, onControl: false, ...over });

test('Alt up and down reorder in Edit only, Alt left goes back, F2 renames, Esc cancels a drag first', () => {
  assert.equal(actionForKey(key({ key: 'ArrowUp', alt: true }), 'edit'), 'moveChapterUp');
  assert.equal(actionForKey(key({ key: 'ArrowDown', alt: true }), 'edit'), 'moveChapterDown');
  assert.equal(actionForKey(key({ key: 'ArrowUp', alt: true }), 'read'), null);
  assert.equal(actionForKey(key({ key: 'ArrowUp', alt: true, inField: true }), 'edit'), null);
  assert.equal(actionForKey(key({ key: 'ArrowLeft', alt: true }), 'edit'), 'back');
  assert.equal(actionForKey(key({ key: 'ArrowLeft', alt: true }), 'read'), 'back');
  assert.equal(actionForKey(key({ key: 'ArrowLeft', alt: true, inField: true }), 'read'), 'back');
  assert.equal(actionForKey(key({ key: 'ArrowLeft', alt: true, shift: true }), 'edit'), null);
  assert.equal(actionForKey(key({ key: 'ArrowLeft', alt: true }), 'palette'), null);
  assert.equal(actionForKey(key({ key: 'ArrowLeft', alt: true }), 'dialog'), null);
  assert.equal(actionForKey(key({ key: 'ArrowRight', alt: true }), 'read'), null);
  assert.equal(actionForKey(key({ key: 'F2' }), 'edit'), 'renameChapter');
  assert.equal(actionForKey(key({ key: 'F2' }), 'read'), null);
  assert.equal(actionForKey(key({ key: 'Escape' }), 'edit', { dragging: true }), 'dragCancel');
  assert.equal(actionForKey(key({ key: 'Escape' }), 'edit'), 'esc');
});
