import { draftDiscard, draftDrop, draftOffer, draftRecover, draftWrite } from './ipc.js';

export const DRAFT_IDLE_MS = 2000;

export function createDraftWriter({ pending }) {
  let timer = null;
  let kept = false;
  let writing = Promise.resolve();

  const stop = () => {
    clearTimeout(timer);
    timer = null;
  };

  const write = () => {
    timer = null;
    const now = pending();
    if (!now) return;
    writing = writing
      .then(() => draftWrite(now.ix, now.edits))
      .then(() => { kept = now.edits.changes.length > 0; }, () => {});
  };

  const drop = () => {
    stop();
    writing = writing.then(() => {
      if (!kept) return undefined;
      kept = false;
      return draftDrop();
    }).catch(() => {});
    return writing;
  };

  return {
    changed(dirty) {
      stop();
      if (dirty) timer = setTimeout(write, DRAFT_IDLE_MS);
      else void drop();
    },
    hold() {
      stop();
      return writing;
    },
    saved() {
      kept = false;
    },
    drop,
  };
}

function describe(offer) {
  const when = offer.savedAt ? ` from ${new Date(offer.savedAt).toLocaleString()}` : '';
  if (offer.fit === 'fits') return `Unsaved edits${when} were kept when Jadeslip closed without saving.`;
  if (offer.fit === 'bookChanged') return `Unsaved edits${when} were kept, but this book has changed since, so they cannot be applied.`;
  return 'Unsaved edits were kept, but the file holding them could not be read.';
}

export function createDraftOffer({ bar, text, recoverButton, discardButton, apply, showToast }) {
  let offer = null;

  const hide = () => {
    offer = null;
    bar.hidden = true;
  };

  const show = (found) => {
    offer = found;
    text.textContent = describe(found);
    recoverButton.hidden = found.fit !== 'fits';
    bar.hidden = false;
  };

  recoverButton.addEventListener('click', async () => {
    if (!offer) return;
    let recovered;
    try {
      recovered = await draftRecover();
    } catch (failure) {
      show({ ...offer, fit: 'bookChanged' });
      showToast(String(failure?.message ?? failure));
      return;
    }
    hide();
    await apply(recovered);
  });

  const check = () => {
    hide();
    return draftOffer().then((found) => { if (found) show(found); }, () => {});
  };

  discardButton.addEventListener('click', async () => {
    try {
      await draftDiscard();
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
      return;
    }
    await check();
  });

  return {
    check,
    reset: hide,
    blocking: () => offer !== null,
  };
}
