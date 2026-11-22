const SKIPPED = new Set(['img', 'image', 'svg', 'math', 'video', 'audio', 'canvas', 'picture']);

export function walkBlock(block, onText, onBreak) {
  if (SKIPPED.has(block.localName)) return;
  for (let node = block.firstChild; node; node = node.nextSibling) {
    if (node.nodeType === Node.TEXT_NODE || node.nodeType === Node.CDATA_SECTION_NODE) {
      onText(node.data, node);
    } else if (node.nodeType === Node.ELEMENT_NODE) {
      if (node.localName === 'br') onBreak(node);
      walkBlock(node, onText, onBreak);
    }
  }
}

export function blockLength(block) {
  let units = 0;
  walkBlock(block, (text) => { units += text.length; }, () => { units += 1; });
  return units;
}

export const blocksOf = (root) => root.querySelectorAll('[data-ss-b]');

export function unitsBefore(block, container, offset) {
  const boundary = document.createRange();
  boundary.setStart(container, offset);
  let units = 0;
  let found = null;
  const startsAfter = (node) => boundary.comparePoint(node, 0) > 0;
  walkBlock(block, (text, node) => {
    if (found !== null) return;
    if (node === container) found = units + offset;
    else if (startsAfter(node)) found = units;
    else units += text.length;
  }, (node) => {
    if (found !== null) return;
    if (startsAfter(node)) found = units;
    else units += 1;
  });
  return found ?? units;
}

const breakPoint = (node, after) => ({
  node: node.parentNode,
  offset: [...node.parentNode.childNodes].indexOf(node) + after,
});

export function pointsAt(block, sortedUnits) {
  const points = [];
  let at = 0;
  let before = 0;
  let last = () => ({ node: block, offset: block.childNodes.length });
  const settle = (reached, place) => {
    while (at < sortedUnits.length && sortedUnits[at] <= reached) {
      points.push(place(sortedUnits[at]));
      at += 1;
    }
  };
  walkBlock(block, (text, node) => {
    if (at === sortedUnits.length) return;
    settle(before + text.length, (units) => ({ node, offset: units - before }));
    before += text.length;
    last = () => ({ node, offset: text.length });
  }, (node) => {
    if (at === sortedUnits.length) return;
    settle(before, () => breakPoint(node, 0));
    before += 1;
    last = () => breakPoint(node, 1);
  });
  while (points.length < sortedUnits.length) points.push(last());
  return points;
}

export const pointAt = (block, units) => pointsAt(block, [units])[0];

function lowerBound(count, isAtOrAfter) {
  let low = 0;
  let high = count;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (isAtOrAfter(middle)) high = middle;
    else low = middle + 1;
  }
  return low;
}

const hasInk = (text) => /\S/.test(text);

function textRunsOf(block) {
  const runs = [];
  let before = 0;
  walkBlock(block, (text, node) => {
    if (hasInk(text)) runs.push({ node, before });
    before += text.length;
  }, () => { before += 1; });
  return runs;
}

function laidOutFrom(count, at, rectOf) {
  for (let probe = at; probe < count; probe += 1) {
    const box = rectOf(probe);
    if (box) return box;
  }
  return null;
}

function nodeRect(range, node) {
  range.selectNodeContents(node);
  return range.getClientRects().length > 0 ? range.getBoundingClientRect() : null;
}

function charRect(range, node, offset) {
  range.setStart(node, offset);
  range.setEnd(node, offset + 1);
  return range.getClientRects()[0] ?? null;
}

export const DOWN = { start: 'top', end: 'bottom' };
export const ACROSS = { start: 'left', end: 'right' };

function firstVisibleUnit(block, top, range, axis) {
  const runs = textRunsOf(block);
  const startingAt = lowerBound(runs.length, (at) => {
    const box = laidOutFrom(runs.length, at, (probe) => nodeRect(range, runs[probe].node));
    return box === null || box[axis.end] > top;
  });
  for (let at = startingAt; at < runs.length; at += 1) {
    const { node, before } = runs[at];
    const length = node.data.length;
    const offset = lowerBound(length, (char) => {
      const box = laidOutFrom(length, char, (probe) => charRect(range, node, probe));
      return box === null || box[axis.start] >= top - 0.5;
    });
    const first = laidOutFrom(length, offset, (probe) => (charRect(range, node, probe) ? { at: probe } : null));
    if (first) return before + first.at;
  }
  return null;
}

export function pointAtTop(root, top, axis = DOWN) {
  const blocks = blocksOf(root);
  const range = document.createRange();
  const startingAt = lowerBound(blocks.length, (at) => {
    const box = laidOutFrom(blocks.length, at, (probe) => (blocks[probe].getClientRects().length > 0 ? blocks[probe].getBoundingClientRect() : null));
    return box === null || box[axis.end] > top;
  });
  for (let at = startingAt; at < blocks.length; at += 1) {
    const offset = firstVisibleUnit(blocks[at], top, range, axis);
    if (offset !== null) return { block: Number(blocks[at].getAttribute('data-ss-b')), offset_u16: offset };
  }
  return null;
}

function runHolding(runs, units) {
  const after = lowerBound(runs.length, (at) => runs[at].before > units);
  if (after > 0) {
    const run = runs[after - 1];
    if (units < run.before + run.node.data.length) return { run, offset: units - run.before };
  }
  return after < runs.length ? { run: runs[after], offset: 0 } : null;
}

export function topOfPoint(root, point, axis = DOWN) {
  const block = root.querySelector(`[data-ss-b="${point.block}"]`);
  if (!block) return null;
  const found = runHolding(textRunsOf(block), point.offset_u16);
  if (found) {
    const range = document.createRange();
    const { node } = found.run;
    const box = laidOutFrom(node.data.length, found.offset, (probe) => charRect(range, node, probe));
    if (box) return box[axis.start];
  }
  return block.getBoundingClientRect()[axis.start];
}

function pointOf(root, node, offset) {
  const start = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
  const block = start?.closest('[data-ss-b]');
  if (!block || !root.contains(block)) return null;
  return { block: Number(block.getAttribute('data-ss-b')), offset_u16: unitsBefore(block, node, offset) };
}

export function selectedRange(shadow) {
  const root = shadow.querySelector('.ss-body');
  const [range] = document.getSelection().getComposedRanges({ shadowRoots: [shadow] });
  if (!root || !range || range.collapsed) return null;
  const start = pointOf(root, range.startContainer, range.startOffset);
  const end = pointOf(root, range.endContainer, range.endOffset);
  return start && end ? { start, end } : null;
}
