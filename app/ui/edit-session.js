import { blocksOf, pointAt, unitsBefore } from './textmap.js';

const XHTML = 'http://www.w3.org/1999/xhtml';
const MARK = 'data-ss-b';
const WRAPPER = 'data-ss-w';
const INLINE = new Set([
  'a', 'abbr', 'b', 'bdi', 'bdo', 'big', 'br', 'cite', 'code', 'del', 'dfn', 'em', 'font', 'i', 'img', 'ins',
  'kbd', 'mark', 'q', 'rp', 'rt', 'ruby', 's', 'samp', 'small', 'span', 'strike', 'strong', 'sub', 'sup', 'time',
  'tt', 'u', 'var', 'wbr',
]);
const INLINE_INSIDE_LINE = new Set([...INLINE].filter((name) => name !== 'br'));

const serializer = new XMLSerializer();
const numberOf = (element) => Number(element.getAttribute(MARK));
const isBlank = (node) => node.nodeType !== Node.ELEMENT_NODE && /^\s*$/.test(node.data ?? '');
const isMarked = (element) => element.hasAttribute(MARK);

function serializeBlock(element, root) {
  if (element !== root) return serializer.serializeToString(element);
  const clean = element.cloneNode(true);
  clean.removeAttribute('contenteditable');
  return serializer.serializeToString(clean);
}

function serializeLoose(nodes) {
  const wrapper = document.createElementNS(XHTML, 'div');
  for (const node of nodes) wrapper.append(node.cloneNode(true));
  return serializer.serializeToString(wrapper);
}

function occurrencesIn(root, gaps) {
  if (isMarked(root)) return [{ element: root, number: numberOf(root) }];
  const found = [];
  const visit = (container) => {
    let loose = [];
    const flush = () => {
      if (loose.some((node) => !isBlank(node))) found.push({ loose });
      loose = [];
    };
    for (const node of container.childNodes) {
      if (node.nodeType !== Node.ELEMENT_NODE) {
        if (node.nodeType === Node.TEXT_NODE && (loose.length > 0 || !isBlank(node))) loose.push(node);
      } else if (isMarked(node)) {
        flush();
        found.push({ element: node, number: numberOf(node) });
      } else if (gaps.has(node)) {
        flush();
      } else if (node.hasAttribute(WRAPPER)) {
        flush();
        visit(node);
      } else if (INLINE.has(node.localName) && !node.querySelector(`[${MARK}]`)) {
        if (node.localName !== 'br' || loose.length > 0) loose.push(node);
      } else {
        flush();
        found.push({ element: node, number: null });
      }
    }
    flush();
  };
  visit(root);
  return found;
}

function gapElements(root) {
  const gaps = new WeakSet();
  if (isMarked(root)) return gaps;
  const visit = (container) => {
    for (const node of container.children) {
      if (node.hasAttribute(WRAPPER)) visit(node);
      else if (!isMarked(node)) gaps.add(node);
    }
  };
  visit(root);
  return gaps;
}

export function writingPoint(shadow) {
  const blocks = shadow?.querySelectorAll(`.ss-body [${MARK}]`) ?? [];
  const last = blocks[blocks.length - 1];
  return last && last.textContent === '' ? { node: last, offset: 0 } : null;
}

function selectionPoint(shadow) {
  const [range] = document.getSelection().getComposedRanges({ shadowRoots: [shadow] });
  const root = shadow.querySelector('.ss-body');
  if (!range || !root?.contains(range.startContainer)) return null;
  return { node: range.startContainer, offset: range.startOffset };
}

function elementOf(node) {
  return node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
}

function dominantBreakRun(block) {
  const counts = new Map();
  let run = 0;
  const finish = () => {
    if (run > 0) counts.set(run, (counts.get(run) ?? 0) + 1);
    run = 0;
  };
  for (const node of block.childNodes) {
    if (node.localName === 'br') run += 1;
    else if (!isBlank(node)) finish();
  }
  finish();
  let best = 1;
  let bestCount = 0;
  for (const [length, count] of counts) {
    if (count > bestCount) {
      best = length;
      bestCount = count;
    }
  }
  return best;
}

function isLineBlock(block, root) {
  if (block === root) return true;
  return block.localName === 'div' && [...block.children].some((child) => child.localName === 'br');
}

function caretInsideNestedShape(node, block) {
  for (let at = elementOf(node); at && at !== block; at = at.parentElement) {
    if (!INLINE_INSIDE_LINE.has(at.localName)) return true;
  }
  return false;
}

function parsedDisplay(display) {
  const parsed = new DOMParser().parseFromString(`<x xmlns="${XHTML}" xmlns:xlink="http://www.w3.org/1999/xlink">${display}</x>`, 'application/xhtml+xml');
  if (parsed.getElementsByTagName('parsererror').length > 0) return null;
  return parsed.documentElement;
}

