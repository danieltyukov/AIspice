/*
 * Two small enhancements. The page is complete without either.
 *
 * Copy buttons: each copies the text of the <code> beside it. Without
 * clipboard access (an insecure context, or a browser that refuses) the button
 * says so and the text is still there to select.
 *
 * Your platform: the download for the visitor's operating system is shown as
 * the primary button. Nothing is hidden, and an unknown platform, a phone
 * included, leaves all four buttons equal.
 */
for (const button of document.querySelectorAll('.copy')) {
  const label = button.textContent;
  button.addEventListener('click', async () => {
    const text = button.parentElement.querySelector('code').textContent;
    try {
      await navigator.clipboard.writeText(text);
      button.textContent = 'Copied';
    } catch {
      button.textContent = 'Select and copy';
    }
    setTimeout(() => {
      button.textContent = label;
    }, 1500);
  });
}

const platform = (navigator.userAgentData?.platform || navigator.platform || '').toLowerCase();
const mobile = /android|iphone|ipad/i.test(navigator.userAgent);
const yours = mobile
  ? null
  : platform.startsWith('win')
    ? 'x64-setup.exe'
    : platform.startsWith('mac')
      ? 'universal.dmg'
      : platform.includes('linux')
        ? 'x86_64.AppImage'
        : null;

if (yours) {
  document.querySelector(`.downloads a[href$="${yours}"]`)?.classList.add('is-yours');
}
