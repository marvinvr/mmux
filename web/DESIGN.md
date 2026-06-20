# mmux.org — design & build contract

This is the **single source of truth** for the static marketing site in `web/`. Every file is
built against the interfaces pinned here. If two files must agree on a name (a CSS class, a DOM
id, a state field, a scene key), it is defined here and nowhere else.

The site is a love letter to the mmux TUI: a dark, monospace, box-drawn page whose centerpiece is
a fake mmux terminal you scroll *through* (a scripted 9-state walkthrough) and then *play*
(a keyboard-driven sandbox). It must look like the real app leaked onto a webpage.

---

## 0. Non-negotiables

- **Static only.** Plain HTML + CSS + vanilla JS. No framework, no bundler, no build step.
- **Zero external calls.** No CDNs, no Google Fonts, no analytics, no remote images/scripts. The
  page must render fully offline and over `file://`. (Hence: plain `<script defer>` + globals, not
  ES modules — ES modules break on `file://`.)
- **Flat modern-dev aesthetic.** No CRT scanlines, no glow, no grain. Texture comes from
  box-drawing chrome, one hairline rule, the blinking block cursor, and tight monospace rhythm.
- **`prefers-reduced-motion`** fully honored: skip all scroll choreography and typing animations;
  land directly on the finished, playable sandbox state. Nothing essential is motion-only.
- **Accessible.** Semantic landmarks, logical heading order, keyboard operable, visible focus,
  AA contrast. The sandbox is focus-trapped only while engaged and always escapable.

---

## 1. The one rule: color = signal, never decoration

Everything is grayscale by default. Color is *earned* and only ever means one of two things:

| token       | hue            | meaning                                            | where it may appear |
|-------------|----------------|----------------------------------------------------|---------------------|
| `--accent`  | cyan `#5ec8d6` | **interaction** — you're touching it               | `:hover`, `:focus-visible`, active nav link, the selected sidebar row's `▌` bar + the focused pane border in the fake TUI |
| `--alert`   | red `#e5534b`  | **attention / error** — it needs you / it broke    | the bell `●` dot, the notification toast accent, any error state |

No other hue appears anywhere. Reviewers reject any decorative color. The app's green "running"
and "+ New" are rendered **grayscale** on the site (filled vs hollow glyph carries state, not hue)
so that the red attention dot in scene 6 is the only warm color the visitor has seen.

---

## 2. Design tokens (CSS custom properties on `:root`)

```
/* surface */
--bg:          #0b0c0e;   /* page */
--panel:       #111317;   /* cards, the fake TUI body */
--panel-2:     #15181d;   /* raised: selected row, code blocks, active project header */
--line:        #23262d;   /* hairline borders / box-drawing chrome */
--line-bright: #2f333b;   /* emphasized border (focused pane uses --accent instead) */

/* ink */
--fg:        #c9d1d9;     /* body text */
--fg-bright: #e6edf3;     /* headings, emphasis, the wordmark */
--muted:     #8b939e;     /* secondary text, key hints, captions' meta, placeholders — AA-tuned (raised from #6e7681 to clear 4.5:1 on bg/panel/panel-2) */
--faint:     #4b525c;     /* dimmest, DECORATIVE ONLY: inactive glyphs, hollow ○, the $/##/· marks — never meaningful text */

/* signal (see §1) */
--accent:     #5ec8d6;
--alert:      #e5534b;
--alert-dim:  #3a2422;    /* notification toast bg tint */

/* type */
--font: ui-monospace, "SF Mono", "Cascadia Code", "JetBrains Mono", "Fira Code",
        Menlo, Consolas, "Liberation Mono", monospace;
/* the wordmark uses --font too (no separate display face — cohesion over flash) */

/* scale (monospace ch-based where it helps box-drawing line up) */
--measure: 68ch;          /* max prose width */
--page-max: 1080px;       /* max content width */
--gap: 1.5rem;
--radius: 0;              /* terminals don't have rounded corners. keep it 0 (or 2px max) */

/* type sizes */
--fs-wordmark: clamp(2.5rem, 8vw, 5rem);
--fs-h2:       clamp(1.4rem, 3.5vw, 2rem);
--fs-body:     0.95rem;
--fs-small:    0.8rem;
--leading:     1.6;
```

