import {
  addDropped, addOpenBook, changedOnDisk, closeAsked, closeBook, closeWindow, exportCopy, firstText, followLink, openDropped,
  openShelfBook, pickAndAdd, pickAndOpen, relinkBook, reloadBook, removeFromShelf, renderChapter, saveChapter, savePlace,
  setCloseGuard, shelfRows,
} from './ipc.js';
import { addedText, cardOf } from './shelf-model.js';
import { MountError, clearChapter, mountChapter } from './chapter-view.js';
import { EditSession, pointFromClick, visiblePoint, writingPoint } from './edit-session.js';
import { READ, hasUnsavedText, lookOf, step } from './edit-machine.js';
import { actionForKey, describeKey } from './keys.js';
import { createFixOffer } from './fix-offer.js';
import { createApplier } from './replace.js';
import { createSearch } from './search.js';
import { sameRev } from './search-model.js';
import { createToc } from './toc.js';
import { createPalette } from './palette.js';
import { createFocus } from './focus.js';
import { createPreferences } from './preferences.js';
import { createPrefs } from './prefs.js';
import { createPlaces } from './place.js';
import { createPages } from './pages.js';
import { pointAt } from './textmap.js';
import { createChapters } from './chapters.js';
import { createShelf } from './shelf.js';
import { createMarks } from './marks.js';
import { createRevert } from './revert.js';
import { createHistory } from './history.js';
import { createSpelling } from './spelling.js';
import { createDraftOffer, createDraftWriter } from './draft.js';
import { createBack } from './back.js';
import { placeName, positionText, shortName } from './chapter-number.js';
import { createShortcuts, paintTips } from './shortcuts.js';
import { createWordCount } from './word-count.js';
import { createFailureCard } from './failure-card.js';
import { createUndoBar } from './undo-bar.js';
import { createCleanup } from './cleanup.js';
import { createFormatPill } from './format-pill.js';

const XLINK = 'http://www.w3.org/1999/xlink';

const element = (id) => document.getElementById(id);
const host = element('book-host');
const scroller = element('scroller');
const notice = element('notice');
const prevButton = element('prev');
const nextButton = element('next');
const toolbar = element('toolbar');
const failbar = element('failbar');
const failText = element('fail-text');
const reloadButton = element('fail-reload');
const unsavedStatus = element('unsaved');
const saveButton = element('save');
const saveLabel = element('save-label');
const wordCount = createWordCount(element('word-count'));
const narrowWindow = window.matchMedia('(max-width: 1099px)');
const readButton = element('mode-read');
const editButton = element('mode-edit');
const sheet = document.querySelector('.sheet');
const dialog = element('dialog');
const toast = element('toast');
const TOAST_MS = 2500;
const CARET_MARGIN = 24;
const CARET_CLEARANCE = 8;
const LINE_SCROLL_PX = 48;
const PAGE_OVERLAP_PX = 56;
const END_SLACK_PX = 2;
const WHEEL_STEP = 30;
const WHEEL_REST_MS = 200;
const READOUT_MS = 900;

let book = null;
let chapter = null;
let position = { prev: null, next: null };
let latestTurn = 0;
let navigating = 0;
let machine = READ;
let placePill = () => {};
let contentsPref = true;
let contentsOver = false;
let session = null;
let enterPoint = null;
let savedCaret = null;
let toastTimer = null;
let wroteHere = false;
let onShelf = false;
let shelved = false;
let duplicate = null;
let picking = false;
let checkingDisk = false;
let wheelSum = 0;
let wheelAt = 0;
let zoomSum = 0;
let readoutTimer = null;

const pages = createPages({ scroller, host });
const undoBar = createUndoBar();

function showNotice(text, isError) {
  notice.textContent = text;
  notice.classList.toggle('error', isError);
  notice.hidden = false;
}

function clearNotice() {
  notice.hidden = true;
  notice.textContent = '';
}

function readable(failure) {
  return failure instanceof MountError ? failure.message : String(failure?.message ?? failure);
}

function showFailure(failure) {
  clearChapter(host);
  spelling.cleared();
  chapter = null;
  showNotice(readable(failure), true);
}

function showFailedChapter(ix, failure) {
  showFailure(failure);
  showPosition({ ix, number: null, prev: ix > 0 ? ix - 1 : null, next: ix + 1 });
  toc.setCurrent(ix, null);
}

function showProgress() {
  const pageWhere = element('page-where');
  pageWhere.hidden = !pages.on() || !chapter;
  if (!pageWhere.hidden) pageWhere.textContent = pages.label();
  const number = position.number ?? null;
  if (!book || number === null || book.chapters === 0) {
    element('fill').style.width = '0%';
    element('pct').textContent = '0.0%';
    return;
  }
  const room = scroller.scrollHeight - scroller.clientHeight;
  const scrolled = room > 0 ? Math.min(1, scroller.scrollTop / room) : 1;
  const within = pages.on() ? pages.progress() : scrolled;
  const share = (number - 1 + within) / book.chapters;
  element('fill').style.width = `${share * 100}%`;
  element('pct').textContent = `${(share * 100).toFixed(1)}%`;
}

function placeTitle(shown) {
  return shown.title ?? toc.labelOf(shown.ix) ?? '';
}

function hereNow() {
  const shown = chapter ?? (position.ix === undefined ? null : position);
  return book && shown ? { ix: shown.ix, title: placeTitle(shown), number: shown.number ?? null } : null;
}

function showPosition(shown) {
  position = shown;
  prevButton.disabled = shown.prev === null;
  nextButton.disabled = shown.next === null;
  const title = placeTitle(shown);
  const where = element('where-title');
  where.textContent = placeName(title, shown.number);
  where.title = shown.number === null ? positionText(null) : `${positionText(shown.number)} of ${book.chapters.toLocaleString('en-US')}`;
  showProgress();
  search.placeChanged();
}

function showChapter(next, fragment, keepScroll = false, atEnd = false) {
  const top = scroller.scrollTop;
  const shadow = mountChapter(host, next);
  chapter = next;
  marks.forgetSelection();
  clearNotice();
  if (next.warnings.length > 0) showNotice(next.warnings.join(' '), false);
  scroller.scrollTop = keepScroll ? top : 0;
  pages.mounted(keepScroll);
  if (fragment) revealFragment(fragment);
  if (atEnd && pages.on()) pages.toEnd();
  showPosition(next);
  back.shown(next.ix);
  toc.setCurrent(next.ix, next.title);
  places.chapterShown(keepScroll);
  spelling.chapterShown(shadow, next);
}

