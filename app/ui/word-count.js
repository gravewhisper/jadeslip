const QUIET_MS = 400;

const isSpace = (code) => code <= 32 || code === 160 || (code >= 0x2000 && code <= 0x200b) || code === 0x3000;

export function countWords(text) {
  let words = 0;
  let inWord = false;
  for (let at = 0; at < text.length; at += 1) {
    const space = isSpace(text.charCodeAt(at));
    if (!space && !inWord) words += 1;
    inWord = !space;
  }
  return words;
}

export function createWordCount(target) {
  let timer = null;

  function show(root) {
    clearTimeout(timer);
    timer = null;
    const words = countWords(root.textContent);
    target.textContent = `${words.toLocaleString('en-US')} ${words === 1 ? 'word' : 'words'}`;
  }

  return {
    show,
    later(root) {
      clearTimeout(timer);
      timer = setTimeout(() => show(root), QUIET_MS);
    },
    clear() {
      clearTimeout(timer);
      timer = null;
      target.textContent = '';
    },
  };
}
