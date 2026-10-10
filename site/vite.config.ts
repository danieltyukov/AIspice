import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';

/*
 * The project site: two static pages and no framework. It is a workspace
 * package so one `npm ci` at the root covers it, and it cannot pull in React
 * or any application code by accident.
 *
 * What it shares with the app is `app/src/tokens.css`, imported by style.css.
 * Palette, type scale, radii, motion and the @font-face rules come from the
 * file the app reads, so the two cannot drift. Vite rebases the font URLs
 * against the file that declared them, so the site self-hosts the same subsets
 * without a second copy in the repository.
 */
const repoRoot = fileURLToPath(new URL('..', import.meta.url));

/*
 * The social card has to sit at a fixed URL, because Open Graph and Twitter
 * card tags need an absolute address that does not change with a content hash.
 * It is drawn once, in assets/brand, and copied into the build here rather than
 * kept as a second copy in public/.
 */
function socialCard(): Plugin {
  return {
    name: 'aispice-social-card',
    apply: 'build',
    generateBundle() {
      this.emitFile({
        type: 'asset',
        fileName: 'og.png',
        source: readFileSync(new URL('../assets/brand/og.png', import.meta.url)),
      });
    },
  };
}

export default defineConfig({
  /*
   * Absolute, unlike owl-transfer's './'. GitHub Pages serves 404.html for any
   * missing path at any depth (/aispice/a/b/c), and a relative asset URL would
   * resolve against that depth and miss. With the base fixed, every asset URL
   * is /aispice/assets/..., which is right from anywhere. `vite preview` serves
   * under the same base, so local checks see the real paths.
   */
  base: '/aispice/',
  plugins: [socialCard()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2020',
    assetsInlineLimit: 0,
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL('./index.html', import.meta.url)),
        404: fileURLToPath(new URL('./404.html', import.meta.url)),
      },
    },
  },
  server: {
    port: 5175,
    strictPort: true,
    // tokens.css, the fonts and the favicon live above this root.
    fs: { allow: [repoRoot] },
  },
  preview: {
    port: 4175,
    strictPort: true,
  },
});
