export const LIMITS = { saved: 200, recent: 50, nameChars: 80, textBytes: 16 << 10 };
const NAME_FROM_CHARS = 40;

export const emptyMemory = () => ({ saved: [], recent: [] });

export const queryOf = ({ text, regex, matchCase }) => ({ text, regex, matchCase });

export const sameQuery = (a, b) => a.text === b.text && a.regex === b.regex && a.matchCase === b.matchCase;

const nameKey = (name) => name.toLowerCase();

const textBytes = (text) => new TextEncoder().encode(text).length;

export function nameFrom(text) {
  const flat = text.replace(/\s+/g, ' ').trim();
  const chars = [...flat];
  return chars.length > NAME_FROM_CHARS ? `${chars.slice(0, NAME_FROM_CHARS - 1).join('').trimEnd()}…` : flat;
}

function queryProblem(query) {
  if (query.text.length === 0) return 'Type a search first, then save it.';
  if (textBytes(query.text) > LIMITS.textBytes) return `That search is longer than ${LIMITS.textBytes >> 10} KB, so it cannot be kept.`;
  return null;
}

function nameProblem(memory, name, keep) {
  if (name.length === 0) return 'Give the saved search a name.';
  if ([...name].length > LIMITS.nameChars) return `Keep the name to ${LIMITS.nameChars} characters.`;
  if (/\p{Cc}/u.test(name)) return 'A saved search name cannot hold line breaks.';
  const taken = memory.saved.find((entry) => entry !== keep && nameKey(entry.name) === nameKey(name));
  return taken ? `A saved search is already called "${taken.name}".` : null;
}

const cleanName = (name) => name.replace(/\s+/g, ' ').trim();

export const savedOf = (memory, query) => memory.saved.find((entry) => sameQuery(entry, query)) ?? null;

export function save(memory, rawName, query) {
  const problem = queryProblem(query);
  if (problem) return { error: problem };
  const existing = savedOf(memory, query);
  const name = cleanName(rawName) || nameFrom(query.text);
  const badName = nameProblem(memory, name, existing);
  if (badName) return { error: badName };
  if (existing) {
    return { memory: { ...memory, saved: memory.saved.map((entry) => (entry === existing ? { ...entry, name } : entry)) }, name };
  }
  if (memory.saved.length >= LIMITS.saved) return { error: `You have ${LIMITS.saved} saved searches. Delete one to save another.` };
  return { memory: { ...memory, saved: [{ name, ...queryOf(query) }, ...memory.saved] }, name };
}

export function rename(memory, from, rawName) {
  const entry = memory.saved.find((saved) => saved.name === from);
  if (!entry) return { error: 'That saved search is gone.' };
  const name = cleanName(rawName);
  const badName = nameProblem(memory, name, entry);
  if (badName) return { error: badName };
  return { memory: { ...memory, saved: memory.saved.map((saved) => (saved === entry ? { ...saved, name } : saved)) }, name };
}

export function remove(memory, name) {
  const at = memory.saved.findIndex((entry) => entry.name === name);
  if (at < 0) return { memory, removed: null };
  return { memory: { ...memory, saved: memory.saved.filter((_, index) => index !== at) }, removed: { entry: memory.saved[at], at } };
}

export function restore(memory, removed) {
  if (!removed) return memory;
  const { entry, at } = removed;
  if (savedOf(memory, entry) || nameProblem(memory, entry.name, null) || memory.saved.length >= LIMITS.saved) return memory;
  const saved = [...memory.saved];
  saved.splice(Math.min(at, saved.length), 0, entry);
  return { ...memory, saved };
}

export function record(memory, query) {
  if (queryProblem(query)) return memory;
  const recent = [queryOf(query), ...memory.recent.filter((entry) => !sameQuery(entry, query))].slice(0, LIMITS.recent);
  return { ...memory, recent };
}

export const forget = (memory, query) => ({ ...memory, recent: memory.recent.filter((entry) => !sameQuery(entry, query)) });

export const clearRecent = (memory) => ({ ...memory, recent: [] });

export function rowsOf(memory, filter = '') {
  const wanted = filter.toLowerCase();
  const hit = (...texts) => !wanted || texts.some((text) => text.toLowerCase().includes(wanted));
  const saved = memory.saved.filter((entry) => hit(entry.name, entry.text)).map((entry) => ({ kind: 'saved', name: entry.name, query: queryOf(entry) }));
  const recent = memory.recent
    .filter((entry) => !savedOf(memory, entry) && hit(entry.text))
    .map((entry) => ({ kind: 'recent', name: null, query: queryOf(entry) }));
  return [...saved, ...recent];
}

export function moveHighlight(rows, at, by) {
  if (rows.length === 0) return null;
  if (at === null) {
    if (by > 0) return 0;
    const newest = rows.findIndex((row) => row.kind === 'recent');
    return newest >= 0 ? newest : rows.length - 1;
  }
  const next = at + by;
  if (next < 0) return null;
  return Math.min(next, rows.length - 1);
}

export function tagOf(query) {
  const parts = [query.regex ? 'Regex' : 'Plain'];
  if (query.matchCase) parts.push('Match case');
  return parts.join(' · ');
}
