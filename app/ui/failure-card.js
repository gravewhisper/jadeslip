const element = (id) => document.getElementById(id);

const TITLES = { missing: 'Book not found', unreadable: 'This book cannot be opened' };
const BLOCKED = ['#reader .top .tools', '#reader .top .seg', '#contents-rail', '#reader .footrow'];

export function createFailureCard({ onBack, onFind, onRemove }) {
  const card = element('open-failure');
  let shown = null;

  function block(on) {
    for (const selector of BLOCKED) document.querySelector(selector).inert = on;
  }

  function show(failure) {
    shown = failure;
    element('open-failure-title').textContent = TITLES[failure.kind] ?? TITLES.unreadable;
    element('open-failure-text').textContent = failure.message;
    element('failure-find').hidden = failure.kind !== 'missing';
    element('failure-remove').hidden = !failure.key;
    card.hidden = false;
    block(true);
    element(failure.kind === 'missing' ? 'failure-find' : 'failure-back').focus();
  }

  function hide() {
    if (!shown) return;
    shown = null;
    card.hidden = true;
    block(false);
  }

  element('failure-back').addEventListener('click', () => onBack());
  element('failure-find').addEventListener('click', () => onFind(shown));
  element('failure-remove').addEventListener('click', () => onRemove(shown));

  return {
    show,
    hide,
    isShown: () => shown !== null,
    retell: (message) => show({ ...shown, message }),
  };
}
