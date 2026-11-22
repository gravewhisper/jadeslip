import { prefsGet, prefsSet } from './ipc.js';

export const FONT_RANGE = { min: 14, max: 28 };
const element = (id) => document.getElementById(id);

const px = (value) => `${value}px`;
const pxLabel = (value) => `${value} px`;
const hundredths = (value) => (value / 100).toFixed(2);

export const PRESETS = {
  compact: { fontPx: 17, lineHeight: 150, paraGap: 40 },
  comfortable: { fontPx: 18, lineHeight: 160, paraGap: 60 },
};

export const isPreset = (prefs, values) => Object.entries(values).every(([key, value]) => prefs[key] === value);

export const LAYOUT = [
  { key: 'width', label: 'Column width', min: 400, max: 1400, step: 4, start: 636, prop: '--measure', css: px, show: pxLabel, group: 'layout-column' },
  { key: 'marginTop', label: 'Top', min: 0, max: 200, step: 2, start: 44, prop: '--margin-top', css: px, show: pxLabel, group: 'layout-margins' },
  { key: 'marginBottom', label: 'Bottom', min: 0, max: 200, step: 2, start: 72, prop: '--margin-bottom', css: px, show: pxLabel, group: 'layout-margins' },
  { key: 'marginLeft', label: 'Left', min: 0, max: 160, step: 2, start: 32, prop: '--margin-left', css: px, show: pxLabel, group: 'layout-margins' },
  { key: 'marginRight', label: 'Right', min: 0, max: 160, step: 2, start: 32, prop: '--margin-right', css: px, show: pxLabel, group: 'layout-margins' },
  { key: 'lineHeight', label: 'Line spacing', min: 120, max: 240, step: 2, start: PRESETS.comfortable.lineHeight, prop: '--line-height', css: hundredths, show: hundredths, group: 'layout-spacing' },
  { key: 'paraGap', label: 'Paragraph spacing', min: 0, max: 250, step: 5, start: PRESETS.comfortable.paraGap, prop: '--para-gap', css: (value) => `${hundredths(value)}em`, show: (value) => `${hundredths(value)} em`, group: 'layout-spacing' },
  { key: 'indent', label: 'First-line indent', min: 0, max: 30, step: 5, start: 0, prop: '--para-indent', css: (value) => (value === 0 ? null : `${value / 10}em`), show: (value) => (value === 0 ? "Book's own" : `${value / 10} em`), group: 'layout-spacing' },
];

export const FONTS = {
  literata: { label: 'Literata', family: "'Literata',Georgia,serif" },
  plexSans: { label: 'IBM Plex Sans', family: "'IBM Plex Sans',system-ui,sans-serif" },
  serif: { label: 'System serif', family: 'serif' },
  sans: { label: 'System sans', family: 'sans-serif' },
  mono: { label: 'System mono', family: 'monospace' },
};

const LAYOUT_START = Object.fromEntries([...LAYOUT.map((control) => [control.key, control.start]), ['font', 'literata']]);

function sliderRow(control) {
  const row = document.createElement('label');
  row.className = 'slide';
  const name = document.createElement('span');
  name.className = 'poplabel';
  name.textContent = control.label;
  const now = document.createElement('span');
  now.className = 'slidenow';
  now.id = `layout-${control.key}-now`;
  const input = document.createElement('input');
  input.type = 'range';
  input.id = `layout-${control.key}`;
  Object.assign(input, { min: control.min, max: control.max, step: control.step });
  row.append(name, now, input);
  return row;
}

const TABS = ['text', 'layout', 'spelling'];

function buildLayout() {
  for (const control of LAYOUT) element(control.group).append(sliderRow(control));
  const fonts = element('layout-font');
  for (const [value, face] of Object.entries(FONTS)) fonts.append(new Option(face.label, value));
}

