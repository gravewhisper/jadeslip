function subtreeEnd(toc, at) {
  const { depth } = toc.entries[at];
  let end = at + 1;
  while (end < toc.entries.length && toc.entries[end].depth > depth) end += 1;
  return end;
}

export const isInside = (toc, moved, target) => target >= moved && target < subtreeEnd(toc, moved);

function isSection(toc, at) {
  const { ix } = toc.entries[at];
  return ix !== null && toc.index.first.get(ix) !== at;
}

export function rowActions(toc, at) {
  const entry = toc.entries[at];
  if (!entry) return { move: false, rename: false, delete: false };
  const section = isSection(toc, at);
  const hasChapters = entry.ix !== null || (entry.isParent && entry.first !== null);
  return {
    move: !section && hasChapters,
    rename: true,
    delete: entry.ix !== null && !section,
  };
}

export const rowRef = (toc, at) => (toc.source === 'headings' ? { chapter: toc.entries[at].ix } : { entry: at });

function previousSibling(toc, at) {
  const { depth, parent } = toc.entries[at];
  for (let probe = at - 1; probe >= 0; probe -= 1) {
    const entry = toc.entries[probe];
    if (entry.depth < depth) return null;
    if (entry.depth === depth && entry.parent === parent) return probe;
  }
  return null;
}

function nextSibling(toc, at) {
  const end = subtreeEnd(toc, at);
  const next = toc.entries[end];
  return next && next.parent === toc.entries[at].parent ? end : null;
}

export function stepSpot(toc, at, direction) {
  const { parent } = toc.entries[at];
  if (direction < 0) {
    const previous = previousSibling(toc, at);
    if (previous !== null) return { row: rowRef(toc, previous), after: false };
    return parent === null ? null : { row: rowRef(toc, parent), after: false };
  }
  const next = nextSibling(toc, at);
  if (next !== null) return { row: rowRef(toc, next), after: true };
  return parent === null ? null : { row: rowRef(toc, parent), after: true };
}

export function dropSpot(toc, shown, row, lower, moved) {
  const target = shown[row];
  if (target === undefined || isInside(toc, moved, target)) return null;
  const below = shown[row + 1];
  if (lower && below !== undefined && toc.entries[below].parent === target) {
    return isInside(toc, moved, below) ? null : { row: rowRef(toc, below), after: false };
  }
  return { row: rowRef(toc, target), after: lower };
}
