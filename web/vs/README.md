# web/vs/ — "mmux vs X" comparison pages

One static page per comparison at `https://mmux.org/vs/<slug>/`, plus the index at `/vs/`.
Same rules as the rest of the site (`../DESIGN.md`): plain HTML, no JS of our own, strict CSP.

```
vs/
  index.html          /vs/ — the list of all comparisons (review pass edits it)
  vs.css              shared article styles, loaded after ../styles.css
  _template/          the page to copy (never deployed: .dockerignore)
  <slug>/index.html   one comparison each
  README.md           this file (never deployed)
```

## Writing one comparison (one agent, one folder)

1. **Copy the template** to a new folder and touch nothing else:
   ```sh
   cp -r web/vs/_template web/vs/<slug>
   ```
   The slug is the other tool's name in lowercase, `a-z0-9` and dashes: `tmux`, `zellij`,
   `claude-squad`. No `mmux-vs-` prefix, the URL already says `/vs/`.
2. **Fill every `{{PLACEHOLDER}}`** and remove the `EDIT:` comments. The header comment
   in the template lists them. `grep -n '{{' web/vs/<slug>/index.html` must return nothing.
3. **Keep the structure:** title + lede, TL;DR, table, What is X, Where X is better, Where mmux is
   better, When to use which, Migration or using both, FAQ, Sources, Other comparisons, Try mmux.
   Drop a section only if it truly has nothing to say (a migration section with no real steps).
4. **Keep the FAQPage JSON-LD in sync** with the visible FAQ: same questions, same answers, as
   plain text.
5. **Set `dateModified`** in the WebPage JSON-LD to the day you checked the facts
   (`YYYY-MM-DD`). The sitemap reads it.
6. Do **not** edit `vs/index.html`, `vs/vs.css`, other pages, `sitemap.xml` or the home page. The
   sitemap is generated at build time, and the review pass adds the page to the `/vs/` list and
   fills every page's "Other comparisons" block. If the page needs a style `vs.css` doesn't have,
   say so in your hand-off instead of adding a `<style>` (the CSP blocks inline styles anyway).

## Content rules

- **English.** Plain, specific sentences. Sentence case for headings.
- **Honest.** Every page has a real "Where X is better" section. If X is the better tool for a
  use case, say so plainly in the TL;DR and the table. A comparison where mmux wins every row is
  not credible and will be sent back.
- **Fact-based, with sources.** Every claim about X is backed by a primary source (its docs,
  README, source code, changelog, release notes) listed under Sources. Note the version you
  checked. If you could not verify something, leave it out.
- **No invented features, on either side.** mmux facts come from this repo: `docs/` is the
  source of truth (`docs/03-usage.md`, `docs/04-configuration.md`, `docs/05-notifications.md`,
  `docs/06-architecture.md`), plus `README.md`. Do not describe roadmap items or guess. Only real
  commands and config keys in code blocks.
- **No em dashes** (`—`) anywhere on the page, including meta tags and JSON-LD. Use a colon, a
  comma, parentheses or two sentences. En dashes too, except in number ranges.
- **No marketing slop.** No "seamless", "supercharge", "game-changer", "blazing fast",
  "effortless", "unlock", "elevate", "robust", "powerful", "in today's fast-paced world". No
  exclamation marks. Concrete beats adjectives: "sessions survive `ssh` disconnects" instead of
  "rock-solid persistence".
- **No disparaging X.** Describe trade-offs, not flaws. Name the other project the way it
  names itself.
- **Title** ~50-60 characters, **meta description** 140-160, both specific to this pair.

## Table marks

Each cell is one mark plus a short phrase. The glyph is decorative; the `sr-only` word is what
screen readers hear:

| Mark | Markup | Meaning |
|---|---|---|
| ✓ | `<span class="mark mark-yes" aria-hidden="true">✓</span><span class="sr-only">yes: </span>` | supported |
| ~ | `<span class="mark mark-part" aria-hidden="true">~</span><span class="sr-only">partly: </span>` | partly, with a caveat |
| ✗ | `<span class="mark mark-no" aria-hidden="true">✗</span><span class="sr-only">no: </span>` | not supported |
| · | `<span class="mark mark-na" aria-hidden="true">·</span>` | neutral fact (license, language) |

## Preview

Open `web/vs/<slug>/index.html` directly in a browser (relative paths work over `file://`), or
build the production image from `web/` and open `http://localhost:8080/vs/<slug>/`:

```sh
docker build -t mmux-web . && docker run --rm -p 8080:80 mmux-web
```

## Review pass (after the pages are written)

- Add one `<li>` per page to the list in `vs/index.html` (markup in its header comment),
  alphabetical by X. The "No comparisons published yet" line hides itself.
- Fill `<!-- vs:others -->` in every page with the same `<li>` entries, minus the page itself.
- Check each page against the rules above: no `{{`, no `—`, sources present, FAQ JSON-LD
  matches the visible FAQ.
- Optionally mention `/vs/` in `../llms.txt`.

## How it ships

The Docker build (`../Dockerfile`) copies `vs/` without `_template/` and this README,
fingerprints `vs.css` like the other assets, and generates `sitemap.xml` with
`../scripts/gen-sitemap.sh` (home, `/vs/` and every `vs/<slug>/`). nginx serves `/vs/<slug>/`
from the folder's `index.html`, redirects `/vs/<slug>` to the trailing slash, and returns a
real 404 for unknown slugs. Nothing else to wire up: a new folder is live with the next deploy
of the image.
