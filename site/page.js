/*
 * Small enhancements. The page is complete without any of them.
 *
 * - Copy buttons copy the text of the <code> beside them.
 * - The download for the visitor's platform becomes the hero button's target
 *   and is marked in the download list. An unknown platform, a phone
 *   included, leaves the hero button pointing at the download list.
 * - The header gains a hairline once the page scrolls.
 * - The product window flattens from a slight tilt as it scrolls into view,
 *   and sections fade in once. Both are off with reduced motion.
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
    ? { file: 'x64-setup.exe', name: 'Windows' }
    : platform.startsWith('mac')
      ? { file: 'universal.dmg', name: 'macOS' }
      : platform.includes('linux')
        ? { file: 'x86_64.AppImage', name: 'Linux' }
        : null;

if (yours) {
  const link = document.querySelector(`.downloads a[href$="${yours.file}"]`);
  link?.classList.add('is-yours');
  const hero = document.querySelector('.js-download');
  if (link && hero) {
    hero.href = link.href;
    hero.textContent = `Download for ${yours.name}`;
  }
}

const head = document.querySelector('.head');
const onScroll = () => head?.classList.toggle('is-scrolled', window.scrollY > 8);
window.addEventListener('scroll', onScroll, { passive: true });
onScroll();

const calm = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

if (!calm) {
  const win = document.querySelector('.window');
  if (win) {
    let frame = 0;
    const tilt = () => {
      frame = 0;
      const rect = win.getBoundingClientRect();
      const vh = window.innerHeight;
      // 10 degrees when the window's top sits at the bottom of the screen,
      // flat by the time it reaches 40% of the way up.
      const t = Math.min(1, Math.max(0, (vh - rect.top) / (vh * 0.6)));
      win.style.setProperty('--tilt', `${(10 * (1 - t)).toFixed(2)}deg`);
    };
    window.addEventListener(
      'scroll',
      () => {
        if (!frame) frame = requestAnimationFrame(tilt);
      },
      { passive: true },
    );
    tilt();
  }

  if ('IntersectionObserver' in window) {
    const targets = document.querySelectorAll(
      '.feature-copy, .feature-art, .band-copy, .numbers-grid > div, .use-card, .download-inner',
    );
    const seen = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            e.target.classList.add('in');
            seen.unobserve(e.target);
          }
        }
      },
      { rootMargin: '0px 0px -10% 0px' },
    );
    for (const el of targets) {
      // Only what is still below the fold fades in; nothing visible on load
      // flickers.
      if (el.getBoundingClientRect().top > window.innerHeight) {
        el.classList.add('reveal');
        seen.observe(el);
      }
    }
  }
}
