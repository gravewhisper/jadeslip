import { historyChanges, historyDiff, historyList, historyRestore, historySnapshot } from './ipc.js';
import { reshapingRun } from './revert.js';

const element = (id) => document.getElementById(id);
const SHOWN_CHAPTERS = 3;
const KEPT_LINES = 2;
const KIND_TEXT = { changed: 'changed since', added: 'added since', removed: 'removed since' };

const timeOf = (at) => new Date(at).toLocaleString('en-US', {
  month: 'short',
  day: 'numeric',
  hour: 'numeric',
  minute: '2-digit',
});

function summary(chapters) {
  if (chapters.length === 0) return '';
  const shown = chapters.slice(0, SHOWN_CHAPTERS).join(', ');
  const more = chapters.length - SHOWN_CHAPTERS;
  return more > 0 ? `${shown} and ${more.toLocaleString('en-US')} more` : shown;
}

function collapsed(text) {
  const lines = text.split('\n');
  if (lines.length <= KEPT_LINES * 2 + 1) return text;
  return [...lines.slice(0, KEPT_LINES), '…', ...lines.slice(-KEPT_LINES)].join('\n');
}

const DIFF_TAG = { same: 'span', gone: 'del', new: 'ins' };

export function createHistory({ shownIx, requestWhole, bookReshaped, landOn, refreshPosition, refocus, showToast, afterChange }) {
  const scrim = element('history-dialog');
  const title = element('history-title');
  const take = element('history-take-row');
  const nameField = element('history-name');
  const list = element('history-list');
  const empty = element('history-empty');
  const pointView = element('history-point');
  const pointText = element('history-point-text');
  const changes = element('history-changes');
  const diffView = element('history-diff');
  const backButton = element('history-back');
  const wholeButton = element('history-whole');
  const closeButton = element('history-close');
  const reshape = reshapingRun({ requestWhole, bookReshaped, landOn, refreshPosition, showToast, afterChange });
  let points = [];
  let view = { kind: 'list' };

  const failed = (failure) => showToast(String(failure?.message ?? failure));

  function heading(point) {
    if (point.kind === 'named') return point.name;
    if (point.kind === 'save') return 'Saved';
    const target = points.find((found) => found.id === point.to);
    return target ? `Restored to ${timeOf(target.at)}` : 'Restored';
  }

  function show(next) {
    view = next;
    take.hidden = view.kind !== 'list';
    list.hidden = view.kind !== 'list';
    empty.hidden = view.kind !== 'list' || points.length > 0;
    pointView.hidden = view.kind !== 'point';
    diffView.hidden = view.kind !== 'diff';
    backButton.hidden = view.kind === 'list';
    wholeButton.hidden = view.kind !== 'point' || !view.whole;
    title.textContent = view.kind === 'diff' ? `${view.row.label}: then and now` : 'History';
  }

  function pointRow(point) {
    const row = document.createElement('button');
    row.className = 'history-row';
    row.dataset.point = String(point.id);
    const when = document.createElement('span');
    when.className = 'history-when';
    when.textContent = timeOf(point.at);
    const what = document.createElement('span');
    what.className = point.kind === 'named' ? 'history-what named' : 'history-what';
    what.textContent = heading(point);
    const chapters = document.createElement('span');
    chapters.className = 'history-chapters';
    chapters.textContent = point.kind === 'named' ? 'Snapshot' : summary(point.chapters);
    row.append(when, what, chapters);
    row.addEventListener('click', () => void openPoint(point));
    return row;
  }

  async function showList() {
    try {
      points = await historyList();
    } catch (failure) {
      failed(failure);
      points = [];
    }
    list.replaceChildren(...points.map(pointRow));
    show({ kind: 'list' });
  }

  function changeRow(point, row) {
    const line = document.createElement('div');
    line.className = 'deleted-row history-change';
    line.dataset.entry = row.entry;
    const label = document.createElement('span');
    label.textContent = row.label;
    const kind = document.createElement('em');
    kind.textContent = KIND_TEXT[row.kind];
    const compare = document.createElement('button');
    compare.className = 'btn line small history-compare';
    compare.textContent = 'Compare';
    compare.addEventListener('click', () => void openDiff(point, row));
    line.append(label, kind, compare);
    if (row.kind === 'changed') {
      const restore = document.createElement('button');
      restore.className = 'btn line small history-restore';
      restore.textContent = 'Restore';
      restore.addEventListener('click', () => {
        close();
        void reshape(
          () => historyRestore(point.id, { kind: 'chapter', entry: row.entry }, shownIx()),
          'Chapter restored to that point.',
          'This chapter already matches that point.',
        );
      });
      line.append(restore);
    }
    return line;
  }

  async function openPoint(point) {
    let found;
    try {
      found = await historyChanges(point.id);
    } catch (failure) {
      failed(failure);
      return;
    }
    const parts = [`${heading(point)}, ${timeOf(point.at)}.`];
    if (found.rows.length === 0) parts.push('The book matches this point.');
    else parts.push('These differ from the book now:');
    if (!found.complete) parts.push('Some saves after it were not recorded, so only single chapters can be restored.');
    pointText.textContent = parts.join(' ');
    changes.replaceChildren(...found.rows.map((row) => changeRow(point, row)));
    show({ kind: 'point', point, whole: found.complete && found.rows.length > 0 });
  }

  async function openDiff(point, row) {
    let parts;
    try {
      parts = await historyDiff(point.id, row.entry);
    } catch (failure) {
      failed(failure);
      return;
    }
    diffView.replaceChildren(...parts.map((part) => {
      const node = document.createElement(DIFF_TAG[part.kind]);
      node.textContent = part.kind === 'same' ? collapsed(part.text) : part.text;
      return node;
    }));
    show({ kind: 'diff', point, row, whole: false });
    diffView.scrollTop = 0;
  }

  async function takeSnapshot() {
    const name = nameField.value.trim();
    if (name === '') {
      showToast('Give the snapshot a name.');
      nameField.focus();
      return;
    }
    try {
      await historySnapshot(name);
    } catch (failure) {
      failed(failure);
      return;
    }
    nameField.value = '';
    showToast('Snapshot taken.');
    await showList();
  }

  function close() {
    scrim.hidden = true;
    refocus();
  }

  async function open() {
    nameField.value = '';
    await showList();
    scrim.hidden = false;
    closeButton.focus();
  }

  function back() {
    if (view.kind === 'diff') void openPoint(view.point);
    else void showList();
  }

  element('history-take').addEventListener('click', () => void takeSnapshot());
  nameField.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    void takeSnapshot();
  });
  backButton.addEventListener('click', back);
  closeButton.addEventListener('click', close);
  wholeButton.addEventListener('click', () => {
    if (view.kind !== 'point') return;
    const { point } = view;
    close();
    void reshape(
      () => historyRestore(point.id, { kind: 'wholeBook' }, shownIx()),
      'The whole book is back to that point.',
      'The book already matches that point.',
    );
  });

  return { open, isAsking: () => !scrim.hidden, cancel: close };
}
