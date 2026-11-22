import test from 'node:test';
import assert from 'node:assert/strict';
import { BINDINGS, actionForKey } from '../../app/ui/keys.js';

const key = (name, extra = {}) => ({ key: name, ctrl: false, shift: false, alt: false, meta: false, composing: false, inField: false, ...extra });

const CASES = [
  ['read', key('f', { ctrl: true }), 'search'],
  ['read', key('F', { ctrl: true }), 'search'],
  ['edit', key('f', { ctrl: true }), 'search'],
  ['read', key('f', { ctrl: true, inField: true }), 'search'],
  ['edit', key('f', { ctrl: true, inField: true }), 'search'],
  ['read', key('f'), 'focus'],
  ['read', key('F'), 'focus'],
  ['dialog', key('f', { ctrl: true }), null],
  ['read', key('t', { ctrl: true }), 'contents'],
  ['read', key('T', { ctrl: true }), 'contents'],
  ['edit', key('t', { ctrl: true }), 'contents'],
  ['read', key('t', { ctrl: true, inField: true }), 'contents'],
  ['edit', key('t', { ctrl: true, inField: true }), 'contents'],
  ['read', key('T', { ctrl: true, shift: true }), null],
  ['edit', key('T', { ctrl: true, shift: true }), null],
  ['read', key('t', { ctrl: true, alt: true }), null],
  ['read', key('t', { ctrl: true, meta: true }), null],
  ['read', key('t'), null],
  ['shelf', key('t', { ctrl: true }), null],
  ['palette', key('t', { ctrl: true, inField: true }), null],
  ['overlay', key('t', { ctrl: true }), null],
  ['dialog', key('t', { ctrl: true }), null],
  ['read', key('Escape', { inField: true }), 'paneEsc'],
  ['edit', key('Escape', { inField: true }), 'paneEsc'],
  ['edit', key('Enter', { inField: true }), null],
  ['edit', key('z', { ctrl: true, inField: true }), null],
  ['edit', key('b', { ctrl: true, inField: true }), null],
  ['edit', key('s', { ctrl: true, inField: true }), 'saveKey'],
  ['read', key('s', { ctrl: true, inField: true }), null],
  ['read', key('e'), 'enter'],
  ['read', key('E'), 'enter'],
  ['read', key('ArrowRight'), 'next'],
  ['read', key('ArrowLeft'), 'prev'],
  ['read', key('e', { inField: true }), null],
  ['read', key('ArrowRight', { inField: true }), null],
  ['read', key('e', { ctrl: true }), null],
  ['read', key('e', { alt: true }), null],
  ['read', key('Escape'), 'paneEsc'],
  ['read', key('s', { ctrl: true }), null],
  ['read', key('z', { ctrl: true }), null],
  ['read', key('Enter'), null],
  ['edit', key('Escape'), 'esc'],
  ['edit', key('s', { ctrl: true }), 'saveKey'],
  ['edit', key('S', { ctrl: true, shift: true }), 'saveKey'],
  ['edit', key('z', { ctrl: true }), 'undo'],
  ['edit', key('Z', { ctrl: true, shift: true }), 'redo'],
  ['edit', key('y', { ctrl: true }), 'redo'],
  ['edit', key('b', { ctrl: true }), 'bold'],
  ['edit', key('i', { ctrl: true }), 'italic'],
  ['edit', key('Enter'), 'paragraph'],
  ['edit', key('Enter', { shift: true }), null],
  ['edit', key('ArrowRight'), null],
  ['edit', key('ArrowLeft'), null],
  ['edit', key('e'), null],
  ['edit', key('Escape', { composing: true }), null],
  ['edit', key('Enter', { composing: true }), null],
  ['edit', key('Dead', { composing: true }), null],
  ['dialog', key('Escape'), 'dialogKeep'],
  ['dialog', key('e'), null],
  ['dialog', key('ArrowRight'), null],
  ['dialog', key('Enter'), null],
  ['read', key(' '), 'pageDown'],
  ['read', key(' ', { shift: true }), 'pageUp'],
  ['read', key('PageDown'), 'pageDown'],
  ['read', key('PageUp'), 'pageUp'],
  ['read', key('ArrowDown'), 'lineDown'],
  ['read', key('ArrowUp'), 'lineUp'],
  ['read', key(' ', { inField: true }), null],
  ['read', key(' ', { onControl: true }), null],
  ['read', key('ArrowRight', { onControl: true }), 'next'],
  ['edit', key(' '), null],
  ['edit', key('ArrowDown'), null],
  ['edit', key('PageDown'), null],
  ['dialog', key(' '), null],
  ['overlay', key(' '), null],
  ['palette', key(' '), null],
  ['read', key('F11'), 'fullscreen'],
  ['edit', key('F11'), 'fullscreen'],
  ['read', key('F11', { inField: true }), 'fullscreen'],
  ['edit', key('F11', { inField: true }), 'fullscreen'],
  ['read', key('F11', { shift: true }), 'focus'],
  ['edit', key('F11', { shift: true }), 'focus'],
  ['edit', key('F11', { shift: true, inField: true }), 'focus'],
  ['read', key('F11', { ctrl: true }), null],
  ['shelf', key('F11'), null],
  ['dialog', key('F11'), null],
  ['overlay', key('F11'), null],
  ['palette', key('F11'), null],
  ['edit', key('f'), null],
  ['read', key('f', { inField: true }), null],
  ['read', key('k', { ctrl: true }), 'palette'],
  ['read', key('K', { ctrl: true }), 'palette'],
  ['edit', key('k', { ctrl: true }), 'palette'],
  ['read', key('k', { ctrl: true, inField: true }), 'palette'],
  ['edit', key('k', { ctrl: true, inField: true }), 'palette'],
  ['dialog', key('k', { ctrl: true }), null],
  ['overlay', key('k', { ctrl: true }), null],
  ['read', key('k'), null],
  ['palette', key('k', { ctrl: true }), 'paletteClose'],
  ['read', key('P', { ctrl: true, shift: true }), 'palette'],
  ['edit', key('P', { ctrl: true, shift: true }), 'palette'],
  ['read', key('P', { ctrl: true, shift: true, inField: true }), 'palette'],
  ['palette', key('P', { ctrl: true, shift: true }), 'paletteClose'],
  ['dialog', key('P', { ctrl: true, shift: true }), null],
  ['shelf', key('P', { ctrl: true, shift: true }), null],
  ['read', key('p', { ctrl: true }), null],
  ['read', key('K', { ctrl: true, shift: true }), null],
  ['palette', key('Enter', { inField: true }), 'paletteRun'],
  ['palette', key('ArrowDown', { inField: true }), 'paletteDown'],
  ['palette', key('ArrowUp', { inField: true }), 'paletteUp'],
  ['palette', key('ArrowRight', { inField: true }), null],
  ['palette', key('e', { inField: true }), null],
  ['palette', key('f', { ctrl: true, inField: true }), null],
  ['palette', key('s', { ctrl: true, inField: true }), null],
  ['read', key('Enter', { inField: true, inSearch: true }), 'searchNext'],
  ['edit', key('Enter', { inField: true, inSearch: true }), null],
  ['read', key('Enter', { shift: true, inField: true, inSearch: true }), 'searchPrev'],
  ['edit', key('Enter', { shift: true, inField: true, inSearch: true }), null],
  ['read', key('Enter', { ctrl: true, inField: true, inSearch: true }), null],
  ['read', key('Enter', { inField: true }), null],
  ['read', key('Enter', { composing: true, inField: true, inSearch: true }), null],
  ['read', key('F3'), 'searchNext'],
  ['read', key('F3', { shift: true }), 'searchPrev'],
  ['edit', key('F3'), null],
  ['edit', key('F3', { shift: true }), null],
  ['read', key('F3', { inField: true, inSearch: true }), 'searchNext'],
  ['read', key('F3', { shift: true, inField: true }), 'searchPrev'],
  ['read', key('F3', { ctrl: true }), null],
  ['shelf', key('F3'), null],
  ['dialog', key('F3'), null],
  ['overlay', key('F3'), null],
  ['palette', key('F3'), null],
  ['read', key('=', { ctrl: true }), 'textLarger'],
  ['edit', key('+', { ctrl: true, shift: true }), 'textLarger'],
  ['read', key('-', { ctrl: true }), 'textSmaller'],
  ['edit', key('-', { ctrl: true, inField: true }), 'textSmaller'],
  ['read', key('0', { ctrl: true }), 'textReset'],
  ['read', key('0'), null],
  ['read', key('-'), null],
  ['shelf', key('=', { ctrl: true }), null],
  ['palette', key('-', { ctrl: true, inField: true }), null],
  ['dialog', key('0', { ctrl: true }), null],
  ['overlay', key('=', { ctrl: true }), null],
];

