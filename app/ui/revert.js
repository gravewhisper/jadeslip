import { revertBook, revertPreview } from './ipc.js';

const element = (id) => document.getElementById(id);
const count = (value) => value.toLocaleString('en-US');
const plural = (value, one, many) => `${count(value)} ${value === 1 ? one : many}`;

export function reshapingRun({ requestWhole, bookReshaped, landOn, refreshPosition, showToast, afterChange }) {
  return (call, doneText, sameText) => requestWhole(async () => {
    try {
      const result = await call();
      await bookReshaped(result);
      return {
        kind: 'done',
        after: async () => {
          showToast(result.wrote ? doneText : sameText);
          if (result.shown === null && result.landing !== null) await landOn(result.landing, false);
          else await refreshPosition();
          await afterChange();
          if (result.warnings.length > 0) showToast(result.warnings.join(' '));
        },
      };
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
      return { kind: 'failed' };
    }
  });
}

export function createRevert({ shownIx, requestWhole, bookReshaped, landOn, refreshPosition, refocus, showToast, afterChange }) {
  const scrim = element('revert-dialog');
  const text = element('revert-text');
  const restores = element('revert-restores');
  const chapterButton = element('revert-chapter');
  const wholeButton = element('revert-whole');
  const cancelButton = element('revert-cancel');

  function close() {
    scrim.hidden = true;
    refocus();
  }

  function fill(wording, { chapter, whole, closeLabel, deleted }) {
    text.textContent = wording;
    chapterButton.hidden = !chapter;
    chapterButton.disabled = chapter !== 'changed';
    wholeButton.hidden = !whole;
    cancelButton.textContent = closeLabel;
    restores.replaceChildren(...deleted.map((row) => {
      const line = document.createElement('div');
      line.className = 'deleted-row';
      const title = document.createElement('span');
      title.textContent = row.title;
      line.append(title);
      return line;
    }));
    restores.hidden = deleted.length === 0;
    scrim.hidden = false;
    cancelButton.focus();
  }

  function fillValid(preview) {
    const changed = preview.changed.length;
    const parts = ['It goes back to the copy from before your first edit.'];
    parts.push(preview.chapter === 'changed'
      ? 'Reverting this chapter leaves the others as they are.'
      : 'This chapter already matches the backup, or was added after it.');
    parts.push(`${plural(changed, 'file differs', 'files differ')} from the backup.`);
    if (preview.deleted.length > 0) parts.push('Reverting the whole book brings these deleted chapters back:');
    fill(parts.join(' '), {
      chapter: preview.chapter,
      whole: true,
      closeLabel: 'Cancel',
      deleted: preview.deleted,
    });
  }

  async function open() {
    let preview;
    try {
      preview = await revertPreview(shownIx());
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
      return;
    }
    if (preview.state === 'none') {
      showToast('This book has no original backup. The next edit makes one from the file as it is now.');
    } else if (preview.state === 'otherFileOnly') {
      fill('The backup kept for this book was made from a different copy of the file, so it will not be used. The next edit starts a new backup from the file as it is now.', {
        chapter: null,
        whole: false,
        closeLabel: 'Close',
        deleted: [],
      });
    } else {
      fillValid(preview);
    }
  }

  const reshape = reshapingRun({ requestWhole, bookReshaped, landOn, refreshPosition, showToast, afterChange });

  function run(choice, doneText) {
    scrim.hidden = true;
    return reshape(() => revertBook(choice, shownIx()), doneText, 'This already matches the backup.');
  }

  cancelButton.addEventListener('click', close);
  chapterButton.addEventListener('click', () => {
    const ix = shownIx();
    if (ix !== null) void run({ kind: 'chapter', ix }, 'Chapter reverted to the original.');
  });
  wholeButton.addEventListener('click', () => void run({ kind: 'wholeBook' }, 'The whole book is back to your original.'));

  return { open, isAsking: () => !scrim.hidden, cancel: close };
}