function revealFragment(fragment) {
  const target = host.shadowRoot?.getElementById(fragment);
  if (!target) return;
  if (!pages.on()) {
    target.scrollIntoView();
    return;
  }
  pages.showRect(target.getClientRects()[0] ?? target.getBoundingClientRect());
  places.moved();
}

function revealRange(range) {
  if (pages.on()) {
    pages.showRect(range.getClientRects()[0] ?? range.getBoundingClientRect());
    places.moved();
    return;
  }
  const box = scroller.getBoundingClientRect();
  const spot = range.getBoundingClientRect();
  scroller.scrollTop += spot.top - box.top - box.height / 3;
}

async function navigate(ix, fragment, hit, atEnd, point) {
  fixOffer.leftChapter(ix);
  const turn = ++latestTurn;
  navigating += 1;
  try {
    const next = await renderChapter(ix);
    if (turn === latestTurn) {
      showChapter(next, fragment, false, atEnd);
      if (point) places.restore(point);
      search.chapterShown(next, hit);
    }
  } catch (failure) {
    if (turn === latestTurn) showFailedChapter(ix, failure);
  } finally {
    navigating -= 1;
  }
}

export const shownChapter = () => chapter?.ix ?? null;

export function goTo(ix, fragment = null, hit = null, atEnd = false) {
  return dispatch({ type: 'go', target: { ix, fragment, hit, atEnd } });
}

function turnPage(direction) {
  const target = position[direction];
  if (target !== null) goTo(target);
}

function leap(to, step = false) {
  if (chapter) back.leap({ ix: chapter.ix, point: places.pointNow(), name: shortName(chapter.title ?? toc.labelOf(chapter.ix), chapter.number) }, to, step);
}

function goBack(origin) {
  if (chapter?.ix === origin.ix) {
    places.restore(origin.point);
    return;
  }
  flushDirty();
  dispatch({ type: 'go', target: { ix: origin.ix, fragment: null, hit: null, atEnd: false, point: origin.point } });
}

function openChapter(ix, fragment = null) {
  flushDirty();
  leap(ix);
  return goTo(ix, fragment);
}

function atChapterEnd() {
  return scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - END_SLACK_PX;
}

function pageDown() {
  if (!atChapterEnd()) {
    scroller.scrollTop += Math.max(1, scroller.clientHeight - PAGE_OVERLAP_PX);
    return;
  }
  if (position.next === null) showToast('This is the last chapter.');
  else turnPage('next');
}

function pageUp() {
  scroller.scrollTop -= Math.max(1, scroller.clientHeight - PAGE_OVERLAP_PX);
}

function pageForward() {
  if (pages.turn(1)) places.moved();
  else if (position.next === null) showToast('This is the last chapter.');
  else turnPage('next');
}

function pageBack() {
  if (pages.turn(-1)) places.moved();
  else if (position.prev !== null) goTo(position.prev, null, null, true);
}

const whenPaged = (paged, scrolling) => () => (pages.on() ? paged() : scrolling());

function onWheel(event) {
  if (!pages.on() || event.ctrlKey) return;
  const now = performance.now();
  if (now - wheelAt < WHEEL_REST_MS) return;
  wheelSum += event.deltaY;
  if (Math.abs(wheelSum) < WHEEL_STEP) return;
  const forward = wheelSum > 0;
  wheelSum = 0;
  wheelAt = now;
  if (forward) pageForward();
  else pageBack();
}

function showTextSize() {
  const readout = element('size-readout');
  readout.textContent = `${prefs.fontPx()} px`;
  readout.hidden = false;
  clearTimeout(readoutTimer);
  readoutTimer = setTimeout(() => { readout.hidden = true; }, READOUT_MS);
}

function sizeText(change) {
  const look = currentLook();
  if (look !== 'read' && look !== 'edit') return;
  change();
  showTextSize();
}

function onZoomWheel(event) {
  if (!event.ctrlKey) return;
  event.preventDefault();
  zoomSum += event.deltaY;
  if (Math.abs(zoomSum) < WHEEL_STEP) return;
  const by = zoomSum < 0 ? 1 : -1;
  zoomSum = 0;
  sizeText(() => prefs.bumpFont(by));
}

function onEdgeClick(event) {
  if (!pages.on() || event.button !== 0 || lookOf(machine) !== 'read' || linkIn(event)) return;
  const box = sheet.getBoundingClientRect();
  const style = getComputedStyle(sheet);
  if (event.clientX < box.left + parseFloat(style.paddingLeft)) pageBack();
  else if (event.clientX > box.right - parseFloat(style.paddingRight)) pageForward();
}

function showToast(text) {
  clearTimeout(toastTimer);
  toast.textContent = text;
  toast.hidden = false;
  toastTimer = setTimeout(() => { toast.hidden = true; }, TOAST_MS);
}

function showFailureBar(failure) {
  failText.textContent = failure.message;
  reloadButton.hidden = failure.kind !== 'changedOnDisk' && failure.kind !== 'planStale';
  failbar.hidden = false;
}

async function recheckDisk() {
  if (!book || checkingDisk) return;
  checkingDisk = true;
  const message = await changedOnDisk().catch(() => null);
  checkingDisk = false;
  if (message && book) withSteadyText(() => showFailureBar({ kind: 'changedOnDisk', message }));
}

function hideFailureBar() {
  failbar.hidden = true;
  failText.textContent = '';
}

function withSteadyText(change) {
  const before = host.getBoundingClientRect().top;
  change();
  scroller.scrollTop += host.getBoundingClientRect().top - before;
}

function caretPlace() {
  const caret = session?.caret();
  return caret ? { block: caret.number, offset_u16: caret.units } : undefined;
}

function setLook(look, lead = undefined) {
  const editLook = look !== 'read';
  toc.setManaging(editLook);
  focus.showMode(editLook);
  paintTips(editLook ? 'edit' : 'read');
  const change = () => {
    toolbar.hidden = !editLook;
    placePill();
    saveButton.hidden = !editLook;
    if (!editLook) failbar.hidden = true;
    pages.setEditing(editLook);
  };
  if (pages.flipsFor(editLook)) places.keep(change, lead);
  else withSteadyText(change);
  host.classList.toggle('editing', editLook);
  document.querySelector('.app').classList.toggle('edit-look', editLook);
  readButton.classList.toggle('on', !editLook);
  editButton.classList.toggle('on', editLook);
}

function showDirty() {
  const dirty = hasUnsavedText(machine);
  unsavedStatus.hidden = !dirty;
  saveButton.classList.toggle('primary', dirty);
  saveLabel.textContent = dirty ? 'Save' : 'Saved';
}