for (const [look, input, expected] of CASES) {
  const name = `${look}: ${input.ctrl ? 'ctrl ' : ''}${input.shift ? 'shift ' : ''}${input.composing ? 'composing ' : ''}${input.inSearch ? 'in search ' : input.inField ? 'in field ' : ''}${input.key}`;
  test(name, () => assert.equal(actionForKey(input, look), expected));
}

test('an overlay takes Escape and leaves every other key alone', () => {
  const key = (name, extra = {}) => ({ key: name, ctrl: false, shift: false, alt: false, meta: false, composing: false, inField: false, ...extra });
  assert.equal(actionForKey(key('Escape'), 'overlay'), 'overlayEsc');
  for (const name of ['e', 'ArrowRight', 'Enter', 'Tab']) assert.equal(actionForKey(key(name), 'overlay'), null, name);
  assert.equal(actionForKey(key('f', { ctrl: true }), 'overlay'), null);
  assert.equal(actionForKey(key('s', { ctrl: true }), 'overlay'), null);
});

const ESCAPE_ORDER = [
  ['the apply overlay outranks everything', 'overlay', { comfort: true, focus: true }, false, 'overlayEsc'],
  ['the unsaved dialog outranks focus', 'dialog', { comfort: true, focus: true }, false, 'dialogKeep'],
  ['the palette outranks focus and comfort', 'palette', { comfort: true, focus: true }, true, 'paletteClose'],
  ['the text controls outrank Edit', 'edit', { comfort: true }, false, 'comfortClose'],
  ['the text controls outrank a field', 'read', { comfort: true }, true, 'comfortClose'],
  ['a field outranks Edit', 'edit', {}, true, 'paneEsc'],
  ['a field outranks focus', 'read', { focus: true }, true, 'paneEsc'],
  ['Edit keeps its save meaning in focus', 'edit', { focus: true }, false, 'esc'],
  ['Edit keeps its save meaning with panes', 'edit', {}, false, 'esc'],
  ['Read in focus brings the panes back', 'read', { focus: true }, false, 'focusExit'],
  ['Read in full screen leaves full screen', 'read', { fullscreen: true }, false, 'focusExit'],
  ['the text controls outrank full screen', 'read', { comfort: true, fullscreen: true }, false, 'comfortClose'],
  ['a field outranks full screen', 'read', { fullscreen: true }, true, 'paneEsc'],
  ['Edit keeps its save meaning in full screen', 'edit', { fullscreen: true }, false, 'esc'],
  ['Read with panes closes the search pane', 'read', {}, false, 'paneEsc'],
  ['the search pane outranks focus from the page', 'read', { focus: true, search: true }, false, 'paneEsc'],
  ['the search pane outranks full screen from the page', 'read', { fullscreen: true, search: true }, false, 'paneEsc'],
  ['the search field outranks focus', 'read', { focus: true, search: true }, true, 'paneEsc'],
  ['Edit keeps its save meaning with the search pane in focus', 'edit', { focus: true, search: true }, false, 'esc'],
  ['the text controls outrank the search pane', 'read', { comfort: true, focus: true, search: true }, false, 'comfortClose'],
];

