# fonts/

The site ships **no font binaries**. It uses the system monospace stack so it makes
**zero external calls** and renders fully offline / over `file://` (DESIGN.md §0).

If you want a consistent monospace face across machines (notably Linux, which has no
guaranteed good system mono), self-host one here:

1. Drop a `woff2` into this folder, e.g. `fonts/jetbrains-mono.woff2`
   (JetBrains Mono or Geist Mono both fit the aesthetic).
2. Uncomment the `@font-face` block in `styles.css` and point it at the file.

The font must be local — never an `@import` or a remote `url()` (that would break §0 and
the strict CSP). Ready-to-paste example:

```css
@font-face {
  font-family: "MMUX Mono";
  src: url("fonts/jetbrains-mono.woff2") format("woff2");
  font-weight: 100 800;       /* drop if not a variable font */
  font-style: normal;
  font-display: swap;
}
```

Then prepend `"MMUX Mono"` to the `--font-mono` stack in `styles.css`. This stays within
the CSP (`style-src 'self'` covers the stylesheet; the `woff2` is same-origin).
