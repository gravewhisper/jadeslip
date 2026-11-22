const BASE_CSS = `
:host { display: block; }
:where(.ss-book) { overflow-wrap: break-word; }
:where(.ss-book p) { margin: 0 0 var(--para-gap, 0.6em); text-indent: var(--para-indent); }
:where(.ss-book h1, .ss-book h2, .ss-book h3) { line-height: 1.2; letter-spacing: -.01em; }
.ss-book img, .ss-book svg, .ss-book video { max-width: 100%; height: auto; }
:where(.ss-book a) { color: var(--accent); }
.ss-body[contenteditable] { outline: none; }
@layer guards {
  .ss-book, .ss-body {
    display: block !important; visibility: visible !important; opacity: 1 !important;
    position: static !important; transform: none !important;
    height: auto !important; max-height: none !important; overflow: visible !important;
  }
  :host(.theme-ink) .ss-book, :host(.theme-ink) .ss-book * {
    color: var(--ink) !important; background-color: transparent !important;
  }
  :host(.theme-ink) .ss-book a { color: var(--accent) !important; }
}
:host(.paged) .ss-book img, :host(.paged) .ss-book svg, :host(.paged) .ss-book video { max-height: var(--page-h); object-fit: contain; break-inside: avoid; }
::highlight(ss-search) { background-color: var(--mark); }
::highlight(ss-search-current) { background-color: var(--accent); color: var(--on); }
::highlight(ss-spell) { text-decoration: underline wavy #D9483B; }
`;

const baseSheet = new CSSStyleSheet();
baseSheet.replaceSync(BASE_CSS);

export class MountError extends Error {}

function parseChapter(xhtml) {
  const parsed = new DOMParser().parseFromString(xhtml, 'application/xhtml+xml');
  if (parsed.getElementsByTagName('parsererror').length > 0) {
    throw new MountError('This chapter could not be displayed because its markup did not parse in the reader.');
  }
  return parsed.documentElement;
}

export function mountChapter(host, chapter) {
  const root = document.adoptNode(parseChapter(chapter.xhtml));
  const shadow = host.shadowRoot ?? host.attachShadow({ mode: 'open' });
  shadow.adoptedStyleSheets = [baseSheet];
  const sheets = chapter.css.map((text) => {
    const sheet = document.createElement('style');
    sheet.textContent = text;
    return sheet;
  });
  const book = document.createElement('div');
  book.className = 'ss-book';
  book.append(root);
  shadow.replaceChildren(...sheets, book);
  return shadow;
}

export function clearChapter(host) {
  host.shadowRoot?.replaceChildren();
}
