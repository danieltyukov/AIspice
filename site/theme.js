/*
 * The theme toggle. The blocking script in the page head has already applied a
 * stored choice before first paint; this handles the click and keeps the
 * button's pressed state true to what is on screen.
 *
 * The visible word is the button's accessible name and the state is
 * aria-pressed, rather than an aria-label that describes an action. "Follow
 * the system" comes back by clearing the stored value, which is what a
 * browser's own site-data control does.
 */
const root = document.documentElement;
const button = document.getElementById('theme');
const media = window.matchMedia('(prefers-color-scheme: dark)');

function isDark() {
  const choice = root.dataset.theme;
  return choice === 'dark' || (choice === undefined && media.matches);
}

function sync() {
  button.setAttribute('aria-pressed', String(isDark()));
}

if (button) {
  button.addEventListener('click', () => {
    const next = isDark() ? 'light' : 'dark';
    root.dataset.theme = next;
    try {
      localStorage.setItem('aispice-theme', next);
    } catch {
      // Storage refused. The choice still applies to this page view.
    }
    sync();
  });

  // Only matters while the visitor follows the system: an explicit choice
  // outranks the system changing underneath it.
  media.addEventListener('change', sync);
  sync();
}