function domPoint(place) {
  const block = place && host.shadowRoot?.querySelector(`.ss-body [data-ss-b="${place.block}"]`);
  return block ? pointAt(block, place.offset_u16) : null;
}

function beginEdit() {
  const shadow = host.shadowRoot;
  const lead = pages.on() ? domPoint(places.pointNow()) : null;
  const pageBox = scroller.getBoundingClientRect();
  setLook('edit');
  const hostBox = host.getBoundingClientRect();
  const barHeight = scroller.getBoundingClientRect().top - pageBox.top;
  const point = enterPoint ?? lead ?? visiblePoint(shadow, pageBox.top + barHeight + CARET_CLEARANCE, pageBox.bottom - CARET_MARGIN, hostBox.left + 1);
  enterPoint = null;
  search.clearHighlights();
  void setCloseGuard(true);
  session = startSession(shadow);
  wordCount.show(session.root);
  if (point) document.getSelection().setBaseAndExtent(point.node, point.offset, point.node, point.offset);
}

function endEdit() {
  void drafts.drop();
  const lead = caretPlace();
  if (session) session.end();
  session = null;
  wordCount.clear();
  void setCloseGuard(false);
  hideFailureBar();
  setLook('read', lead);
  if (chapter) search.chapterShown(chapter, null);
}

function startSession(shadow) {
  const started = new EditSession(shadow);
  started.onPasteRefused = () => showToast('Only plain text can be pasted.');
  started.begin((dirty) => {
    dispatch({ type: 'dirtyChanged', dirty });
    drafts.changed(dirty);
    wordCount.later(started.root);
  });
  return started;
}

async function remount(keepEditing) {
  const next = await renderChapter(chapter.ix);
  const caret = keepEditing ? session.caret() : null;
  const queued = session.queuedText;
  session.end();
  showChapter(next, null, true);
  session = startSession(host.shadowRoot);
  wordCount.show(session.root);
  session.queuedText = queued;
  session.frozen = true;
  session.setCaret(caret);
  if (!keepEditing) session.setEditable(false);
}

async function runSave() {
  const changes = session.collect();
  savedCaret = session.caret();
  session.freeze();
  await drafts.hold();
  let result;
  try {
    result = await saveChapter(chapter.ix, { base: chapter.rev, changes });
  } catch (failure) {
    session.thaw();
    drafts.changed(true);
    return dispatch({ type: 'failed', failure: asSaveFailure(failure) });
  }
  drafts.saved();
  chapter = { ...chapter, rev: result.rev };
  if (!session.reconcile(result.after)) await remountAfterSave();
  if (machine.then?.to === 'stay') session.rebase();
  return dispatch({ type: 'saved', result });
}

async function remountAfterSave() {
  try {
    await remount(machine.then?.to === 'stay');
  } catch (failure) {
    showNotice(readable(failure), true);
  }
}

async function runWholeBook(run) {
  const wasEditing = machine.mode === 'edit' && session !== null;
  if (wasEditing) savedCaret = session.caret() ?? savedCaret;
  if (wasEditing) session.setEditable(false);
  const end = await run();
  if (end.kind === 'done') {
    fixOffer.clear();
    await refreshMounted(wasEditing);
    if (end.titles > 0) await toc.load();
    await end.after?.();
  } else if (wasEditing && machine.mode === 'edit') {
    session.setEditable(true);
    refocusEditor();
  }
  await end.resume?.();
}

async function restoreChapter() {
  const turn = latestTurn;
  try {
    const next = await renderChapter(chapter.ix);
    if (turn === latestTurn) showChapter(next, null, true);
  } catch (failure) {
    showNotice(readable(failure), true);
  }
}

async function refreshMounted(wasEditing) {
  if (!chapter) return;
  try {
    const next = await renderChapter(chapter.ix);
    const changed = !sameRev(next.rev, chapter.rev);
    if (wasEditing && machine.mode === 'edit') {
      session.setEditable(true);
      if (changed) {
        await remount(true);
        session.rebase();
      }
      session.root.focus({ preventScroll: true });
      session.setCaret(savedCaret);
    } else if (changed) {
      showChapter(next, null, true);
      search.chapterShown(next, null);
    }
  } catch (failure) {
    showNotice(readable(failure), true);
  }
}

function asSaveFailure(failure) {
  if (failure?.kind && failure?.message) return failure;
  return { kind: 'other', message: `${readable(failure)} Your text is still on screen.` };
}

async function reload() {
  try {
    toc.stopHeadings();
    await reloadBook();
    hideFailureBar();
    void toc.load();
    await navigate(chapter?.ix ?? position.next ?? 0, null);
  } catch (failure) {
    showNotice(readable(failure), true);
  }
}

const EFFECTS = {
  beginEdit,
  endEdit,
  refuseEdit: () => showToast('This page has no text to edit.'),
  navigate: ({ target }) => navigate(target.ix, target.fragment, target.hit, target.atEnd, target.point),
  closeWindow: () => places.flushWhenStill().finally(() => closeWindow()),
  reload,
  save: runSave,
  settle: ({ result }) => {
    hideFailureBar();
    if (result?.wrote) showToast('Saved. Your original is backed up.');
    if (result?.wrote) {
      wroteHere = true;
      void places.flush();
      void marks.refresh();
    }
    if (result?.wrote && chapter) fixOffer.offerFrom(result, chapter.ix);
    if (result?.wrote) refreshContents(result.contents);
    if (result?.warnings?.length > 0) showNotice(result.warnings.join(' '), false);
  },
  lookRead: () => {
    setLook('read', caretPlace());
    session?.setEditable(false);
  },
  lookEdit: () => {
    setLook('edit');
    session?.setEditable(true);
    session?.root.focus({ preventScroll: true });
    session?.setCaret(savedCaret);
  },
  showFailure: ({ failure }) => showFailureBar(failure),
  askUnsaved: () => {
    savedCaret = session?.caret() ?? savedCaret;
    void closeAsked();
    dialog.hidden = false;
    element('dialog-save').focus();
  },
  hideDialog: () => { dialog.hidden = true; },
  applyWhole: ({ run }) => runWholeBook(run),
  restoreChapter: () => restoreChapter(),
  refocus: () => {
    session?.root.focus({ preventScroll: true });
    session?.setCaret(savedCaret);
  },
};

export function dispatch(event) {
  const { state, effects } = step(machine, event);
  machine = state;
  showDirty();
  const running = effects.map((effect) => EFFECTS[effect.fx]({ ...effect, result: event.result }));
  return Promise.all(running);
}

function flushDirty() {
  if (machine.mode === 'edit' && session) {
    dispatch({ type: 'dirtyChanged', dirty: session.refresh() });
  }
}

