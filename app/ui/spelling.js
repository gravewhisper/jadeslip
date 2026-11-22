import { spellAdd, spellBlock, spellChapter, spellLanguageSet, spellLanguages, spellSuggest } from './ipc.js';
import { pointFromClick } from './edit-session.js';
import { blocksOf, pointsAt, walkBlock } from './textmap.js';

const NAME = 'ss-spell';
const MARK = 'data-ss-b';
const XML = 'http://www.w3.org/XML/1998/namespace';
const NOT_PROSE = new Set(['code', 'pre', 'kbd', 'samp', 'var', 'tt']);
const ENGLISH = ['en', 'und', 'mul', 'zxx'];
const SETTLE_MS = 300;

const elementOf = (node) => (node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement);

const primaryOf = (tag) => tag.trim().split(/[-_]/)[0].toLowerCase();

function isExempt(node, tags) {
  let language = null;
  for (let at = elementOf(node); at; at = at.parentElement) {
    if (NOT_PROSE.has(at.localName)) return true;
    language ??= at.getAttributeNS(XML, 'lang') || at.getAttribute('lang') || null;
  }
  return language !== null && !tags.has(primaryOf(language));
}

export function languageName(code) {
  try {
    return new Intl.DisplayNames(['en'], { type: 'language' }).of(code.replace(/_/g, '-')) ?? code;
  } catch {
    return code;
  }
}

const sameChoice = (a, b) => a.kind === b.kind && (a.code ?? null) === (b.code ?? null);

function blockText(block) {
  let text = '';
  walkBlock(block, (data) => { text += data; }, () => { text += '\n'; });
  return text;
}

function rangesIn(block, found, tags) {
  const units = found.flatMap(({ start, end }) => [start.offset_u16, end.offset_u16]);
  const points = pointsAt(block, units);
  const ranges = [];
  for (let at = 0; at < points.length; at += 2) {
    const range = document.createRange();
    range.setStart(points[at].node, points[at].offset);
    range.setEnd(points[at + 1].node, points[at + 1].offset);
    if (!isExempt(range.endContainer, tags)) ranges.push(range);
  }
  return ranges;
}

