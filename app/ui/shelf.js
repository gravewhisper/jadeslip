import {
  SORTS, bookLabel, cardOf, lookOf, menuOf, movingText, openedText, recentOf, removalText, shelfView, sizeText, sortOf, statusText, stepIndex, tabCounts,
} from './shelf-model.js';
import { exportShelfCopy, fillCovers, libraryMoving, removalInfo, removeBook, revealShelfBook, setOwn } from './ipc.js';

const element = (id) => document.getElementById(id);
const SVG = 'http://www.w3.org/2000/svg';

function make(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text !== undefined) node.textContent = text;
  return node;
}

const pointer = (event) => ({ left: event.clientX, right: event.clientX, top: event.clientY, bottom: event.clientY });

function icon(name) {
  const svg = document.createElementNS(SVG, 'svg');
  svg.setAttribute('class', 'icon');
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(SVG, 'use');
  use.setAttribute('href', `#i-${name}`);
  svg.append(use);
  return svg;
}

const buttonOf = (item) => (item.matches('button') ? item : item.querySelector('.cardgo'));

const ICON_OF_STATE = { mine: 'pen-line', finished: 'check' };
const GO_ICON = { mine: 'pencil', reading: 'book-open', new: 'book-open', finished: 'book-open' };

export function createShelf({ openBook, openPicker, addPicker, newBook, switchTheme, themeLabel, reload, showToast, shelfPrefs, setShelfPrefs, onExport, cleanBook }) {
  const lib = element('lib');
  const grid = element('shelf-grid');
  const rows = element('shelf-rows');
  const filter = element('shelf-filter');
  const failure = element('shelf-failure');
  const bookMenu = element('book-menu');
  const sortMenu = element('sort-menu');
  const menus = [bookMenu, sortMenu, element('reader-menu')];
  const removeDialog = element('remove-dialog');
  let books = [];
  let shown = [];
  let chosen = null;
  let tab = 'all';
  let filling = false;
  let menuFor = null;
  let menuReturn = null;
  let removing = null;
  let watching = false;

  const view = () => shelfPrefs().shelfView;
  const byKey = (key) => books.find((book) => book.key === key) ?? null;
  const selected = () => byKey(chosen);

  function coverOf(book, lazy) {
    const look = lookOf(book.key);
    const holder = make('div', `cover ${look.cls} ${look.art}`);
    holder.append(make('span', 'covertitle', book.title));
    if (!book.cover) return holder;
    const img = document.createElement('img');
    img.alt = '';
    img.decoding = 'async';
    if (lazy) img.loading = 'lazy';
    img.addEventListener('error', () => {
      img.remove();
      holder.classList.remove('has');
    });
    img.src = book.cover;
    holder.classList.add('has');
    holder.append(img);
    return holder;
  }

  function progressBar(fraction) {
    const track = make('span', 'track');
    const fill = make('span', 'fill');
    fill.style.width = `${Math.round(fraction * 1000) / 10}%`;
    track.append(fill);
    return track;
  }

  function dotted(parts) {
    const line = make('span', 'dotted');
    parts.forEach((part, index) => {
      if (index > 0) line.append(make('span', 'dot'));
      line.append(make('span', '', part));
    });
    return line;
  }

  function moreButton(book, cls) {
    const more = make('button', `btn ibtn ${cls}`);
    more.setAttribute('aria-label', `More for ${book.title}`);
    more.setAttribute('aria-haspopup', 'menu');
    more.append(icon('ellipsis'));
    more.addEventListener('click', (event) => {
      event.stopPropagation();
      openMenu(book.key, more);
    });
    return more;
  }

  function recentCard(book, index) {
    const card = cardOf(book);
    const big = index === 0;
    const node = make('article', `rcard${big ? ' big' : ''}`);
    node.dataset.key = book.key;
    if (big) node.id = 'hero';
    const body = make('div', 'rbody');
    const kick = make('p', 'kick');
    const kicker = make('span', '', card.kicker);
    if (big) kicker.id = 'hero-kicker';
    kick.append(kicker, make('span', 'dot'), make('span', '', openedText(book.touchedAt, Date.now())));
    const title = make('h3', 'rtitle', book.title);
    const line = make('p', 'pline', card.line);
    if (big) {
      title.id = 'hero-title';
      line.id = 'hero-line';
    }
    body.append(kick, title, line);
    if (big) {
      const quote = make('p', 'quote', card.passage);
      quote.id = 'hero-quote';
      quote.hidden = card.passage === '';
      body.append(quote);
    }
    const prog = make('div', 'prog');
    if (card.progress !== null) prog.append(progressBar(card.progress));
    prog.append(make('span', 'stat', card.stat));
    const acts = make('div', 'racts');
    const go = make('button', `btn ${big ? 'primary' : 'line'}`);
    go.append(icon(GO_ICON[card.state]), make('span', '', card.go.label));
    if (big) go.id = 'hero-go';
    go.addEventListener('click', () => openBook(book.key, card.go));
    acts.append(go, moreButton(book, 'more'));
    body.append(prog, acts);
    node.append(coverOf(book, false), body);
    node.addEventListener('contextmenu', (event) => {
      event.preventDefault();
      openMenu(book.key, go, pointer(event));
    });
    return node;
  }

  function paintRecent() {
    const recent = recentOf(books);
    element('recent-cards').replaceChildren(...recent.map(recentCard));
    element('recent-note').textContent = recent.length === 1 ? 'The last book you opened' : `Last ${recent.length} books you opened`;
    const folded = view() === 'list' && !shelfPrefs().shelfRecent;
    element('recent').classList.toggle('folded', folded);
    element('recent-cards').hidden = folded;
    element('recent-fold').hidden = view() !== 'list';
    element('recent-fold').setAttribute('aria-expanded', String(!folded));
    element('recent-fold-label').textContent = folded ? 'Show' : 'Hide';
    element('recent-fold').querySelector('use').setAttribute('href', folded ? '#i-chevron-down' : '#i-chevron-up');
  }

  function paintTabs() {
    const tabs = tabCounts(books).map((entry) => {
      const button = make('button', `tab${entry.id === tab ? ' on' : ''}`);
      button.id = `shelf-tab-${entry.id}`;
      button.setAttribute('role', 'tab');
      button.setAttribute('aria-selected', String(entry.id === tab));
      button.append(make('span', '', entry.label), make('span', 'tabcount', entry.count.toLocaleString('en-US')));
      button.addEventListener('click', () => {
        tab = entry.id;
        paintBooks();
      });
      return button;
    });
    element('shelf-tabs').replaceChildren(...tabs);
  }

  function gridCard(book) {
    const card = cardOf(book);
    const node = make('div', `card${book.key === chosen ? ' sel' : ''}`);
    node.setAttribute('role', 'listitem');
    node.dataset.key = book.key;
    const go = make('button', 'cardgo');
    go.dataset.key = book.key;
    go.tabIndex = book.key === chosen ? 0 : -1;
    go.setAttribute('aria-current', String(book.key === chosen));
    go.setAttribute('aria-label', `${card.go.label} ${bookLabel(book)}`);
    const cover = coverOf(book, true);
    if (card.state === 'reading') {
      const bar = make('span', 'coverbar');
      bar.append(make('span', 'coverfill'));
      bar.firstChild.style.width = `${Math.round(card.progress * 1000) / 10}%`;
      cover.append(bar);
    }
    if (ICON_OF_STATE[card.state]) {
      const badge = make('span', 'badge');
      badge.append(icon(ICON_OF_STATE[card.state]), make('span', '', card.state === 'mine' ? 'Mine' : 'Finished'));
      cover.append(badge);
    }
    go.append(cover, make('span', 'cardtitle', book.title), dotted(card.meta));
    go.addEventListener('click', () => {
      select(book.key, false);
      openBook(book.key, card.go);
    });
    go.addEventListener('focus', () => select(book.key, false));
    node.append(go, moreButton(book, 'cardmore'));
    node.addEventListener('contextmenu', (event) => {
      event.preventDefault();
      select(book.key, false);
      openMenu(book.key, go, pointer(event));
    });
    return node;
  }

  function listRow(book) {
    const card = cardOf(book);
    const row = make('button', `trow${book.key === chosen ? ' sel' : ''}`);
    row.setAttribute('role', 'listitem');
    row.dataset.key = book.key;
    row.tabIndex = book.key === chosen ? 0 : -1;
    row.setAttribute('aria-label', bookLabel(book));
    row.setAttribute('aria-current', String(book.key === chosen));
    const first = make('span', 'td tdtitle');
    const names = make('span', 'names');
    names.append(make('span', 'rt', book.title), make('span', book.author ? 'ru' : 'ru faint', book.author ?? 'No author in file'));
    first.append(coverOf(book, true), names);
    const progress = make('span', 'td');
    if (card.state === 'reading') progress.append(progressBar(card.progress));
    else if (ICON_OF_STATE[card.state]) progress.append(icon(ICON_OF_STATE[card.state]));
    progress.append(make('span', card.state === 'new' ? 'stat faint' : 'stat', card.stat));
    row.append(
      first,
      progress,
      make('span', 'td num', book.chapters.toLocaleString('en-US')),
      make('span', 'td num', sizeText(book.bytes)),
      make('span', 'td', openedText(book.touchedAt, Date.now())),
    );
    row.addEventListener('click', () => select(book.key, true));
    row.addEventListener('dblclick', () => openBook(book.key, card.go));
    row.addEventListener('contextmenu', (event) => {
      event.preventDefault();
      select(book.key, true);
      openMenu(book.key, row, pointer(event));
    });
    return row;
  }

  function paintDetail() {
    const book = selected();
    element('shelf-detail').hidden = !book;
    if (!book) return;
    const card = cardOf(book);
    element('detail-cover').replaceWith(Object.assign(coverOf(book, false), { id: 'detail-cover' }));
    element('detail-kicker').textContent = card.kicker;
    element('detail-title').textContent = book.title;
    element('detail-author').textContent = book.author ?? '';
    element('detail-line').textContent = card.line;
    element('detail-quote').textContent = card.passage;
    element('detail-quote').hidden = card.passage === '';
    element('detail-prog').firstChild.hidden = card.progress === null;
    element('detail-fill').style.width = `${Math.round((card.progress ?? 0) * 1000) / 10}%`;
    element('detail-stat').textContent = card.stat;
    element('detail-go').replaceChildren(icon(GO_ICON[card.state]), make('span', '', card.go.label));
    const facts = [
      ['Chapters', book.chapters.toLocaleString('en-US')],
      [book.own ? 'Words' : 'Size', book.own ? card.stat : sizeText(book.bytes)],
      ['Last opened', openedText(book.touchedAt, Date.now())],
      ['Edits', book.backedUp ? 'Original backed up' : 'No edits yet'],
    ];
    element('detail-facts').replaceChildren(...facts.flatMap(([name, value]) => [make('dt', '', name), make('dd', '', value)]));
  }

  function paintSort() {
    const sort = sortOf(shelfPrefs().shelfSort);
    element('shelf-sort-label').textContent = SORTS[sort].label;
    for (const head of element('shelf-head').children) {
      const on = head.dataset.sort === sort;
      head.classList.toggle('on', on);
      head.setAttribute('aria-sort', on ? 'descending' : 'none');
    }
  }

  function paintBooks() {
    const sort = sortOf(shelfPrefs().shelfSort);
    shown = shelfView(books, { tab, query: filter.value, sort });
    if (shown.length > 0 && !shown.some((book) => book.key === chosen)) chosen = shown[0].key;
    const list = view() === 'list';
    const empty = books.length === 0;
    element('shelf-empty').hidden = !empty;
    element('recent').hidden = empty;
    element('lib-tools').hidden = empty;
    element('shelf-drop').hidden = empty;
    element('shelf-none').hidden = empty || shown.length > 0;
    grid.hidden = empty || list;
    element('shelf-list').hidden = empty || !list;
    element('view-grid').classList.toggle('on', !list);
    element('view-list').classList.toggle('on', list);
    element('view-grid').setAttribute('aria-pressed', String(!list));
    element('view-list').setAttribute('aria-pressed', String(list));
    paintTabs();
    paintSort();
    paintRecent();
    if (list) {
      grid.replaceChildren();
      rows.replaceChildren(...shown.map(listRow));
      element('shelf-status').replaceChildren(dotted(statusText(books)));
      paintDetail();
    } else {
      rows.replaceChildren();
      grid.replaceChildren(...shown.map(gridCard));
    }
  }

  function itemOf(key) {
    return (view() === 'list' ? rows : grid).querySelector(`[data-key="${key}"]`);
  }

  function mark(key, on) {
    const item = key && itemOf(key);
    if (!item) return null;
    item.classList.toggle('sel', on);
    const button = buttonOf(item);
    button.tabIndex = on ? 0 : -1;
    button.setAttribute('aria-current', String(on));
    return item;
  }

  function select(key, focus) {
    if (chosen !== key) {
      mark(chosen, false);
      chosen = key;
      mark(key, true);
      if (view() === 'list') paintDetail();
    }
    hideFailure();
    const item = focus && itemOf(key);
    if (!item) return;
    const button = buttonOf(item);
    if (document.activeElement !== button) button.focus({ preventScroll: true });
    item.scrollIntoView({ block: 'nearest' });
  }

  function columns() {
    if (view() === 'list') return 1;
    const cards = grid.children;
    if (cards.length === 0) return 1;
    const top = cards[0].offsetTop;
    let across = 0;
    while (across < cards.length && cards[across].offsetTop === top) across += 1;
    return Math.max(1, across);
  }

  function move(direction) {
    if (shown.length === 0) return;
    const index = shown.findIndex((book) => book.key === chosen);
    const step = view() === 'list' && (direction === 'left' || direction === 'right') ? (direction === 'left' ? 'up' : 'down') : direction;
    const next = stepIndex(index, shown.length, columns(), step);
    if (next >= 0) select(shown[next].key, true);
  }

  function openSelected(edit = false) {
    const book = selected();
    if (!book) return;
    const card = cardOf(book);
    openBook(book.key, edit ? card.alt : card.go);
  }

  function placeMenu(menu, box) {
    menu.hidden = false;
    const room = document.querySelector('.app').getBoundingClientRect();
    const width = menu.offsetWidth;
    const height = menu.offsetHeight;
    const left = Math.min(Math.max(8, box.left - room.left), room.width - width - 8);
    const below = box.bottom - room.top + 4;
    const top = below + height > room.height - 8 ? Math.max(8, box.top - room.top - height - 4) : below;
    menu.style.left = `${left}px`;
    menu.style.top = `${top}px`;
  }

  function showMenu(menu, anchor, at = null) {
    closeMenus();
    menuReturn = anchor;
    placeMenu(menu, at ?? anchor.getBoundingClientRect());
    anchor.setAttribute('aria-expanded', 'true');
    menu.querySelector('[role^="menuitem"]').focus();
  }

  function openMenu(key, anchor, at = null) {
    const book = byKey(key);
    if (!book) return;
    menuFor = key;
    const items = Object.fromEntries(menuOf(book).map((item) => [item.id, item]));
    element('menu-go-label').textContent = items.go.label;
    element('menu-alt-label').textContent = items.alt.label;
    element('menu-own-label').textContent = items.own.label;
    element('menu-go-icon').setAttribute('href', `#i-${GO_ICON[cardOf(book).state]}`);
    element('menu-alt-icon').setAttribute('href', book.own ? '#i-book-open' : '#i-pencil');
    showMenu(bookMenu, anchor, at);
  }

  function openSortMenu() {
    const sort = sortOf(shelfPrefs().shelfSort);
    sortMenu.replaceChildren(...Object.entries(SORTS).map(([name, entry]) => {
      const item = make('button', 'menuitem');
      item.id = `sort-${name}`;
      item.setAttribute('role', 'menuitemradio');
      item.setAttribute('aria-checked', String(name === sort));
      const mark = icon('check');
      mark.style.visibility = name === sort ? 'visible' : 'hidden';
      item.append(mark, make('span', 'grow', entry.label));
      item.addEventListener('click', () => {
        closeMenus();
        setSort(name);
      });
      return item;
    }));
    showMenu(sortMenu, element('shelf-sort'));
  }

  function closeMenus(refocus = false) {
    const open = menus.some((menu) => !menu.hidden);
    for (const menu of menus) menu.hidden = true;
    menuReturn?.setAttribute('aria-expanded', 'false');
    if (open && refocus && menuReturn?.isConnected) menuReturn.focus();
    if (!refocus) menuReturn = null;
  }

  function menuStep(by) {
    const menu = menus.find((open) => !open.hidden) ?? bookMenu;
    const items = [...menu.querySelectorAll('[role^="menuitem"]')];
    const at = items.indexOf(document.activeElement);
    items[(at + by + items.length) % items.length]?.focus();
  }

  function setSort(name) {
    setShelfPrefs({ shelfSort: name });
    paintBooks();
  }

  function setView(name) {
    if (view() === name) return;
    setShelfPrefs({ shelfView: name });
    paintBooks();
  }

  async function runMenu(action) {
    const book = byKey(menuFor);
    closeMenus(action !== 'go' && action !== 'alt' && action !== 'clean');
    if (!book) return;
    const card = cardOf(book);
    try {
      if (action === 'go') openBook(book.key, card.go);
      else if (action === 'alt') openBook(book.key, card.alt);
      else if (action === 'clean') cleanBook(book.key, card.go);
      else if (action === 'own') {
        await setOwn(book.key, !book.own);
        await refresh();
      } else if (action === 'folder') await revealShelfBook(book.key);
      else if (action === 'export') {
        const done = await onExport(() => exportShelfCopy(book.key));
        if (done?.state === 'saved') showToast(`Copy of ${book.title} saved.`);
      } else if (action === 'remove') await askRemove(book);
    } catch (wrong) {
      showFailure(String(wrong));
    }
  }

  function paintRemoveButton() {
    const trashing = element('remove-data').checked || element('remove-file').checked;
    element('remove-ok').textContent = trashing ? 'Remove and move to trash' : 'Remove from shelf';
  }

  async function askRemove(book) {
    const info = await removalInfo(book.key);
    const text = removalText(info);
    removing = book.key;
    element('remove-title').textContent = text.title;
    element('remove-text').textContent = text.body;
    element('remove-data-row').hidden = text.data === null;
    element('remove-data-label').textContent = text.data ?? '';
    element('remove-data').checked = false;
    element('remove-file').checked = false;
    element('remove-file-row').hidden = text.file === null;
    element('remove-file-label').textContent = text.file ?? '';
    element('remove-error').hidden = true;
    paintRemoveButton();
    removeDialog.hidden = false;
    element('remove-cancel').focus();
  }

  function cancelRemove() {
    removing = null;
    removeDialog.hidden = true;
    const item = chosen && itemOf(chosen);
    if (item) buttonOf(item).focus();
  }

  async function confirmRemove() {
    const key = removing;
    const book = byKey(key);
    if (!book) return cancelRemove();
    const data = element('remove-data').checked;
    const file = element('remove-file').checked;
    element('remove-ok').disabled = true;
    try {
      await removeBook(key, data, file);
    } catch (wrong) {
      element('remove-error').textContent = String(wrong);
      element('remove-error').hidden = false;
      return;
    } finally {
      element('remove-ok').disabled = false;
    }
    removing = null;
    removeDialog.hidden = true;
    const trashed = [data && 'its data', file && "Jadeslip's copy of the EPUB"].filter(Boolean);
    showToast(trashed.length > 0 ? `Removed ${book.title}. Moved ${trashed.join(' and ')} to the trash.` : `Removed ${book.title} from the shelf.`);
    await refresh();
  }

  function hideFailure() {
    failure.hidden = true;
  }

  function showFailure(message) {
    element('shelf-failure-text').textContent = message;
    failure.hidden = false;
  }

  function fillLater() {
    if (filling) return;
    filling = true;
    requestAnimationFrame(() => setTimeout(async () => {
      try {
        if (await fillCovers().catch(() => 0) > 0) show(await reload());
      } finally {
        filling = false;
      }
    }));
  }

  function paintTheme() {
    const button = element('shelf-theme');
    const label = themeLabel();
    button.setAttribute('aria-label', label);
    button.title = label;
    button.querySelector('use').setAttribute('href', label === 'Light theme' ? '#i-sun' : '#i-moon');
  }

  function paintMoving(state) {
    const text = movingText(state);
    element('shelf-moving-text').textContent = text;
    element('shelf-moving').hidden = text === '';
    element('shelf-moving').classList.toggle('done', state.finished);
  }

  async function watchMoving() {
    if (watching) return;
    watching = true;
    let state = await libraryMoving().catch(() => ({ finished: true, total: 0, done: 0, copied: 0, failed: [] }));
    let seen = false;
    while (!state.finished) {
      seen = true;
      paintMoving(state);
      await new Promise((resolve) => setTimeout(resolve, 150));
      state = await libraryMoving().catch(() => ({ ...state, finished: true }));
    }
    if (seen || state.copied > 0 || state.failed.length > 0) paintMoving(state);
  }

  function selectKey(key) {
    if (!byKey(key)) return;
    if (!shown.some((book) => book.key === key)) {
      tab = 'all';
      filter.value = '';
      paintBooks();
    }
    select(key, true);
  }

  function openKey(key) {
    const book = byKey(key);
    if (book) openBook(key, cardOf(book).go);
  }

  function show(next, fresh = false) {
    void watchMoving();
    books = next;
    if (fresh || !selected()) chosen = recentOf(books)[0]?.key ?? null;
    paintTheme();
    paintBooks();
    fillLater();
  }

  async function refresh() {
    show(await reload());
  }

  bookMenu.addEventListener('click', (event) => {
    const item = event.target.closest('[role="menuitem"]');
    if (item) void runMenu(item.id.replace('menu-', ''));
  });
  document.addEventListener('mousedown', (event) => {
    if (!event.target.closest?.('.menu')) closeMenus();
  }, true);
  lib.addEventListener('scroll', () => closeMenus(), { passive: true });
  element('detail-go').addEventListener('click', () => openSelected());
  element('detail-more').addEventListener('click', () => chosen && openMenu(chosen, element('detail-more')));
  element('shelf-sort').addEventListener('click', () => (sortMenu.hidden ? openSortMenu() : closeMenus(true)));
  element('shelf-head').addEventListener('click', (event) => {
    const head = event.target.closest('[data-sort]');
    if (head) setSort(head.dataset.sort);
  });
  element('view-grid').addEventListener('click', () => setView('grid'));
  element('view-list').addEventListener('click', () => setView('list'));
  element('recent-fold').addEventListener('click', () => {
    setShelfPrefs({ shelfRecent: !shelfPrefs().shelfRecent });
    paintRecent();
  });
  element('remove-data').addEventListener('change', paintRemoveButton);
  element('remove-file').addEventListener('change', paintRemoveButton);
  element('remove-cancel').addEventListener('click', cancelRemove);
  element('remove-ok').addEventListener('click', () => void confirmRemove());
  filter.addEventListener('input', paintBooks);
  element('shelf-open').addEventListener('click', openPicker);
  element('shelf-add').addEventListener('click', addPicker);
  element('shelf-empty-add').addEventListener('click', addPicker);
  element('shelf-moving').addEventListener('click', () => {
    if (element('shelf-moving').classList.contains('done')) element('shelf-moving').hidden = true;
  });
  element('shelf-new').addEventListener('click', newBook);
  element('shelf-empty-new').addEventListener('click', newBook);
  element('shelf-theme').addEventListener('click', () => {
    switchTheme();
    paintTheme();
  });

  return {
    show,
    refresh,
    selectKey,
    openKey,
    showFailure,
    hideFailure,
    move,
    openSelected,
    openMenuForSelected: () => {
      if (!chosen) return;
      const item = itemOf(chosen);
      openMenu(chosen, item ? buttonOf(item) : element('shelf-sort'));
    },
    menuOpen: () => menus.some((menu) => !menu.hidden),
    showMenu,
    closeMenus: () => closeMenus(true),
    menuStep,
    isAsking: () => removing !== null,
    cancelAsking: cancelRemove,
    paintTheme,
  };
}