function requestEnter() {
  if (navigating > 0) return;
  if (draftOffer.blocking()) {
    showToast('Recover or discard the unsaved edits first.');
    return;
  }
  const editable = chapter !== null && chapter.kind !== 'image' && Boolean(host.shadowRoot?.querySelector('.ss-body'));
  dispatch({ type: 'enter', editable });
}

function requestLeave() {
  flushDirty();
  dispatch({ type: 'esc' });
}

function requestSave() {
  flushDirty();
  dispatch({ type: 'saveKey' });
}

function requestGo(direction) {
  flushDirty();
  turnPage(direction);
}

function runEditCommand(name, value) {
  if (machine.mode !== 'edit' || !session) return;
  session.root.focus({ preventScroll: true });
  document.execCommand(name, false, value);
  placePill();
}

function linkIn(event) {
  return event.composedPath().find((node) => node.localName === 'a' || node.localName === 'area');
}

async function openLink(from, href) {
  try {
    const target = await followLink(from, href);
    if (target.kind === 'chapter') {
      leap(target.ix);
      if (target.ix === from && target.fragment) {
        revealFragment(target.fragment);
      } else {
        await goTo(target.ix, target.fragment);
      }
    } else if (target.kind === 'blocked') {
      showNotice('That link was blocked.', false);
    }
  } catch (failure) {
    showNotice(readable(failure), true);
  }
}

function onBookClick(event) {
  const link = linkIn(event);
  if (!link) {
    if (event.detail >= 2 && event.button === 0) onDoubleClick(event);
    return;
  }
  event.preventDefault();
  if (lookOf(machine) !== 'read' || !event.isTrusted || event.button !== 0 || !chapter) return;
  const href = link.getAttribute('href') ?? link.getAttributeNS(XLINK, 'href');
  if (href !== null) openLink(chapter.ix, href);
}

function onAuxClick(event) {
  if (linkIn(event)) event.preventDefault();
}

function onDoubleClick(event) {
  if (machine.mode !== 'read' || !host.shadowRoot) return;
  enterPoint = pointFromClick(host.shadowRoot, event.clientX, event.clientY);
  requestEnter();
}

function runBlockCommand(run) {
  if (machine.mode !== 'edit' || !session) return;
  session.root.focus({ preventScroll: true });
  run(session);
  placePill();
}

function refocusEditor() {
  if (machine.mode !== 'edit' || !session) return;
  session.root.focus({ preventScroll: true });
  session.setCaret(savedCaret);
}

function rememberCaret() {
  if (machine.mode === 'edit' && session && document.activeElement?.localName !== 'input') savedCaret = session.caret();
}

function openSearch() {
  rememberCaret();
  search.find();
}

function openPalette() {
  rememberCaret();
  prefs.closeComfort();
  palette.open();
}

function afterPalette(returnTo) {
  if (machine.mode === 'edit' && session) refocusEditor();
  else returnTo?.focus?.({ preventScroll: true });
}

function leaveFields() {
  rememberCaret();
  prefs.closeComfort();
  const active = document.activeElement;
  if (active?.localName === 'input') active.blur();
  if (machine.mode === 'edit') refocusEditor();
}

function onFieldEsc() {
  if (toc.isRenaming(document.activeElement)) {
    toc.cancelRename();
    refocusEditor();
  } else if (toc.isFilter(document.activeElement)) toc.escape();
  else if (contentsOver) setContents(false);
  else if (marks.isField(document.activeElement)) {
    document.activeElement.blur();
    refocusEditor();
  } else search.close();
}

const applier = createApplier({ showToast });

const search = createSearch({
  host,
  reveal: revealRange,
  chapterNow: () => chapter,
  hereNow,
  placeNow: () => places.pointNow(),
  inRead: () => machine.mode === 'read',
  openHit: (ix, target) => {
    flushDirty();
    return goTo(ix, null, target);
  },
  onLeap: (ix) => leap(ix, true),
  showToast,
  onClosed: refocusEditor,
  requestApply: requestWholeBook,
  applyWhole: applier.run,
  onOpen: () => marks.close(),
  keepPlace: (change) => places.keep(change),
  matchCase: () => prefs.matchCase(),
  setMatchCase: (on) => prefs.setMatchCase(on),
  undoBar,
});

function requestWholeBook(run) {
  flushDirty();
  return dispatch({ type: 'apply', run });
}

const fixOffer = createFixOffer({
  applyWhole: (slot, id, exclude, changes) => new Promise((settled) => {
    requestWholeBook(async () => {
      const end = await applier.run(slot, id, exclude, changes);
      settled(end);
      return end;
    });
  }),
  review: (offer) => search.review(offer),
  showMessage: (text) => search.showMessage(text),
  refocus: refocusEditor,
  steady: withSteadyText,
});

function overlayEsc() {
  if (picking) return;
  if (applier.isBusy()) applier.requestStop();
  else if (fixOffer.isAsking()) fixOffer.cancelDialog();
  else if (cleanup.isAsking()) cleanup.cancel();
  else if (chapters.isAsking()) chapters.cancelAsking();
  else if (revert.isAsking()) revert.cancel();
  else if (historyPanel.isAsking()) historyPanel.cancel();
  else if (shelf.isAsking()) shelf.cancelAsking();
  else if (duplicate) closeDuplicate();
  else shortcuts.close();
}

const spelling = createSpelling({
  host,
  isEditing: () => machine.mode === 'edit',
  isOn: () => prefs.spellingOn(),
  showToast,
});

const places = createPlaces({
  scroller,
  host,
  pages,
  chapterNow: () => chapter,
  save: (ix, point) => {
    const writing = wroteHere || machine.mode !== 'read';
    wroteHere = false;
    return savePlace(ix, point, writing).catch((failure) => showToast(`Your place could not be saved. ${failure}`));
  },
  onProgress: showProgress,
});

const chapters = createChapters({
  undoBar,
  tocData: () => toc.data(),
  beginRename: (entry) => renameEntry(entry),
  requestWhole: requestWholeBook,
  refocus: refocusEditor,
  shownIx: () => chapter?.ix ?? null,
  showToast,
  bookReshaped,
  landOn: (ix, thenEdit) => landOn(ix, thenEdit),
  refreshPosition,
  leaveEdit: async () => {
    if (machine.mode === 'edit') await dispatch({ type: 'esc' });
    await places.flush();
  },
  showNewBook,
});

const back = createBack({ go: goBack });

const toc = createToc({
  openEntry: (...args) => {
    if (contentsOver) setContents(false);
    return openChapter(...args);
  },
  afterFilterEsc: refocusEditor,
  onLoaded: () => palette.refresh(),
  chapters,
});

