import test from 'node:test';
import assert from 'node:assert/strict';
import { PAGE_MODES } from '../../app/ui/pages.js';

const look = (fontPx) => ({ fontPx, marginLeft: 32, marginRight: 32 });

test('the page modes are scroll and auto', () => {
  assert.deepEqual(Object.keys(PAGE_MODES), ['scroll', 'auto']);
  assert.equal(PAGE_MODES.scroll(3000, look(18)), 0);
});

test('auto shows two pages only when each page holds 36 em of text', () => {
  assert.equal(PAGE_MODES.auto(1256, look(14)), 2);
  assert.equal(PAGE_MODES.auto(1256, look(18)), 1);
  assert.equal(PAGE_MODES.auto(960, look(18)), 1);
  assert.equal(PAGE_MODES.auto(1920, look(24)), 2);
  assert.equal(PAGE_MODES.auto(1920, look(25)), 1);
  assert.equal(PAGE_MODES.auto(960, look(14)), 1);
});