export function createSpelling({ host, isEditing, isOn, showToast }) {
  const menu = document.getElementById('spell-menu');
  const languageRow = document.getElementById('spelling-language');
  const languageSeg = document.getElementById('spelling-languages');
  const autoHint = document.getElementById('spelling-auto');
  let tags = new Set(ENGLISH);
  let shown = null;
  let turn = 0;
  let marks = new Map();
  let touched = new Set();
  let timer = null;
  let asking = 0;
  let near = new Set();
  const nearby = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (entry.isIntersecting) near.add(entry.target);
      else near.delete(entry.target);
    }
    paint();
  }, { root: document.getElementById('scroller'), rootMargin: '25%' });

  function paint() {
    const visible = [...marks].filter(([block]) => near.has(block)).flatMap(([, ranges]) => ranges);
    CSS.highlights.set(NAME, new Highlight(...visible));
  }

  function mark(block, ranges) {
    if (ranges.length > 0) {
      marks.set(block, ranges);
      nearby.observe(block);
    } else {
      marks.delete(block);
      nearby.unobserve(block);
    }
  }

  async function recheck(blocks) {
    const started = turn;
    await Promise.all(blocks.map(async (block) => {
      const text = blockText(block);
      const found = await spellBlock(Number(block.getAttribute(MARK)), text);
      if (started !== turn || !block.isConnected || blockText(block) !== text) return;
      mark(block, rangesIn(block, found, tags));
    }));
    if (started === turn) paint();
  }

  const observer = new MutationObserver((records) => {
    for (const record of records) {
      const block = elementOf(record.target)?.closest(`[${MARK}]`);
      if (block) touched.add(block);
      for (const node of record.addedNodes) {
        if (node.nodeType !== Node.ELEMENT_NODE) continue;
        if (node.matches(`[${MARK}]`)) touched.add(node);
        for (const inner of blocksOf(node)) touched.add(inner);
      }
    }
    clearTimeout(timer);
    timer = setTimeout(() => {
      const blocks = [...touched].filter((block) => block.isConnected);
      for (const block of [...marks.keys()]) {
        if (!block.isConnected) mark(block, []);
      }
      recheck(blocks).catch(() => {});
    }, SETTLE_MS);
  });

  function closeMenu() {
    asking += 1;
    menu.hidden = true;
    menu.replaceChildren();
  }

  function reset() {
    turn += 1;
    closeMenu();
    clearTimeout(timer);
    observer.disconnect();
    nearby.disconnect();
    marks = new Map();
    near = new Set();
    touched = new Set();
    CSS.highlights.delete(NAME);
  }

  async function checkChapter(root, chapter, started) {
    const asked = performance.now();
    const reply = await spellChapter(chapter.ix, chapter.rev);
    if (started !== turn) return true;
    if (reply.state !== 'ranges') return false;
    tags = new Set(reply.tags);
    if (reply.notice) showToast(reply.notice);
    const blocks = new Map();
    for (const block of blocksOf(root)) {
      const number = Number(block.getAttribute(MARK));
      if (!blocks.has(number)) blocks.set(number, block);
    }
    if (root.hasAttribute(MARK)) blocks.set(Number(root.getAttribute(MARK)), root);
    const grouped = new Map();
    for (const range of reply.ranges) {
      const block = blocks.get(range.start.block);
      if (!block || touched.has(block) || range.end.block !== range.start.block) continue;
      if (!grouped.has(block)) grouped.set(block, []);
      grouped.get(block).push(range);
    }
    for (const [block, found] of grouped) mark(block, rangesIn(block, found, tags));
    paint();
    performance.mark('ss-spell-painted', { detail: { ms: performance.now() - asked, marks: reply.ranges.length } });
    return true;
  }

  function chapterShown(shadow, chapter) {
    reset();
    shown = { shadow, chapter };
    const root = shadow.querySelector('.ss-body');
    if (!root || !isOn()) return;
    observer.observe(root, { subtree: true, childList: true, characterData: true });
    const started = turn;
    requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(() => {
      if (started === turn) checkChapter(root, chapter, started).catch(() => {});
    })));
  }

  function restart() {
    reset();
    const root = shown?.shadow.querySelector('.ss-body');
    if (!root?.isConnected || !isOn()) return;
    observer.observe(root, { subtree: true, childList: true, characterData: true });
    if (!isEditing()) {
      const started = turn;
      checkChapter(root, shown.chapter, started)
        .then((done) => (done || started !== turn ? null : recheck(blocksOf(root))))
        .catch(() => {});
      return;
    }
    spellChapter(shown.chapter.ix, shown.chapter.rev)
      .then((reply) => {
        if (reply.state === 'ranges') tags = new Set(reply.tags);
        if (reply.notice) showToast(reply.notice);
        return recheck(blocksOf(root));
      })
      .catch(() => {});
  }

  function paintLanguages(state) {
    const choices = [
      { label: 'Auto', choice: { kind: 'auto' } },
      { label: 'English', choice: { kind: 'english' } },
      ...state.installed.map((code) => ({ label: languageName(code), choice: { kind: 'installed', code } })),
    ];
    languageSeg.replaceChildren(...choices.map(({ label, choice }) => {
      const button = document.createElement('button');
      button.textContent = label;
      button.classList.toggle('on', sameChoice(choice, state.choice));
      button.addEventListener('click', () => choose(choice));
      return button;
    }));
    const active = state.active?.kind === 'english' ? 'English' : languageName(state.active?.code ?? '');
    autoHint.textContent = state.active === null ? 'No dictionary matches this book, so it gets no marks.'
      : state.choice.kind === 'auto' ? `Auto checks this book in ${active}.`
        : `This book is checked in ${active}.`;
    languageRow.hidden = false;
  }

  async function choose(choice) {
    try {
      paintLanguages(await spellLanguageSet(choice));
      restart();
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
    }
  }

  async function showLanguages() {
    try {
      paintLanguages(await spellLanguages());
    } catch {
      languageRow.hidden = true;
    }
    return !languageRow.hidden;
  }

  function markAt(point) {
    const block = elementOf(point.node)?.closest(`[${MARK}]`);
    const ranges = block && marks.get(block);
    const range = ranges?.find((candidate) => candidate.comparePoint(point.node, point.offset) === 0);
    return range ? { block, range } : null;
  }

  function fix(found, word) {
    closeMenu();
    const { range } = found;
    document.getSelection().setBaseAndExtent(range.startContainer, range.startOffset, range.endContainer, range.endOffset);
    document.execCommand('insertText', false, word);
  }

  async function addWord(word) {
    closeMenu();
    try {
      await spellAdd(word);
      await recheck([...marks.keys()]);
    } catch (failure) {
      showToast(String(failure?.message ?? failure));
    }
  }

  function item(text, action) {
    const button = document.createElement('button');
    button.className = 'btn';
    button.setAttribute('role', 'menuitem');
    button.textContent = text;
    button.addEventListener('click', action);
    return button;
  }

  async function openMenu(found, x, y) {
    closeMenu();
    const asked = asking;
    const word = found.range.toString();
    const suggestions = await spellSuggest(word).catch(() => []);
    if (asked !== asking) return;
    const rows = suggestions.map((suggestion) => item(suggestion, () => fix(found, suggestion)));
    if (rows.length === 0) {
      const none = document.createElement('p');
      none.textContent = 'No suggestions';
      rows.push(none);
    }
    rows.push(item('Add to dictionary', () => addWord(word)));
    menu.replaceChildren(...rows);
    menu.hidden = false;
    const box = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(0, Math.min(x, innerWidth - box.width))}px`;
    menu.style.top = `${Math.max(0, Math.min(y, innerHeight - box.height))}px`;
  }

  host.addEventListener('contextmenu', (event) => {
    if (!isEditing()) return;
    const point = pointFromClick(host.shadowRoot, event.clientX, event.clientY);
    const found = point && markAt(point);
    if (!found) return;
    event.preventDefault();
    openMenu(found, event.clientX, event.clientY).catch(() => {});
  });
  menu.addEventListener('mousedown', (event) => event.preventDefault());
  document.addEventListener('mousedown', (event) => {
    if (!menu.hidden && !menu.contains(event.target)) closeMenu();
  }, true);
  document.addEventListener('scroll', () => {
    if (!menu.hidden) closeMenu();
  }, true);
  document.addEventListener('keydown', (event) => {
    if (menu.hidden) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
    }
    closeMenu();
  }, true);

  function cleared() {
    shown = null;
    reset();
  }

  return { chapterShown, cleared, restart, showLanguages };
}
