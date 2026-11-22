import { applyCancel, applyNext, applyStart } from './ipc.js';
import { appliedText, progressText } from './replace-model.js';

const element = (id) => document.getElementById(id);

export function createApplier({ showToast }) {
  const dialog = element('apply-dialog');
  const title = element('apply-title');
  const text = element('apply-progress');
  const fill = element('apply-fill');
  const stop = element('apply-stop');
  let nextId = 0;
  let running = null;

  function show(batch) {
    text.textContent = progressText(batch);
    fill.style.width = batch.total === 0 ? '0%' : `${Math.round((batch.done / batch.total) * 100)}%`;
    stop.disabled = batch.total > 0 && batch.done >= batch.total;
  }

  function requestStop() {
    if (running !== null) void applyCancel(running);
  }

  stop.addEventListener('click', requestStop);

  async function run(slot, previewId, exclude, changes, { heading = null, quiet = false } = {}) {
    nextId += 1;
    const id = nextId;
    running = id;
    title.textContent = heading ?? `Saving ${changes.toLocaleString('en-US')} ${changes === 1 ? 'change' : 'changes'}`;
    show({ done: 0, total: 0 });
    dialog.hidden = false;
    stop.focus();
    try {
      await applyStart(slot, previewId, id, exclude);
      for (;;) {
        const batch = await applyNext(id);
        show(batch);
        if (batch.end) {
          if (batch.end.kind === 'done' && !quiet) {
            showToast(appliedText(batch.end));
          } else if (batch.end.kind === 'cancelled') {
            showToast('Stopped. The book was not changed.');
          }
          return batch.end;
        }
      }
    } catch (failure) {
      return { kind: 'failed', reason: 'other', message: String(failure?.message ?? failure) };
    } finally {
      running = null;
      dialog.hidden = true;
    }
  }

  return { run, isBusy: () => running !== null, requestStop };
}