function applyDisplay(element, display, number, root) {
  const parsed = parsedDisplay(display);
  if (!parsed) return null;
  const only = parsed.firstElementChild;
  if (only && only === parsed.lastElementChild && isMarked(only) && parsed.childNodes.length === 1) {
    only.setAttribute(MARK, String(number));
    if (serializer.serializeToString(only) === serializeBlock(element, root)) return element;
    const adopted = document.adoptNode(only);
    element.replaceWith(adopted);
    return adopted;
  }
  const shell = element.cloneNode(false);
  shell.removeAttribute('contenteditable');
  for (const child of [...parsed.childNodes]) shell.append(child);
  if (serializer.serializeToString(shell) === serializeBlock(element, root)) return element;
  element.replaceChildren(...[...shell.childNodes].map((child) => document.adoptNode(child)));
  return element;
}

export class EditSession {
  constructor(shadow) {
    this.shadow = shadow;
    this.root = shadow.querySelector('.ss-body');
    this.baseline = new Map();
    this.sequence = [];
    this.dirtyBlocks = new Set();
    this.structural = false;
    this.frozen = false;
    this.queuedText = '';
    this.gaps = new WeakSet();
    this.onDirty = null;
    this.frame = null;
    this.sent = null;
    this.onPasteRefused = null;
    this.observer = new MutationObserver((records) => {
      this.note(records);
      this.scheduleRefresh();
    });
    this.handlers = {
      beforeinput: (event) => this.onBeforeInput(event),
      paste: (event) => this.onPaste(event),
      drop: (event) => event.preventDefault(),
      dragover: (event) => event.preventDefault(),
    };
  }

  captureBaseline() {
    this.baseline = new Map();
    this.gaps = gapElements(this.root);
    this.sequence = occurrencesIn(this.root, this.gaps).map((item) => item.number);
    const blocks = isMarked(this.root) ? [this.root] : blocksOf(this.root);
    for (const block of blocks) {
      const number = numberOf(block);
      if (!this.baseline.has(number)) this.baseline.set(number, serializeBlock(block, this.root));
    }
    this.dirtyBlocks.clear();
    this.structural = false;
  }

  begin(onDirty) {
    this.onDirty = onDirty;
    this.root.setAttribute('contenteditable', 'true');
    this.captureBaseline();
    for (const [name, handler] of Object.entries(this.handlers)) this.root.addEventListener(name, handler);
    this.watch();
    this.root.focus({ preventScroll: true });
  }

