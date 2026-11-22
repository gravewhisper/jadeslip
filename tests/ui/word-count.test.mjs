import test from 'node:test';
import assert from 'node:assert/strict';
import { countWords } from '../../app/ui/word-count.js';

test('words are runs between spaces of any kind', () => {
  assert.equal(countWords(''), 0);
  assert.equal(countWords('   '), 0);
  assert.equal(countWords('One'), 1);
  assert.equal(countWords('  Han Mo smiled.\n\nThe end '), 5);
  assert.equal(countWords('non breaking thin　ideographic'), 4);
  assert.equal(countWords("don't stop—now"), 2);
});