for (const [name, look, layers, inField, expected] of ESCAPE_ORDER) {
  test(`Escape: ${name}`, () => assert.equal(actionForKey(key('Escape', { inField }), look, layers), expected));
}

test('a composing Escape does nothing at any layer', () => {
  for (const look of ['read', 'edit', 'palette', 'overlay', 'dialog']) {
    assert.equal(actionForKey(key('Escape', { composing: true }), look, { focus: true, comfort: true }), null, look);
  }
});

test('the shelf ignores reading keys', () => {
  for (const name of ['f', ' ', 'Escape', 'k', 'PageDown', 'F3']) {
    assert.equal(actionForKey(key(name, { ctrl: name === 'k' }), 'shelf'), null, name);
  }
  assert.equal(actionForKey(key('l', { ctrl: true }), 'shelf'), null);
});

test('the shelf moves the selection, resumes and opens the book menu', () => {
  assert.equal(actionForKey(key('ArrowRight'), 'shelf'), 'shelfRight');
  assert.equal(actionForKey(key('ArrowDown'), 'shelf'), 'shelfDown');
  assert.equal(actionForKey(key('Enter'), 'shelf'), 'shelfOpen');
  assert.equal(actionForKey(key('Enter', { onControl: true, onBook: true }), 'shelf'), 'shelfOpen');
  assert.equal(actionForKey(key('e'), 'shelf'), 'shelfEdit');
  assert.equal(actionForKey(key('ContextMenu'), 'shelf'), 'shelfMenu');
  assert.equal(actionForKey(key('F10', { shift: true }), 'shelf'), 'shelfMenu');
  assert.equal(actionForKey(key('ContextMenu', { onControl: true }), 'shelf'), 'shelfMenu');
});

test('the search box and other buttons keep their own keys on the shelf', () => {
  for (const name of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Enter', 'e']) {
    assert.equal(actionForKey(key(name, { inField: true }), 'shelf'), null, name);
  }
  assert.equal(actionForKey(key('Enter', { onControl: true }), 'shelf'), null);
  assert.equal(actionForKey(key('F10'), 'shelf'), null);
  assert.equal(actionForKey(key('ArrowRight', { shift: true }), 'shelf'), null);
});

