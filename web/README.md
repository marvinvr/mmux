# web/ — the mmux.org static site

A single static page: plain HTML + CSS + vanilla JS, no framework, no dev build step
(open `index.html` directly). It renders fully offline; the **one** external call is the
self-hosted umami analytics script (allow-listed in the CSP — see `DESIGN.md` §0), and the
page must work perfectly with it blocked.

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

- `index.html`, `styles.css`, `scenes.js`, `tui.js` — the site (see `DESIGN.md`).
- `fonts/` — the self-hosted Departure Mono display face + its OFL license (see `fonts/README.md`).
- `banner.txt` — the plain-text card nginx serves when `curl`/`wget`/`httpie` hits `/`.
- `robots.txt`, `sitemap.xml`, `llms.txt` — crawler/agent surface: a permissive robots policy
  (search + named AI bots), the canonical sitemap, and an [llms.txt](https://llmstxt.org)-format
  guide LLMs can read. The `<head>` adds canonical, Open Graph/Twitter, and a schema.org
  `SoftwareApplication` JSON-LD block.
- `assets/og-image.png` — the 1200×630 social card. Its editable source is
  `../assets/og-image.src.html` (with the repo's other brand sources, outside the
  deployed tree) — a standalone HTML canvas in the site's identity; the regeneration
  commands are in its header comment.
- `Dockerfile`, `nginx.conf`, `.dockerignore` — production serving (gzip, cache headers,
  strict CSP with no external origins and no `unsafe-inline`).

**`DESIGN.md` is the contract.** Any change here must satisfy it (one allow-listed external
origin and nothing else, realistic terminal content, accessibility, reduced-motion). Read it
before editing.
