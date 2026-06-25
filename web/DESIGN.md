# mmux.org — design & build contract (v2)

Single source of truth for the static site in `web/`. v2 is a **ground-up redesign**. v1 looked
like cramped ASCII art; v2 is a **premium dark dev-tool landing page** (Warp / Zed / Ghostty
caliber) where the ONE thing that looks like a terminal is a single, gorgeous, high-fidelity
**terminal window** — and inside it you watch a **real Claude Code session, a real shell, a real
vite server, and real lazygit**, syntax-colored and legible.

If two files must agree on a name (class, id, state field, scene key, token tone), it is defined
here and nowhere else.

---

## 0. Non-negotiables

- **Static only.** Plain HTML + CSS + vanilla JS. No framework, no bundler, no build step.
- **Zero external calls.** No CDNs, fonts, analytics, remote images/scripts. Works offline and over
  `file://`. Plain `<script defer>` + globals (NOT ES modules).
- **NO ASCII-ART CHROME.** This is the headline change. Borders, frames, cards, the terminal window,
  the how-it-works diagram are **real CSS** (1px borders, radii, shadows, gradients, SVG) — never
  box-drawing characters used as layout. Box-drawing chars may appear ONLY as authentic *content*
  inside the terminal screen if a real program would print them (e.g. lazygit panel rules) — never
  as the site's own structure. It must read as a real, modern website.
- **Panes show REAL, legible content.** No `{name} streams ✓ lines` abstractions. The main pane
  shows an actual Claude Code session / shell / vite output; the panel shows actual lazygit. See §8.
- **Install is ONE command:** `brew install marvinvr/mmux/mmux`. No cargo, no from-source. Remove them.
- **`prefers-reduced-motion`** fully honored (no scrub/typing; land on the finished playable state).
- **Accessible:** landmarks, heading order, keyboard operable, visible focus, AA contrast, decorative
  bits `aria-hidden`.

---

## 1. Color philosophy (v2 — vibrant, not boring)

Dark base, but color is used **generously and tastefully**, two distinct roles:

1. **Brand** = a blue→indigo→magenta gradient (`--brand-grad`). It runs through the wordmark, the
   primary button, link hovers, focus rings, section accents, and soft radial **glows**. This is the
   personality. Use it on purpose: gradient headings, glowing CTAs, an accent left-edge on the active
   sidebar row, a bloom behind the terminal. Don't flood every surface — punctuate.
2. **Terminal semantics** = the colors a real terminal/program uses, INSIDE the window: green
   `--running`/`--add`, coral `--attention`/`--del`, amber `--warn`, sky `--info`, magenta for the
   Claude `●` bullet. These make the content legible and authentic and add life.

Backgrounds stay dark; most prose is gray. Color appears as: brand gradient accents + glows, and
syntax/status color in the terminal. The page should feel alive and designed, not monochrome.

---

## 2. Design tokens (`:root`)

```
/* base surfaces — deep, slightly cool, with elevation steps */
--bg:        #08090f;   /* page */
--bg-2:      #0b0d15;   /* alternating section band */
--surface:   #111320;   /* cards, terminal body */
--surface-2: #181b2b;   /* raised: titlebar, selected row, code chips */
--surface-3: #20243a;   /* hover elevation */
--border:    #232844;   /* default 1px hairline */
--border-2:  #343c66;   /* hover / emphasis border */

/* ink */
--text:    #e9ebf5;     /* primary */
--text-2:  #b3b9d2;     /* secondary */
--muted:   #828aa6;     /* tertiary — AA-safe on --bg/--surface */
--faint:   #555d7e;     /* dim decorative only */

/* brand — blue → indigo → magenta */
--brand-1: #38bdf8;
--brand-2: #818cf8;
--brand-3: #e879f9;
--brand-grad:  linear-gradient(110deg, #38bdf8 0%, #818cf8 48%, #e879f9 100%);
--accent:      #818cf8;            /* single solid pick for focus rings / link hover */
--accent-soft: rgba(129,140,248,.14);

/* terminal / semantic signal (inside the window + status dots) */
--running:   #4ade80;   /* green */
--attention: #fb7185;   /* coral — the bell */
--warn:      #fbbf24;   /* amber */
--info:      #38bdf8;   /* sky */
--add:       #4ade80;
--del:       #fb7185;
--ai:        #e879f9;   /* Claude ● bullet / agent accent */

/* syntax token tones (terminal content, §5.4) */
--tok-kw:   #e879f9;  --tok-fn:  #38bdf8;  --tok-str: #86efac;
--tok-num:  #fbbf24;  --tok-comment: #6b7390;  --tok-type: #818cf8;
--tok-path: #7dd3fc;  --tok-op: #b3b9d2;

/* glows */
--glow-brand: 0 0 80px -16px rgba(129,140,248,.55), 0 0 40px -16px rgba(232,121,249,.4);
--glow-sky:   0 0 50px -14px rgba(56,189,248,.5);
--ring:       0 0 0 3px var(--accent-soft);

/* shape / depth */
--radius:    14px;      /* window, cards */
--radius-sm: 9px;
--shadow:    0 24px 60px -24px rgba(0,0,0,.7), 0 8px 24px -16px rgba(0,0,0,.6);

/* type */
--font: ui-monospace, "SF Mono", "JetBrains Mono", "Cascadia Code", "Fira Code", Menlo, Consolas, monospace;

/* scale */
--page-max: 1180px;
--measure: 60ch;
```