const prefs = createPrefs({
  keepPlace: places.keep,
  pages,
  showToast,
  onContents: (shown) => {
    if (shown) toc.shown();
  },
  paintContents,
  onSpelling: () => spelling.restart(),
  onComfortOpen: () => spelling.showLanguages(),
  onMatchCase: () => search.caseChanged(),
});

const focus = createFocus({ keepPlace: places.keep, beforeEnter: leaveFields, showToast });

const palette = createPalette({
  tocData: () => toc.data(),
  bookChapters: () => book?.chapters ?? 0,
  openChapter,
  onClosed: afterPalette,
  commands: () => [
    { label: 'Switch theme', run: () => prefs.switchTheme() },
    { label: 'Focus mode', run: () => focus.enter() },
    { label: 'Full screen', run: () => focus.toggleFullscreen() },
    { label: 'Reset layout', run: () => prefs.resetLayout() },
    { label: 'Scrolling view', run: () => prefs.setPages('scroll') },
    { label: 'Auto pages, one or two to fit', run: () => prefs.setPages('auto') },
    { label: 'Search this book', run: openSearch },
    { label: 'Show or hide contents', run: toggleContents },
    { label: 'Larger text', run: () => prefs.bumpFont(1) },
    { label: 'Smaller text', run: () => prefs.bumpFont(-1) },
    { label: 'Spelling marks on or off', run: () => prefs.toggleSpelling() },
    { label: 'New book', run: () => chapters.openNewBook() },
    { label: 'Add chapter', run: () => chapters.addAfterShown() },
    { label: 'Rename this chapter', run: () => renameShown() },
    { label: 'Move this chapter up', run: () => moveShown(-1) },
    { label: 'Move this chapter down', run: () => moveShown(1) },
    { label: 'Delete this chapter', run: () => chapters.removeShown() },
    { label: 'Restore a deleted chapter', run: () => chapters.openDeleted() },
    { label: 'Go to library', run: () => goLibrary() },
    { label: 'Open an EPUB', run: () => openPicked() },
    { label: 'Add this book to the shelf', run: () => (shelved ? showToast('This book is on your shelf.') : addThisBook()) },
    { label: 'Add an EPUB to the shelf', run: () => addPicked() },
    { label: 'Export a copy', run: () => runExport() },
    { label: 'Revert to backup', run: () => askRevert() },
    { label: 'History and snapshots', run: () => askHistory() },
    { label: 'Clean up book…', run: () => askCleanup() },
    { label: 'Open notes', run: () => marks.open() },
    { label: 'Bookmark this chapter', run: () => marks.addOne() },
    { label: 'Fix everywhere', run: () => (fixOffer.ready() ? fixOffer.ask() : showToast('Nothing to fix everywhere. Correct a word in Edit and save, and Jadeslip offers to fix it in every chapter.')) },
    { label: 'Keyboard shortcuts', run: () => shortcuts.open() },
    { label: 'Preferences', run: () => preferences.open() },
    ...search.savedCommands(),
  ],
});

function showCounts() {
  element('crumb').textContent = book.title;
  document.title = `${book.title} - Jadeslip`;
}

async function bookReshaped(result) {
  book = result.book;
  back.forget();
  showCounts();
  if (chapter) chapter = result.shown === null ? null : { ...chapter, ix: result.shown };
  if (!chapter) position = { ...position, ix: undefined };
  search.bookChanged();
  await toc.load();
  void chapters.refreshDeleted();
  void marks.refresh();
}

function refreshContents(navSaved) {
  if (navSaved) void toc.load();
  else if (toc.data()?.source === 'headings') void refreshPosition();
}

async function refreshPosition() {
  if (!chapter) return;
  try {
    const fresh = await renderChapter(chapter.ix);
    showPosition(fresh);
    toc.setCurrent(fresh.ix, fresh.title);
  } catch (failure) {
    showNotice(readable(failure), true);
  }
}

async function landOn(ix, thenEdit) {
  await goTo(ix);
  if (!thenEdit) return;
  enterPoint = writingPoint(host.shadowRoot);
  requestEnter();
}

function paintContents(pref = contentsPref) {
  contentsPref = pref;
  const shown = narrowWindow.matches ? contentsOver : pref;
  document.querySelector('.app').classList.toggle('contents-over', narrowWindow.matches && contentsOver);
  element('toggle-contents').classList.toggle('on', shown);
  for (const id of ['toggle-contents', 'hide-contents']) element(id).setAttribute('aria-expanded', String(shown));
}

const contentsShown = () => (narrowWindow.matches ? contentsOver : prefs.contentsShown());

function setContents(shown) {
  if (!shown && element('contents-rail').contains(document.activeElement)) leaveFields();
  if (!narrowWindow.matches) {
    prefs.setContents(shown);
    return;
  }
  contentsOver = shown;
  paintContents();
  if (shown) toc.shown();
}

function toggleContents() {
  if (focus.isOn()) {
    focus.exit();
    setContents(true);
  } else setContents(!contentsShown());
}

narrowWindow.addEventListener('change', () => {
  contentsOver = false;
  paintContents();
});
scroller.addEventListener('mousedown', () => {
  if (contentsOver) setContents(false);
});

const otherTab = () => (search.isOpen() && !document.querySelector('.app').classList.contains('find-bar')) || marks.isOpen();

function showContentsTab() {
  search.close();
  marks.close();
  if (!contentsShown()) setContents(true);
}

function contentsKey() {
  if (focus.isOn()) toggleContents();
  else togglePanel();
}

let closedTab = 'contents';

function togglePanel() {
  if (!otherTab()) {
    if (contentsShown() || closedTab === 'contents') {
      if (contentsShown()) closedTab = 'contents';
      toggleContents();
      return;
    }
    rememberCaret();
    if (closedTab === 'search') search.open();
    else marks.open();
    return;
  }
  closedTab = marks.isOpen() ? 'notes' : 'search';
  search.close();
  marks.close();
  if (contentsShown()) setContents(false);
}

element('toggle-contents').addEventListener('click', togglePanel);
element('hide-contents').addEventListener('click', togglePanel);
element('tab-contents').addEventListener('click', showContentsTab);
element('tab-search').addEventListener('click', () => {
  rememberCaret();
  search.open();
});
element('tab-notes').addEventListener('click', () => marks.open());

function renameEntry(entry) {
  rememberCaret();
  setContents(true);
  toc.startRename(entry);
}

function renameShown() {
  const entry = chapter ? toc.entryOfChapter(chapter.ix) : null;
  if (entry !== null) renameEntry(entry);
  else if (chapter) showToast('This chapter is not in the table of contents.');
}

