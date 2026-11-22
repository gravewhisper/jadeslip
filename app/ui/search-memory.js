import { searchesGet, searchesSet } from './ipc.js';
import {
  LIMITS, clearRecent, emptyMemory, forget, moveHighlight, nameFrom, record, remove, rename, restore, rowsOf, save, sameQuery, savedOf, tagOf,
} from './search-memory-model.js';

const SVG = 'http://www.w3.org/2000/svg';
const element = (id) => document.getElementById(id);

function icon(name) {
  const svg = document.createElementNS(SVG, 'svg');
  svg.setAttribute('class', 'icon');
  svg.setAttribute('aria-hidden', 'true');
  const use = document.createElementNS(SVG, 'use');
  use.setAttribute('href', name);
  svg.append(use);
  return svg;
}

function mini(name, label, act) {
  const button = document.createElement('button');
  button.className = 'mini';
  button.setAttribute('aria-label', label);
  button.title = label;
  button.append(icon(name));
  button.addEventListener('click', act);
  return button;
}

function span(className, text) {
  const node = document.createElement('span');
  node.className = className;
  node.textContent = text;
  return node;
}

function head(title, action) {
  const row = document.createElement('div');
  row.className = 'sshead';
  row.append(span('sstitle', title));
  if (action) row.append(action);
  return row;
}

const label = (button, text) => {
  button.setAttribute('aria-label', text);
  button.title = text;
};

