const READ_KEYS = {
  e: 'enter',
  E: 'enter',
  f: 'focus',
  F: 'focus',
  ArrowRight: 'next',
  ArrowLeft: 'prev',
  ArrowDown: 'lineDown',
  ArrowUp: 'lineUp',
  PageDown: 'pageDown',
  PageUp: 'pageUp',
  ' ': 'pageDown',
};

const EDIT_CTRL_KEYS = { s: 'saveKey', y: 'redo', b: 'bold', i: 'italic' };

const PALETTE_KEYS = {
  Escape: 'paletteClose',
  Enter: 'paletteRun',
  ArrowDown: 'paletteDown',
  ArrowUp: 'paletteUp',
};

const SIZE_KEYS = { '=': 'textLarger', '+': 'textLarger', '-': 'textSmaller', 0: 'textReset' };

const SHELF_KEYS = { ArrowLeft: 'shelfLeft', ArrowRight: 'shelfRight', ArrowUp: 'shelfUp', ArrowDown: 'shelfDown', e: 'shelfEdit', E: 'shelfEdit' };

const MENU_KEYS = { Escape: 'menuClose', Tab: 'menuClose', ArrowDown: 'menuDown', ArrowUp: 'menuUp' };

const ESCAPE_OF_LOOK = { overlay: 'overlayEsc', dialog: 'dialogKeep', palette: 'paletteClose' };

const isCtrl = (key, letter) => key.ctrl && key.key.toLowerCase() === letter;
const isPaletteKey = (key) => (isCtrl(key, 'k') && !key.shift) || (isCtrl(key, 'p') && key.shift);

function escapeAction(key, look, layers) {
  if (ESCAPE_OF_LOOK[look]) return ESCAPE_OF_LOOK[look];
  if (layers.comfort) return 'comfortClose';
  if (layers.contentsOver) return 'paneEsc';
  if (key.inField) return 'paneEsc';
  if (look === 'edit') return 'esc';
  return (layers.focus || layers.fullscreen) && !layers.search ? 'focusExit' : 'paneEsc';
}

function inRead(key) {
  if (key.key === '?' && !key.ctrl) return 'shortcuts';
  if (key.key === ' ' && key.onControl) return null;
  if (key.key === ' ' && key.shift) return 'pageUp';
  if (key.ctrl || key.shift) return null;
  return READ_KEYS[key.key] ?? null;
}

function inEdit(key) {
  if (key.ctrl) {
    const letter = key.key.toLowerCase();
    if (letter === 'z') return key.shift ? 'redo' : 'undo';
    return EDIT_CTRL_KEYS[letter] ?? null;
  }
  if (key.key === 'Enter' && !key.shift) return 'paragraph';
  return null;
}

const inField = (key, look) => (look === 'edit' && isCtrl(key, 's') ? 'saveKey' : null);

const REORDER_KEYS = { ArrowUp: 'moveChapterUp', ArrowDown: 'moveChapterDown' };

function altAction(key, look) {
  if (key.ctrl || key.shift || key.meta || key.composing) return null;
  if (key.key === 'ArrowLeft') return look === 'read' || look === 'edit' ? 'back' : null;
  if (look !== 'edit' || key.inField) return null;
  return REORDER_KEYS[key.key] ?? null;
}

function inShelf(key) {
  if (isCtrl(key, ',') && !key.shift && !key.alt && !key.meta && !key.composing) return 'preferences';
  if (key.alt || key.ctrl || key.meta || key.composing) return null;
  if (key.key === 'ContextMenu' || (key.key === 'F10' && key.shift)) return 'shelfMenu';
  if (key.inField || key.shift) return null;
  if (key.key === 'Enter') return key.onBook || !key.onControl ? 'shelfOpen' : null;
  return SHELF_KEYS[key.key] ?? null;
}