test('an open book menu takes the arrows and Escape, and the failure card takes nothing', () => {
  assert.equal(actionForKey(key('ArrowDown'), 'menu'), 'menuDown');
  assert.equal(actionForKey(key('ArrowUp'), 'menu'), 'menuUp');
  assert.equal(actionForKey(key('Escape'), 'menu'), 'menuClose');
  assert.equal(actionForKey(key('Tab'), 'menu'), 'menuClose');
  assert.equal(actionForKey(key('Enter'), 'menu'), null);
  assert.equal(actionForKey(key('ArrowRight'), 'menu'), null);
  for (const name of ['ArrowRight', 'Enter', 'e', 'Escape']) assert.equal(actionForKey(key(name), 'card'), null, name);
});

const NAMED = { Menu: 'ContextMenu', Esc: 'Escape', Space: ' ', PgDn: 'PageDown', PgUp: 'PageUp', '→': 'ArrowRight', '←': 'ArrowLeft', '↑': 'ArrowUp', '↓': 'ArrowDown' };

function pressed(label) {
  const parts = label.split(' ');
  const name = parts.pop();
  const shift = parts.includes('Shift') || name === '?';
  const letter = name.length === 1 && /[A-Z]/.test(name) ? (shift ? name : name.toLowerCase()) : name;
  return key(NAMED[name] ?? letter, { ctrl: parts.includes('Ctrl'), shift, alt: parts.includes('Alt') });
}

test('every listed shortcut does what the list says', () => {
  for (const binding of BINDINGS) {
    for (const [look, labels] of Object.entries(binding.keys)) {
      for (const label of labels) {
        assert.equal(actionForKey(pressed(label), look), binding.action, `${label} in ${look}`);
      }
    }
  }
});

test('Ctrl+, opens Preferences in Read, Edit, a field and the shelf', () => {
  for (const look of ['read', 'edit']) {
    assert.equal(actionForKey(key(',', { ctrl: true }), look), 'preferences', look);
    assert.equal(actionForKey(key(',', { ctrl: true, inField: true }), look), 'preferences', `${look} field`);
  }
  assert.equal(actionForKey(key(',', { ctrl: true }), 'shelf'), 'preferences');
  assert.equal(actionForKey(key(',', { ctrl: true, inField: true }), 'shelf'), 'preferences');
  assert.equal(actionForKey(key(',', { ctrl: true, shift: true }), 'shelf'), null);
  assert.equal(actionForKey(key(',', { ctrl: true, alt: true }), 'shelf'), null);
  assert.equal(actionForKey(key(','), 'shelf'), null);
  for (const look of ['palette', 'overlay', 'dialog', 'menu', 'card']) assert.equal(actionForKey(key(',', { ctrl: true }), look), null, look);
});

test('Ctrl+Shift+A opens the Aa panel, and plain Ctrl+A stays select all', () => {
  for (const look of ['read', 'edit']) {
    assert.equal(actionForKey(key('A', { ctrl: true, shift: true }), look), 'comfort', look);
    assert.equal(actionForKey(key('A', { ctrl: true, shift: true, inField: true }), look), 'comfort', `${look} field`);
    assert.equal(actionForKey(key('a', { ctrl: true }), look), null, look);
    assert.equal(actionForKey(key('a', { ctrl: true, inField: true }), look), null, `${look} field`);
    assert.equal(actionForKey(key('A', { shift: true }), look), null, look);
  }
  assert.equal(actionForKey(key('A', { ctrl: true, shift: true }), 'shelf'), null);
  assert.equal(actionForKey(key('A', { ctrl: true, shift: true }), 'dialog'), null);
});

test('Escape in the Preferences window goes through the dialog look', () => {
  assert.equal(actionForKey(key('Escape'), 'dialog', { comfort: true }), 'dialogKeep');
  assert.equal(actionForKey(key('Escape', { inField: true }), 'dialog'), 'dialogKeep');
  for (const name of ['ArrowDown', 'Enter', 'e', ' ']) assert.equal(actionForKey(key(name), 'dialog'), null, name);
});

test('new keys do not fire from text fields or the shelf', () => {
  assert.equal(actionForKey(key('?', { shift: true, inField: true }), 'read'), null);
  assert.equal(actionForKey(key('?', { shift: true }), 'edit'), null);
  assert.equal(actionForKey(key('F1'), 'shelf'), null);
  assert.equal(actionForKey(key('F1'), 'palette'), null);
});