  watch() {
    this.observer.observe(this.root, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
    });
  }

  end() {
    this.observer.disconnect();
    if (this.frame !== null) cancelAnimationFrame(this.frame);
    this.frame = null;
    for (const [name, handler] of Object.entries(this.handlers)) this.root.removeEventListener(name, handler);
    this.root.removeAttribute('contenteditable');
    this.root.blur();
  }

  note(records) {
    for (const record of records) {
      if (record.target === this.root && !isMarked(this.root)) {
        if (record.type !== 'attributes') this.structural = true;
        continue;
      }
      const inside = elementOf(record.target);
      const block = inside?.closest(`[${MARK}]`);
      if (!block || !this.root.contains(block)) {
        this.structural = true;
        continue;
      }
      this.dirtyBlocks.add(numberOf(block));
      if (record.type === 'childList' && (record.addedNodes.length > 0 || record.removedNodes.length > 0)) {
        for (const node of [...record.addedNodes, ...record.removedNodes]) {
          if (node.nodeType === Node.ELEMENT_NODE && (isMarked(node) || node.querySelector(`[${MARK}]`))) this.structural = true;
        }
      }
    }
  }

  scheduleRefresh() {
    if (this.frame !== null) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = null;
      this.onDirty?.(this.refresh());
    });
  }

  refresh() {
    this.note(this.observer.takeRecords());
    const items = occurrencesIn(this.root, this.gaps);
    const firsts = new Map();
    for (const item of items) {
      if (item.number !== null && !firsts.has(item.number)) firsts.set(item.number, item.element);
    }
    for (const number of [...this.dirtyBlocks]) {
      const element = firsts.get(number);
      if (element && serializeBlock(element, this.root) === this.baseline.get(number)) this.dirtyBlocks.delete(number);
    }
    const same = items.length === this.sequence.length
      && items.every((item, at) => item.number === this.sequence[at]);
    this.structural = !same;
    return this.dirtyBlocks.size > 0 || this.structural;
  }

  collect() {
    const { changes, owners } = this.gather();
    this.sent = owners;
    return changes;
  }

  pending() {
    return this.gather().changes;
  }

  gather() {
    this.refresh();
    const items = occurrencesIn(this.root, this.gaps);
    const changes = [];
    const owners = [];
    const seen = new Set();
    let anchor = null;
    let highest = -1;
    const insert = (item) => {
      const xml = item.loose ? serializeLoose(item.loose) : serializeBlock(item.element, this.root);
      changes.push({ kind: 'insertAfter', after: anchor, xml });
      owners.push({ item, change: changes.length - 1 });
    };
    for (const item of items) {
      const isNew = typeof item.number !== 'number';
      if (isNew || seen.has(item.number) || item.number < highest) {
        insert(item);
      } else {
        seen.add(item.number);
        highest = item.number;
        anchor = item.number;
        if (this.dirtyBlocks.has(item.number)) {
          changes.push({ kind: 'replace', of: item.number, xml: serializeBlock(item.element, this.root) });
          owners.push({ item, change: changes.length - 1 });
        } else {
          owners.push({ item, change: null });
        }
      }
    }
    for (const number of this.baseline.keys()) {
      if (!seen.has(number)) changes.push({ kind: 'delete', of: number });
    }
    return { changes, owners };
  }

  replay(changes) {
    const blockOf = (number) => (isMarked(this.root) ? this.root : this.root.querySelector(`[${MARK}='${number}']`));
    const parsed = (xml) => {
      const holder = parsedDisplay(xml);
      return holder ? [...holder.childNodes].map((node) => document.adoptNode(node)) : null;
    };
    let last = null;
    for (const change of changes) {
      if (change.kind === 'delete') {
        blockOf(change.of)?.remove();
        last = null;
        continue;
      }
      const nodes = parsed(change.xml);
      if (!nodes) return false;
      if (change.kind === 'replace') {
        const element = blockOf(change.of);
        if (!element) return false;
        if (element === this.root) {
          const only = nodes.length === 1 ? nodes[0] : null;
          element.replaceChildren(...(only?.nodeType === Node.ELEMENT_NODE ? [...only.childNodes] : nodes));
        } else {
          element.replaceWith(...nodes);
        }
        this.dirtyBlocks.add(change.of);
        last = { after: change.of, node: nodes[nodes.length - 1] };
        continue;
      }
      if (last && last.after === change.after && last.node?.isConnected) {
        last.node.after(...nodes);
      } else if (change.after === null) {
        const first = this.root.querySelector(`[${MARK}]`);
        if (first) first.before(...nodes);
        else this.root.append(...nodes);
      } else {
        const anchor = blockOf(change.after);
        if (!anchor) return false;
        anchor.after(...nodes);
      }
      last = { after: change.after, node: nodes[nodes.length - 1] };
    }
    return true;
  }

  setEditable(editable) {
    if (editable) this.root.setAttribute('contenteditable', 'true');
    else this.root.removeAttribute('contenteditable');
  }

  freeze() {
    this.frozen = true;
    this.observer.disconnect();
  }

  reconcile(after) {
    const owners = this.sent;
    this.sent = null;
    if (!after || after.length !== owners.length) return false;
    const place = this.place(owners);
    for (let at = 0; at < after.length; at += 1) {
      const { origin } = after[at];
      const owner = owners[at];
      if (owner.item.loose) return false;
      if (origin.kind === 'kept' && owner.item.number !== origin.ix) return false;
      if (origin.kind === 'change' && owner.change !== origin.ix) return false;
    }
    for (let at = 0; at < after.length; at += 1) {
      const { origin, display } = after[at];
      const { item } = owners[at];
      let element = item.element;
      if (origin.kind === 'change' && typeof display === 'string') {
        element = applyDisplay(element, display, at, this.root);
        if (!element) return false;
        item.element = element;
      }
      element.setAttribute(MARK, String(at));
    }
    this.restore(place, owners);
    return true;
  }

  place(owners) {
    const point = selectionPoint(this.shadow);
    if (!point) return null;
    const at = owners.findIndex(({ item }) => item.element?.contains(point.node));
    if (at < 0) return null;
    return { at, units: unitsBefore(owners[at].item.element, point.node, point.offset) };
  }

  restore(place, owners) {
    if (!place) return;
    const point = selectionPoint(this.shadow);
    if (point && owners[place.at].item.element.contains(point.node)) return;
    const found = pointAt(owners[place.at].item.element, place.units);
    document.getSelection().setBaseAndExtent(found.node, found.offset, found.node, found.offset);
  }

  rebase() {
    this.observer.disconnect();
    this.captureBaseline();
    this.thaw();
  }

  thaw() {
    this.frozen = false;
    this.watch();
    const queued = this.queuedText;
    this.queuedText = '';
    if (queued) document.execCommand('insertText', false, queued);
  }

  caret() {
    const point = selectionPoint(this.shadow);
    if (!point || !this.root.contains(point.node)) return null;
    const block = elementOf(point.node).closest(`[${MARK}]`);
    if (!block) return null;
    return { number: numberOf(block), units: unitsBefore(block, point.node, point.offset) };
  }

  setCaret(caret) {
    if (!caret) return;
    const block = isMarked(this.root) ? this.root : this.root.querySelector(`[${MARK}='${caret.number}']`);
    if (!block) return;
    const found = pointAt(block, caret.units);
    document.getSelection().setBaseAndExtent(found.node, found.offset, found.node, found.offset);
  }

  insertParagraph() {
    const point = selectionPoint(this.shadow);
    const block = point && elementOf(point.node).closest(`[${MARK}]`);
    if (block && isLineBlock(block, this.root) && !caretInsideNestedShape(point.node, block)) {
      const run = dominantBreakRun(block);
      for (let count = 0; count < run; count += 1) document.execCommand('insertLineBreak');
      return;
    }
    document.execCommand('insertParagraph');
  }

  toggleBlock(tag) {
    const point = selectionPoint(this.shadow);
    if (!point) return;
    const inside = elementOf(point.node).closest(tag);
    const plain = elementOf(point.node).closest(`[${MARK}]`)?.localName === 'div' ? 'div' : 'p';
    this.blockCommand('formatBlock', inside && this.root.contains(inside) ? plain : tag);
  }

  blockCommand(name, value) {
    const point = selectionPoint(this.shadow);
    const number = point ? elementOf(point.node).closest(`[${MARK}]`)?.getAttribute(MARK) : null;
    document.execCommand(name, false, value);
    if (number === null || number === undefined) return;
    if (this.root.querySelector(`[${MARK}='${number}']`) || isMarked(this.root)) return;
    const now = selectionPoint(this.shadow);
    if (!now) return;
    let top = elementOf(now.node);
    while (top.parentElement !== this.root && !top.parentElement?.hasAttribute(WRAPPER)) top = top.parentElement;
    if (top && !isMarked(top)) top.setAttribute(MARK, number);
  }

  onBeforeInput(event) {
    if (this.frozen) {
      event.preventDefault();
      if (event.inputType === 'insertText' && event.data) this.queuedText += event.data;
      return;
    }
    if (['insertFromPaste', 'insertFromDrop', 'insertFromYank', 'insertFromPasteAsQuotation'].includes(event.inputType)) {
      event.preventDefault();
    }
  }

  onPaste(event) {
    event.preventDefault();
    if (this.frozen) return;
    const text = event.clipboardData?.getData('text/plain') ?? '';
    if (!text) {
      this.onPasteRefused?.();
      return;
    }
    this.insertPlainText(text);
  }

  insertPlainText(text) {
    const lines = text.replace(/\r\n?/g, '\n').split('\n');
    const point = selectionPoint(this.shadow);
    const block = point && elementOf(point.node).closest(`[${MARK}]`);
    const lineShaped = block && isLineBlock(block, this.root) && !caretInsideNestedShape(point.node, block);
    lines.forEach((line, at) => {
      if (at > 0) {
        if (lineShaped) document.execCommand('insertLineBreak');
        else if (line.length > 0 || at === lines.length - 1) document.execCommand('insertParagraph');
      }
      if (line.length > 0) document.execCommand('insertText', false, line);
    });
  }

  command(name, value) {
    return document.execCommand(name, false, value);
  }
}

