# web/ — the mmux.org static site

A single static page: plain HTML + CSS + vanilla JS, no framework, no dev build step
(open `index.html` directly). It renders fully offline and makes **zero external calls**.

The production Docker image fingerprints the CSS/JS at build time (renames each to
`<name>.<contenthash>.<ext>` and rewrites the refs in `index.html`) so a long
`immutable` edge cache is safe and deploys never need a Cloudflare purge.

## View it

Open the file directly:

```sh
open index.html        # macOS  (xdg-open on Linux)
```

Or serve it with nginx via Docker (matches production):

```sh
docker build -t mmux-web .
docker run --rm -p 8080:80 mmux-web
# → http://localhost:8080
```

## Files

- `index.html`, `styles.css`, `scenes.js`, `tui.js` — the site (see `DESIGN.md` §8).
- `robots.txt`, `sitemap.xml`, `llms.txt` — crawler/agent surface: a permissive robots policy
  (search + named AI bots), the canonical sitemap, and an [llms.txt](https://llmstxt.org)-format
  guide LLMs can read. The `<head>` adds canonical, Open Graph/Twitter, and a schema.org
  `SoftwareApplication` JSON-LD block.
- `assets/og-image.png` — the 1200×630 social card (`og-image.svg` is its editable source).
  Regenerate the raster on macOS after editing the SVG:

  ```sh
  cd assets && sips -s format png og-image.svg --out og-image.png
  ```

  ImageIO honours the first installed font family (Menlo), so the PNG renders monospace.
- `fonts/` — empty by default; see `fonts/README.md` to self-host a monospace face.
- `Dockerfile`, `nginx.conf`, `.dockerignore` — production serving (gzip, cache headers,
  strict CSP with no external origins and no `unsafe-inline`).

**`DESIGN.md` is the contract.** Any change here must satisfy it (zero network requests,
color-as-signal, accessibility, reduced-motion). Read it before editing.