export function createPrefs({ keepPlace, pages, showToast, onContents, paintContents, onSpelling, onComfortOpen, onMatchCase }) {
  const app = document.querySelector('.app');
  const sheet = document.querySelector('.sheet');
  const pop = element('comfort');
  const toggle = element('toggle-comfort');
  const contents = element('toggle-contents');
  let prefs = { theme: 'dark', fontPx: PRESETS.comfortable.fontPx, contents: true, spelling: true, pages: 'scroll', matchCase: false, shelfView: 'grid', shelfSort: 'opened', shelfRecent: true, keepSnapshots: 'forever', ...LAYOUT_START };
  const listeners = new Set();

  buildLayout();

  function paint() {
    app.classList.toggle('dark', prefs.theme === 'dark');
    app.classList.toggle('light', prefs.theme === 'light');
    element('book-host').classList.toggle('theme-ink', prefs.theme === 'dark');
    app.style.setProperty('--reader-font', `${prefs.fontPx}px`);
    document.documentElement.style.fontSize = `${prefs.fontPx}px`;
    for (const control of LAYOUT) {
      const value = prefs[control.key];
      const css = control.css(value);
      if (css === null) sheet.style.removeProperty(control.prop);
      else sheet.style.setProperty(control.prop, css);
      element(`layout-${control.key}`).value = value;
      element(`layout-${control.key}-now`).textContent = control.show(value);
    }
    sheet.style.setProperty('--reading-font', (FONTS[prefs.font] ?? FONTS.literata).family);
    element('layout-font').value = prefs.font;
    app.classList.toggle('no-contents', !prefs.contents);
    paintContents(prefs.contents);
    element('font-now').textContent = `${prefs.fontPx}px`;
    element('font-smaller').disabled = prefs.fontPx <= FONT_RANGE.min;
    element('font-larger').disabled = prefs.fontPx >= FONT_RANGE.max;
    element('search-case').classList.toggle('on', prefs.matchCase);
    element('search-case').setAttribute('aria-pressed', String(prefs.matchCase));
    for (const [name, values] of Object.entries(PRESETS)) element(`preset-${name}`).classList.toggle('on', isPreset(prefs, values));
    pages.configure({ mode: prefs.pages, look: { fontPx: prefs.fontPx, marginLeft: prefs.marginLeft, marginRight: prefs.marginRight } });
    for (const listener of listeners) listener(prefs);
  }

  function showTab(name) {
    for (const tab of TABS) {
      const button = element(`comfort-tab-${tab}`);
      button.classList.toggle('on', tab === name);
      button.setAttribute('aria-selected', String(tab === name));
      element(`comfort-${tab}`).hidden = tab !== name;
    }
  }

  function show(patch) {
    prefs = { ...prefs, ...patch };
    keepPlace(paint);
  }

  function save() {
    prefsSet(prefs).catch((failure) => showToast(`Your settings could not be saved. ${failure}`));
  }

  function change(patch) {
    show(patch);
    save();
  }

  const bumpFont = (by) => change({
    fontPx: Math.min(FONT_RANGE.max, Math.max(FONT_RANGE.min, prefs.fontPx + by)),
  });

  function paintSpellingTab(shown) {
    element('comfort-tab-spelling').hidden = !shown;
    if (!shown && !element('comfort-spelling').hidden) showTab('text');
  }

  function setComfort(open) {
    pop.hidden = !open;
    toggle.classList.toggle('on', open);
    if (open) onComfortOpen().then(paintSpellingTab);
  }

  toggle.disabled = false;
  contents.disabled = false;
  toggle.addEventListener('click', () => setComfort(pop.hidden));
  element('font-smaller').addEventListener('click', () => bumpFont(-1));
  element('font-larger').addEventListener('click', () => bumpFont(1));
  for (const control of LAYOUT) {
    const input = element(`layout-${control.key}`);
    input.addEventListener('input', () => show({ [control.key]: Number(input.value) }));
    input.addEventListener('change', save);
  }
  element('layout-font').addEventListener('change', (event) => change({ font: event.target.value }));
  element('layout-reset').addEventListener('click', () => change(LAYOUT_START));
  const setContents = (shown) => {
    if (shown === prefs.contents) return;
    change({ contents: shown });
    onContents(shown);
  };
  const setSpelling = (on) => {
    if (on === prefs.spelling) return;
    change({ spelling: on });
    onSpelling(on);
  };
  element('search-case').addEventListener('click', () => {
    change({ matchCase: !prefs.matchCase });
    onMatchCase();
  });
  for (const [name, values] of Object.entries(PRESETS)) element(`preset-${name}`).addEventListener('click', () => change(values));
  for (const tab of TABS) element(`comfort-tab-${tab}`).addEventListener('click', () => showTab(tab));

  return {
    async load() {
      prefs = await prefsGet();
      paint();
    },
    current: () => prefs,
    change,
    subscribe: (listener) => listeners.add(listener),
    switchTheme: () => change({ theme: prefs.theme === 'dark' ? 'light' : 'dark' }),
    shelfPrefs: () => ({ shelfView: prefs.shelfView, shelfSort: prefs.shelfSort, shelfRecent: prefs.shelfRecent }),
    setShelfPrefs: (patch) => {
      prefs = { ...prefs, ...patch };
      save();
    },
    themeLabel: () => (prefs.theme === 'dark' ? 'Light theme' : 'Dark theme'),
    spellingOn: () => prefs.spelling,
    setSpelling,
    toggleSpelling: () => setSpelling(!prefs.spelling),
    matchCase: () => prefs.matchCase,
    setMatchCase: (on) => {
      if (on !== prefs.matchCase) change({ matchCase: on });
    },
    bumpFont,
    fontPx: () => prefs.fontPx,
    resetFont: () => change({ fontPx: PRESETS.comfortable.fontPx }),
    resetLayout: () => change(LAYOUT_START),
    setPages: (mode) => change({ pages: mode }),
    contentsShown: () => prefs.contents,
    setContents,
    isComfortOpen: () => !pop.hidden,
    closeComfort: () => setComfort(false),
    toggleComfort: () => setComfort(pop.hidden),
  };
}
