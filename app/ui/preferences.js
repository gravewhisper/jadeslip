import { deleteOriginalBackup, dictionariesInfo, storageClean, storageInfo, storagePreview, storageReveal } from './ipc.js';
import {
  KEEP_PERIODS, backupAsk, cleanedText, isShorter, keepAsk, lastCleanupText, storageRows, wordsText,
} from './preferences-model.js';
import { FONTS, FONT_RANGE, PRESETS, isPreset } from './prefs.js';
import { sizeText } from './shelf-model.js';
import { sheetRows } from './shortcuts.js';
import { languageName } from './spelling.js';

const element = (id) => document.getElementById(id);

const SECTIONS = ['general', 'reading', 'spelling', 'storage', 'shortcuts'];

function make(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

const plainError = (failure) => String(failure?.message ?? failure);

export function createPreferences({ prefs, showToast, onClosed }) {
  const scrim = element('prefs-dialog');
  const confirm = element('prefs-confirm');
  const nav = element('prefs-nav');
  const books = element('prefs-books');
  const booksStatus = element('prefs-books-status');
  const dictionaries = element('prefs-dictionaries');
  const dictionariesStatus = element('prefs-dictionaries-status');
  const cleanNow = element('prefs-clean-now');
  const cleanResult = element('prefs-clean-result');
  let section = 'general';
  let returnTo = null;
  let answer = null;
  let busy = false;
  let storageShown = false;

  const fonts = element('prefs-font');
  for (const [value, face] of Object.entries(FONTS)) fonts.append(new Option(face.label, value));
  element('prefs-keys').replaceChildren(...sheetRows());

  const pick = (pairs) => {
    for (const [id, on] of pairs) element(id).classList.toggle('on', on);
  };

  function paint(now) {
    pick([
      ['prefs-theme-dark', now.theme === 'dark'],
      ['prefs-theme-light', now.theme === 'light'],
      ['prefs-pages-scroll', now.pages === 'scroll'],
      ['prefs-pages-auto', now.pages === 'auto'],
      ['prefs-spelling-on', now.spelling],
      ['prefs-spelling-off', !now.spelling],
      ...Object.entries(PRESETS).map(([name, values]) => [`prefs-preset-${name}`, isPreset(now, values)]),
      ...KEEP_PERIODS.map((period) => [`prefs-keep-${period.value}`, now.keepSnapshots === period.value]),
    ]);
    element('prefs-font-now').textContent = `${now.fontPx}px`;
    element('prefs-font-smaller').disabled = now.fontPx <= FONT_RANGE.min;
    element('prefs-font-larger').disabled = now.fontPx >= FONT_RANGE.max;
    fonts.value = now.font;
  }

  function showSection(name, focus = false) {
    section = name;
    for (const id of SECTIONS) {
      const tab = element(`prefs-nav-${id}`);
      const on = id === name;
      tab.classList.toggle('on', on);
      tab.setAttribute('aria-selected', String(on));
      tab.tabIndex = on ? 0 : -1;
      element(`prefs-section-${id}`).hidden = !on;
    }
    element('prefs-body').scrollTop = 0;
    if (focus) element(`prefs-nav-${name}`).focus();
  }

  function bookRow(book) {
    const row = make('div', 'prow');
    row.setAttribute('role', 'row');
    row.dataset.key = book.key;
    const cell = make('div', 'pbook');
    cell.setAttribute('role', 'cell');
    cell.append(make('span', 'ptitle', book.title));
    const notes = [book.author, book.onShelf ? null : 'Not on the shelf', book.open ? 'Open now' : null].filter(Boolean);
    if (notes.length > 0) cell.append(make('span', 'pnote', notes.join(', ')));
    if (book.backup > 0) {
      const drop = make('button', 'btn small line', 'Delete original backup');
      drop.id = `prefs-backup-delete-${book.key}`;
      drop.dataset.key = book.key;
      drop.addEventListener('click', () => void dropBackup(book));
      cell.append(drop);
    }
    const history = make('span', 'num', sizeText(book.history));
    history.title = `${book.points.toLocaleString('en-US')} automatic, ${book.named.toLocaleString('en-US')} named`;
    const cells = [make('span', 'num', book.book > 0 ? sizeText(book.book) : 'None'), history, make('span', 'num', book.backup > 0 ? sizeText(book.backup) : 'None'), make('span', 'num', sizeText(book.other)), make('span', 'num total', sizeText(book.total))];
    for (const one of cells) one.setAttribute('role', 'cell');
    row.append(cell, ...cells);
    return row;
  }

  function paintStorage(info) {
    storageShown = true;
    element('prefs-data-dir').textContent = info.dataDir;
    element('prefs-total').textContent = sizeText(info.total);
    const rows = storageRows(info.books);
    books.replaceChildren(...rows.map(bookRow));
    booksStatus.textContent = 'No book data yet.';
    booksStatus.hidden = rows.length > 0;
    element('prefs-last-cleanup').textContent = lastCleanupText(info.lastCleanup, info.lastFreed, Date.now());
  }

  async function loadStorage() {
    if (!storageShown) {
      booksStatus.textContent = 'Counting what each book keeps.';
      booksStatus.hidden = false;
    }
    try {
      paintStorage(await storageInfo());
    } catch (failure) {
      showToast(`Storage could not be read. ${plainError(failure)}`);
    }
  }

  function dictionaryRow(language) {
    const row = make('div', 'prow pdict');
    row.setAttribute('role', 'listitem');
    row.id = `prefs-dictionary-${language.code}`;
    const name = make('div', 'pbook');
    name.append(make('span', 'ptitle', language.name ?? languageName(language.code)), make('span', 'pnote', language.code));
    row.append(name, make('span', 'pnote', language.bundled ? 'Built in' : 'Added'), make('span', 'num', wordsText(language.words)));
    return row;
  }

  async function loadDictionaries() {
    try {
      const info = await dictionariesInfo();
      dictionaries.replaceChildren(...info.languages.map(dictionaryRow));
      dictionariesStatus.hidden = true;
    } catch (failure) {
      dictionariesStatus.textContent = 'The dictionaries could not be listed.';
      dictionariesStatus.hidden = false;
      showToast(`The dictionaries could not be listed. ${plainError(failure)}`);
    }
  }

  function ask(question) {
    const back = document.activeElement;
    element('prefs-confirm-title').textContent = question.title;
    element('prefs-confirm-text').textContent = question.text;
    element('prefs-confirm-ok').textContent = question.ok;
    confirm.hidden = false;
    element('prefs-confirm-cancel').focus();
    return new Promise((settle) => {
      answer = (yes) => {
        confirm.hidden = true;
        answer = null;
        (back?.isConnected ? back : element(`prefs-nav-${section}`)).focus({ preventScroll: true });
        settle(yes);
      };
    });
  }

  async function dropBackup(book) {
    if (!await ask(backupAsk(book))) return;
    try {
      await deleteOriginalBackup(book.key);
    } catch (failure) {
      showToast(`The original backup could not be deleted. ${plainError(failure)}`);
    }
    await loadStorage();
  }

  async function guarded(run) {
    if (busy) return;
    busy = true;
    cleanNow.disabled = true;
    try {
      await run();
    } catch (failure) {
      cleanResult.textContent = '';
      showToast(`Old history could not be cleaned up. ${plainError(failure)}`);
    } finally {
      busy = false;
      cleanNow.disabled = false;
    }
  }

  async function clean(keep) {
    cleanResult.textContent = 'Cleaning up';
    cleanResult.textContent = cleanedText(await storageClean(keep));
    await loadStorage();
  }

  const chooseKeep = (keep) => guarded(async () => {
    const saved = prefs.current().keepSnapshots;
    if (keep === saved) return;
    if (!isShorter(keep, saved)) {
      prefs.change({ keepSnapshots: keep });
      return;
    }
    const preview = await storagePreview(keep);
    if (preview.points > 0 && !await ask(keepAsk(preview, keep))) return;
    prefs.change({ keepSnapshots: keep });
    if (preview.points > 0) await clean(keep);
  });

  const reveal = (which) => storageReveal(which).catch((failure) => showToast(`The folder could not be shown. ${plainError(failure)}`));

  function open(opener = null) {
    if (!scrim.hidden) return;
    returnTo = opener ?? document.activeElement;
    cleanResult.textContent = '';
    scrim.hidden = false;
    showSection(section, true);
    void loadDictionaries();
    void loadStorage();
  }

  function close() {
    if (scrim.hidden) return;
    if (answer) answer(false);
    scrim.hidden = true;
    onClosed(returnTo);
    if (scrim.contains(document.activeElement)) document.activeElement.blur();
  }

  prefs.subscribe(paint);
  paint(prefs.current());

  for (const id of SECTIONS) element(`prefs-nav-${id}`).addEventListener('click', () => showSection(id, true));
  nav.addEventListener('keydown', (event) => {
    const by = { ArrowDown: 1, ArrowUp: -1 }[event.key];
    if (!by) return;
    event.preventDefault();
    showSection(SECTIONS[(SECTIONS.indexOf(section) + by + SECTIONS.length) % SECTIONS.length], true);
  });
  element('prefs-theme-dark').addEventListener('click', () => prefs.change({ theme: 'dark' }));
  element('prefs-theme-light').addEventListener('click', () => prefs.change({ theme: 'light' }));
  element('prefs-pages-scroll').addEventListener('click', () => prefs.change({ pages: 'scroll' }));
  element('prefs-pages-auto').addEventListener('click', () => prefs.change({ pages: 'auto' }));
  for (const [name, values] of Object.entries(PRESETS)) element(`prefs-preset-${name}`).addEventListener('click', () => prefs.change(values));
  element('prefs-font-smaller').addEventListener('click', () => prefs.bumpFont(-1));
  element('prefs-font-larger').addEventListener('click', () => prefs.bumpFont(1));
  fonts.addEventListener('change', () => prefs.change({ font: fonts.value }));
  element('prefs-reset-look').addEventListener('click', () => prefs.resetLayout());
  element('prefs-spelling-on').addEventListener('click', () => prefs.setSpelling(true));
  element('prefs-spelling-off').addEventListener('click', () => prefs.setSpelling(false));
  element('prefs-dictionaries-folder').addEventListener('click', () => void reveal('dictionaries'));
  element('prefs-data-reveal').addEventListener('click', () => void reveal('data'));
  for (const period of KEEP_PERIODS) element(`prefs-keep-${period.value}`).addEventListener('click', () => void chooseKeep(period.value));
  cleanNow.addEventListener('click', () => void guarded(() => clean(prefs.current().keepSnapshots)));
  element('prefs-confirm-cancel').addEventListener('click', () => answer?.(false));
  element('prefs-confirm-ok').addEventListener('click', () => answer?.(true));
  element('prefs-close').addEventListener('click', close);
  scrim.addEventListener('mousedown', (event) => {
    if (event.target === scrim) {
      event.preventDefault();
      close();
    }
  });
  confirm.addEventListener('mousedown', (event) => {
    if (event.target === confirm) {
      event.preventDefault();
      answer?.(false);
    }
  });

  return {
    open,
    close,
    escape: () => (answer ? answer(false) : close()),
    isOpen: () => !scrim.hidden,
  };
}
