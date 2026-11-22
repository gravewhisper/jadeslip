const GAP = 8;
const EDGE = 8;

export function createFormatPill({ toolbar, scroller, host, editing }) {
  const app = document.querySelector('.app');
  let dragging = false;

  function selection() {
    const shadow = host.shadowRoot;
    const body = shadow?.querySelector('.ss-body');
    if (!body) return null;
    const [picked] = document.getSelection().getComposedRanges({ shadowRoots: [shadow] });
    if (!picked || picked.collapsed || !body.contains(picked.startContainer) || !body.contains(picked.endContainer)) return null;
    const range = document.createRange();
    range.setStart(picked.startContainer, picked.startOffset);
    range.setEnd(picked.endContainer, picked.endOffset);
    return range;
  }

  function place() {
    const range = !dragging && editing() ? selection() : null;
    const rects = range ? [...range.getClientRects()].filter((rect) => rect.width > 0 || rect.height > 0) : [];
    const view = scroller.getBoundingClientRect();
    const top = rects[0];
    const bottom = rects[rects.length - 1];
    const shown = top !== undefined && bottom.bottom > view.top && top.top < view.bottom;
    toolbar.classList.toggle('near', shown);
    if (!shown) return;
    const frame = app.getBoundingClientRect();
    const width = toolbar.offsetWidth;
    const height = toolbar.offsetHeight;
    const lead = rects.find((rect) => rect.top >= view.top) ?? top;
    const above = lead.top - GAP - height;
    const y = above >= view.top + EDGE ? above : Math.min(bottom.bottom + GAP, view.bottom - EDGE - height);
    const middle = (Math.min(...rects.map((rect) => rect.left)) + Math.max(...rects.map((rect) => rect.right))) / 2;
    const x = Math.min(Math.max(middle - width / 2, view.left + EDGE), view.right - EDGE - width);
    toolbar.style.left = `${x - frame.left}px`;
    toolbar.style.top = `${y - frame.top}px`;
  }

  document.addEventListener('selectionchange', place);
  scroller.addEventListener('scroll', place, { passive: true });
  window.addEventListener('resize', place);
  host.addEventListener('pointerdown', () => {
    dragging = true;
    place();
  });
  for (const ending of ['pointerup', 'pointercancel', 'blur']) {
    window.addEventListener(ending, () => {
      if (!dragging) return;
      dragging = false;
      place();
    });
  }

  return { place };
}