function moveShown(direction) {
  const entry = chapter ? toc.entryOfChapter(chapter.ix) : null;
  if (entry !== null) chapters.step(entry, direction);
  else if (chapter) showToast('This chapter is not in the table of contents.');
}

function showView(name) {
  onShelf = name === 'library';
  element('library').hidden = name !== 'library';
  element('reader').hidden = name !== 'reader';
}

function resetBookState() {
  latestTurn += 1;
  toc.reset();
  if (chapters.isAsking()) chapters.cancelAsking();
  if (revert.isAsking()) revert.cancel();
  if (historyPanel.isAsking()) historyPanel.cancel();
  cleanup.reset();
  shelf.closeMenus();
  palette.close(false);
  focus.exit();
  prefs.closeComfort();
  search.reset();
  back.forget();
  fixOffer.clear();
  undoBar.hide();
  marks.reset();
  draftOffer.reset();
  spelling.cleared();
  hideFailureBar();
  clearNotice();
  card.hide();
}

function showNewBook(opened) {
  resetBookState();
  showView('reader');
  showOpened(opened);
  void marks.refresh();
  return Promise.all([toc.load().then(() => chapters.refreshDeleted()), draftOffer.check()]);
}

async function whilePicking(pick) {
  picking = true;
  element('picker-wait').hidden = false;
  try {
    return await pick();
  } finally {
    picking = false;
    element('picker-wait').hidden = true;
  }
}

function openInto(load, thenEdit = false, onFail = null) {
  flushDirty();
  return requestWholeBook(async () => {
    if (machine.mode === 'edit') await dispatch({ type: 'esc' });
    await places.flush();
    let view;
    try {
      view = await load();
    } catch (failure) {
      if (onFail) onFail(failure);
      else if (onShelf) shelf.showFailure(readable(failure));
      else showToast(readable(failure));
      return { kind: 'failed' };
    }
    if (view.state === 'cancelled') return { kind: 'failed' };
    shelf.hideFailure();
    await showNewBook(view);
    return {
      kind: 'done',
      after: () => {
        if (thenEdit) requestEnter();
      },
    };
  });
}

function openPicked() {
  return openInto(() => whilePicking(pickAndOpen));
}

function openFromShelf(key, action) {
  flushDirty();
  return requestWholeBook(async () => {
    let view;
    try {
      view = await openShelfBook(key, action.writing);
    } catch (failure) {
      shelf.hideFailure();
      showOpenFailure(failure);
      return { kind: 'failed' };
    }
    shelf.hideFailure();
    await showNewBook(view);
    return {
      kind: 'done',
      after: () => {
        if (action.edit) requestEnter();
      },
    };
  });
}

function openDrop() {
  let others = 0;
  const load = async () => {
    const dropped = await openDropped();
    others = dropped.others;
    return dropped.opened;
  };
  return openInto(load).then(() => {
    if (others > 0) showToast('Opened the first file. Drop books on the library to add them to your shelf.');
  });
}

async function showAdds(adds, failed = []) {
  if (onShelf) {
    shelf.show(await shelfRows());
    if (adds.length > 0) shelf.selectKey(adds[0].key);
    if (failed.length > 0) shelf.showFailure(failed.join(' '));
  } else if (failed.length > 0) showToast(failed.join(' '));
  if (adds.length === 1 && adds[0].state === 'alreadyOnShelf') {
    offerDuplicate(adds[0], false);
    return;
  }
  const text = addedText(adds);
  if (text) showToast(text);
}

async function addDrop() {
  try {
    const result = await addDropped();
    await showAdds(result.added, result.failed);
  } catch (failure) {
    if (onShelf) shelf.showFailure(readable(failure));
    else showToast(readable(failure));
  }
}

async function addPicked() {
  try {
    const result = await whilePicking(pickAndAdd);
    if (result.state !== 'cancelled') await showAdds([result]);
  } catch (failure) {
    if (onShelf) shelf.showFailure(readable(failure));
    else showToast(readable(failure));
  }
}

function addThisBook() {
  flushDirty();
  return requestWholeBook(async () => {
    if (machine.mode === 'edit') await dispatch({ type: 'esc' });
    await places.flush();
    let added;
    try {
      added = await addOpenBook();
    } catch (failure) {
      showToast(readable(failure));
      return { kind: 'failed' };
    }
    if (added.state !== 'added') return { kind: 'asked', resume: () => offerDuplicate(added, true) };
    let view;
    try {
      view = await openShelfBook(added.key, false);
    } catch (failure) {
      showOpenFailure(failure);
      return { kind: 'failed' };
    }
    await showNewBook(view);
    showToast("Added to your shelf. Edits now go to Jadeslip's copy.");
    return { kind: 'done' };
  });
}

function offerDuplicate(add, reading) {
  duplicate = add;
  element('dup-text').textContent = reading
    ? `${add.title} is already on your shelf as Jadeslip's own copy. You are reading a separate file, and edits here change that file.`
    : `${add.title} is already on your shelf, so no second copy was made.`;
  element('dup-cancel').textContent = reading ? 'Keep reading this file' : 'Close';
  element('dup-open').textContent = reading ? 'Open the shelf copy' : 'Open it';
  element('dup-dialog').hidden = false;
  element('dup-open').focus();
}

function closeDuplicate() {
  duplicate = null;
  element('dup-dialog').hidden = true;
}

async function openDuplicate() {
  const key = duplicate?.key;
  closeDuplicate();
  if (!key) return;
  const row = (await shelfRows()).find((book) => book.key === key);
  await openFromShelf(key, row ? cardOf(row).go : { writing: false, edit: false });
}

function goLibrary() {
  flushDirty();
  return requestWholeBook(async () => {
    if (machine.mode === 'edit') await dispatch({ type: 'esc' });
    await places.flush();
    try {
      await closeBook();
    } catch (failure) {
      showToast(readable(failure));
      return { kind: 'failed' };
    }
    resetBookState();
    clearChapter(host);
    chapter = null;
    book = null;
    shelved = false;
    element('add-to-shelf').hidden = true;
    document.title = 'Jadeslip';
    shelf.show(await shelfRows(), true);
    showView('library');
    return { kind: 'left' };
  });
}

async function runExport() {
  try {
    const done = await whilePicking(exportCopy);
    if (done.state === 'saved') showToast('Copy saved. This book stays open where it is.');
  } catch (failure) {
    showToast(readable(failure));
  }
}

function askRevert() {
  flushDirty();
  return requestWholeBook(async () => ({ kind: 'asked', resume: () => revert.open() }));
}

