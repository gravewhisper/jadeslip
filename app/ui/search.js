import { locateHits, replaceStart, searchCancel, searchNext, searchStart } from './ipc.js';
import {
  ROW_HEIGHT, WHOLE_BOOK, WHOLE_RANGE, applyBatch, applyLabel, barText, emptyResults, failResults, filterTo, footerText, inRange, indexFrom, rangeStrip, reanchored, rerunFor, toggled,
  sameRev, snippetBefore, statusOf, stepIndex, whereOf, windowOf, wireOf,
} from './search-model.js';
import {
  canApply, canChange, emptyChoices, exclusionsOf, isChapterOut, isHitOut, noteOf, previewFooter, remaining, summaryText, toggleChapter, toggleHit,
} from './replace-model.js';
import { blockLength, blocksOf, pointsAt } from './textmap.js';
import { createSearchMemory } from './search-memory.js';
import {
  canClean, changesOf, exclusionsOf as cleanExclusions, failedText, headerText, labelOf, previewFooter as cleanFooter, remaining as cleanRemaining, removingText, summaryText as cleanSummary, viewOf,
} from './cleanup-model.js';
import { sameQuery } from './search-memory-model.js';

const ALL_HITS = 'ss-search';
const CURRENT_HIT = 'ss-search-current';
const MAX_PAINTED = 2000;
const PLAIN_DELAY_MS = 60;
const REGEX_DELAY_MS = 150;
const NOTED_BEFORE_CHARS = 10;
const CHECK_MS = 6000;
const CHECK_POLL_MS = 30;

const element = (id) => document.getElementById(id);

function buildRow() {
  const row = document.createElement('button');
  row.className = 'result';
  const where = document.createElement('span');
  where.className = 'where';
  const snip = document.createElement('span');
  snip.className = 'snip';
  const before = document.createTextNode('');
  const mark = document.createElement('span');
  mark.className = 'mark';
  const inserted = document.createElement('ins');
  const after = document.createTextNode('');
  snip.append(before, mark, inserted, after);
  const note = document.createElement('span');
  note.className = 'note';
  note.hidden = true;
  row.append(where, snip, note);
  const tick = document.createElement('button');
  tick.className = 'row-tick';
  tick.textContent = '\u2713';
  tick.setAttribute('aria-label', 'Change this match');
  tick.hidden = true;
  const skip = document.createElement('button');
  skip.className = 'row-skip';
  skip.hidden = true;
  return { row, where, before, mark, inserted, after, note, tick, skip };
}

function setRowText(slot, { chapter, hit }, note) {
  slot.where.textContent = whereOf(chapter);
  slot.before.data = note.length > 0 ? snippetBefore(hit.before, NOTED_BEFORE_CHARS) : snippetBefore(hit.before);
  slot.mark.textContent = hit.matched;
  slot.inserted.textContent = hit.replaced ?? '';
  slot.after.data = hit.after;
  slot.note.textContent = note;
  slot.note.hidden = note.length === 0;
  slot.row.classList.toggle('noted', note.length > 0);
  slot.row.classList.toggle('cleaning', Boolean(hit.rule));
}

function setRowChoice(slot, row, choices, inPreview) {
  const changeable = inPreview && canChange(row.hit);
  const chapterOut = isChapterOut(choices, row);
  const hitOut = isHitOut(choices, row);
  slot.row.classList.toggle('preview', inPreview);
  slot.row.classList.toggle('out', inPreview && (chapterOut || hitOut || !canChange(row.hit)));
  slot.tick.hidden = !changeable;
  slot.tick.disabled = chapterOut;
  slot.tick.setAttribute('aria-pressed', String(!hitOut && !chapterOut));
  slot.skip.hidden = !inPreview;
  slot.skip.textContent = chapterOut ? 'Keep chapter' : 'Skip chapter';
  slot.skip.setAttribute('aria-pressed', String(chapterOut));
}

