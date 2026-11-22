export function createBack({ go }) {
  const chip = document.getElementById('back');
  const label = document.getElementById('back-label');
  let origin = null;
  let landed = null;
  let stepping = false;

  function show() {
    chip.hidden = origin === null;
    if (origin) label.textContent = `Back to ${origin.name}`;
  }

  function forget() {
    origin = null;
    landed = null;
    stepping = false;
    show();
  }

  function leap(from, to, step = false) {
    if (!(step && stepping && from.ix === landed)) origin = from;
    landed = to;
    stepping = step;
    show();
  }

  function shown(ix) {
    if (origin !== null && ix !== landed) forget();
  }

  function back() {
    if (origin === null) return;
    const to = origin;
    forget();
    go(to);
  }

  chip.addEventListener('click', back);
  return { leap, shown, back, forget };
}
