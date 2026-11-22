import { replaceCancel, replaceNext, replaceStart } from './ipc.js';
import { WHOLE_BOOK } from './search-model.js';
import { offerDialog, offerText } from './replace-model.js';

const element = (id) => document.getElementById(id);

export function createFixOffer({ applyWhole, review, showMessage, refocus, steady }) {
  const bar = element('fix-offer');
  const barText = element('fix-offer-text');
  const barFix = element('fix-offer-fix');
  const barDismiss = element('fix-offer-dismiss');
  const dialog = element('fix-dialog');
  const dialogTitle = element('fix-dialog-title');
  const dialogText = element('fix-dialog-text');
  const dialogApply = element('fix-dialog-apply');
  const dialogReview = element('fix-dialog-review');
  const dialogCancel = element('fix-dialog-cancel');
  let nextId = 0;
  let current = null;

  function render() {
    const ready = current?.preview ?? null;
    steady(() => { bar.hidden = current === null; });
    barFix.disabled = ready === null;
    if (current) barText.textContent = offerText(ready);
  }

  function clear() {
    if (current && current.preview === null) void replaceCancel('offer', current.id);
    current = null;
    closeDialog();
    render();
  }

  function closeDialog() {
    dialog.hidden = true;
  }

  async function count(entry) {
    await replaceStart('offer', entry.id, { kind: 'fix', from: entry.offer.from, to: entry.offer.to }, WHOLE_BOOK);
    for (;;) {
      const batch = await replaceNext('offer', entry.id);
      if (current !== entry) return;
      if (batch.end) {
        if (batch.preview?.changes > 0 && batch.preview.limited === 0) {
          entry.preview = batch.preview;
          render();
        } else {
          clear();
        }
        return;
      }
    }
  }

  function offerFrom(saved, chapterIx) {
    clear();
    const offer = saved?.fixOffers?.[0];
    if (!offer) return;
    nextId += 1;
    current = { id: nextId, offer, chapterIx, preview: null };
    render();
    const entry = current;
    count(entry).catch(() => {
      if (current === entry) clear();
    });
  }

  function ask() {
    if (!current?.preview) return;
    const shown = offerDialog(current.offer, current.preview);
    dialogTitle.textContent = shown.title;
    dialogText.textContent = shown.text;
    dialog.hidden = false;
    dialogApply.focus();
  }

  async function runApply() {
    const entry = current;
    closeDialog();
    const end = await applyWhole('offer', entry.id, [], entry.preview.changes);
    if (end.reason === 'stale') clear();
    if (end.kind === 'failed') showMessage(end.message);
    return end;
  }

  function startReview() {
    const { offer } = current;
    closeDialog();
    review(offer);
  }

  function cancelDialog() {
    closeDialog();
    refocus();
  }

  barFix.addEventListener('click', ask);
  barDismiss.addEventListener('click', clear);
  dialogApply.addEventListener('click', runApply);
  dialogReview.addEventListener('click', startReview);
  dialogCancel.addEventListener('click', cancelDialog);

  return {
    offerFrom,
    clear,
    leftChapter: (ix) => {
      if (current && current.chapterIx !== ix) clear();
    },
    ask,
    ready: () => Boolean(current?.preview),
    isAsking: () => !dialog.hidden,
    cancelDialog,
  };
}