function lineBox(point) {
  const range = document.createRange();
  range.setStart(point.node, point.offset);
  return range.getClientRects()[0] ?? elementOf(point.node).getBoundingClientRect();
}

function caretAt(shadow, x, y) {
  const found = document.caretPositionFromPoint?.(x, y, { shadowRoots: [shadow] });
  return found && shadow.contains(found.offsetNode) ? { node: found.offsetNode, offset: found.offset } : null;
}

export function visiblePoint(shadow, top, bottom, left) {
  const point = selectionPoint(shadow);
  if (point) {
    const box = lineBox(point);
    if (box.top >= top && box.bottom <= bottom) return point;
  }
  let y = top + 1;
  for (let tries = 0; tries < 4 && y < bottom; tries += 1) {
    const found = caretAt(shadow, left, y);
    if (!found) break;
    const box = lineBox(found);
    if (box.top >= top) return found;
    y = Math.max(y + 1, box.bottom + 1);
  }
  const walker = document.createTreeWalker(shadow.querySelector('.ss-body'), NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  while (walker.nextNode()) {
    const node = walker.currentNode;
    if (isBlank(node)) continue;
    range.selectNodeContents(node);
    const box = range.getBoundingClientRect();
    if (box.height > 0 && box.top >= top) return { node, offset: 0 };
  }
  return null;
}

export function pointFromClick(shadow, x, y) {
  const found = document.caretPositionFromPoint?.(x, y, { shadowRoots: [shadow] });
  if (found && shadow.contains(found.offsetNode)) return { node: found.offsetNode, offset: found.offset };
  return selectionPoint(shadow);
}
