import { headingsCancel, headingsNext, headingsStart, loadToc } from './ipc.js';
import { dropSpot, rowActions } from './chapters-model.js';
import {
  ROW_HEIGHT, buildToc, countText, queryOf, rowLabel, rowOfChapter, scrollToKeep, setHeading, visibleRows, windowOf,
} from './toc-model.js';

const MAX_INDENT_DEPTH = 6;
const INDENT_PX = 14;
const PARENT_INDENT_PX = 8;
const LEAF_INDENT_PX = 22;
const DRAG_START_PX = 5;
const DRAG_EDGE_PX = 40;
const DRAG_SCROLL_PX = 18;
const element = (id) => document.getElementById(id);

function buildSlot(spacer) {
  const row = document.createElement('button');
  row.className = 'row';
  const chevron = document.createElement('span');
  chevron.className = 'chev';
  chevron.setAttribute('aria-hidden', 'true');
  const number = document.createElement('span');
  number.className = 'num';
  const title = document.createElement('span');
  title.className = 't';
  row.append(chevron, number, title);
  spacer.append(row);
  return { row, chevron, number, title, entry: -1 };
}

export function createToc({ openEntry, afterFilterEsc, onLoaded, chapters }) {
  const input = element('filter');
  const list = element('toc-list');
  const spacer = element('toc-spacer');
  const empty = element('toc-empty');
  const footer = element('count');
  const slots = [];
  const closed = new Set();
  let toc = null;
  let loadTurn = 0;
  let shown = [];
  let query = '';
  let current = null;
  let currentEntry = null;
  let headingId = 0;
  let filling = false;
  let filled = 0;
  let managing = false;
  let actionRow = null;
  let renaming = null;
  let drag = null;
  let swallowClick = false;
  const actions = element('rail-actions');
  const renameInput = element('rail-rename');
  const dropLine = element('rail-drop');

  function placeActions() {
    const at = actionRow ?? (currentEntry === null ? -1 : shown.indexOf(currentEntry));
    const allowed = managing && toc && !query && !renaming && !drag && at >= 0 && at < shown.length;
    actions.hidden = !allowed;
    if (!allowed) return;
    const offered = rowActions(toc, shown[at]);
    actions.dataset.entry = String(shown[at]);
    actions.style.transform = `translateY(${at * ROW_HEIGHT}px)`;
    element('rail-up').hidden = !offered.move;
    element('rail-down').hidden = !offered.move;
    element('rail-rename-open').hidden = !offered.rename;
    element('rail-delete').hidden = !offered.delete;
  }

  function paintRow(slot, at) {
    const entry = toc.entries[shown[at]];
    const indent = Math.min(entry.depth, MAX_INDENT_DEPTH) * INDENT_PX + (entry.isParent ? PARENT_INDENT_PX : LEAF_INDENT_PX);
    slot.row.hidden = false;
    slot.row.style.transform = `translateY(${at * ROW_HEIGHT}px)`;
    slot.row.style.paddingLeft = `${indent}px`;
    slot.row.dataset.entry = String(shown[at]);
    if (slot.entry !== shown[at]) {
      slot.entry = shown[at];
      slot.title.textContent = entry.label;
      slot.number.textContent = rowLabel(entry);
    }
    const isCurrent = shown[at] === currentEntry;
    slot.row.className = `row${entry.isParent ? ' vol' : ''}${isCurrent ? ' cur' : ''}`;
    if (isCurrent) slot.row.setAttribute('aria-current', 'true');
    else slot.row.removeAttribute('aria-current');
    if (entry.isParent) {
      slot.chevron.textContent = closed.has(entry.key) ? '›' : '⌄';
      slot.row.setAttribute('aria-expanded', String(!closed.has(entry.key)));
    } else {
      slot.chevron.textContent = '';
      slot.row.removeAttribute('aria-expanded');
    }
  }

  function renderWindow() {
    if (!toc) return;
    const { first, end } = windowOf(list.scrollTop, list.clientHeight, shown.length);
    while (slots.length < end - first) slots.push(buildSlot(spacer));
    slots.forEach((slot, at) => {
      const index = first + at;
      if (index < end) paintRow(slot, index);
      else {
        slot.row.hidden = true;
        slot.entry = -1;
      }
    });
    placeActions();
  }

  function refreshCurrent() {
    if (!toc || current === null) return;
    const row = rowOfChapter(toc, shown, current);
    currentEntry = row >= 0 ? shown[row] : null;
  }

  function keepCurrentInView() {
    if (!toc || current === null || query) return;
    const row = Math.max(0, rowOfChapter(toc, shown, current));
    const next = scrollToKeep(row, list.scrollTop, list.clientHeight);
    if (next !== list.scrollTop) list.scrollTop = next;
  }

  function showFooter() {
    if (!toc) return;
    const count = countText(toc, shown.length, query);
    footer.textContent = filling ? `${count}. Titling chapters, ${filled.toLocaleString('en-US')} so far` : count;
  }

  function reshape() {
    shown = visibleRows(toc, closed, query);
    spacer.style.height = `${shown.length * ROW_HEIGHT}px`;
    empty.hidden = shown.length > 0;
    refreshCurrent();
    showFooter();
  }

  function setQuery(text) {
    const next = queryOf(text);
    if (next === query) return;
    query = next;
    list.scrollTop = 0;
    reshape();
    if (!query) keepCurrentInView();
    renderWindow();
  }

  function toggle(at) {
    const { key } = toc.entries[at];
    if (closed.has(key)) closed.delete(key);
    else closed.add(key);
    reshape();
    renderWindow();
  }

  function startRename(entry) {
    if (!toc || !rowActions(toc, entry).rename) return;
    const at = shown.indexOf(entry);
    if (at < 0) return;
    const next = scrollToKeep(at, list.scrollTop, list.clientHeight);
    if (next !== list.scrollTop) list.scrollTop = next;
    renaming = entry;
    renameInput.value = toc.entries[entry].label;
    renameInput.style.transform = `translateY(${at * ROW_HEIGHT}px)`;
    renameInput.hidden = false;
    placeActions();
    renameInput.focus();
    renameInput.select();
  }

  function endRename() {
    renaming = null;
    renameInput.hidden = true;
    placeActions();
  }

  function rowAtPointer(clientY) {
    const box = list.getBoundingClientRect();
    return Math.floor((clientY - box.top + list.scrollTop) / ROW_HEIGHT);
  }

  function paintDrop() {
    if (!drag?.spot) {
      dropLine.hidden = true;
      return;
    }
    dropLine.hidden = false;
    dropLine.style.transform = `translateY(${drag.lineAt * ROW_HEIGHT}px)`;
  }

  function aimDrag(clientY) {
    const box = list.getBoundingClientRect();
    if (clientY < box.top + DRAG_EDGE_PX) list.scrollTop -= DRAG_SCROLL_PX;
    else if (clientY > box.bottom - DRAG_EDGE_PX) list.scrollTop += DRAG_SCROLL_PX;
    const offset = clientY - box.top + list.scrollTop;
    const row = Math.floor(offset / ROW_HEIGHT);
    const lower = offset / ROW_HEIGHT - row >= 0.5;
    const spot = row >= 0 && row < shown.length ? dropSpot(toc, shown, row, lower, drag.entry) : null;
    drag.spot = spot;
    drag.lineAt = lower ? row + 1 : row;
    paintDrop();
  }

  function endDrag(commit) {
    const ended = drag;
    drag = null;
    dropLine.hidden = true;
    document.removeEventListener('mousemove', onDragMove);
    document.removeEventListener('mouseup', onDragUp);
    list.classList.remove('dragging');
    placeActions();
    if (commit && ended?.started && ended.spot) chapters.move(ended.entry, ended.spot);
  }

  function onDragMove(event) {
    if (!drag) return;
    if (!drag.started) {
      if (Math.abs(event.clientY - drag.fromY) < DRAG_START_PX) return;
      drag.started = true;
      swallowClick = true;
      list.classList.add('dragging');
      placeActions();
    }
    aimDrag(event.clientY);
  }

  function onDragUp() {
    endDrag(true);
  }

  function onRowPress(event) {
    if (!managing || query || event.button !== 0 || !toc) return;
    const slot = event.target.closest('.row');
    if (!slot) return;
    const entry = Number(slot.dataset.entry);
    if (!rowActions(toc, entry).move) return;
    drag = { entry, fromY: event.clientY, started: false, spot: null, lineAt: 0 };
    document.addEventListener('mousemove', onDragMove);
    document.addEventListener('mouseup', onDragUp);
  }

  function onRowHover(event) {
    if (!managing || drag || renaming !== null) return;
    if (event.target.closest('.rail-actions')) return;
    const row = rowAtPointer(event.clientY);
    const next = row >= 0 && row < shown.length ? row : null;
    if (next !== actionRow) {
      actionRow = next;
      placeActions();
    }
  }

  function onRowClick(event) {
    if (swallowClick) {
      swallowClick = false;
      return;
    }
    const slot = event.target.closest('.row');
    if (!slot || !toc) return;
    const at = Number(slot.dataset.entry);
    const entry = toc.entries[at];
    if (event.target.closest('.chev') && entry.isParent) {
      toggle(at);
      return;
    }
    if (entry.ix === null) {
      if (entry.isParent) toggle(at);
      return;
    }
    open(entry);
  }

  function open(entry) {
    input.blur();
    openEntry(entry.ix, entry.fragment);
  }

  async function pullHeadings(id) {
    while (id === headingId) {
      const batch = await headingsNext(id);
      if (id !== headingId) return;
      let changed = false;
      for (const [ix, title] of batch.titles) {
        if (setHeading(toc, ix, title)) {
          filled += 1;
          changed = true;
        }
      }
      if (batch.done) filling = false;
      if (changed || batch.done) {
        if (query) reshape();
        else showFooter();
        for (const slot of slots) slot.entry = -1;
        renderWindow();
      }
      if (batch.done) return;
    }
  }

  function startHeadings() {
    headingId += 1;
    filling = true;
    filled = 0;
    const id = headingId;
    headingsStart(id).then(() => pullHeadings(id)).catch(() => {
      filling = false;
      showFooter();
    });
  }

  function stopHeadings() {
    if (filling) void headingsCancel(headingId);
    headingId += 1;
    filling = false;
  }

  async function load() {
    stopHeadings();
    const turn = ++loadTurn;
    let view;
    try {
      view = await loadToc();
    } catch (failure) {
      if (turn !== loadTurn) return;
      footer.textContent = `The table of contents could not be read. ${failure?.message ?? failure}`;
      return;
    }
    if (turn !== loadTurn) return;
    toc = buildToc(view);
    for (const slot of slots) slot.entry = -1;
    input.disabled = false;
    reshape();
    keepCurrentInView();
    renderWindow();
    onLoaded();
    if (toc.source === 'headings') startHeadings();
  }

  input.addEventListener('input', () => setQuery(input.value));
  input.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter' || event.isComposing || !toc) return;
    const first = shown.map((at) => toc.entries[at]).find((entry) => entry.ix !== null);
    if (first) open(first);
  });
  list.addEventListener('scroll', renderWindow, { passive: true });
  list.addEventListener('click', onRowClick);
  list.addEventListener('mousedown', onRowPress);
  list.addEventListener('mousemove', onRowHover);
  list.addEventListener('mouseleave', () => {
    actionRow = null;
    placeActions();
  });
  element('rail-up').addEventListener('click', () => chapters.step(Number(actions.dataset.entry), -1));
  element('rail-down').addEventListener('click', () => chapters.step(Number(actions.dataset.entry), 1));
  element('rail-rename-open').addEventListener('click', () => chapters.beginRename(Number(actions.dataset.entry)));
  element('rail-delete').addEventListener('click', () => chapters.remove(Number(actions.dataset.entry)));
  renameInput.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter' || event.isComposing) return;
    event.preventDefault();
    const entry = renaming;
    const title = renameInput.value;
    endRename();
    if (entry !== null) chapters.rename(entry, title);
  });
  renameInput.addEventListener('blur', () => {
    if (renaming !== null) endRename();
  });
  new ResizeObserver(renderWindow).observe(list);

  return {
    load,
    data: () => toc,
    labelOf(ix) {
      if (!toc) return null;
      const at = toc.index.chosen.get(ix);
      return at === undefined ? null : toc.entries[at].label;
    },
    setCurrent(ix, title) {
      current = ix;
      if (!toc) return;
      if (toc.source === 'headings' && title && setHeading(toc, ix, title)) {
        for (const slot of slots) slot.entry = -1;
      }
      refreshCurrent();
      keepCurrentInView();
      renderWindow();
    },
    shown() {
      keepCurrentInView();
      renderWindow();
    },
    escape() {
      if (input.value) {
        input.value = '';
        setQuery('');
      } else {
        input.blur();
        afterFilterEsc();
      }
    },
    isFilter: (target) => target === input,
    stopHeadings,
    reset() {
      stopHeadings();
      loadTurn += 1;
      if (renaming !== null) endRename();
      input.value = '';
      query = queryOf('');
      closed.clear();
      toc = null;
      shown = [];
      spacer.style.height = '0px';
      renderWindow();
    },
    setManaging(on) {
      managing = on;
      element('add-chapter').hidden = !on;
      if (!on && renaming !== null) endRename();
      if (!on && drag) endDrag(false);
      placeActions();
    },
    isRenaming: (target) => target === renameInput,
    cancelRename: endRename,
    startRename,
    isDragging: () => Boolean(drag?.started),
    cancelDrag: () => endDrag(false),
    entryOfChapter: (ix) => (toc ? toc.index.chosen.get(ix) ?? null : null),

  };
}
