import { setFullscreen } from './ipc.js';

const EDGE_PX = 6;
const element = (id) => document.getElementById(id);

export const isAtEdge = (x, y, width, height) => x <= EDGE_PX || y <= EDGE_PX || y >= height - 1 - EDGE_PX;

export function createFocus({ keepPlace, beforeEnter, showToast }) {
  const app = document.querySelector('.app');
  const exitButton = element('exit-focus');
  const enterButton = element('toggle-focus');
  const fullButton = element('toggle-fullscreen');
  const searchPane = element('search-pane');
  let on = false;
  let armed = false;
  let full = false;

  function paint(editing) {
    element('exit-focus-key').textContent = editing ? 'Shift F11' : 'Esc';
  }

  function set(next) {
    if (next === on) return;
    if (next) beforeEnter();
    on = next;
    armed = false;
    keepPlace(() => {
      app.classList.toggle('focus', on);
      exitButton.hidden = !on;
    });
  }

  function setFull(next) {
    if (next === full) return;
    full = next;
    keepPlace(() => fullButton.classList.toggle('on', full));
    setFullscreen(full).catch((failure) => {
      full = !next;
      fullButton.classList.toggle('on', full);
      showToast(`Full screen is not available. ${failure}`);
    });
  }

  document.addEventListener('mousemove', (event) => {
    if (!on || (!searchPane.hidden && event.clientX >= searchPane.getBoundingClientRect().left)) return;
    const atEdge = isAtEdge(event.clientX, event.clientY, window.innerWidth, window.innerHeight);
    if (atEdge && armed) set(false);
    else if (!atEdge) armed = true;
  }, { passive: true });
  exitButton.addEventListener('click', () => set(false));
  enterButton.disabled = false;
  enterButton.addEventListener('click', () => set(true));
  fullButton.addEventListener('click', () => setFull(!full));

  return {
    isOn: () => on,
    isFullscreen: () => full,
    enter: () => set(true),
    exit: () => set(false),
    toggle: () => set(!on),
    toggleFullscreen: () => setFull(!full),
    leaveAll: () => {
      set(false);
      setFull(false);
    },
    showMode: paint,
  };
}