export function actionForKey(key, look, layers = {}) {
  if (look === 'card') return null;
  if (look === 'shelf') return inShelf(key);
  if (look === 'menu') return key.alt || key.ctrl || key.composing ? null : MENU_KEYS[key.key] ?? null;
  if (key.alt) return altAction(key, look);
  if (key.composing || key.meta) return null;
  if (key.key === 'Escape' && layers.dragging) return 'dragCancel';
  if (key.key === 'Escape') return key.ctrl ? null : escapeAction(key, look, layers);
  if (look === 'overlay' || look === 'dialog') return null;
  if (look === 'palette') return isPaletteKey(key) ? 'paletteClose' : PALETTE_KEYS[key.key] ?? null;
  if (key.ctrl && SIZE_KEYS[key.key]) return SIZE_KEYS[key.key];
  if (isPaletteKey(key)) return 'palette';
  if (isCtrl(key, 'f') && !key.shift) return 'search';
  if (isCtrl(key, 't') && !key.shift) return 'contents';
  if (isCtrl(key, 'd') && !key.shift) return 'notes';
  if (isCtrl(key, ',') && !key.shift) return 'preferences';
  if (isCtrl(key, 'a') && key.shift) return 'comfort';
  if (isCtrl(key, 'l') && !key.shift) return 'library';
  if (key.key === 'F1' && !key.ctrl && !key.shift) return 'shortcuts';
  if (key.key === 'F11' && !key.ctrl) return key.shift ? 'focus' : 'fullscreen';
  if (look === 'read' && !key.ctrl && (key.key === 'F3' || (key.key === 'Enter' && key.inSearch))) return key.shift ? 'searchPrev' : 'searchNext';
  if (key.key === 'F2' && !key.ctrl && !key.shift && !key.inField && look === 'edit') return 'renameChapter';
  if (key.inField) return inField(key, look);
  return look === 'edit' ? inEdit(key) : inRead(key);
}

export const describeKey = (event) => ({
  key: event.key,
  ctrl: event.ctrlKey,
  shift: event.shiftKey,
  alt: event.altKey,
  meta: event.metaKey,
  composing: event.isComposing,
  inField: event.target?.localName === 'input' || Boolean(event.target?.closest?.('.rail.right')),
  inSearch: event.target?.id === 'search-input',
  onControl: event.target?.localName === 'button',
  onBook: Boolean(event.target?.closest?.('[data-key]')),
});

const both = (keys) => ({ read: keys, edit: keys });