Breakpoints: `--bp-md: 900px` (terminal panel hides, grid → fewer cols), `--bp-sm: 640px`
(single column, nav condenses).

---

## 3. Typography & surfaces

- **Monospace everywhere** (the dev identity), but composed like a real product site — never default.
- **Wordmark / big headings:** mono, heavy weight (650–700), tight tracking (`-0.02em`), filled with
  `--brand-grad` via `background-clip: text`. Hero wordmark `clamp(3rem, 9vw, 6rem)`.
- **Section headings (h2):** `clamp(1.7rem, 4vw, 2.6rem)`, `--text`, optionally one accent word in
  gradient. A small uppercase brand-colored eyebrow/kicker above each (e.g. `// the demo`).
- **Body:** 0.98rem, line-height 1.75, `--text-2`. Max width `--measure` for prose.
- **Surfaces:** cards/sections use real `--surface` fills, 1px `--border`, `--radius`, soft shadows.
  Generous padding and whitespace — this is the antidote to the cramped v1.
- **Nav:** sticky, translucent `rgba(8,9,15,.6)` + `backdrop-filter: blur(12px)`, 1px bottom border.
- **Background life:** the page has subtle, fixed radial brand glows (e.g. a sky bloom top-left, a
  magenta bloom behind the terminal) at low opacity — depth without noise. `prefers-reduced-motion`
  keeps them (they're static), but no animation.

---

## 4. Page sections (semantic, in order) + verbatim copy

### 4.1 `<header class="site-nav">` — sticky, translucent
- `.brand` wordmark `mmux` (gradient), links to `#top`.
- `.nav-links`: `the demo · features · how it works · github` (github → https://github.com/marvinvr/mmux).
- `.btn.btn-brand` "install" → scrolls to `#install` (glowing gradient button).

### 4.2 `<section id="hero">` — text hero + glow (NO terminal frame art)
- kicker: `a terminal multiplexer for AI agents`
- `h1.hero-wordmark`: **mmux** (gradient) — plus a block cursor `▮`.
- `p.hero-tagline` (the lede): **persistent terminals for your coding agents.**
- `p.hero-sub`: **spawn agents, run your dev processes, and keep every session alive in one place — even after you close the terminal or drop ssh.**
- `.hero-install`: a single prominent install row — `$ brew install marvinvr/mmux/mmux` + a `[copy]`
  button (the ONLY install command on the page besides §4.6).
- `.hero-meta`: three small chips — `one rust binary` · `works over ssh` · `MIT licensed`.
- `.scroll-cue`: `scroll to watch it work ↓` (hidden under reduced-motion).

### 4.3 `<section id="demo">` — THE CENTERPIECE (§5, §6, §7, §8)
A tall scroll section; a sticky stage pins the **terminal window** while captions track beside it.
Nine scenes of realistic content, then the playable sandbox.

### 4.4 `<section id="features">`
kicker `// what you get`, h2 **everything in one place.** A real card grid of **6** `.feature`
cards (icon + title + desc; hover = lift + brand-tinted border/glow). Copy verbatim:

1. **per-directory & persistent** — one mmux per directory, kept alive inside a tmux session. close the terminal or drop ssh, reattach, and it's exactly where you left it.
2. **agents on demand** — spawn claude, codex, or any agent. each runs in its own pane, started and restarted right from the sidebar.
3. **processes you watch** — start, stop and tail your dev server, tests and tasks without ever leaving the multiplexer.
4. **attention, caught** — a bell or a notification escape becomes a sidebar dot and a real desktop notification. even over ssh.
5. **linked projects** — group sibling clones into one sidebar, each its own section; the panel follows whichever project is active.
6. **one binary, any terminal** — a single rust binary with no daemon to babysit. it runs anywhere a terminal does.

Each card has a small inline **SVG icon** (1.25rem, `currentColor`/stroke, brand-tinted) — NOT an
emoji, NOT a box-drawing glyph. Simple line icons (folder, sparkle/agent, activity pulse, bell,
layers, terminal). Define them inline in the HTML.

### 4.5 `<section id="how">` — real diagram (NOT ascii)
kicker `// how it works`, h2 **a tui inside a tmux session.** Build a real CSS/SVG diagram (§9):
the visitor's terminal → a glowing "jail" container (the per-directory tmux session) → a mini
rendition of the mmux window inside it. Plus three short points:
- the TUI runs **inside** a per-directory tmux session, so closing the terminal or losing ssh never kills it.
- every agent, terminal and process is one pty-backed pane behind a single unified lifecycle.
- bells and notification escapes are captured and turned into desktop notifications — even over ssh.

### 4.6 `<section id="install">` — single command CTA
kicker `// get it`, h2 **install in one line.** One big code row:
`brew install marvinvr/mmux/mmux` + `[copy]`. Then: `then, in any project directory:` → `mmux`.
Buttons row: `.btn.btn-brand` → github, a ghost button → the README. NO cargo / from-source.

### 4.7 `<footer class="site-footer">`
`mmux` (small gradient) · `MIT` · `github`. Muted, roomy, a final `▮`.

---

## 5. The terminal window — DOM, chrome, state, rendering

A single reusable, high-fidelity component. **`index.html` ships the static skeleton**;
**`tui.js`'s `renderTUI(state)`** fills it. CSS styles exactly the classes below.

### 5.1 DOM skeleton (ids/classes are LAW)

```html
<div id="tw" class="tw" role="img" aria-label="a simulated mmux terminal session">
  <div class="tw-bar">
    <span class="tw-lights" aria-hidden="true"><i></i><i></i><i></i></span>
    <span class="tw-titlebar-name">mmux</span>
    <span class="tw-titlebar-path">~/dev/app</span>
    <span class="tw-titlebar-meta" aria-hidden="true">⌁ tmux</span>
  </div>
  <div class="tw-body">
    <div class="tw-sidebar" aria-hidden="true"><!-- JS: projects + sections + rows --></div>
    <div class="tw-main">
      <div class="tw-tab"></div>            <!-- focused program label, e.g. "claude" -->
      <div class="tw-screen"></div>          <!-- realistic lines (§5.4) or placeholder -->
    </div>
    <div class="tw-panel" hidden>            <!-- lazygit; toggled visible -->
      <div class="tw-panel-head"> git </div>
      <div class="tw-panel-screen"></div>
    </div>
  </div>
  <div class="tw-status"></div>             <!-- key hints, per focus -->
  <div class="tw-toast" hidden></div>        <!-- notification toast (scene 6) -->
  <div class="tw-overlay" hidden></div>      <!-- ssh disconnect / reattach (scene 4) -->
  <div class="tw-sandbox-hint" hidden></div> <!-- "click in to play" (finale) -->
</div>
```

Sidebar inner (JS-produced):
```html
<div class="sb-section">
  <div class="sb-head">AGENTS</div>
  <div class="sb-row sb-row--active" data-id="claude">
    <span class="sb-dot" data-status="running"></span>      <!-- colored status dot -->
    <span class="sb-name">claude</span>
    <span class="sb-sub">refactoring auth</span>            <!-- optional OSC-title subtitle -->
    <span class="sb-bell" hidden>●</span>                    <!-- coral bell when attention -->
  </div>
  <div class="sb-row sb-row--launcher" data-id="new-claude">
    <span class="sb-plus" aria-hidden="true">+</span><span class="sb-name">New Claude</span>
  </div>
</div>
<!-- linked workspaces: ONE clone's rows show at a time; this pager (built only
     when projects.length > 1) is pinned to the sidebar bottom and switches them. -->
<div class="sb-switch">
  <button class="sb-switch-arrow" data-switch="prev" aria-label="previous project">‹</button>
  <span class="sb-switch-mid">
    <span class="sb-switch-name">app</span>
    <span class="sb-switch-dots" aria-hidden="true"><span class="sb-switch-dot sb-switch-dot--on"></span><span class="sb-switch-dot"></span></span>
  </span>
  <button class="sb-switch-arrow" data-switch="next" aria-label="next project">›</button>
</div>
```

### 5.2 Chrome details
- **Window:** `--surface` bg, 1px `--border`, `--radius`, `--shadow`, and a soft brand glow bloom
  behind it (a `::before`/sibling, low opacity, `--glow-brand`). Slight inner top highlight optional.
- **Title bar `.tw-bar`:** `--surface-2`, 3 traffic-light dots (`.tw-lights i`) — tasteful; either
  monochrome `--faint` or subtly tinted (`#fb7185 / #fbbf24 / #4ade80` at ~0.8). Center-left the
  `mmux` name (bright) + path (muted). Right: `.tw-titlebar-meta` `⌁ tmux` (faint), or branch.
- **Sidebar `.tw-sidebar`:** ~190px col, `--bg-2`, 1px right border. Section heads `.sb-head`
  uppercase, tracked, `--muted`, small. Rows comfortable (not cramped). Active row: `--accent-soft`
  bg + a 2px brand-gradient left edge + brighter text. Status dot `.sb-dot` colored by
  `data-status` (running → `--running`, exited → `--faint`, stopped → `--muted`). Launchers: a `+`
  in `--muted`, name in `--muted`, hover → brand. Bell `.sb-bell` coral.
- **Main `.tw-main`:** the focused program. `.tw-tab` = a small top label (program name, a faint
  status). `.tw-screen` = the content (§5.4), comfortable line-height (~1.6), left-padded.
- **Panel `.tw-panel`:** ~210px right col, `--bg-2`, 1px left border; `.tw-panel-head` = ` git `
  chip. lazygit-style content inside.
- **Status bar `.tw-status`:** thin bottom bar, `--surface-2`, muted key hints; keys in `--text-2`.
- **Workspace pager `.sb-switch`:** linked clones render **one at a time** (stacking N clones is
  unreadable in a ~120–190px column). Only the active clone's rows show; a quiet footer pinned to
  the sidebar bottom — `‹ name •∘ ›` (chevrons + active name + position dots) — switches between
  them, and is built only when there's more than one clone. Switch by tapping the chevrons,
  swiping the terminal horizontally, or the `[` / `]` keys (sandbox only); the title-bar path and
  the focused main pane follow the active clone.

### 5.3 `state` shape (contract between tui.js / scenes.js / renderTUI)

```js
state = {
  title: "~/dev/app",                 // path shown in the title bar (follows the active clone)
  multiProject: false,                // when true: render only the active clone + a pager
  projects: [{ name, active }],       // exactly one active; the pager switches which
  sidebar: [ { kind:"AGENTS"|"TERMINAL"|"PROCESSES", rows: [
      // session row:
      { id, name, sub?, status:"running"|"exited"|"stopped", active?, attention?, project? },
      // launcher row:
      { id, launcher:true, name:"New Claude" },   // rendered as "+ New Claude"
  ]}],
  main: {
    program: "claude"|"zsh"|"vite"|null,   // small tab label; null hides the tab
    title: " claude — running ",            // (kept for a11y / optional); tab uses program
    lines: [ Line ],                        // realistic content (§5.4); placeholder beats lines
    placeholder: str|null,
    cursor: bool,                           // block cursor after last line
  },
  panel: { visible, branch, lines: [ Line ] },   // lazygit content
  focus: "sidebar"|"main"|"panel"|"sandbox",
  toast: { app, title, body } | null,            // desktop-notification toast
  overlay: "disconnected"|"reattached" | null,
}
```

### 5.4 Line / token model — how realistic content is rendered

`Line` is one of:
- `"plain string"` → a `.screen-line` with that text.
- `{ text: "…", cls?: "ln-add"|"ln-del"|"ln-cmd"|"ln-dim" }` → line-level styled.
- `{ tokens: [ {t:"text", c:"tone"} , … ], cls?: "…" }` → spans `.tok-<tone>` per token.

Tones (`c`): `kw fn str num comment type path op add del ok warn info ai dim prompt brand`.
Renderer `renderLine(line)`:
- div `.screen-line` (+ `cls`); string → text node; `{text}` → text node; `{tokens}` → one
  `<span class="tok-<c>">` per token (no class if `c` empty/omitted). Append a `.screen-cursor` `▮`
  to the last line when `main.cursor`. Whitespace preserved (`white-space: pre-wrap`).

Tone → color: `add/ok→--add`, `del→--del`, `warn→--warn`, `info/path→--info/--tok-path`,
`ai→--ai`, `kw→--tok-kw`, `fn→--tok-fn`, `str→--tok-str`, `num→--tok-num`,
`comment/dim→--muted/--tok-comment`, `prompt→` brand, `brand→` brand. `ln-add`/`ln-del` give the
whole line a faint green/coral tint + colored gutter feel; `ln-cmd` = a shell command line;
`ln-dim` = muted.

This token model is what makes the panes look real and colorful. scenes.js authors content with it.

---

## 6. Drivers (in tui.js)

One renderer, two drivers (keep the v1 architecture; it worked).

### 6.1 Scroll driver (§6.1 of v1, retained)
- `#demo` tall; a `position: sticky` `.demo-stage` (~100vh) pins `#tw` while `.demo-caption` blocks
  cross-fade beside it. rAF-throttled scroll→progress→scene index over `window.MMUX_SCENES`;
  IntersectionObserver gates it. Last scene = sandbox hand-off.
- **Streaming reveal** for the Claude scene: reveal the agent's `main.lines` progressively (line by
  line, ≤ ~900ms total) so it feels like Claude is working. Typing reveal for scene 0's `mmux`.
- reduced-motion: no scrub; render the last scene statically; captions become a stacked list;
  enable the sandbox immediately.

### 6.2 Sandbox driver (retained, adapted)
- Finale makes `#tw` interactive (`tabindex`/role managed in JS — decorative `role="img"` until
  playable, then `role="application"`; trap keys only while engaged; Esc/click-out releases).
- Keys: `↑/k` `↓/j` move the selection over the flat row list (launchers included), `Enter`
  activates (launcher → append a real session of that kind + focus main + stream its realistic
  content; running row → focus its pane), `x` stops the selected session, `Esc` main→sidebar / releases.
- Authentic: spawning is via the `+ New …` launchers, no invented hotkeys.

---

## 7. scenes.js contract

`window.MMUX_SCENES = [ …9 scenes ]`, pure data. Each: `{ id, caption:{kicker?,title,body}, type?, state }`.
Captions terse, lowercase, confident. Caption copy (use verbatim):

- 0 — **it's one command.** / mmux runs in any terminal — one binary, one directory.
- 1 — **your work, in a sidebar.** / agents you spawn, terminals you open, processes you watch — one pane for the focused one.
- 2 — **spawn an agent.** / pick "+ New Claude" and it goes to work in its own pane, right beside everything else.
- 3 — **terminals and processes too.** / a shell here, your dev server there — started, watched, never lost in tabs.
- 4 — **it survives you.** / the whole thing lives in a per-directory tmux session. close the terminal, drop ssh, come back — nothing lost.
- 5 — **keep a panel pinned.** / lazygit right where you work, following whichever project is active.
- 6 — **it taps you on the shoulder.** / a bell becomes a dot — and a real desktop notification. even over ssh.
- 7 — **many clones, one sidebar.** / link sibling projects; each gets its own section.
- 8 — **your turn.** / ↑↓ move · ⏎ open · x close — spawn an agent from a + New row.

Each scene's `state` carries the realistic content below. scenes.js authors the full state; these
are the canonical content blocks (match the spirit; minor wording fine, keep it authentic & legible).

**Scene 2 — Claude Code session** (`main.program:"claude"`, streamed). Lines (tones in parens):
```
> refactor auth to use the new TokenService                 ({tokens}: "> " dim, rest text)
                                                            (blank)
●  Read  src/auth.rs, src/token.rs                          ("●" ai, "Read" fn, paths path)
●  Edit  src/auth.rs                                         ("●" ai, "Edit" fn, path path)
     -  let token = generate_token(user_id);                (cls ln-del)
     +  let token = self.tokens.issue(user_id)?;            (cls ln-add)
●  Bash  cargo test auth                                     ("●" ai, "Bash" fn)
     test result: ok. 12 passed; 0 failed                   ("ok." ok, rest dim)
●  auth now delegates to TokenService. ✓                     ("●" ai, "✓" ok)
```
sidebar: AGENTS → claude (running, sub "refactoring auth", active) + launcher.

**Scene 3 — shell + vite** (main shows zsh; process running). zsh `main.program:"zsh"`:
```
~/dev/app  on  main                                         (path muted, "main" branch=ai/magenta)
❯ cargo run                                                  ("❯" prompt brand, cmd text)
   Compiling app v0.2.0                                      (dim)
    Finished `dev` profile in 3.41s                          (dim, "Finished" ok)
     Running `target/debug/app`                              (dim)
  ➜  listening on http://localhost:5173                      ("➜" info, url path)
❯ ▮
```
sidebar adds TERMINAL → zsh (running) and PROCESSES → dev server (running, sub "vite · :5173").

**Scene 5 — lazygit panel** (`panel.visible`, branch "main"). panel.lines (lazygit-ish):
```
 Files                                                       (head dim)
  M src/auth.rs                                              ("M" warn)
  M src/token.rs                                             ("M" warn)
  A src/lib.rs                                               ("A" add)
 Branches                                                    (head dim)
  ✓ main                                                     ("✓" ok, name text)
    feat/tokens                                              (dim)
 Commits                                                     (head dim)
  e2e6087 add token service                                 (hash info, msg text)
  fce46df drag-select scrollback                            (hash info, msg text)
```
Also vite process pane content (when PROCESSES/dev-server is focused), `main.program:"vite"`:
```
  VITE v5.2.0  ready in 412 ms                               ("VITE" brand bold, "ready" ok)
  ➜  Local:    http://localhost:5173/                        ("➜" info, url path)
  ➜  Network:  http://192.168.1.4:5173/                      (info/path)
  ➜  press h + enter to show help                            (dim)
                                                            (blank)
 10:42:01 [vite] hmr update /src/App.tsx                     (time dim, "[vite]" brand, path path)
```

**Scene 4 — overlay:** `overlay:"disconnected"` (text like `ssh disconnected — session kept alive`),
then the narrative implies reattach. Keep sidebar/main state intact underneath (dimmed by overlay).

**Scene 6 — attention:** claude row `attention:true` (coral bell pulses) + `toast:{ app:"claude",
title:"needs your input", body:"approve the edit to src/auth.rs?" }`. Main may show claude paused
awaiting input.

**Linked workspaces** (`multiProject:true`, e.g. `app` active + `app-2`): the sidebar shows **one
clone at a time** — only the active clone's rows render, with a bottom pager (`‹ name •∘ ›`) to
switch (no stacked per-project headers). Demonstrated live in the always-playable sandbox
(`#tw-how`, tui.js `DEFAULT_STATE`), where `app-2` carries its own claude (sub "running tests") and
dev server; switch via the chevrons, a horizontal swipe, or `[` / `]`.

**Scene 8 — finale:** a sensible populated state (matches tui.js DEFAULT_STATE), focus sidebar,
ready for the sandbox; sandbox hint shown.

Footer hints (`.tw-status`, by focus): sidebar `↑↓ move   ⏎ open   x close   r restart   d detach`,
main/panel `keys → pane   drag = copy   ⌃b   h back   x close`, sandbox `↑↓ move   ⏎ open   x close   ·   click out to scroll`.

---

## 8. Realistic content rule (the "make it make sense" mandate)

The whole point of v2: a visitor scrolling the demo sees **recognizable software**. The Claude scene
must read like a real Claude Code transcript (the `●` tool bullets, `Read`/`Edit`/`Bash`, a `+/-`
diff, a test result, a closing summary). The shell must read like a real zsh+cargo run. vite like a
real vite banner. lazygit like real lazygit. Use the token model (§5.4) so it's syntax-colored.
Reviewers reject abstract/placeholder content ("streams ✓ lines", lorem, nonsense paths).

---

## 9. The how-it-works diagram (real, no ascii)

Build with real elements (divs + an inline SVG for connectors), styled and glowing:
- `.how-node--terminal`: a small rounded chip "your terminal" with a sub `local · ssh · tmux client`.
- a downward connector (SVG line/path with a brand stroke + a subtle animated dash on scroll;
  static under reduced-motion) ending in an arrowhead.
- `.how-jail`: a large rounded container with a **dashed, brand-glowing** border and a header chip
  `tmux session · one per directory`, plus a small badge `survives detach · ssh`. Inside it:
- `.how-mmux`: a miniature of the terminal window — three labeled columns `sidebar` · `main pane` ·
  `panel`, each a real bordered box with a couple of faux rows (use the same surface/border tokens).
- caption under the jail: `every pane = a real PTY + a vt100 parser`.
Responsive: stacks vertically on small screens, connectors simplify. It should look like a designed
product diagram, not characters.

---

## 10. Motion

- Scroll-driven scene transitions: content cross-fades + a small translate; the active pane's glow
  intensifies. Claude output streams in. Status dots gently pulse; the bell pulses on attention.
- Hero: a slow idle shimmer on the brand glow (very subtle). Buttons: brand glow on hover.
- Everything gated by `prefers-reduced-motion` (then: no scrub, no streaming, no pulse; static end state).

---

## 11. File manifest & wiring

```
web/
  index.html    # all sections (§4) + the #tw skeleton (§5.1) + inline SVG icons. Loads:
                #   <link rel="stylesheet" href="styles.css">
                #   <script defer src="scenes.js"></script>   (before tui.js)
                #   <script defer src="tui.js"></script>
  styles.css    # the whole v2 system (§2,§3) + every class in §4/§5/§9. no @import/remote url().
  scenes.js     # window.MMUX_SCENES (§7) with realistic content (§8). pure data.
  tui.js        # renderTUI(state)+renderLine (§5.4) + scroll & sandbox drivers + copy + nav.
  Dockerfile    # unchanged (nginx:alpine). nginx.conf unchanged.
  nginx.conf    # unchanged.
  DESIGN.md     # this file.
  README.md / fonts/README.md / .dockerignore  # unchanged.
```
- Copy buttons: `button.copy[data-copy]` → clipboard + textarea fallback; flash `copied` ~1s.
- No inline JS handlers (CSP `script-src 'self'`). Inline SVG icons + a `data:` favicon are fine.
- One global namespace `window.MMUX` + `window.MMUX_SCENES`.

---

## 12. Definition of done (reviewers check all)

- [ ] Looks like a **premium real website**, not ASCII art. No box-drawing used as site structure
      (only as authentic content inside the terminal screen, if at all). Real borders/cards/diagram.
- [ ] The terminal window is high-fidelity: window chrome (lights, title), CSS-bordered columns,
      a status bar, brand glow. The panes show **real, syntax-colored, legible** content per §8
      (Claude session, shell, vite, lazygit) — zero placeholder/abstract content.
- [ ] Palette is the v2 blue→magenta brand gradient + green/coral terminal semantics, used with life
      but not garish. Wordmark/CTA/links/glows carry the gradient.
- [ ] Install shows ONLY `brew install marvinvr/mmux/mmux` (hero + §4.6). No cargo / from-source.
- [ ] how-it-works is a real diagram (§9), not characters.
- [ ] Scroll demo scrubs 9 scenes smoothly; Claude output streams; sandbox is keyboard-playable,
      focus-trapped only while engaged, escapable.
- [ ] reduced-motion: no scrub/stream; lands on the playable finale; captions legible.
- [ ] Responsive: panel hides < 900px, single column < 640px, no x-overflow. Generous spacing.
- [ ] Landmarks, heading order, visible focus, AA contrast, decorative bits aria-hidden.
- [ ] Zero network requests (offline + file://). `node --check` passes on scenes.js + tui.js. No
      console errors. No global leaks beyond MMUX + MMUX_SCENES.
```