function rangesFrom(shadow, located) {
  const root = shadow.querySelector('.ss-body') ?? shadow;
  const blocks = new Map([...blocksOf(root)].map((block) => [Number(block.getAttribute('data-ss-b')), block]));
  const wanted = new Map();
  const ask = (point) => {
    const block = blocks.get(point.block);
    if (!block) return null;
    if (!wanted.has(block)) wanted.set(block, []);
    const query = { units: point.offset_u16, found: null };
    wanted.get(block).push(query);
    return query;
  };
  const queries = located.map(({ start, end }) => ({ start: ask(start), end: ask(end) }));
  for (const [block, asked] of wanted) {
    asked.sort((a, b) => a.units - b.units);
    pointsAt(block, asked.map((query) => query.units)).forEach((point, at) => { asked[at].found = point; });
  }
  const lastBlock = [...blocks.values()].pop();
  return queries.flatMap(({ start, end }) => {
    if (!start) return [];
    const stop = end?.found ?? (lastBlock && pointsAt(lastBlock, [blockLength(lastBlock)])[0]);
    if (!stop) return [];
    const range = document.createRange();
    range.setStart(start.found.node, start.found.offset);
    range.setEnd(stop.node, stop.offset);
    return [range];
  });
}

export function createSearch({ host, reveal, chapterNow, hereNow, placeNow, inRead, openHit, onLeap, showToast, onClosed, requestApply, applyWhole, onOpen, keepPlace, matchCase, setMatchCase, undoBar }) {
  const app = document.querySelector('.app');
  const pane = element('search-pane');
  const toggle = element('toggle-search');
  const input = element('search-input');
  const plain = element('search-plain');
  const regex = element('search-regex');
  const rangeBox = element('search-range');
  const sideButtons = { before: element('search-before'), after: element('search-after') };
  const hereButton = element('search-here');
  const status = element('search-status');
  const scanned = element('search-scanned');
  const fill = element('search-fill');
  const action = element('search-action');
  const message = element('search-message');
  const list = element('search-results');
  const spacer = element('search-spacer');
  const footer = element('search-count');
  const where = element('search-where');
  const caseToggle = element('search-case');
  const replaceToggle = element('replace-toggle');
  const replaceSection = element('replace-section');
  const replaceInput = element('replace-input');
  const replaceSummary = element('replace-summary');
  const replaceApply = element('replace-apply');
  const options = pane.querySelector('.searchopts');
  const cleanHead = element('clean-head');
  const cleanRules = element('clean-rules');

  let range = WHOLE_RANGE;
  let results = emptyResults();
  let currentId = 0;
  let running = null;
  let regexMode = false;
  let asBar = false;
  let pendingStart = null;
  let renderQueued = false;
  let paintTurn = 0;
  let replaceOpen = false;
  let choices = emptyChoices();
  let fixRequest = null;
  let previewId = 0;
  let applyNote = '';
  let current = null;
  let queuedStep = 0;
  let jumping = 0;
  let placing = null;
  let lastRun = null;
  let recordOnEnd = false;
  let usedWhilePending = false;
  let cleaning = null;
  const slots = [];

  const currentQuery = () => ({ text: input.value, regex: regexMode, matchCase: matchCase() });
  const inPreview = () => cleaning !== null || (replaceOpen && previewId === currentId);
  const noteFor = (hit) => (cleaning ? labelOf(cleaning.state, hit.rule) : noteOf(hit));

  function forgetRows() {
    for (const slot of slots) slot.index = -1;
  }

  function leaveCleaning() {
    if (!cleaning) return;
    cleaning = null;
    forgetRows();
  }

  function renderList() {
    const { first, end } = windowOf(list.scrollTop, list.clientHeight, results.shown.length);
    while (slots.length < end - first) {
      const slot = buildRow();
      spacer.append(slot.row, slot.tick, slot.skip);
      slots.push(slot);
    }
    slots.forEach((slot, at) => {
      const index = first + at;
      if (index >= end) {
        slot.row.hidden = true;
        slot.tick.hidden = true;
        slot.skip.hidden = true;
        slot.index = -1;
        return;
      }
      slot.row.hidden = false;
      const top = `${index * ROW_HEIGHT}px`;
      slot.row.style.top = top;
      slot.tick.style.top = top;
      slot.skip.style.top = top;
      slot.row.dataset.index = String(index);
      slot.tick.dataset.index = String(index);
      slot.skip.dataset.index = String(index);
      if (slot.index !== index) {
        slot.index = index;
        setRowText(slot, results.shown[index], noteFor(results.shown[index].hit));
      }
      slot.row.classList.toggle('current', index === current);
      setRowChoice(slot, results.shown[index], choices, inPreview());
    });
  }

  function render() {
    renderQueued = false;
    const shown = statusOf(results);
    status.hidden = !shown.visible || cleaning !== null;
    scanned.textContent = shown.scanned;
    fill.style.width = shown.fill;
    action.textContent = shown.action;
    message.hidden = results.phase !== 'bad' && !applyNote;
    message.textContent = applyNote || results.message;
    paintRange();
    renderReplace();
    footer.textContent = footerOf();
    where.textContent = barText(results, current);
    spacer.style.height = `${results.shown.length * ROW_HEIGHT}px`;
    renderList();
  }

  function paintRange() {
    const shown = rangeStrip(range, hereNow());
    for (const [key, button] of Object.entries(sideButtons)) {
      const side = shown[key];
      button.classList.toggle('on', side.on);
      button.setAttribute('aria-pressed', String(side.on));
      button.disabled = side.disabled;
      button.title = side.hint;
    }
    hereButton.textContent = shown.here.label;
    hereButton.title = shown.here.hint;
    hereButton.disabled = shown.here.disabled;
    input.placeholder = shown.placeholder;
  }

  function footerOf() {
    if (cleaning) return cleanFooter(cleaning.state, results.shown.length);
    return results.preview ? previewFooter(results.preview, results.shown.length) : footerText(results, current);
  }

  function renderReplace() {
    options.hidden = cleaning !== null;
    regex.hidden = cleaning !== null;
    caseToggle.hidden = cleaning !== null;
    rangeBox.hidden = cleaning !== null;
    cleanHead.hidden = cleaning === null;
    replaceToggle.hidden = cleaning !== null;
    replaceInput.hidden = cleaning !== null;
    if (cleaning) {
      const left = cleanRemaining(cleaning.state, choices);
      replaceSection.hidden = false;
      replaceApply.textContent = 'Clean up';
      cleanRules.textContent = headerText(cleaning.state);
      replaceSummary.textContent = cleanSummary(left);
      replaceApply.disabled = !canClean(cleaning.state, left);
      return;
    }
    replaceSection.hidden = !replaceOpen;
    replaceToggle.textContent = replaceOpen ? 'Hide replace' : 'Replace';
    replaceApply.textContent = applyLabel(results.range);
    const preview = replaceOpen ? results.preview : null;
    const left = preview ? remaining(preview, results, choices) : { changes: 0, chapters: 0 };
    replaceSummary.textContent = preview ? summaryText(preview, left) : '';
    replaceApply.disabled = !canApply(preview, left, results.phase === 'running');
  }

  function requestRender() {
    if (renderQueued) return;
    renderQueued = true;
    requestAnimationFrame(render);
  }

  async function pull(id) {
    while (id === currentId) {
      const batch = await searchNext(id);
      if (id !== currentId) return;
      const mounted = chapterNow();
      const touchesMounted = mounted && batch.groups.some((group) => group.ix === mounted.ix);
      applyBatch(results, batch);
      if (queuedStep !== 0 && results.shown.length > 0 && !placing) {
        const by = queuedStep;
        queuedStep = 0;
        step(by);
      }
      requestRender();
      if (touchesMounted) repaint();
      if (batch.end) {
        running = null;
        if (recordOnEnd && lastRun && results.phase !== 'bad') memory.record(lastRun);
        recordOnEnd = false;
        return;
      }
    }
  }

  function cancelRunning() {
    clearTimeout(pendingStart);
    pendingStart = null;
    if (running !== null) {
      const id = running;
      running = null;
      currentId += 1;
      void searchCancel(id);
    }
  }

  function start(wire = replaceOpen ? wireOf(range) : WHOLE_BOOK) {
    cancelRunning();
    const text = input.value;
    lastRun = text.length > 0 && !fixRequest ? currentQuery() : null;
    recordOnEnd = usedWhilePending;
    usedWhilePending = false;
    results = emptyResults(text.length === 0 ? 'idle' : 'running', wire, range);
    current = null;
    choices = emptyChoices();
    list.scrollTop = 0;
    clearHighlights();
    render();
    if (text.length === 0) return;
    currentId += 1;
    running = currentId;
    const id = currentId;
    const started = replaceOpen
      ? replaceStart('pane', id, fixRequest ?? { kind: 'text', text, regex: regexMode, caseSensitive: matchCase(), replacement: replaceInput.value }, wire)
      : searchStart(id, { text, regex: regexMode, caseSensitive: matchCase(), wholeWord: false }, wire);
    if (replaceOpen) previewId = id;
    started
      .then(() => pull(id))
      .catch((failure) => {
        if (id !== currentId) return;
        failResults(results, failure);
        running = null;
        requestRender();
      });
  }

  function scheduleStart() {
    leaveCleaning();
    fixRequest = null;
    applyNote = '';
    usedWhilePending = false;
    cancelRunning();
    results.phase = 'running';
    render();
    pendingStart = setTimeout(start, regexMode ? REGEX_DELAY_MS : PLAIN_DELAY_MS);
  }

  function setRange(next) {
    if (next === range) return;
    range = next;
    applyNote = '';
    const rerun = rerunFor(results, range, replaceOpen);
    if (rerun) {
      start(rerun);
      return;
    }
    const was = current === null ? null : results.shown[current];
    filterTo(results, range);
    forgetRows();
    const at = was ? results.shown.indexOf(was) : -1;
    current = at >= 0 ? at : null;
    if (current === null) list.scrollTop = 0;
    const mounted = chapterNow();
    if (mounted && !inRange(range, mounted.ix)) clearHighlights();
    else repaint();
    render();
    if (current === null) return;
    keepRowInView();
    renderList();
  }

  function paintMode() {
    plain.classList.toggle('on', !regexMode);
    regex.classList.toggle('on', regexMode);
  }

  function setMode(toRegex) {
    if (toRegex === regexMode) return;
    regexMode = toRegex;
    paintMode();
    memory.paint();
    scheduleStart();
  }

  function noteUse() {
    if (pendingStart !== null) {
      usedWhilePending = true;
      return;
    }
    if (!lastRun) return;
    if (results.phase === 'running') recordOnEnd = true;
    else if (results.phase !== 'bad' && results.phase !== 'idle') memory.record(lastRun);
  }

  function recordIfDone() {
    if (lastRun && pendingStart === null && results.phase === 'done') memory.record(lastRun);
  }

  function runQuery(query) {
    if (pane.hidden) setOpen(true);
    memory.closeForm();
    input.value = query.text;
    regexMode = query.regex;
    paintMode();
    setMatchCase(query.matchCase);
    leaveCleaning();
    fixRequest = null;
    applyNote = '';
    start();
    recordOnEnd = true;
    memory.paint();
    memory.closeList();
  }

  async function checkCurrent() {
    if (pendingStart === null && !(lastRun && sameQuery(lastRun, currentQuery()))) start();
    const until = performance.now() + CHECK_MS;
    const unsettled = () => pendingStart !== null || (results.phase === 'running' && results.done === 0 && results.rows.length === 0);
    while (unsettled() && performance.now() < until) await new Promise((wake) => setTimeout(wake, CHECK_POLL_MS));
    return results.phase === 'bad' ? results.message : null;
  }

  function clearHighlights() {
    paintTurn += 1;
    CSS.highlights.delete(ALL_HITS);
    CSS.highlights.delete(CURRENT_HIT);
  }

  async function paint(chapter, target) {
    clearHighlights();
    const turn = paintTurn;
    if (!inRead() || (pane.hidden && !target) || !inRange(results.range, chapter.ix)) return;
    const rows = results.byChapter.get(chapter.ix) ?? [];
    const fresh = rows.filter((row) => sameRev(row.chapter.rev, chapter.rev)).slice(0, MAX_PAINTED);
    if (target && !sameRev(target.rev, chapter.rev)) {
      showToast('This chapter changed since the search. Run it again to find the match.');
    }
    const usable = target && sameRev(target.rev, chapter.rev) ? target.hit : null;
    const hits = fresh.map((row) => row.hit).filter((hit) => hit !== usable);
    const wanted = usable ? [usable, ...hits] : hits;
    if (wanted.length === 0) return;
    const located = await locateHits(chapter.ix, chapter.rev, wanted.map((hit) => hit.range));
    if (turn !== paintTurn || chapterNow() !== chapter || !host.shadowRoot || !inRead()) return;
    if (located.state !== 'ranges') return;
    const ranges = rangesFrom(host.shadowRoot, located.ranges);
    if (usable) {
      const [first, ...rest] = ranges;
      if (first) {
        CSS.highlights.set(CURRENT_HIT, new Highlight(first));
        reveal(first);
      }
      CSS.highlights.set(ALL_HITS, new Highlight(...rest));
    } else {
      CSS.highlights.set(ALL_HITS, new Highlight(...ranges));
    }
  }

  function repaint() {
    const chapter = chapterNow();
    if (chapter && CSS.highlights.has(CURRENT_HIT)) return;
    if (chapter) paint(chapter, null).catch(() => {});
  }

  function showPane(open) {
    keepPlace(() => {
      pane.hidden = !open;
      app.classList.toggle('searching', open);
      app.classList.toggle('find-bar', open && asBar);
    });
    toggle.classList.toggle('on', open);
  }

  function dock() {
    if (!asBar) return;
    asBar = false;
    memory.closeList();
    keepPlace(() => app.classList.remove('find-bar'));
    render();
    if (current !== null) keepRowInView();
  }

  const panelShown = () => getComputedStyle(element('contents-rail')).display !== 'none' || !element('notes-pane').hidden;

  function setOpen(open, bar = false) {
    if (open === pane.hidden) {
      asBar = open && bar;
      showPane(open);
    } else if (open && !bar) dock();
    if (open) {
      onOpen();
      render();
      input.focus();
      input.select();
      memory.openList();
      repaint();
    }
  }

  function close() {
    if (pane.hidden) return;
    const wasRunning = running !== null || pendingStart !== null;
    cancelRunning();
    if (wasRunning && results.phase === 'running') results.phase = 'stopped';
    recordIfDone();
    memory.closeList();
    memory.closeForm();
    setOpen(false);
    clearHighlights();
    onClosed();
  }

  function onRowClick(event) {
    const row = event.target.closest?.('.result');
    if (!row) return;
    const picked = results.shown[Number(row.dataset.index)];
    if (!picked) return;
    noteUse();
    memory.closeList();
    if (event.detail > 0) row.blur();
    current = Number(row.dataset.index);
    render();
    jumpTo(picked);
  }

  function jumpTo(row) {
    onLeap(row.chapter.ix);
    const target = { hit: row.hit, rev: row.chapter.rev };
    const shown = chapterNow();
    if (jumping === 0 && shown?.ix === row.chapter.ix && inRead()) {
      paint(shown, target).catch(() => {});
      return;
    }
    jumping += 1;
    Promise.resolve(openHit(row.chapter.ix, target)).finally(() => { jumping -= 1; });
  }

  function keepRowInView() {
    const top = current * ROW_HEIGHT;
    if (top < list.scrollTop) list.scrollTop = top;
    else if (top + ROW_HEIGHT > list.scrollTop + list.clientHeight) list.scrollTop = top + ROW_HEIGHT - list.clientHeight;
  }

  function land(next) {
    current = next;
    keepRowInView();
    render();
    jumpTo(results.shown[current]);
  }

  async function hitsBefore(shown) {
    const rows = results.byChapter.get(shown.ix) ?? [];
    const point = placeNow();
    if (rows.length === 0 || !point || !sameRev(rows[0].chapter.rev, shown.rev)) return 0;
    const located = await locateHits(shown.ix, shown.rev, rows.map((row) => row.hit.range));
    if (located.state !== 'ranges') return 0;
    return located.ranges.filter(({ start }) => start.block < point.block || (start.block === point.block && start.offset_u16 < point.offset_u16)).length;
  }

  async function stepFromPlace(by, shown) {
    const asked = results;
    const passed = await hitsBefore(shown).catch(() => 0);
    if (results !== asked) return;
    const next = indexFrom(results.shown, { ix: shown.ix, passed }, by, results.phase !== 'running');
    if (next === null) {
      if (results.phase === 'running') queuedStep = by;
      return;
    }
    land(next);
  }

  function step(by) {
    if (pane.hidden) setOpen(true);
    noteUse();
    memory.closeList();
    if (placing) {
      placing.then(() => step(by));
      return;
    }
    if (pendingStart !== null) {
      queuedStep = by;
      return;
    }
    const shown = chapterNow();
    const onCurrent = current !== null && (jumping > 0 || results.shown[current].chapter.ix === shown?.ix);
    if (shown && !onCurrent) {
      placing = stepFromPlace(by, shown).finally(() => { placing = null; });
      return;
    }
    const next = stepIndex(current, by, results.shown.length);
    if (next === null) {
      if (running !== null) queuedStep = by;
      return;
    }
    land(next);
  }

  function onChoiceClick(event) {
    const button = event.target.closest?.('.row-tick, .row-skip');
    if (!button) return;
    const row = results.shown[Number(button.dataset.index)];
    if (!row) return;
    if (button.classList.contains('row-tick')) toggleHit(choices, row);
    else toggleChapter(choices, row);
    render();
  }

  function setReplaceOpen(open) {
    leaveCleaning();
    replaceOpen = open;
    if (!open) fixRequest = null;
    else dock();
    scheduleStart();
    if (open) replaceInput.focus();
  }

  function showMessage(text) {
    applyNote = text;
    render();
  }

  async function applyCleaning() {
    const { state, cleaned } = cleaning;
    const left = cleanRemaining(state, choices);
    const end = await applyWhole('clean', state.previewId, cleanExclusions(state, choices), changesOf(left), { heading: removingText(left), quiet: true });
    if (end.kind === 'done') {
      leaveCleaning();
      applyNote = '';
      start();
      return { ...end, after: () => cleaned(end) };
    }
    if (end.kind === 'failed') {
      if (end.reason === 'stale') {
        leaveCleaning();
        start();
      }
      showMessage(failedText(end));
    }
    return end;
  }

  async function applyNow() {
    if (cleaning) return applyCleaning();
    const left = remaining(results.preview, results, choices);
    const end = await applyWhole('pane', previewId, exclusionsOf(choices), left.changes);
    if (end.kind === 'done') {
      replaceOpen = false;
      fixRequest = null;
      applyNote = '';
      start();
    } else if (end.kind === 'failed') {
      if (end.reason === 'stale') start();
      showMessage(end.message);
    }
    return end;
  }

  function onApply() {
    requestApply(applyNow);
  }

  function reviewClean(shown) {
    setOpen(true);
    memory.closeList();
    memory.closeForm();
    cancelRunning();
    leaveCleaning();
    cleaning = shown;
    input.value = '';
    replaceOpen = false;
    fixRequest = null;
    applyNote = '';
    lastRun = null;
    results = viewOf(shown.state);
    choices = emptyChoices();
    current = null;
    queuedStep = 0;
    list.scrollTop = 0;
    clearHighlights();
    render();
    repaint();
  }

  function review(offer) {
    leaveCleaning();
    setOpen(true);
    memory.closeList();
    replaceOpen = true;
    input.value = offer.from;
    replaceInput.value = offer.to;
    setMode(false);
    clearTimeout(pendingStart);
    fixRequest = { kind: 'fix', from: offer.from, to: offer.to };
    range = WHOLE_RANGE;
    start();
  }

  function onAction() {
    if (results.phase === 'running') {
      cancelRunning();
      results.phase = 'stopped';
      render();
    } else {
      start(results.scope);
      recordOnEnd = true;
    }
  }

  const memory = createSearchMemory({ input, currentQuery, run: runQuery, checkCurrent, showToast, undoBar });

  toggle.disabled = false;
  const find = () => setOpen(true, pane.hidden ? !panelShown() && !app.classList.contains('focus') : asBar);

  input.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && event.ctrlKey && !event.isComposing) {
      event.preventDefault();
      event.stopPropagation();
      dock();
      return;
    }
    if (!memory.onKey(event)) return;
    event.preventDefault();
    event.stopPropagation();
  });
  toggle.addEventListener('click', () => (pane.hidden ? find() : close()));
  element('search-prev').addEventListener('click', () => step(-1));
  element('search-next').addEventListener('click', () => step(1));
  element('search-close').addEventListener('click', close);
  input.addEventListener('input', (event) => {
    if (!event.isComposing) scheduleStart();
  });
  input.addEventListener('compositionend', scheduleStart);
  plain.addEventListener('click', () => setMode(false));
  regex.addEventListener('click', () => setMode(!regexMode));
  for (const [side, button] of Object.entries(sideButtons)) button.addEventListener('click', () => setRange(toggled(range, side, hereNow())));
  hereButton.addEventListener('click', () => setRange(reanchored(range, hereNow())));
  action.addEventListener('click', onAction);
  list.addEventListener('scroll', renderList);
  list.addEventListener('click', onRowClick);
  list.addEventListener('click', onChoiceClick);
  replaceToggle.addEventListener('click', () => setReplaceOpen(!replaceOpen));
  replaceInput.addEventListener('input', (event) => {
    if (!event.isComposing) scheduleStart();
  });
  replaceInput.addEventListener('compositionend', scheduleStart);
  replaceApply.addEventListener('click', onApply);

  return {
    open: () => setOpen(true),
    find,
    close,
    isOpen: () => !pane.hidden,
    step,
    review,
    reviewClean,
    showMessage: (text) => {
      setOpen(true);
      memory.closeList();
      showMessage(text);
    },
    clearHighlights,
    bookChanged: () => {
      if (input.value) start();
    },
    caseChanged: () => {
      memory.paint();
      if (input.value && !fixRequest) scheduleStart();
    },
    savedCommands: () => memory.saved().map((entry) => ({ label: `Search: ${entry.name}`, hint: 'Saved search', run: () => runQuery(entry) })),
    reset: () => {
      leaveCleaning();
      recordIfDone();
      memory.closeList();
      memory.closeForm();
      cancelRunning();
      input.value = '';
      replaceInput.value = '';
      replaceOpen = false;
      applyNote = '';
      range = WHOLE_RANGE;
      results = emptyResults();
      current = null;
      queuedStep = 0;
      showPane(false);
      clearHighlights();
      render();
    },
    placeChanged: paintRange,
    chapterShown: (chapter, target) => paint(chapter, target).catch(() => {}),
  };
}