function askHistory() {
  flushDirty();
  return requestWholeBook(async () => ({ kind: 'asked', resume: () => historyPanel.open() }));
}

function askCleanup() {
  flushDirty();
  return requestWholeBook(async () => ({ kind: 'asked', resume: () => cleanup.open() }));
}

async function cleanShelfBook(key, go) {
  await openFromShelf(key, { ...go, edit: false });
  if (book && !onShelf && !card.isShown()) await cleanup.open();
}

const READER_MENU = {
  'reader-clean': askCleanup,
  'reader-history': askHistory,
  'reader-export': runExport,
  'open-prefs': () => openPreferences(element('reader-more')),
};

const drafts = createDraftWriter({
  pending: () => (machine.mode === 'edit' && machine.dirty && session && !session.frozen && chapter
    ? { ix: chapter.ix, edits: { base: chapter.rev, changes: session.pending() } }
    : null),
});

async function recoverDraft({ ix, edits }) {
  if (machine.mode === 'edit' && !machine.dirty) await dispatch({ type: 'esc' });
  if (machine.mode !== 'read') {
    showToast('Save or discard the edits on screen first.');
    return;
  }
  await goTo(ix);
  if (!chapter || chapter.ix !== ix || !sameRev(chapter.rev, edits.base)) {
    showToast('This chapter changed, so the unsaved edits were not applied.');
    return;
  }
  requestEnter();
  if (machine.mode !== 'edit' || !session) return;
  if (!session.replay(edits.changes)) {
    await dispatch({ type: 'dirtyChanged', dirty: false });
    await dispatch({ type: 'esc' });
    await restoreChapter();
    showToast('The unsaved edits could not be applied to this chapter.');
    return;
  }
  flushDirty();
}

const draftOffer = createDraftOffer({
  bar: element('draft-offer'),
  text: element('draft-offer-text'),
  recoverButton: element('draft-recover'),
  discardButton: element('draft-discard'),
  apply: recoverDraft,
  showToast,
});

const shelf = createShelf({
  openBook: openFromShelf,
  openPicker: openPicked,
  addPicker: addPicked,
  newBook: () => chapters.openNewBook(),
  switchTheme: () => prefs.switchTheme(),
  themeLabel: () => prefs.themeLabel(),
  reload: shelfRows,
  showToast,
  shelfPrefs: prefs.shelfPrefs,
  setShelfPrefs: prefs.setShelfPrefs,
  onExport: whilePicking,
  cleanBook: (key, go) => void cleanShelfBook(key, go),
});

const marks = createMarks({
  host,
  chapterNow: () => chapter,
  placeNow: () => places.pointNow(),
  hasUnsaved: () => {
    flushDirty();
    return hasUnsavedText(machine);
  },
  jumpTo: (row) => {
    flushDirty();
    leap(row.ix);
    return goTo(row.ix, null, { hit: { range: row.range }, rev: row.rev });
  },
  showToast,
  onOpen: () => search.close(),
});

const revert = createRevert({
  shownIx: () => chapter?.ix ?? null,
  requestWhole: requestWholeBook,
  bookReshaped,
  landOn,
  refreshPosition,
  refocus: refocusEditor,
  showToast,
  afterChange: () => marks.refresh(),
});

const historyPanel = createHistory({
  shownIx: () => chapter?.ix ?? null,
  requestWhole: requestWholeBook,
  bookReshaped,
  landOn,
  refreshPosition,
  refocus: refocusEditor,
  showToast,
  afterChange: () => marks.refresh(),
});

const cleanup = createCleanup({
  requestWhole: requestWholeBook,
  applyRun: applier.run,
  review: (shown) => search.reviewClean(shown),
  undoBar,
  shownIx: () => chapter?.ix ?? null,
  bookReshaped,
  landOn,
  refreshPosition,
  showToast,
  afterChange: () => marks.refresh(),
  refocus: refocusEditor,
  allowed: () => book !== null && !onShelf && !card.isShown(),
});

const shortcuts = createShortcuts({ onClosed: (returnTo) => afterPalette(returnTo) });

const preferences = createPreferences({ prefs, showToast, onClosed: (returnTo) => afterPalette(returnTo) });
prefs.subscribe(() => shelf.paintTheme());

function openPreferences(opener) {
  palette.close(false);
  prefs.closeComfort();
  preferences.open(opener);
}

const asOpenFailure = (failure) => (failure?.kind ? failure : { kind: 'unreadable', message: readable(failure), key: null });

const card = createFailureCard({
  onBack: () => goLibrary(),
  onFind: (failure) => openInto(() => whilePicking(() => relinkBook(failure.key)), false, (wrong) => card.retell(readable(wrong))),
  onRemove: async (failure) => {
    try {
      await removeFromShelf(failure.key);
    } catch (wrong) {
      card.retell(readable(wrong));
      return;
    }
    await goLibrary();
  },
});

function showOpenFailure(failure) {
  showView('reader');
  card.show(asOpenFailure(failure));
}

function currentLook() {
  if (picking || duplicate || shortcuts.isOpen() || applier.isBusy() || fixOffer.isAsking() || cleanup.isAsking() || chapters.isAsking() || revert.isAsking() || historyPanel.isAsking() || shelf.isAsking()) return 'overlay';
  if (machine.mode === 'asking' || preferences.isOpen()) return 'dialog';
  if (shelf.menuOpen()) return 'menu';
  if (onShelf) return 'shelf';
  if (card.isShown()) return 'card';
  return palette.isOpen() ? 'palette' : lookOf(machine);
}

