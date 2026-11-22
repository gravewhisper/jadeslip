import { cleanupRules, historyList, historyRestore, replaceCancel, replaceNext, replaceStart } from './ipc.js';
import { emptyChoices } from './replace-model.js';
import { WHOLE_BOOK } from './search-model.js';
import { reshapingRun } from './revert.js';
import {
  ALREADY, INTRO, NEWER_SAVE, UNDONE, canClean, changesOf, removingText, countingState, doneText, exclusionsOf, failState, failedText, remaining, ruleCountText, statusText, summaryText, takeBatch, toggleRule,
} from './cleanup-model.js';

const element = (id) => document.getElementById(id);

const newest = (points) => points[0] ?? null;

export function createCleanup({ requestWhole, applyRun, review, undoBar, shownIx, bookReshaped, landOn, refreshPosition, showToast, afterChange, refocus, allowed }) {
  const scrim = element('cleanup-dialog');
  const rulesBox = element('cleanup-rules');
  const progress = element('cleanup-progress');
  const status = element('cleanup-status');
  const stop = element('cleanup-stop');
  const cancel = element('cleanup-cancel');
  const show = element('cleanup-show');
  const apply = element('cleanup-apply');
  const reshape = reshapingRun({ requestWhole, bookReshaped, landOn, refreshPosition, showToast, afterChange });
  let specs = null;
  let nextId = 0;
  let state = null;
  let counts = [];

  element('cleanup-intro').textContent = INTRO;

  function ruleRow(entry) {
    const row = document.createElement('label');
    row.className = 'check cleanup-rule';
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.checked = entry.on;
    box.dataset.rule = entry.rule;
    const words = document.createElement('span');
    words.className = 'grow';
    const label = document.createElement('strong');
    label.textContent = entry.label;
    const detail = document.createElement('small');
    detail.textContent = entry.detail;
    words.append(label, detail);
    const found = document.createElement('span');
    found.className = 'cleanup-count';
    row.append(box, words, found);
    return { row, found };
  }

  function render() {
    if (!state) return;
    state.rules.forEach((entry, at) => { counts[at].textContent = ruleCountText(state, entry); });
    const left = remaining(state, emptyChoices());
    status.textContent = state.phase === 'ready' && statusText(state) === '' ? summaryText(left) : statusText(state);
    stop.hidden = state.phase !== 'counting';
    progress.classList.toggle('counting', state.phase === 'counting');
    show.disabled = state.phase !== 'ready' || changesOf(left) === 0;
    apply.disabled = !canClean(state, left);
  }

  async function count(entry) {
    await replaceStart('clean', entry.previewId, { kind: 'clean', rules: entry.rules.map(({ rule }) => rule) }, WHOLE_BOOK);
    for (;;) {
      const batch = await replaceNext('clean', entry.previewId);
      if (state !== entry) return;
      takeBatch(entry, batch);
      render();
      if (batch.end) return;
    }
  }

  function stopCounting() {
    if (state?.phase === 'counting') void replaceCancel('clean', state.previewId);
  }

  function hide() {
    scrim.hidden = true;
  }

  function close() {
    stopCounting();
    state = null;
    hide();
    refocus();
  }

  async function open() {
    if (!allowed()) {
      showToast('Open a book to clean it up.');
      return;
    }
    try {
      specs ??= await cleanupRules();
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
      return;
    }
    stopCounting();
    nextId += 1;
    state = countingState(specs, nextId);
    const rows = state.rules.map(ruleRow);
    counts = rows.map(({ found }) => found);
    rulesBox.replaceChildren(...rows.map(({ row }) => row));
    render();
    scrim.hidden = false;
    cancel.focus();
    const entry = state;
    count(entry).catch((failure) => {
      if (state !== entry) return;
      failState(entry, failure);
      render();
    });
  }

  function undoTo(point) {
    void reshape(async () => {
      const latest = newest(await historyList());
      if (latest?.id !== point.id) throw new Error(NEWER_SAVE);
      return historyRestore(point.id, { kind: 'wholeBook' }, shownIx());
    }, UNDONE, ALREADY);
  }

  async function offerUndo(end) {
    let point;
    try {
      point = newest(await historyList());
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
      return;
    }
    if (point) undoBar.show(doneText(end), () => undoTo(point));
  }

  function cleanNow() {
    const entry = state;
    const choices = emptyChoices();
    const left = remaining(entry, choices);
    const exclude = exclusionsOf(entry, choices);
    state = null;
    hide();
    return requestWhole(async () => {
      const end = await applyRun('clean', entry.previewId, exclude, changesOf(left), { heading: removingText(left), quiet: true });
      if (end.kind === 'failed') showToast(failedText(end));
      return end.kind === 'done' ? { ...end, after: () => offerUndo(end) } : end;
    });
  }

  function showMatches() {
    const entry = state;
    state = null;
    hide();
    review({ state: entry, cleaned: offerUndo });
  }

  rulesBox.addEventListener('change', (event) => {
    const rule = event.target.dataset?.rule;
    if (!state || !rule) return;
    toggleRule(state, rule);
    render();
  });
  stop.addEventListener('click', stopCounting);
  cancel.addEventListener('click', close);
  show.addEventListener('click', showMatches);
  apply.addEventListener('click', () => void cleanNow());

  return {
    open,
    isAsking: () => !scrim.hidden,
    cancel: close,
    reset: () => {
      stopCounting();
      state = null;
      hide();
    },
  };
}
