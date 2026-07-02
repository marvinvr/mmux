# fonts/

One self-hosted face ships here: **Departure Mono** (`DepartureMono-Regular.woff2`,
~22 KB), the pixel monospace used for the wordmark, headings, caption titles and the
demo's overlay line — the display voice that matches the squared-off brand tile. It is
licensed under the SIL Open Font License 1.1 (see `LICENSE`; © Helena Zhang,
https://departuremono.com). Single weight: pixel fonts have no true bold, so nothing
set in it may ask for one (faux-bold smears the pixels — `font-weight: 400` only).

Everything else — body copy and *all* terminal content — stays on the **system
monospace stack** (`--font` in `styles.css`): real terminals render in the system
mono, so the simulated one must too.

Fonts must be local — never an `@import` or a remote `url()` (that would break
DESIGN.md's no-external-origins rule and the strict CSP). The `@font-face` lives at
the top of `styles.css`; `index.html` preloads the woff2 to avoid a flash. To swap or
add a face:

1. Drop the `woff2` in this folder.
2. Add / edit the `@font-face` block in `styles.css` and the `--font-display`
   (or `--font`) stack.
3. Keep the preload `<link>` in `index.html` in sync.

This stays within the CSP (`style-src 'self'` covers the stylesheet; the `woff2` is
same-origin, covered by `default-src 'self'`).
