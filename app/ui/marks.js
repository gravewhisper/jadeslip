import { addMark, deleteMark, listMarks, resolveMark } from './ipc.js';
import { selectedRange } from './textmap.js';
import { placeName } from './chapter-number.js';

const element = (id) => document.getElementById(id);

function whereOf(row) {
  if (row.state === 'detached') return 'Detached. This passage can no longer be found.';
  return placeName(row.label, row.number);
}

export function createMarks({ host, chapterNow, placeNow, hasUnsaved, jumpTo, showToast, onOpen }) {
  const pane = element('notes-pane');
  const toggle = element('toggle-notes');
  const list = element('notes-list');
  const input = element('note-text');
  const add = element('note-add');
  let held = null;
  let refreshTurn = 0;

  function paintAdd() {
    add.textContent = held ? 'Bookmark this passage' : 'Bookmark this chapter';
  }

  function noteSelection() {
    if (document.activeElement === input || !host.shadowRoot) return;
    held = selectedRange(host.shadowRoot);
    paintAdd();
  }

  function rowOf(row) {
    const line = document.createElement('div');
    line.className = `note${row.state === 'detached' ? ' detached' : ''}`;
    line.dataset.mark = String(row.id);
    line.dataset.state = row.state;
    const jump = document.createElement('button');
    jump.className = 'notejump';
    jump.disabled = row.state === 'detached';
    const where = document.createElement('span');
    where.className = 'notewhere';
    where.textContent = whereOf(row);
    const text = document.createElement('span');
    text.className = 'notetext';
    text.textContent = row.note ?? (row.quote || 'Bookmark');
    jump.append(where, text);
    jump.addEventListener('click', () => open(row.id));
    const remove = document.createElement('button');
    remove.className = 'mini';
    remove.setAttribute('aria-label', 'Delete note');
    remove.textContent = '×';
    remove.addEventListener('click', () => removeMark(row.id));
    line.append(jump, remove);
    return line;
  }

  async function refresh() {
    if (pane.hidden) return;
    const turn = ++refreshTurn;
    try {
      const rows = await listMarks();
      if (turn === refreshTurn) list.replaceChildren(...rows.map(rowOf));
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
    }
  }

  async function open(id) {
    try {
      const row = await resolveMark(id);
      if (row.state === 'detached') {
        showToast('This passage can no longer be found.');
        await refresh();
        return;
      }
      await jumpTo(row);
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
    }
  }

  async function removeMark(id) {
    try {
      await deleteMark(id);
      await refresh();
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
    }
  }

  async function addOne() {
    const chapter = chapterNow();
    if (!chapter) return;
    if (hasUnsaved()) {
      showToast('Save this chapter first, so the bookmark can find its place.');
      return;
    }
    const point = placeNow() ?? { block: 0, offset_u16: 0 };
    const at = held ?? { start: point, end: point };
    try {
      await addMark(chapter.ix, at, input.value.trim() || null);
      input.value = '';
      held = null;
      paintAdd();
      showToast('Bookmark added.');
      await refresh();
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
    }
  }

  function setOpen(open) {
    pane.hidden = !open;
    document.querySelector('.app').classList.toggle('noting', open);
    toggle.classList.toggle('on', open);
    if (open) {
      onOpen();
      void refresh();
    }
  }

  document.addEventListener('selectionchange', noteSelection);
  add.addEventListener('click', addOne);
  input.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && !event.isComposing) void addOne();
  });
  toggle.addEventListener('click', () => setOpen(pane.hidden));

  return {
    refresh,
    open: () => setOpen(true),
    close: () => setOpen(false),
    toggle: () => setOpen(pane.hidden),
    isOpen: () => !pane.hidden,
    addOne,
    isField: (target) => target === input,
    forgetSelection() {
      held = null;
      paintAdd();
    },
    reset() {
      refreshTurn += 1;
      held = null;
      input.value = '';
      list.replaceChildren();
      paintAdd();
    },
  };
}
