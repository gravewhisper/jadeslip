const UNDO_MS = 10000;

export function createUndoBar() {
  const bar = document.getElementById('undo-bar');
  const text = document.getElementById('undo-text');
  let undo = null;
  let timer = null;

  function hide() {
    clearTimeout(timer);
    bar.hidden = true;
    undo = null;
  }

  document.getElementById('undo-action').addEventListener('click', () => {
    const run = undo;
    hide();
    run?.();
  });

  return {
    show(message, run) {
      undo = run;
      text.textContent = message;
      bar.hidden = false;
      clearTimeout(timer);
      timer = setTimeout(hide, UNDO_MS);
    },
    hide,
  };
}
