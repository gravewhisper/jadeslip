import { backupStatus, manageChapters, newBook } from './ipc.js';
import { rowActions, rowRef, stepSpot } from './chapters-model.js';

const element = (id) => document.getElementById(id);

export function createChapters({ undoBar, tocData, beginRename, requestWhole, refocus, shownIx, showToast, bookReshaped, landOn, refreshPosition, leaveEdit, showNewBook }) {
  const confirmScrim = element('chapter-dialog');
  const deletedScrim = element('deleted-dialog');
  const newBookScrim = element('newbook-dialog');
  const deletedRows = element('deleted-rows');
  const deletedOpen = element('deleted-open');
  const newBookError = element('newbook-error');
  let pendingDelete = null;
  let running = false;

  function runOp(op, finish) {
    if (running) return Promise.resolve();
    return requestWhole(async () => {
      running = true;
      try {
        const result = await manageChapters(op, shownIx());
        await bookReshaped(result);
        return {
          kind: 'done',
          after: async () => {
            await finish(result);
            if (result.warnings.length > 0) showToast(result.warnings.join(' '));
          },
        };
      } catch (failure) {
        showToast(String(failure?.message ?? failure));
        return { kind: 'failed' };
      } finally {
        running = false;
      }
    });
  }

  const keepShown = () => refreshPosition();

  async function refreshDeleted() {
    let rows = [];
    try {
      const status = await backupStatus();
      rows = status.state === 'valid' ? status.deleted : [];
    } catch {
      rows = [];
    }
    deletedOpen.hidden = rows.length === 0;
    deletedOpen.textContent = `Deleted chapters (${rows.length.toLocaleString('en-US')})`;
    deletedRows.replaceChildren(...rows.map((row) => {
      const line = document.createElement('div');
      line.className = 'deleted-row';
      const title = document.createElement('span');
      title.textContent = row.title;
      const restore = document.createElement('button');
      restore.className = 'btn line small';
      restore.textContent = 'Restore';
      restore.addEventListener('click', () => {
        deletedScrim.hidden = true;
        void restoreDeleted(row.id);
      });
      line.append(title, restore);
      return line;
    }));
    return rows;
  }

  function restoreDeleted(id) {
    undoBar.hide();
    return runOp({ kind: 'undoDelete', deleted: id }, async (result) => {
      showToast('Chapter restored.');
      if (result.landing !== null) await landOn(result.landing, false);
    });
  }

  function addAfterShown() {
    const shown = shownIx();
    return runOp({ kind: 'add', after: shown, title: 'Untitled chapter' }, async (result) => {
      if (result.landing !== null) await landOn(result.landing, true);
    });
  }

  function askDelete(ix, title) {
    pendingDelete = { ix, title };
    element('chapter-dialog-title').textContent = `Delete "${title}"?`;
    element('chapter-dialog-text').textContent = 'The chapter leaves the book and its table of contents. You can restore it from the backup.';
    element('chapter-dialog-ok').textContent = 'Delete chapter';
    confirmScrim.hidden = false;
    element('chapter-dialog-cancel').focus();
  }

  function confirmDelete() {
    const asked = pendingDelete;
    pendingDelete = null;
    confirmScrim.hidden = true;
    if (!asked) return;
    const wasShown = shownIx() === asked.ix;
    void runOp({ kind: 'delete', chapter: asked.ix }, async (result) => {
      if (result.deleted !== null) undoBar.show(`Deleted "${asked.title}".`, () => void restoreDeleted(result.deleted));
      else showToast('Chapter deleted. Reverting the whole book brings it back.');
      if (wasShown && result.landing !== null) await landOn(result.landing, false);
      else await keepShown();
    });
  }

  function closeNewBook() {
    newBookScrim.hidden = true;
    newBookError.hidden = true;
    refocus();
  }

  function openNewBook() {
    newBookError.hidden = true;
    newBookScrim.hidden = false;
    element('newbook-name').focus();
  }

  function metaOfForm() {
    return {
      title: element('newbook-name').value,
      author: element('newbook-author').value,
      language: element('newbook-language').value,
      firstChapter: element('newbook-first').value,
    };
  }

  function showNewBookFailure(failure) {
    newBookError.textContent = String(failure?.message ?? failure);
    newBookError.hidden = false;
    newBookScrim.hidden = false;
  }

  function createBook() {
    const meta = metaOfForm();
    newBookError.hidden = true;
    newBookScrim.hidden = true;
    return requestWhole(async () => {
      let view;
      try {
        await leaveEdit();
        view = await newBook(meta);
      } catch (failure) {
        showNewBookFailure(failure);
        return { kind: 'failed' };
      }
      await showNewBook(view);
      return {
        kind: 'done',
        after: async () => {
          if (shownIx() !== null) await landOn(shownIx(), true);
        },
      };
    });
  }

  element('add-chapter').addEventListener('click', addAfterShown);
  element('chapter-dialog-cancel').addEventListener('click', () => {
    pendingDelete = null;
    confirmScrim.hidden = true;
    refocus();
  });
  element('chapter-dialog-ok').addEventListener('click', confirmDelete);
  deletedOpen.addEventListener('click', async () => {
    await refreshDeleted();
    deletedScrim.hidden = false;
    element('deleted-close').focus();
  });
  element('deleted-close').addEventListener('click', () => {
    deletedScrim.hidden = true;
    refocus();
  });
  element('newbook-cancel').addEventListener('click', closeNewBook);
  element('newbook-create').addEventListener('click', createBook);

  return {
    refreshDeleted,
    beginRename,
    addAfterShown,
    openNewBook,
    isAsking: () => !confirmScrim.hidden || !deletedScrim.hidden || !newBookScrim.hidden,
    cancelAsking() {
      pendingDelete = null;
      confirmScrim.hidden = true;
      deletedScrim.hidden = true;
      newBookScrim.hidden = true;
      refocus();
    },
    async openDeleted() {
      const rows = await refreshDeleted();
      if (rows.length === 0) {
        showToast('No deleted chapters can be restored.');
        return;
      }
      deletedScrim.hidden = false;
      element('deleted-close').focus();
    },
    removeShown() {
      const toc = tocData();
      const shown = shownIx();
      if (shown === null) return;
      const at = toc?.index.chosen.get(shown);
      askDelete(shown, at === undefined ? 'This chapter' : toc.entries[at].label);
    },
    remove(at) {
      const toc = tocData();
      if (!toc || !rowActions(toc, at).delete) return;
      askDelete(toc.entries[at].ix, toc.entries[at].label);
    },
    rename(at, title) {
      const toc = tocData();
      if (!toc || title.trim() === toc.entries[at].label) {
        refocus();
        return Promise.resolve();
      }
      return runOp({ kind: 'rename', row: rowRef(toc, at), title }, keepShown);
    },
    move(at, to) {
      const toc = tocData();
      if (!toc) return Promise.resolve();
      return runOp({ kind: 'move', row: rowRef(toc, at), to }, keepShown);
    },
    step(at, direction) {
      const toc = tocData();
      if (!toc || !rowActions(toc, at).move) return Promise.resolve();
      const to = stepSpot(toc, at, direction);
      if (!to) {
        showToast(direction < 0 ? 'This is the first place it can go.' : 'This is the last place it can go.');
        return Promise.resolve();
      }
      return runOp({ kind: 'move', row: rowRef(toc, at), to }, keepShown);
    },
  };
}