Breakpoints (mirror the app's own constants in spirit):
- `--bp-compact: 680px` — below this the fake TUI collapses to a single column (sidebar stacks
  above main, right panel hidden), echoing the app's `COMPACT_W`. Page sections go single-column.

---

## 3. Typography & chrome

- Body 0.95rem, line-height 1.6, `--fg`. Headings `--fg-bright`, slightly tightened tracking.
- The wordmark `mmux` is lowercase, `--fs-wordmark`, `--fg-bright`, followed by a blinking block
  cursor `▮` (`<span class="cursor" aria-hidden="true">▮</span>`), 1s steps blink, paused under
  reduced-motion.
- **Box-drawing is real text.** Section headers, the hero frame, feature cards, and the how-it-works
  diagram use literal `┌ ─ ┐ │ └ ┘ ├ ┤` characters (in `aria-hidden` spans) rather than CSS
  borders, to read as terminal chrome. Hairline CSS borders (`--line`) are fine for fine structure;
  box-drawing is for the deliberately "TUI" frames.
- One full-width hairline rule (`--line`) separates major sections. No drop shadows.

---

## 4. Page structure (semantic, in order)

`index.html` body, top to bottom. Exact copy below — terse, lowercase, confident. Use it verbatim.

### 4.1 `<header class="nav">` — sticky, thin
- Left: `mmux` wordmark (small, links to `#top`).
- Right: nav links `demo · features · install · github`. `github` → `https://github.com/marvinvr/mmux`.
- Plus an inline install pill: `cargo install mmux` with a `[copy]` button (`button.copy`,
  `data-copy="cargo install mmux"`).

### 4.2 `<section id="hero">`
Rendered inside a box-drawn frame. Content:
- wordmark `mmux▮`
- tagline (`h1`, visually the lede): **persistent terminals for your AI agents.**
- sub (`p.lede-sub`): **one rust binary. spawn agents, watch processes, never lose a session — even over ssh.**
- a prompt line / install block: `<div class="install">` showing `$ cargo install mmux` + `[copy]`.
- a small scroll affordance: `scroll to see it ↓` (`.scroll-hint`, hidden under reduced-motion).

### 4.3 `<section id="demo">` — THE CENTERPIECE (see §5, §6)
Tall scroll section containing the sticky fake-TUI stage and the scene captions, ending in the
playable sandbox.

### 4.4 `<section id="features">`
Heading: `## what you get`. A responsive grid of **6** cards, each a small box-drawn panel
(`.card`) with a `┌─ title ─┐`-style header. Copy verbatim:

1. **per-directory & persistent** — one mmux per directory, kept alive inside a tmux session. detach, drop ssh, reattach — it's all still there.
2. **agents on demand** — spawn claude, codex, whatever. each runs in its own pane, started and restarted straight from the sidebar.
3. **processes you watch** — start, stop and tail your dev server and tasks without ever leaving the multiplexer.
4. **attention, caught** — a bell or a notification escape becomes a sidebar dot and a real desktop notification. even over ssh.
5. **linked projects** — group sibling clones into one sidebar, each its own section; the panel follows whichever project is active.
6. **one binary, any terminal** — a single rust binary. it runs anywhere a terminal does.

### 4.5 `<section id="how">`
Heading: `## how it works`. A box-drawing diagram (monospace `<pre>`, aria-described) showing the
jail model, plus 3 short lines of prose. Diagram content (use this, tweak spacing to align):

```
  your terminal  (local · ssh · tmux client)
        │
        ▼
  ┌─────────────────────────────────────────────┐
  │  tmux session   (one per directory · hidden) │   ← survives detach / disconnect
  │  ┌───────────────────────────────────────┐   │
  │  │  mmux  (ratatui TUI)                   │   │
  │  │   sidebar  │   main pane   │  panel    │   │
  │  │  agents    │  focused      │  lazygit  │   │
  │  │  terminals │  program's    │  (git)    │   │
  │  │  processes │  live screen  │           │   │
  │  └───────────────────────────────────────┘   │
  └─────────────────────────────────────────────┘
   every pane = a real PTY + a vt100 parser
```

Prose (3 `<li>` or short `<p>`s): 
- the TUI runs *inside* a per-directory tmux session, so closing the terminal or losing ssh never kills it.
- each agent, terminal and process is one pty-backed pane behind a single unified lifecycle.
- the bell and notification escapes are captured and turned into desktop notifications.

### 4.6 `<section id="install">`
Heading: `## install`. Two labeled code blocks (no JS tabs needed — show both, stacked/side-by-side):
- **homebrew**: `brew install marvinvr/mmux/mmux`
- **cargo**:    `cargo install mmux`
- **from source** (smaller): `git clone https://github.com/marvinvr/mmux && cd mmux && cargo install --path .`
Each code block has a `[copy]` button. Then one line: `then just run` → `mmux` (in a dir of your choice).

### 4.7 `<footer>`
`mmux · MIT · github` (github links out). Small, muted. A final blinking cursor is a nice touch.

---

## 5. The fake mmux TUI — DOM contract & authentic content

A single reusable widget. **`index.html` ships the static skeleton**; **`tui.js` mutates it** via
`renderTUI(state)`. The CSS styles exactly the classes below — no others.

### 5.1 DOM skeleton (ids/classes are LAW)

```html
<div id="tui" class="tui" role="img" aria-label="a simulated mmux terminal session">
  <div class="tui-titlebar"><span class="tui-title">mmux</span><span class="tui-path"> — ~/dev/app</span></div>
  <div class="tui-body">
    <nav class="tui-sidebar" aria-hidden="true">
      <!-- project headers (multi-project only) + sections rendered here by JS -->
    </nav>
    <main class="tui-main">
      <div class="tui-main-title"></div>     <!-- e.g. " claude — running " -->
      <div class="tui-main-screen"></div>    <!-- streamed lines / placeholder -->
    </main>
    <aside class="tui-panel" hidden>          <!-- right git panel; toggled visible by JS -->
      <div class="tui-panel-title"> git </div>
      <div class="tui-panel-screen"></div>
    </aside>
  </div>
  <div class="tui-footer"></div>             <!-- key hints, set per focus -->
  <div class="tui-toast" hidden></div>        <!-- notification toast, scene 6 -->
  <div class="tui-overlay" hidden></div>      <!-- "detached / reattached", scene 4 -->
  <div class="tui-sandbox-hint" hidden></div> <!-- "click in to play", finale -->
</div>
```

Sidebar inner structure JS produces per section:
```html
<div class="sb-project sb-project--active"> app </div>        <!-- only if >1 project -->
<div class="sb-section">
  <div class="sb-header">AGENTS</div>
  <div class="sb-row [sb-row--selected]" data-id="...">
    <span class="sb-bar">▌</span>            <!-- ▌ when selected (cyan), else space -->
    <span class="sb-glyph">●</span>          <!-- PROCESSES rows only; ●=running ○=exited ·=stopped, grayscale -->
    <span class="sb-name">claude</span>
    <span class="sb-sub">writing src/auth.rs</span>   <!-- optional OSC-title subtitle, muted -->
    <span class="sb-dot" hidden>●</span>      <!-- attention bell, --alert red, scene 6 -->
  </div>
  <div class="sb-row sb-row--launcher"><span class="sb-bar"> </span><span class="sb-name">+ New Claude</span></div>
</div>
```

### 5.2 Authentic content (from the real source — match exactly)

- **Section headers** (uppercase): `AGENTS`, `TERMINAL`, `PROCESSES`. (Yes, the second is singular
  `TERMINAL` in the app.) Multi-project adds project headers ` app ` / ` app-2 ` above the sections,
  active one raised (`--panel-2` bg, `--fg-bright`, a thin `--accent` left tick).
- **Launchers** (verbatim): `+ New Claude`, `+ New Terminal`, `+ New Process`. Grayscale (`--muted`);
  on hover/selected → `--accent`.
- **Glyphs:** the status glyph (`●` running / `○` exited / `·` stopped) renders **only on
  PROCESSES rows** — agents and terminals are name-only and convey status by text color, exactly
  like the app (`sidebar.rs nav_row`: `badge()` is `Kind::Process`-only). Selected-row bar `▌` in
  `--accent`. Attention dot `●` in `--alert`.
- **Main title bar** format: ` {name} — {status} ` (em-dash, surrounding spaces), e.g. ` claude — running `.
  A live session's title carries **no** project suffix even in multi-project mode; the ` · {project}`
  suffix appears **only on the `+ New …` launcher titles** (`pane.rs main_title`). Empty state: ` mmux `.
- **Placeholders** (verbatim): `Press Enter to launch a new Claude.` / `Press Enter to open a new terminal.` /
  `{name} is stopped.` + blank line + `Press Enter or 's' to start it.` (the app uses a `\n\n` break).
- **Right panel** title is ` git ` (it runs lazygit); subtitle = branch, e.g. `main`.
- **Footer hints** (set by focus; use these strings, may trim to fit width):
  - sidebar focus: `↑↓ move   ⏎ open   s start   x close   r restart   d detach   q quit`
  - main/terminal focus: `keys → pane   drag = copy   Ctrl-b   h back   x close`
  - sandbox finale: `↑↓ move   ⏎ open   x close   —   click out to scroll`
- **No `a`/`t` spawn hotkeys.** Spawning = select a `+ New …` launcher row, press Enter. Honor this
  in scenes and in the sandbox.

### 5.3 `state` shape (the contract between `tui.js`, `scenes.js`, and the renderer)

`renderTUI(state)` is a **pure-ish DOM updater**: given a `state`, it makes `#tui` reflect it
(idempotent; safe to call every frame). State shape:

```js
state = {
  multiProject: false,                 // show project headers when true
  projects: [{ name: "app", active: true }, { name: "app-2", active: false }],
  sidebar: [                           // ordered sections
    { kind: "AGENTS", rows: [
        { id: "claude", glyph: "●", name: "claude", sub: "writing src/auth.rs",
          status: "running", selected: true, attention: false, project: "app" },
        { id: "new-claude", launcher: true, name: "+ New Claude" },
    ]},
    { kind: "TERMINAL", rows: [ ... ] },
    { kind: "PROCESSES", rows: [ ... ] },
  ],
  main: {
    title: " claude — running ",
    lines: ["✓ wrote src/auth.rs", "✓ cargo build", "$ "],   // rendered as screen rows
    placeholder: null,                 // if set, show instead of lines (faint)
    cursor: true,                      // show block cursor on last line
  },
  panel: { visible: false, branch: "main", lines: [ ... ] },
  focus: "sidebar",                    // "sidebar" | "main" | "panel" | "sandbox"
  toast: null,                         // { title, body } -> show .tui-toast (alert accent)
  overlay: null,                       // "detached" | "reattached" | null -> .tui-overlay
}
```

`renderTUI` must: render project headers only when `multiProject`; render each section + rows; apply
`--selected`/`--launcher`/attention/glyph; set main title/screen (placeholder takes precedence over
lines); toggle the panel; set footer string from `focus`; show/hide toast & overlay. It must **not**
animate — animation is the drivers' job (they set state over time / add transient classes).

---

## 6. The two drivers (in `tui.js`)

One renderer, two drivers. Both ultimately call `renderTUI(state)`.

### 6.1 Scroll driver (the walkthrough)
- `#demo` is tall (~`min(560vh, 9 * 90vh)`); inside it a `position: sticky; top:…` stage
  (`.demo-stage`, ~100vh) pins `#tui` centered while `.demo-caption` text changes beside/over it.
- A scroll handler (rAF-throttled; `IntersectionObserver` to enable only while `#demo` is in view)
  computes progress `p ∈ [0,1]` across `#demo`, maps to a scene index over `SCENES` (from
  `scenes.js`), sets that scene's `state`, and updates the visible caption (`.demo-caption`).
- Transitions between scenes are **cross-fades + small reveals**, not teleports. Typing/streaming
  effects (scene 0 typing `mmux`, scene 2 streaming agent lines) are time-based reveals the driver
  triggers when a scene becomes active. Keep them short (≤ ~700ms) and skippable.
- Scenes (each = caption + target `state`; full content lives in `scenes.js`, §7):
  0 bare shell → typing `mmux`; 1 boot/sidebar; 2 spawn Claude (via launcher+Enter) + stream;
  3 add terminal + process; 4 detach→reattach overlay (persistence); 5 right git panel slides in;
  6 **attention**: claude finishes, red `●` + toast; 7 linked projects regroup; 8 finale (hand-off).
- Under reduced-motion: no sticky scrubbing — render scene 8's state statically and reveal the
  sandbox immediately; captions become a plain stacked list (still readable).

### 6.2 Keyboard / sandbox driver (the finale)
- Scene 8 turns `#tui` interactive. Show `.tui-sandbox-hint`: `your turn — click in to play`.
- On click/focus into `#tui` (make it `tabindex="0"`): trap keys, set `state.focus="sandbox"`,
  swap footer to the sandbox hint string. `Esc` or click-out releases the trap (back to scrolling).
- Keybinds (authentic subset): `↑`/`k` move selection up, `↓`/`j` move down (wraps within the flat
  list of selectable rows, launchers included), `Enter` activates the selected row:
    - launcher `+ New Claude` → append a `claude`/`claude 2` agent row (status running), focus main,
      stream a couple of lines;
    - launcher `+ New Terminal` → append `zsh` terminal row;
    - launcher `+ New Process` → append `dev server` process row (running);
    - a running session row → set `focus:"main"`, show its screen.
  `x` closes/stops the selected session row (running → stopped `·`, or remove if you prefer; pick
  one and be consistent). `Esc` from main returns focus to sidebar. Keep it small, correct, authentic.
- Must be fully keyboard-operable and screen-reader-sane (the sandbox may be `aria-hidden` while the
  static fallback carries meaning; document whichever you choose).

---

## 7. `scenes.js` contract

Defines a single global: `window.MMUX_SCENES`. No logic, just data the scroll driver consumes.

```js
window.MMUX_SCENES = [
  {
    id: 0,
    caption: { title: "it starts as one command.",
               body: "mmux lives in your terminal — one binary, one directory." },
    type: { target: "main", text: "mmux" },        // optional: typing effect hint for the driver
    state: { /* a full `state` object per §5.3 for this scene */ },
  },
  // ... scenes 1..8
];
```

Author all 9 scenes with authentic content (the §5.2 strings, the §6.1 beats). Captions are terse,
lowercase, confident. Scene 8's caption hands off to the sandbox:
`{ title: "your turn.", body: "↑↓ move · ⏎ open · x close. spawn an agent from a + New row." }`.

Scene-by-scene caption copy (use these):
- 0 — **it starts as one command.** / mmux lives in your terminal — one binary, one directory.
- 1 — **a sidebar of things, one pane for the focused one.** / agents you spawn, terminals you open, processes you watch.
- 2 — **spawn an agent on demand.** / pick "+ New Claude", hit enter — it runs in its own pane.
- 3 — **everything in one list.** / a terminal here, a dev server there — all side by side.
- 4 — **it doesn't die when you do.** / it lives in a per-directory tmux session. detach, drop ssh, come back — nothing lost.
- 5 — **keep a panel pinned.** / lazygit beside your work, following whichever project is active.
- 6 — **when something needs you, you'll know.** / a bell becomes a dot — and a real desktop notification. even over ssh.
- 7 — **many clones, one sidebar.** / link sibling projects; each gets its own section.
- 8 — **your turn.** / ↑↓ move · ⏎ open · x close. spawn an agent from a + New row.

---

## 8. File manifest & wiring

```
web/
  index.html        # skeleton + all sections (§4) + the #tui skeleton (§5.1). loads:
                    #   <link rel="stylesheet" href="styles.css">
                    #   <script defer src="scenes.js"></script>   (must load before tui.js)
                    #   <script defer src="tui.js"></script>
  styles.css        # the whole design system (§2,§3) + every class in §4/§5. no @import, no url() to remote.
  scenes.js         # window.MMUX_SCENES (§7). pure data.
  tui.js            # renderTUI(state) + scroll driver + sandbox driver + copy-button + nav (§5,§6).
  fonts/            # empty + README.md (optional self-host instructions). site uses the system stack.
  DESIGN.md         # this file.
  Dockerfile        # FROM nginx:alpine; copy web/ -> /usr/share/nginx/html; the nginx.conf.
  nginx.conf        # gzip on; sane cache headers; serve index.html; security headers; no external anything.
  .dockerignore
  README.md         # short: what this is, `docker build`/`run`, and "just open index.html".
```

Wiring rules:
- `tui.js` reads `window.MMUX_SCENES`; guard if missing (degrade to static scene 8).
- Copy buttons: `button.copy[data-copy]` → `navigator.clipboard.writeText`, with a textarea fallback;
  flash the button label to `copied` for ~1s. Wire generically in `tui.js`.
- No inline event handlers in HTML (CSP-friendly). All behavior attached in `tui.js`.
- Favicon: a tiny inline `data:` favicon (a block cursor) is fine; do not fetch a remote one.

---

## 9. Definition of done (reviewers check all)

- [ ] Loads & runs with **zero network requests** (offline + `file://` both work).
- [ ] Color budget holds: only `--accent` (interaction) and `--alert` (attention/error) hues appear;
      everything else grayscale. No decorative color. The scene-6 red dot is the first warm color.
- [ ] Fake TUI matches §5.2 authentic strings/glyphs exactly (sections, launchers, footer, titles,
      glyphs `● ○ · ▌`). No invented `a`/`t` spawn hotkey.
- [ ] Scroll driver: 9 scenes scrub smoothly, sticky stage pins, captions track, no jank/teleport.
- [ ] Sandbox: keyboard-operable per §6.2, focus-trapped only while engaged, escapable.
- [ ] `prefers-reduced-motion`: no scrubbing/typing; lands on playable scene-8 state; captions legible.
- [ ] Responsive: ≥ `--bp-compact` three-column TUI; below it collapses cleanly; page never overflows x.
- [ ] Semantic landmarks, heading order, visible `:focus-visible`, AA contrast, `aria-hidden` on
      decorative box-drawing/cursor.
- [ ] `node --check` passes on `scenes.js` and `tui.js`. No console errors. No global leaks beyond
      `MMUX_SCENES` and one optional `MMUX` namespace.
- [ ] Docker image builds and serves `index.html` at `/`.
```