export const BINDINGS = [
  { group: 'Everywhere', action: 'preferences', label: 'Preferences', buttons: ['open-prefs', 'shelf-prefs'], keys: { read: ['Ctrl ,'], edit: ['Ctrl ,'], shelf: ['Ctrl ,'] } },
  { group: 'Library', action: 'shelfLeft', label: 'Previous book', keys: { shelf: ['←'] } },
  { group: 'Library', action: 'shelfRight', label: 'Next book', keys: { shelf: ['→'] } },
  { group: 'Library', action: 'shelfUp', label: 'Book above', keys: { shelf: ['↑'] } },
  { group: 'Library', action: 'shelfDown', label: 'Book below', keys: { shelf: ['↓'] } },
  { group: 'Library', action: 'shelfOpen', label: 'Resume the selected book', keys: { shelf: ['Enter'] } },
  { group: 'Library', action: 'shelfEdit', label: 'Open the selected book in Edit', keys: { shelf: ['E'] } },
  { group: 'Library', action: 'shelfMenu', label: 'Menu for the selected book', keys: { shelf: ['Menu', 'Shift F10'] } },
  { group: 'Library', action: 'menuClose', label: 'Close the menu', keys: { menu: ['Esc'] } },
  { group: 'Anywhere in a book', action: 'palette', label: 'Jump to a chapter or run a command', buttons: ['open-palette'], keys: both(['Ctrl K', 'Ctrl Shift P']) },
  { group: 'Anywhere in a book', action: 'contents', label: 'Show or hide contents', buttons: ['toggle-contents', 'hide-contents'], keys: both(['Ctrl T']) },
  { group: 'Anywhere in a book', action: 'search', label: 'Search this book', buttons: ['toggle-search'], keys: both(['Ctrl F']) },
  { group: 'Anywhere in a book', action: 'notes', label: 'Bookmarks and notes', buttons: ['toggle-notes'], keys: both(['Ctrl D']) },
  { group: 'Anywhere in a book', action: 'comfort', label: 'Text size, font and spacing', buttons: ['toggle-comfort'], keys: both(['Ctrl Shift A']) },
  { group: 'Anywhere in a book', action: 'library', label: 'Back to the library', buttons: ['go-library'], keys: both(['Ctrl L']) },
  { group: 'Anywhere in a book', action: 'textLarger', label: 'Larger text, also Ctrl with the wheel up', buttons: ['font-larger'], keys: both(['Ctrl +']) },
  { group: 'Anywhere in a book', action: 'textSmaller', label: 'Smaller text, also Ctrl with the wheel down', buttons: ['font-smaller'], keys: both(['Ctrl -']) },
  { group: 'Anywhere in a book', action: 'textReset', label: 'Default text size', keys: both(['Ctrl 0']) },
  { group: 'Anywhere in a book', action: 'focus', label: 'Focus mode, only the text', buttons: ['toggle-focus'], keys: { read: ['F'], edit: ['Shift F11'] } },
  { group: 'Anywhere in a book', action: 'back', label: 'Back to where you jumped from', keys: both(['Alt ←']) },
  { group: 'Anywhere in a book', action: 'fullscreen', label: 'Full screen', buttons: ['toggle-fullscreen'], keys: both(['F11']) },
  { group: 'Anywhere in a book', action: 'shortcuts', label: 'This list of shortcuts', keys: { read: ['F1', '?'], edit: ['F1'] } },
  { group: 'Reading', action: 'enter', label: 'Edit this chapter', buttons: ['mode-edit'], keys: { read: ['E'] } },
  { group: 'Reading', action: 'next', label: 'Next chapter, or next page in page view', keys: { read: ['→'] } },
  { group: 'Reading', action: 'prev', label: 'Previous chapter, or previous page in page view', keys: { read: ['←'] } },
  { group: 'Reading', action: 'pageDown', label: 'Down a screen', keys: { read: ['Space', 'PgDn'] } },
  { group: 'Reading', action: 'pageUp', label: 'Up a screen', keys: { read: ['Shift Space', 'PgUp'] } },
  { group: 'Reading', action: 'lineDown', label: 'Down a little', keys: { read: ['↓'] } },
  { group: 'Reading', action: 'lineUp', label: 'Up a little', keys: { read: ['↑'] } },
  { group: 'Reading', action: 'searchNext', label: 'Next search match', keys: { read: ['F3'] } },
  { group: 'Reading', action: 'searchPrev', label: 'Previous search match', keys: { read: ['Shift F3'] } },
  { group: 'Reading', action: 'paneEsc', label: 'Close the open panel', keys: { read: ['Esc'] } },
  { group: 'Editing', action: 'esc', label: 'Back to reading', buttons: ['mode-read'], keys: { edit: ['Esc'] } },
  { group: 'Editing', action: 'saveKey', label: 'Save the book', buttons: ['save'], keys: { edit: ['Ctrl S'] } },
  { group: 'Editing', action: 'undo', label: 'Undo', buttons: ['tool-undo'], keys: { edit: ['Ctrl Z'] } },
  { group: 'Editing', action: 'redo', label: 'Redo', buttons: ['tool-redo'], keys: { edit: ['Ctrl Shift Z', 'Ctrl Y'] } },
  { group: 'Editing', action: 'bold', label: 'Bold', buttons: ['tool-bold'], keys: { edit: ['Ctrl B'] } },
  { group: 'Editing', action: 'italic', label: 'Italic', buttons: ['tool-italic'], keys: { edit: ['Ctrl I'] } },
  { group: 'Editing', action: 'renameChapter', label: 'Rename this chapter', keys: { edit: ['F2'] } },
  { group: 'Editing', action: 'moveChapterUp', label: 'Move this chapter up', keys: { edit: ['Alt ↑'] } },
  { group: 'Editing', action: 'moveChapterDown', label: 'Move this chapter down', keys: { edit: ['Alt ↓'] } },
];

export const keysOf = (binding, look) => (binding.keys[look] ?? binding.keys.read ?? binding.keys.edit ?? Object.values(binding.keys)[0]).join(' or ');
