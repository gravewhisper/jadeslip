import { BINDINGS, keysOf } from './keys.js';

const element = (id) => document.getElementById(id);

const LOOK_NAMES = { read: 'Read', edit: 'Edit' };

function keyCells(binding) {
  const looks = Object.entries(binding.keys);
  const same = looks.length > 1 && looks.every(([, keys]) => keys.join() === looks[0][1].join());
  const shown = same ? [[null, looks[0][1]]] : looks;
  return shown.map(([look, keys]) => {
    const cell = document.createElement('span');
    cell.className = 'keyset';
    for (const label of keys) {
      const kbd = document.createElement('kbd');
      kbd.className = 'key';
      kbd.textContent = label;
      cell.append(kbd);
    }
    if (look && looks.length > 1) cell.append(` in ${LOOK_NAMES[look]}`);
    return cell;
  });
}

export function sheetRows() {
  const groups = new Map();
  for (const binding of BINDINGS) {
    if (!groups.has(binding.group)) groups.set(binding.group, []);
    groups.get(binding.group).push(binding);
  }
  return [...groups].map(([name, bindings]) => {
    const section = document.createElement('section');
    section.className = 'keygroup';
    const title = document.createElement('h3');
    title.textContent = name;
    section.append(title);
    for (const binding of bindings) {
      const row = document.createElement('div');
      row.className = 'keyrow';
      const label = document.createElement('span');
      label.textContent = binding.label;
      const keys = document.createElement('span');
      keys.className = 'keys';
      keys.append(...keyCells(binding));
      row.append(label, keys);
      section.append(row);
    }
    return section;
  });
}

const nameOf = (button) => button.getAttribute('aria-label') ?? button.textContent.trim();

export function paintTips(look) {
  for (const button of document.querySelectorAll('#reader .top button, .toolbar button, .rail-actions button')) button.title = nameOf(button);
  for (const binding of BINDINGS) {
    for (const id of binding.buttons ?? []) {
      const button = element(id);
      button.title = `${nameOf(button)} (${keysOf(binding, look)})`;
    }
  }
}

export function createShortcuts({ onClosed }) {
  const scrim = element('keys-dialog');
  const list = element('keys-list');
  const close = element('keys-close');
  let returnTo = null;

  list.replaceChildren(...sheetRows());

  function hide() {
    if (scrim.hidden) return;
    scrim.hidden = true;
    onClosed(returnTo);
  }

  close.addEventListener('click', hide);
  scrim.addEventListener('mousedown', (event) => {
    if (event.target === scrim) {
      event.preventDefault();
      hide();
    }
  });

  return {
    open() {
      returnTo = document.activeElement;
      scrim.hidden = false;
      close.focus();
    },
    close: hide,
    isOpen: () => !scrim.hidden,
  };
}
