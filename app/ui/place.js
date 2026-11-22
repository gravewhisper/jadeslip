import { pointAtTop, topOfPoint } from './textmap.js';

const SETTLE_MS = 180;
const SAVE_MS = 300;
const STILL_MS = 34;
const STILL_WAIT_MS = 300;

export function createPlaces({ scroller, host, pages, chapterNow, save, onProgress }) {
  let held = null;
  let expected = null;
  let settleTimer = null;
  let saveTimer = null;
  let width = scroller.clientWidth;
  let height = scroller.clientHeight;
  let watched = null;

  const root = () => host.shadowRoot?.querySelector('.ss-body') ?? null;
  const viewTop = () => scroller.getBoundingClientRect().top;

  function capture() {
    const body = root();
    if (body && pages.on()) return { point: pages.firstPoint(body), inset: 0 };
    if (!body || scroller.scrollTop <= 0) return { point: null, inset: 0 };
    const top = viewTop();
    const point = pointAtTop(body, top);
    if (!point) return { point: null, inset: 0 };
    return { point, inset: topOfPoint(body, point) - top };
  }

  function apply(place) {
    const body = root();
    if (!body) return;
    if (pages.on()) {
      pages.show(body, place.point);
      onProgress();
      return;
    }
    const at = place.point ? topOfPoint(body, place.point) : null;
    const target = at === null ? 0 : scroller.scrollTop + at - viewTop() - place.inset;
    scroller.scrollTop = target;
    expected = scroller.scrollTop;
    onProgress();
  }

  function queueSave() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      const shown = chapterNow();
      if (shown) save(shown.ix, held?.point ?? null);
    }, SAVE_MS);
  }

  function settle() {
    held = capture();
    queueSave();
  }

  function onScroll() {
    onProgress();
    const ours = expected !== null && Math.abs(scroller.scrollTop - expected) < 1;
    expected = null;
    if (ours) return;
    held = null;
    clearTimeout(settleTimer);
    settleTimer = setTimeout(settle, SETTLE_MS);
  }

  function hold() {
    clearTimeout(settleTimer);
    held ??= capture();
    return held;
  }

  function keep(change, point) {
    clearTimeout(settleTimer);
    const place = point === undefined ? hold() : { point, inset: 0 };
    change();
    held = place;
    apply(place);
    queueSave();
  }

  function moved() {
    clearTimeout(settleTimer);
    held = capture();
    queueSave();
    onProgress();
  }

  function reapply() {
    pages.refit();
    if (held) apply(held);
    else if (pages.on()) onProgress();
  }

  function restore(point) {
    clearTimeout(settleTimer);
    held = { point, inset: 0 };
    apply(held);
  }

  function chapterShown(keepScroll = false) {
    if (host.shadowRoot !== watched) {
      watched = host.shadowRoot;
      watched?.addEventListener('load', reapply, true);
    }
    clearTimeout(settleTimer);
    held = keepScroll ? capture() : null;
    expected = scroller.scrollTop;
    settleTimer = setTimeout(settle, keepScroll ? SETTLE_MS : 0);
    onProgress();
  }

  scroller.addEventListener('scroll', onScroll, { passive: true });
  new ResizeObserver(() => {
    const paged = pages.on();
    if (scroller.clientWidth === width && (scroller.clientHeight === height || !paged)) return;
    width = scroller.clientWidth;
    height = scroller.clientHeight;
    reapply();
    if (paged || pages.on()) onProgress();
  }).observe(scroller);
  document.fonts.addEventListener('loadingdone', reapply);

  async function untilStill() {
    const deadline = performance.now() + STILL_WAIT_MS;
    let top = scroller.scrollTop;
    for (;;) {
      await new Promise((done) => setTimeout(done, STILL_MS));
      if (scroller.scrollTop === top || performance.now() > deadline) return;
      top = scroller.scrollTop;
    }
  }

  function flush() {
    clearTimeout(saveTimer);
    const shown = chapterNow();
    if (!shown) return Promise.resolve();
    return save(shown.ix, hold().point);
  }

  async function flushWhenStill() {
    await untilStill();
    return flush();
  }

  return { keep, moved, restore, chapterShown, flush, flushWhenStill, pointNow: () => hold().point };
}