const KEY_ACTIONS = {
  search: openSearch,
  contents: contentsKey,
  searchNext: () => search.step(1),
  searchPrev: () => search.step(-1),
  back: () => back.back(),
  paneEsc: onFieldEsc,
  palette: openPalette,
  notes: () => marks.toggle(),
  comfort: () => prefs.toggleComfort(),
  preferences: () => openPreferences(),
  library: () => goLibrary(),
  shelfLeft: () => shelf.move('left'),
  shelfRight: () => shelf.move('right'),
  shelfUp: () => shelf.move('up'),
  shelfDown: () => shelf.move('down'),
  shelfOpen: () => shelf.openSelected(),
  shelfEdit: () => shelf.openSelected(true),
  shelfMenu: () => shelf.openMenuForSelected(),
  menuClose: () => shelf.closeMenus(),
  menuDown: () => shelf.menuStep(1),
  menuUp: () => shelf.menuStep(-1),
  shortcuts: () => {
    palette.close(false);
    prefs.closeComfort();
    shortcuts.open();
  },
  paletteClose: () => palette.close(),
  paletteRun: () => palette.runChosen(),
  paletteDown: () => palette.down(),
  paletteUp: () => palette.up(),
  comfortClose: () => prefs.closeComfort(),
  focus: () => focus.toggle(),
  focusExit: () => focus.leaveAll(),
  fullscreen: () => focus.toggleFullscreen(),
  pageDown: whenPaged(pageForward, pageDown),
  pageUp: whenPaged(pageBack, pageUp),
  lineDown: whenPaged(pageForward, () => { scroller.scrollTop += LINE_SCROLL_PX; }),
  lineUp: whenPaged(pageBack, () => { scroller.scrollTop -= LINE_SCROLL_PX; }),
  enter: requestEnter,
  next: whenPaged(pageForward, () => requestGo('next')),
  prev: whenPaged(pageBack, () => requestGo('prev')),
  esc: requestLeave,
  saveKey: requestSave,
  undo: () => runEditCommand('undo'),
  redo: () => runEditCommand('redo'),
  bold: () => runEditCommand('bold'),
  italic: () => runEditCommand('italic'),
  paragraph: () => {
    if (machine.mode === 'edit') session.insertParagraph();
  },
  dialogKeep: () => (preferences.isOpen() ? preferences.escape() : dispatch({ type: 'dialogKeep' })),
  overlayEsc,
  moveChapterUp: () => moveShown(-1),
  moveChapterDown: () => moveShown(1),
  renameChapter: renameShown,
  dragCancel: () => toc.cancelDrag(),
  textLarger: () => sizeText(() => prefs.bumpFont(1)),
  textSmaller: () => sizeText(() => prefs.bumpFont(-1)),
  textReset: () => sizeText(() => prefs.resetFont()),
};

function onKey(event) {
  const action = actionForKey(describeKey(event), currentLook(), { contentsOver, comfort: prefs.isComfortOpen(), focus: focus.isOn(), fullscreen: focus.isFullscreen(), search: search.isOpen(), dragging: toc.isDragging() });
  if (!action) return;
  event.preventDefault();
  KEY_ACTIONS[action]();
}

const TOOLBAR_ACTIONS = {
  'tool-bold': () => runEditCommand('bold'),
  'tool-italic': () => runEditCommand('italic'),
  'tool-heading': () => runBlockCommand((editor) => editor.toggleBlock('h2')),
  'tool-quote': () => runBlockCommand((editor) => editor.toggleBlock('blockquote')),
  'tool-list': () => runBlockCommand((editor) => editor.blockCommand('insertUnorderedList')),
  'tool-undo': () => runEditCommand('undo'),
  'tool-redo': () => runEditCommand('redo'),
};

function wireEditControls() {
  for (const [id, action] of Object.entries(TOOLBAR_ACTIONS)) {
    element(id).addEventListener('click', action);
  }
  toolbar.addEventListener('mousedown', (event) => event.preventDefault());
  const pill = createFormatPill({ toolbar, scroller, host, editing: () => machine.mode === 'edit' });
  placePill = pill.place;
  saveButton.addEventListener('click', requestSave);
  readButton.addEventListener('click', requestLeave);
  editButton.addEventListener('click', requestEnter);
  document.addEventListener('mousedown', (event) => {
    if (event.target.closest?.('button')) event.preventDefault();
  });
  element('open-palette').addEventListener('click', openPalette);
  element('shelf-prefs').addEventListener('click', (event) => openPreferences(event.currentTarget));
  element('go-library').addEventListener('click', goLibrary);
  element('tool-revert').addEventListener('click', askRevert);
  element('tool-history').addEventListener('click', askHistory);
  element('reader-more').addEventListener('click', () => {
    if (!shelf.menuOpen()) shelf.showMenu(element('reader-menu'), element('reader-more'));
  });
  element('reader-menu').addEventListener('click', (event) => {
    const item = event.target.closest('[role="menuitem"]');
    if (!item) return;
    shelf.closeMenus();
    void READER_MENU[item.id]();
  });
  reloadButton.addEventListener('click', () => {
    flushDirty();
    dispatch({ type: 'reload' });
  });
  element('dialog-save').addEventListener('click', () => dispatch({ type: 'dialogSave' }));
  element('dialog-discard').addEventListener('click', () => dispatch({ type: 'dialogDiscard' }));
  element('dialog-keep').addEventListener('click', () => dispatch({ type: 'dialogKeep' }));
  window.addEventListener('ss-close-flush', () => {
    places.flushWhenStill().finally(() => closeWindow());
  });
  window.addEventListener('ss-book-dropped', () => void (onShelf ? addDrop() : openDrop()));
  element('add-to-shelf').addEventListener('click', () => void addThisBook());
  element('dup-cancel').addEventListener('click', closeDuplicate);
  element('dup-open').addEventListener('click', () => void openDuplicate());
  window.addEventListener('focus', () => void recheckDisk());
  window.addEventListener('ss-close-request', () => {
    flushDirty();
    void closeAsked();
    dispatch({ type: 'close' });
  });
}

function markFirstText() {
  requestAnimationFrame(() => requestAnimationFrame(() => {
    performance.mark('ss-first-text', { detail: Date.now() });
  }));
}

async function start() {
  host.addEventListener('click', onBookClick);
  host.addEventListener('auxclick', onAuxClick);
  host.addEventListener('dblclick', onDoubleClick);
  scroller.addEventListener('click', onEdgeClick);
  scroller.addEventListener('wheel', onWheel, { passive: true });
  document.addEventListener('wheel', onZoomWheel, { passive: false });
  prevButton.addEventListener('click', () => requestGo('prev'));
  nextButton.addEventListener('click', () => requestGo('next'));
  document.addEventListener('keydown', onKey);
  wireEditControls();
  paintTips('read');
  try {
    const [opened] = await Promise.all([firstText(), prefs.load().catch(() => {})]);
    if (opened.state === 'noBook') {
      shelf.show(opened.shelf);
      showView('library');
    } else {
      showView('reader');
      showOpened(opened);
    }
  } catch (failure) {
    showOpenFailure(failure);
  }
  markFirstText();
  if (book) {
    void draftOffer.check();
    requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(() => {
      toc.load();
      void chapters.refreshDeleted();
    })));
  }
}

function showOpened(opened) {
  book = opened.book;
  shelved = opened.onShelf;
  element('add-to-shelf').hidden = shelved;
  showCounts();
  element('count').textContent = '';
  if (opened.chapter.Ok) {
    showChapter(opened.chapter.Ok, null);
    if (opened.place) places.restore(opened.place);
  } else {
    showFailedChapter(opened.start, opened.chapter.Err);
  }
}

start();
