import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('../../app/ui/app.css', import.meta.url), 'utf8');
const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({ selector: selector.trim(), body }));
const CONTROL = /\.btn|\.field|\.mini|\.palclose|\.row-tick|\.row-skip|\.seg button|\.tool\b/;

test('chrome type sizes come from the type scale', () => {
  for (const { selector, body } of rules) {
    for (const [, value] of body.matchAll(/font-size:([^;]+)/g)) {
      assert.match(value, /^var\(--(fs-[a-z]+|reader-font,)/, `${selector} font-size ${value}`);
    }
  }
});

test('control heights come from the height scale', () => {
  for (const { selector, body } of rules) {
    if (!CONTROL.test(selector) || selector.includes('.icon')) continue;
    for (const [, value] of body.matchAll(/(?:^|;)(?:min-)?height:([^;]+)/g)) {
      assert.match(value, /^var\(--h-(control|compact|primary)\)$/, `${selector} height ${value}`);
    }
  }
});
