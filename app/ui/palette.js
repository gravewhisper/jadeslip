import { footerHint, paletteRows } from './palette-model.js';

const element = (id) => document.getElementById(id);

export function createPalette({ tocData, commands, openChapter, bookChapters, onClosed }) {
  const scrim = element('palette');
  const input = element('palette-input');
  const list = element('palette-rows');
  const empty = element('palette-empty');
  const foot = element('palette-foot');
  let rows = [];
  let chosen = 0;
  let returnTo = null;

  function paintChosen() {
    [...list.children].forEach((child, at) => child.classList.toggle('on', at === chosen));
  }

  function render() {
    rows = paletteRows(input.value, tocData(), commands());
    chosen = 0;
    list.replaceChildren(...rows.map((row, at) => {
      const button = document.createElement('button');
      button.className = 'palrow';
      button.dataset.at = String(at);
      const label = document.createElement('span');
      label.className = 'pallabel';
      label.textContent = row.label;
      const hint = document.createElement('span');
      hint.className = 'palhint';
      hint.textContent = row.hint;
      button.append(label, hint);
      return button;
    }));
    empty.hidden = rows.length > 0;
    paintChosen();
  }

  const isOpen = () => !scrim.hidden;

  function open() {
    returnTo = document.activeElement;
    input.value = '';
    foot.textContent = footerHint(bookChapters());
    scrim.hidden = false;
    input.focus();
    render();
  }

  function close(restore = true) {
    if (!isOpen()) return;
    scrim.hidden = true;
    if (restore) onClosed(returnTo);
  }

  function runRow(row) {
    if (!row) return;
    close(row.kind !== 'chapter');
    if (row.kind === 'chapter') {
      input.blur();
      openChapter(row.ix, row.fragment);
    } else {
      row.run();
    }
  }

  function move(by) {
    if (rows.length === 0) return;
    chosen = (chosen + by + rows.length) % rows.length;
    paintChosen();
  }

  input.addEventListener('input', render);
  list.addEventListener('click', (event) => {
    const button = event.target.closest('.palrow');
    if (button) runRow(rows[Number(button.dataset.at)]);
  });
  scrim.addEventListener('mousedown', (event) => {
    if (event.target === scrim) {
      event.preventDefault();
      close();
    }
  });
  element('palette-close').addEventListener('click', () => close());
  element('open-palette').disabled = false;

  return {
    open,
    close,
    isOpen,
    runChosen: () => runRow(rows[chosen]),
    down: () => move(1),
    up: () => move(-1),
    refresh: () => {
      if (isOpen()) render();
    },
  };
}