export function createSearchMemory({ input, currentQuery, run, checkCurrent, showToast, undoBar }) {
  const menu = element('search-memory');
  const saveButton = element('search-save');
  const listButton = element('search-saved');
  const form = element('search-save-form');
  const nameInput = element('search-save-name');
  const formError = element('search-save-error');
  const chips = element('search-chips');
  const chipMenu = element('search-chip-menu');
  let chipFor = null;
  let memory = emptyMemory();
  let touched = false;
  let writing = Promise.resolve();
  let open = false;
  let filter = '';
  let rows = [];
  let highlight = null;
  let renaming = null;
  let pending = null;

  nameInput.maxLength = LIMITS.nameChars;
  label(listButton, 'Saved and recent searches');

  function commit(next) {
    if (next === memory) return;
    memory = next;
    touched = true;
    writing = writing
      .then(() => searchesSet(next))
      .catch((failure) => showToast(`Your saved searches could not be stored. ${failure?.message ?? failure}`));
    paint();
  }

  function rowButton(row, at) {
    const button = document.createElement('button');
    button.className = 'ssrun';
    button.append(icon(row.kind === 'saved' ? '#i-bookmark' : '#i-history'));
    if (row.kind === 'saved') button.append(span('ssname', row.name), span('sstext', row.query.text));
    else button.append(span('sstext', row.query.text));
    button.append(span('sstag', tagOf(row.query)));
    button.addEventListener('click', () => runRow(at));
    return button;
  }

  function renameField(row) {
    const field = document.createElement('input');
    field.className = 'field ssrename';
    field.maxLength = LIMITS.nameChars;
    field.value = row.name;
    field.setAttribute('aria-label', 'New name for this saved search');
    field.addEventListener('keydown', (event) => {
      if (event.isComposing) return;
      if (event.key === 'Enter') {
        event.preventDefault();
        event.stopPropagation();
        finishRename(row.name, field.value);
      } else if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        renaming = null;
        paint();
        input.focus();
      }
    });
    field.addEventListener('blur', () => {
      if (renaming === row.name) {
        renaming = null;
        paint();
      }
    });
    return field;
  }

  function rowOf(row, at) {
    const line = document.createElement('div');
    line.className = 'ssrow';
    line.setAttribute('role', 'option');
    line.setAttribute('aria-selected', String(at === highlight));
    line.classList.toggle('on', at === highlight);
    line.classList.toggle('current', sameQuery(row.query, currentQuery()));
    if (row.kind === 'saved') {
      line.append(
        renaming === row.name ? renameField(row) : rowButton(row, at),
        mini('#i-pencil', 'Rename saved search', () => startRename(row.name)),
        mini('#i-trash-2', 'Delete saved search', () => deleteSaved(row.name)),
      );
    } else {
      line.append(
        rowButton(row, at),
        mini('#i-bookmark-plus', 'Save this search', () => askName(row.query, false)),
        mini('#i-x', 'Remove from recent searches', () => commit(forget(memory, row.query))),
      );
    }
    return line;
  }

  function clearButton() {
    const button = document.createElement('button');
    button.className = 'btn small';
    button.textContent = 'Clear history';
    button.addEventListener('click', clearHistory);
    return button;
  }

  function paintList() {
    rows = rowsOf(memory, filter);
    if (highlight !== null && highlight >= rows.length) highlight = null;
    const shown = open && rows.length > 0;
    menu.hidden = !shown;
    listButton.classList.toggle('on', shown);
    listButton.setAttribute('aria-expanded', String(shown));
    if (!shown) {
      menu.replaceChildren();
      return;
    }
    const children = [];
    const savedCount = rows.filter((row) => row.kind === 'saved').length;
    rows.forEach((row, at) => {
      if (at === 0 && savedCount > 0) children.push(head('Saved'));
      if (at === savedCount) children.push(head('Recent', clearButton()));
      children.push(rowOf(row, at));
    });
    menu.replaceChildren(...children);
    if (renaming !== null) menu.querySelector('.ssrename')?.focus();
    menu.querySelector('.ssrow.on')?.scrollIntoView({ block: 'nearest' });
  }

  function hideChipMenu() {
    chipFor = null;
    chipMenu.hidden = true;
  }

  function toggleChipMenu(name, chip) {
    if (chipFor === name) {
      hideChipMenu();
      return;
    }
    chipFor = name;
    chipMenu.hidden = false;
    const from = chipMenu.offsetParent.getBoundingClientRect();
    const box = chip.getBoundingClientRect();
    chipMenu.style.left = `${Math.min(box.left - from.left, from.width - chipMenu.offsetWidth)}px`;
    chipMenu.style.top = `${box.bottom - from.top + 4}px`;
    chipMenu.querySelector('button').focus();
  }

  function chipOf(row) {
    const chip = document.createElement('div');
    chip.className = 'sschip';
    chip.classList.toggle('current', sameQuery(row.query, currentQuery()));
    const go = document.createElement('button');
    go.className = 'chiprun';
    go.textContent = row.name;
    go.title = `${row.query.text} (${tagOf(row.query)})`;
    go.addEventListener('click', () => {
      hideChipMenu();
      run(row.query);
    });
    const more = mini('#i-ellipsis', `More for "${row.name}"`, () => toggleChipMenu(row.name, chip));
    more.setAttribute('aria-haspopup', 'menu');
    chip.append(go, more);
    return chip;
  }

  function paintChips() {
    const saved = rowsOf(memory, '').filter((row) => row.kind === 'saved');
    chips.hidden = saved.length === 0;
    if (!saved.some((row) => row.name === chipFor)) hideChipMenu();
    chips.replaceChildren(...(saved.length ? [span('chipshead', 'Saved searches'), ...saved.map(chipOf)] : []));
  }

  function paintActive() {
    const entry = savedOf(memory, currentQuery());
    saveButton.classList.toggle('on', Boolean(entry));
    saveButton.querySelector('use').setAttribute('href', entry ? '#i-bookmark-check' : '#i-bookmark-plus');
    label(saveButton, entry ? `Saved as "${entry.name}"` : 'Save this search');
  }

  function paint() {
    paintActive();
    paintList();
    paintChips();
  }

  function openList(filtered) {
    open = true;
    filter = filtered ? input.value : '';
    highlight = null;
    paintList();
  }

  function closeList() {
    if (!open) return;
    open = false;
    highlight = null;
    renaming = null;
    paintList();
  }

  function runRow(at) {
    const row = rows[at];
    if (!row) return;
    closeList();
    run(row.query);
  }

  function startRename(name) {
    renaming = name;
    paintList();
  }

  function finishRename(from, to) {
    const result = rename(memory, from, to);
    if (result.error) {
      showToast(result.error);
      return;
    }
    renaming = null;
    commit(result.memory);
    input.focus();
  }

  function deleteSaved(name) {
    const { memory: next, removed } = remove(memory, name);
    if (!removed) return;
    commit(next);
    undoBar.show(`Deleted saved search "${name}".`, () => commit(restore(memory, removed)));
  }

  function clearHistory() {
    const previous = memory.recent;
    commit(clearRecent(memory));
    undoBar.show('Cleared recent searches.', () => {
      const kept = previous.filter((old) => !memory.recent.some((now) => sameQuery(now, old)));
      commit({ ...memory, recent: [...memory.recent, ...kept].slice(0, LIMITS.recent) });
    });
    input.focus();
  }

  function showFormError(text) {
    formError.textContent = text;
    formError.hidden = !text;
  }

  function askName(query, fromField) {
    closeList();
    pending = { query: fromField ? null : query, fromField };
    form.hidden = false;
    showFormError('');
    nameInput.value = savedOf(memory, query)?.name ?? nameFrom(query.text);
    nameInput.focus();
    nameInput.select();
  }

  function closeForm() {
    pending = null;
    form.hidden = true;
    showFormError('');
  }

  async function confirmName() {
    const asked = pending;
    if (!asked) return;
    if (asked.fromField) {
      const problem = await checkCurrent();
      if (pending !== asked) return;
      if (problem) {
        showFormError(`This regex has an error, so it was not saved. ${problem}`);
        return;
      }
    }
    const result = save(memory, nameInput.value, asked.query ?? currentQuery());
    if (result.error) {
      showFormError(result.error);
      return;
    }
    closeForm();
    commit(result.memory);
    showToast(`Saved "${result.name}".`);
    input.focus();
  }

  function onSaveClick() {
    const query = currentQuery();
    if (query.text.length === 0) {
      showToast('Type a search first, then save it.');
      input.focus();
      return;
    }
    if (!form.hidden && pending?.fromField) {
      closeForm();
      input.focus();
      return;
    }
    askName(query, true);
  }

  function onKey(event) {
    if (event.isComposing || event.ctrlKey || event.altKey || event.metaKey) return false;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      if (!open) openList(false);
      const by = event.key === 'ArrowDown' ? 1 : -1;
      if (rows.length === 0) return false;
      highlight = moveHighlight(rows, highlight, by);
      paintList();
      return true;
    }
    if (event.key === 'Enter' && !event.shiftKey && open) {
      if (highlight !== null) {
        runRow(highlight);
        return true;
      }
      closeList();
      return false;
    }
    if (event.key === 'Escape' && open) {
      const shown = rows.length > 0;
      closeList();
      return shown;
    }
    return false;
  }

  saveButton.addEventListener('click', onSaveClick);
  element('search-chip-rename').addEventListener('click', () => {
    const name = chipFor;
    hideChipMenu();
    openList(false);
    startRename(name);
  });
  element('search-chip-delete').addEventListener('click', () => {
    const name = chipFor;
    hideChipMenu();
    deleteSaved(name);
  });
  chipMenu.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    hideChipMenu();
    input.focus();
  });
  document.addEventListener('mousedown', (event) => {
    if (!chipMenu.hidden && !chipMenu.contains(event.target) && !event.target.closest?.('.sschip')) hideChipMenu();
  });
  listButton.addEventListener('click', () => {
    if (open && rows.length > 0) closeList();
    else {
      openList(false);
      if (rows.length === 0) showToast('No saved or recent searches yet. Searches you run show up here.');
    }
    input.focus();
  });
  element('search-save-ok').addEventListener('click', () => void confirmName());
  element('search-save-cancel').addEventListener('click', () => {
    closeForm();
    input.focus();
  });
  nameInput.addEventListener('keydown', (event) => {
    if (event.isComposing) return;
    if (event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
      void confirmName();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      closeForm();
      input.focus();
    }
  });
  input.addEventListener('click', () => {
    if (!open) openList(false);
  });
  input.addEventListener('input', (event) => {
    if (event.isComposing) return;
    open = true;
    filter = input.value;
    highlight = null;
    paint();
  });
  input.addEventListener('blur', (event) => {
    if (!menu.contains(event.relatedTarget)) closeList();
  });

  searchesGet()
    .then((loaded) => {
      if (!touched) {
        memory = loaded;
        paint();
      }
    })
    .catch(() => {});

  return {
    record(query) {
      commit(record(memory, query));
    },
    paint,
    openList: () => openList(false),
    closeList,
    closeForm,
    onKey,
    saved: () => memory.saved,
  };
}
