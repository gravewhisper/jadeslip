import { ACROSS, pointAtTop, topOfPoint } from './textmap.js';

const AUTO_EM = 36;

export const PAGE_MODES = {
  scroll: () => 0,
  auto: (room, look) => (room >= 2 * (AUTO_EM * look.fontPx + look.marginLeft + look.marginRight) ? 2 : 1),
};

export function createPages({ scroller, host }) {
  const app = document.querySelector('.app');
  let mode = 'scroll';
  let look = null;
  let editing = false;
  let columns = 0;
  let start = 0;
  let shift = 0;
  let measured = null;

  const columnsFor = (inEdit) => (inEdit || !look ? 0 : PAGE_MODES[mode](scroller.clientWidth, look));

  function refit() {
    const next = columnsFor(editing);
    measured = null;
    if (next === columns) return;
    columns = next;
    app.classList.toggle('paged', columns > 0);
    app.classList.toggle('two-pages', columns === 2);
    host.classList.toggle('paged', columns > 0);
    start = 0;
    shift = 0;
    host.style.transform = '';
  }

  function measure() {
    if (measured) return measured;
    const gap = parseFloat(getComputedStyle(host).columnGap) || 0;
    const pitch = (host.clientWidth - (columns - 1) * gap) / columns + gap;
    const left = host.getBoundingClientRect().left + shift;
    const pageHeight = `${host.clientHeight}px`;
    if (host.style.getPropertyValue('--page-h') !== pageHeight) host.style.setProperty('--page-h', pageHeight);
    const pieces = host.shadowRoot?.querySelector('.ss-book')?.getClientRects() ?? [];
    const last = pieces.length > 0 ? Math.max(0, columnAt(pieces[pieces.length - 1].left, { pitch, left })) : 0;
    measured = { pitch, left, pages: last + 1, stops: Math.max(1, last + 2 - columns) };
    return measured;
  }

  const columnAt = (x, { pitch, left }) => Math.floor((x - left + shift + 0.5) / pitch);

  function moveTo(next) {
    const { pitch, stops } = measure();
    start = Math.min(Math.max(0, next), stops - 1);
    shift = start * pitch;
    host.style.transform = shift === 0 ? '' : `translateX(${-shift}px)`;
  }

  const pageAt = (x) => columnAt(x, measure());

  function pageLabel() {
    const { pages } = measure();
    const first = start + 1;
    const last = Math.min(first + columns - 1, pages);
    const of = pages.toLocaleString('en-US');
    return first === last ? `Page ${first} of ${of}` : `Pages ${first}-${last} of ${of}`;
  }

  return {
    configure(next) {
      mode = PAGE_MODES[next.mode] ? next.mode : 'scroll';
      look = next.look;
      refit();
    },
    setEditing(on) {
      editing = on;
      refit();
    },
    refit,
    on: () => columns > 0,
    flipsFor: (inEdit) => (columns > 0) !== (columnsFor(inEdit) > 0),
    mounted(keepPlace) {
      measured = null;
      if (columns > 0) moveTo(keepPlace ? start : 0);
    },
    firstPoint(body) {
      if (start === 0) return null;
      return pointAtTop(body, measure().left, ACROSS);
    },
    show(body, point) {
      measured = null;
      const left = point ? topOfPoint(body, point, ACROSS) : null;
      moveTo(left === null ? 0 : pageAt(left));
    },
    showRect(rect) {
      if (!rect) return;
      const page = pageAt(rect.left);
      if (page < start || page >= start + columns) moveTo(page);
    },
    turn(by) {
      const before = start;
      moveTo(start + by);
      return start !== before;
    },
    toEnd: () => moveTo(Number.MAX_SAFE_INTEGER),
    progress() {
      const { stops } = measure();
      return stops > 1 ? start / (stops - 1) : 1;
    },
    label: pageLabel,
  };
}
