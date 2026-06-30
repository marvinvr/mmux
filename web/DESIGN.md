# mmux.org — design & build contract (v2)

Single source of truth for the static site in `web/`. v2 is a **ground-up redesign**. v1 looked
like cramped ASCII art; v2 is a **premium dark dev-tool landing page** (Warp / Zed / Ghostty
caliber) where the ONE thing that looks like a terminal is a single, gorgeous, high-fidelity
**terminal window** — and inside it you watch the **real Claude Code welcome + session, the real
OpenAI Codex banner, a real shell, a real vite server, and the real native mmux git panel**,
syntax-colored and legible.

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
  inside the terminal screen if a real program would print them (e.g. the git panel's box rules,
  the Codex banner box) — never
  as the site's own structure. It must read as a real, modern website.
- **Panes show REAL, legible content.** No `{name} streams ✓ lines` abstractions. The main pane
  shows an actual Claude Code / Codex session / shell / vite output; the panel shows the actual
  native mmux git panel (Changes / Branches / Recent boxes, mirroring `src/app/view/git.rs`). See §8.
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
- `.brand`: the **mmux mark** (`.brand-mark` — an inline pixel SVG: a green tile, beveled, with
  the `m` knocked out; the same mark drives the `data:` favicon, the apple-touch icon, and
  `assets/logo.svg`/`assets/logo.png` in the repo root) + the `mmux` wordmark, links to `#top`.
- `.nav-links`: `the demo · features · how it works · github` (github → https://github.com/marvinvr/mmux).
  On phones (≤640px) the in-page section links drop out, leaving just the `.brand` logo and `github`.
- `.btn.btn-brand` "install" → scrolls to `#install` (glowing gradient button).

### 4.2 `<section id="hero">` — text hero + glow (NO terminal frame art)
- kicker: `a terminal multiplexer for AI agents`
- `h1.hero-wordmark`: **mmux** (gradient) — plus a block cursor `▮`.
- `p.hero-tagline` (the lede): **persistent terminals for your coding agents.**
- `p.hero-sub`: **spawn agents, run your dev processes, and keep every session alive in one place — even after you close the terminal or drop ssh.**
- `.hero-install`: a single prominent install row — `$ brew install marvinvr/mmux/mmux` + a `[copy]`
  button (the ONLY install command on the page besides §4.6).
- `.hero-meta`: three small chips — `one rust binary` · `works over ssh` · `GPLv3 licensed`.
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
`mmux` (small gradient) · `GPLv3` · `github` · `built by marvinvr` (→ https://marvinvr.ch).
Muted, roomy, a final `▮`.

---

## 5. The terminal window — DOM, chrome, state, rendering

A single reusable, high-fidelity component. **`index.html` ships the static skeleton**;
**`tui.js`'s `renderTUI(state)`** fills it. CSS styles exactly the classes below.

### 5.1 DOM skeleton (ids/classes are LAW)

```html
<div id="tw" class="tw" role="img" aria-label="a simulated mmux terminal session">
  <div class="tw-bar">                       <!-- macOS Terminal window chrome -->
    <span class="tw-lights" aria-hidden="true"><i></i><i></i><i></i></span>  <!-- traffic lights (decorative) -->
    <span class="tw-titlebar-name">mmux</span>  <!-- centered window title -->
  </div>
  <div class="tw-body">
    <!-- each region is its own box-drawing frame (ratatui Block); its title is cut
         into the top border by an absolutely-positioned .tw-region-title chip. -->
    <div class="tw-region tw-region--sidebar">
      <span class="tw-region-title tw-sidebar-title" aria-hidden="true">app</span>  <!-- JS: project name -->
      <div class="tw-sidebar" aria-hidden="true"><!-- JS: projects + sections + rows --></div>
    </div>
    <div class="tw-region tw-region--main">
      <span class="tw-region-title tw-main-title" aria-hidden="true"></span>  <!-- JS: " Claude — running " -->
      <div class="tw-main">
        <div class="tw-tab"></div>            <!-- hidden; the pane label lives on the box border -->
        <div class="tw-screen"></div>          <!-- realistic lines (§5.4) or placeholder -->
      </div>
    </div>
    <div class="tw-panel" hidden>            <!-- native git panel; the .git-box stack IS the column -->
      <div class="tw-panel-head"> git </div>
      <div class="tw-panel-screen"></div>
    </div>
  </div>
  <div class="tw-status"></div>             <!-- key hints, per focus -->
  <div class="tw-toast" hidden></div>        <!-- macOS notification banner (scene 7) -->
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
The model is **a macOS Terminal window hosting a real TUI**: the *outer* window is mac chrome
(rounded, traffic lights, a centered title); the *inside* must read as a terminal TUI, not a
desktop app — every region is its own box-drawing frame (a ratatui `Block` — a 1px `--tui-border`
line with the title cut into the top edge) on **one** terminal background; no per-panel shading.
- **Window:** `--surface` bg, 1px `--border-2`, **rounded** (`border-radius: 10px`, a mac window),
  and a soft, lifted drop shadow (no glow/bloom).
- **Title bar `.tw-bar`:** macOS Terminal chrome — three **traffic lights** `.tw-lights i` on the
  left (`#ff5f57 / #febc2e / #28c840`; decorative, not clickable, but they reveal `✕ – +` on
  hover), a **centered** `mmux` title (`.tw-titlebar-name`, absolutely centered), nothing on the
  right. Bg `--surface-2` (a touch lighter than the screen). The bar is static — no `renderBar`
  data drives it.
- **Region frames `.tw-region`:** the sidebar and main pane are each a `.tw-region` — a bordered box
  whose `.tw-region-title` chip sits on the top border (bg `--surface`, masking the line behind it,
  exactly how ratatui paints a `Block` title). The sidebar's title is the active project name; the
  main pane's is the session label (` Claude — running `, set from `main.title`). The **focused main
  pane is bordered magenta** (`.tw--main-focus` → `--magenta`), matching `Focus::Terminal`.
- **Sidebar `.tw-sidebar`:** ~190px region, transparent (the unified terminal bg), framed by its
  `.tw-region` box. **Order matches the real tool
  (`src/app/nav.rs build_nav`): the `+ New …` launcher comes FIRST in each section, then the
  sessions.** Rows are **compact** (tight vertical padding / line-height) so launchers + sessions for
  agents, terminals and processes all fit without scrolling. Section heads `.sb-head` uppercase,
  tracked, **`--cyan`** (theme.rs paints them cyan), small. Active row: the tool's desaturated indigo
  selection (`#2d2d3c`) + a green cursor edge + brighter text. Status dot `.sb-dot` colored by
  `data-status` (running → `--running`, exited → `--faint`, stopped → `--muted`). Launchers: a green
  `+` and green name (the tool paints them `Color::Green`), hover → `--green`. Bell `.sb-bell` coral.
- **Main `.tw-main`:** the focused program, filling its `.tw-region` box. The pane is labelled the
  way the real mmux labels it — the session name + status cut into the **box's top border**
  (`main_title`, src/app/view/pane.rs), via the `.tw-main-title` chip — **not** a separate tab
  strip, so `.tw-tab` stays hidden (kept only for layout stability). `.tw-screen` = the content
  (§5.4), a tight terminal line-height (~1.5), left-padded. The block cursor is a real CSS block
  sized to the text height (the `▮` glyph renders short).
- **Panel `.tw-panel`:** ~200px right col, transparent, **no outer frame** — the three `.git-box`es
  ARE the column (each its own bordered box). The **native git panel**:
  three bordered `.git-box`es — **Changes** (a file tree with `[✓]`/`[~]`/`[ ]` staging checkboxes,
  names colored by change type, the cursor row on a magenta bar), **Branches** (current `●` green),
  **Recent** (short hash + summary) — each with its title sitting on the top border like a ratatui
  `Block`; the focused box is bordered magenta. The stack **fills the panel's full height** — the
  Changes box (the file tree, first box) takes the slack — so there's no dead space below Recent. A
  long line (e.g. a Recent commit summary) is clipped with an ellipsis, never spilling outside the
  box. In the **sandbox** the Changes box is a live **tree**: every node (`.git-row`, file *or*
  folder) is clickable — a file stages / unstages itself, a folder stages / unstages all of its
  descendants, and each folder's checkbox is **tri-state**, computed from its leaves (`[✓]` all
  staged · `[~]` some · `[ ]` none) so the selection ripples up the tree. The `▌` cursor moves to
  the clicked node; hover gives an affordance. Mirrors `src/app/view/git.rs`.
- **Status bar `.tw-status`:** thin bottom bar. In the scroll demo it shows a per-scene **hint of
  what's happening** (the demo isn't interactive); in the playable sandbox it shows the real,
  working key hints keyed by focus.
- **Bare mode (`state.bare`):** scene 0's "before mmux" terminal — the sidebar region, panel,
  status, tab, the region title chips and the `mmux` titlebar name/meta are hidden (`.tw--bare`),
  and the main region drops its box border so the screen fills a plain full-width shell. The
  reveal types `mmux` into it and **rests there** (no in-scene takeover, so the step stays visible
  while scrolling down — not only when scrolling back up into it). The mmux layout "pops up" in
  **scene 1** (`boot:true` → a one-shot `.tw--boot` animation slides the chrome in).
- **Workspace pager `.sb-switch`:** linked clones render **one at a time** (stacking N clones is
  unreadable in a ~120–190px column). Only the active clone's rows show; a quiet footer pinned to
  the sidebar bottom — `‹ name •∘ ›` (chevrons + active name + position dots) — switches between
  them, and is built only when there's more than one clone. Switch by tapping the chevrons,
  swiping the terminal horizontally, or the `[` / `]` keys (sandbox only); the sidebar rows and the
  focused main pane follow the active clone.

### 5.3 `state` shape (contract between tui.js / scenes.js / renderTUI)

```js
state = {
  title: "~/dev/app",                 // active clone's cwd (no longer shown — bar title is a static "mmux")
  bare: false,                        // true → plain terminal: mmux chrome hidden (scene 0)
  status: "…",                        // bottom-bar hint; falls back to STATUS[focus] (sandbox)
  multiProject: false,                // when true: render only the active clone + a pager
  projects: [{ name, active }],       // exactly one active; the pager switches which
  sidebar: [ { kind:"AGENTS"|"TERMINAL"|"PROCESSES", rows: [   // launchers FIRST, then sessions
      { id, launcher:true, name:"New Claude" },   // rendered as "+ New Claude"
      { id, name, sub?, status:"running"|"exited"|"stopped", active?, attention?, project? },
  ]}],
  main: {
    program: "claude"|"codex"|"zsh"|"vite"|null,  // content flavour only (the tab is hidden)
    title: " Claude — running ",            // the pane label, cut into the box's top border
    lines: [ Line ],                        // realistic content (§5.4); placeholder beats lines
    placeholder: str|null,
    cursor: bool,                           // block cursor after last line
  },
  // native git panel: a column of titled bordered boxes (Changes / Branches / Recent)
  panel: { visible, branch, sections: [ { title, active?, lines: [ Line ] } ] },
  focus: "sidebar"|"main"|"panel"|"sandbox",
  toast: { app, title, body, time? } | null,     // macOS-style notification (time defaults to "now")
  overlay: "disconnected"|"reattached" | null,
}

// Scene 0 additionally carries `term` (a bare-terminal state): the reveal types
// `mmux` into it and rests there. Scenes may also carry `weight` (how much scroll
// the scene gets; default 1) and `boot:true` (replay the .tw--boot "pop up" when
// the scene renders — scene 1, right after `mmux` is typed).
```

### 5.4 Line / token model — how realistic content is rendered

`Line` is one of:
- `"plain string"` → a `.screen-line` with that text.
- `{ text: "…", cls?: "ln-add"|"ln-del"|"ln-cmd"|"ln-dim" }` → line-level styled.
- `{ tokens: [ {t:"text", c:"tone"} , … ], cls?: "…" }` → spans `.tok-<tone>` per token.

Tones (`c`): `kw fn str num comment type path op add del ok warn info ai dim prompt brand`, plus
`claude` (the warm-orange Claude logo) and `codex` (the teal Codex `>_`/`›`). Line `cls` adds
`art` for the agent welcome banners (block-glyph logo / Codex box): it tightens leading to terminal
line-height so the glyph rows tile into one mark instead of tearing apart at the screen's 1.7 leading.
Renderer `renderLine(line)`:
- div `.screen-line` (+ `cls`); string → text node; `{text}` → text node; `{tokens}` → one
  `<span class="tok-<c>">` per token (no class if `c` empty/omitted). Append a `.screen-cursor` `▮`
  to the last line when `main.cursor`. Whitespace preserved (`white-space: pre-wrap`; `art` → `pre`).

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
- **Weighted bands:** each scene gets scroll proportional to its `weight` (default 1); progress `p`
  is mapped to a scene via cumulative edges (`buildBounds`), so the opening "type mmux" scene holds
  the stage a little longer (`weight ~1.7`) and reads clearly while scrolling **down**, not only up.
- **Streaming reveal** for the agent scenes (Claude *and* Codex): reveal the agent's `main.lines`
  progressively (line by line, ≤ ~900ms total) so it feels like the agent is working.
- **Bare reveal** for scene 0: render its bare `term` (a plain terminal) and type `mmux` into it
  char-by-char, then rest. No takeover here — the layout pops up on the next scene. The demo's first
  paint is `term`, so it opens on the plain terminal.
- **Boot pop** for any `boot:true` scene (scene 1): after rendering, replay the one-shot `.tw--boot`
  animation so the mmux chrome visibly slides/fades into the window — the "it pops up" beat.
- reduced-motion: no scrub; render the last scene statically; captions become a stacked list;
  enable the sandbox immediately.

### 6.2 Sandbox driver (`#tw-how` — live & typeable)
- `#tw-how` is interactive whenever on screen (`tabindex`/role managed in JS — decorative
  `role="img"` until focused, then `role="application"`; keys trapped only while engaged;
  Esc/click-out releases).
- **Type out of the box:** the initially-shown pane is a real `freshPane` (the active session), and
  **engaging focuses it** when it's typeable — so you can type into the open Claude/terminal right
  away, without first switching to another row and back. (If the open pane isn't typeable, engaging
  lands on the sidebar list instead.) `Esc` steps main → sidebar → out.
- **Single project:** the sandbox is one project (`app`) — no linked-workspace pager / switcher.
- **Git panel is live:** the Changes box is a tree — click a file to stage/unstage it, or a folder
  to stage/unstage all its children; folder checkboxes are tri-state (`[✓]`/`[~]`/`[ ]`) (§5.2).
- **Sidebar nav** (focus sidebar/sandbox): `↑/k` `↓/j` move the selection over the flat row list
  (launchers included), `Enter` activates (launcher → spawn a session of that kind; running row →
  focus it), `x` stops the selected session.
- **Typeable panes** (focus main): a focused **terminal / Claude / Codex** pane takes keystrokes as
  input (`freshPane`). The terminal runs a handful of hardcoded commands (`runCommand`: `ls`, `pwd`,
  `echo`, `date`, `git status`/`branch`/`log`, `cargo run`/`test`, `clear`, else `command not
  found`). Claude/Codex open **ready for input**; on `Enter` the message you typed commits to the
  scrollback as a **past message** and the agent runs a **finite** turn — a live status tail
  matching each real agent (Claude: a colour-cycling `✻` + a rotating gerund + `(Ns · ↓ N tokens ·
  esc to interrupt)`; Codex: a braille spinner + `Working (Ns · N tokens · esc to interrupt)`), with
  realistic tool-call **events** (`⏺ Read(…)` + its `⎿` result for Claude; `• Edited …` + `└` for
  Codex) appended in order; it walks its event list once, prints a closing `Done — …` line, and
  **hands the prompt back** so you can send another message. The composer stays pinned at the bottom
  the whole time — **greyed and disabled while it works** (`paintWorking` renders a dimmed,
  cursor-less prompt), live again once it finishes (`finishWorking` → `paintPane`). A **process**
  pane is output-only.
- **Cursor rule:** only an *idle* Claude/Codex/terminal pane shows the input block cursor; a process
  pane (and a working agent's dimmed composer) does not.
- `Esc`: interrupts a working agent → main→sidebar → releases the trap. The bottom-bar hint
  (`sandboxStatus`) always reflects the current mode, and every key it names actually works.
- Authentic: spawning is via the `+ New …` launchers, no invented hotkeys.

---

## 7. scenes.js contract

`window.MMUX_SCENES = [ …9 scenes ]`, pure data. Each: `{ id, caption:{kicker?,title,body}, type?,
weight?, boot?, term?, state }`. Captions terse, lowercase, confident. Caption copy (use verbatim):

- 0 — **it starts with one command.** / open any ordinary terminal, in any directory, and type mmux. one binary — nothing else to set up. *(bare terminal; the reveal types `mmux` and rests — no takeover; `weight ~1.7` so it holds the stage)*
- 1 — **everything in one window.** / agents you spawn, terminals you open, processes you watch — each a row; the focused one fills the pane. *(`boot:true` → the mmux layout pops up; both Claude & Codex configured: `+ New Claude`, `+ New Codex`)*
- 2 — **spawn an agent.** / pick "+ New Claude" and the real Claude Code goes to work in its own pane. *(the real Claude welcome banner + session)*
- 3 — **or codex. or whatever you run.** / claude and codex come configured out of the box — any agent is one line of yaml away. *(the real Codex banner + session)*
- 4 — **a terminal when you need one.** / drop into a shell in the same window. *(zsh + cargo run)*
- 5 — **every process in one place.** / your dev server, your tests, your watcher — start, stop and tail them all here, just like your agents. *(PROCESSES: dev server, tests, typecheck + vite pane)*
- 6 — **a git panel, pinned.** / a native git panel right where you work, following whichever project is active. *(Changes/Branches/Recent boxes; files clickable in the sandbox)*
- 7 — **it taps you on the shoulder.** / a bell becomes a dot — and a real desktop notification. even over ssh. *(attention + toast)*
- 8 — **it survives you.** / lives in a per-directory tmux session. close the terminal, drop ssh, come back — nothing lost. *(disconnect overlay — the closer; moved last, it's the payoff)*

The bottom bar (`.tw-status`) shows a per-scene **hint of what's happening** (`state.status`), not key
bindings — the scroll demo isn't interactive (the playable sandbox below it is).

Each scene's `state` carries the realistic content below. scenes.js authors the full state; these
are the canonical content blocks (match the spirit; minor wording fine, keep it authentic & legible).

**Scene 2 — Claude Code session** (`main.program:"claude"`, streamed). The real shape Claude Code
prints — a line of prose, then `⏺` tool calls each with their `⎿` result, a `+/-` diff, a closing
line:
```
> refactor auth to use the new TokenService                 ({tokens}: "> " dim, rest text)
                                                            (blank)
⏺ I'll route token creation through TokenService and …      ("⏺" ai, "TokenService" type)
                                                            (blank)
⏺ Read(src/auth.rs)                                          ("⏺" ai, "Read" fn, "(…)" path)
  ⎿  Read 248 lines                                          (dim)
⏺ Update(src/auth.rs)                                        ("⏺" ai, "Update" fn)
  ⎿  Updated with 1 addition and 1 removal                  (dim)
       -  let token = generate_token(user_id);              (cls ln-del)
       +  let token = self.tokens.issue(user_id)?;          (cls ln-add)
⏺ Bash(cargo test auth)                                      ("⏺" ai, "Bash" fn)
  ⎿  test result: ok. 12 passed; 0 failed                   ("ok." ok, rest dim)
⏺ Done — auth now issues tokens through TokenService.        ("⏺" ai, "TokenService" type)
```
The Claude scene is preceded by the **real Claude Code welcome banner** (its block-glyph logo,
captured verbatim from `claude`; `claude` tone = warm orange; each row `cls:"art"` so the glyph tiles):
```
 ▐▛███▜▌   Claude Code v2.1.193
▝▜█████▛▘  Opus 4.8 (1M context) with xhigh effort · Claude Max
  ▘▘ ▝▝    ~/dev/app
```
(captured verbatim from a running `claude`, with the demo's `~/dev/app` cwd.)
sidebar: AGENTS → `+ New Claude`, `+ New Codex`, then Claude (running, sub "refactoring auth", active).

**Scene 3 — Codex session** (`main.program:"codex"`). The **real OpenAI Codex banner** (captured
verbatim from `codex`; `codex` tone = teal; box rows `cls:"art"`), then a `›` prompt + `•` action
lines each with a `└` result + a diff:
```
╭──────────────────────────────────────────────╮
│ >_ OpenAI Codex (v0.142.2)                   │
│                                              │
│ model:       gpt-5.5 high   /model to change │
│ directory:   ~/dev/app                       │
│ permissions: YOLO mode                       │
╰──────────────────────────────────────────────╯
```
(captured verbatim from a running `codex`, with the demo's `~/dev/app` cwd.)
sidebar AGENTS shows Claude (running) and Codex (running, active) under both launchers.

**Scene 4 — shell** (`main.program:"zsh"`): `❯ cargo run`, Compiling/Finished/Running, "listening on".

**Scene 5 — processes** (`main.program:"vite"`). PROCESSES lists dev server (running, "vite · :5173",
active), tests (running, "vitest · watch"), typecheck (stopped, "tsc --watch"). Main = the vite banner:
```
  VITE v5.2.0  ready in 412 ms                               ("VITE" brand bold, "ready" ok)
  ➜  Local:    http://localhost:5173/                        ("➜" info, url path)
  ➜  Network:  http://192.168.1.4:5173/                      (info/path)
 10:42:01 [vite] hmr update /src/App.tsx                     (time dim, "[vite]" brand, path path)
```

**Scene 6 — native git panel** (`panel.visible`, `panel.sections`, branch "main"). Three boxes,
mirroring `src/app/view/git.rs`:
```
┌ Changes · main ↑1 ┐
 [~] app/                  (root, "[~]" warn, "app/" info=blue)
   [~] src/
▌    [✓] auth.rs           (cursor row: "▌" ai/magenta, "[✓]" ok, name warn=modified, .git-sel bg)
     [ ] token.rs
     [✓] lib.rs            (name ok=added)
┌ Branches ┐
 ● main   origin/main      ("●"+name ok=green, track dim)
   feat/tokens
┌ Recent ┐
 e2e6087 add token service (hash dim, summary text)
```

**Scene 7 — attention:** Claude row `attention:true` (coral bell pulses) + `toast:{ app:"mmux",
title:"Claude needs your input", body:"approve the edit to src/auth.rs?" }`, rendered as a macOS
notification banner (mmux app icon, system font, translucent material) floating top-right over the
terminal. Main shows Claude paused at an approval prompt (below its banner).

**Scene 8 — overlay (the closer):** `overlay:"disconnected"` (text like `ssh disconnected — session
kept alive`). Keep sidebar/main state intact underneath (dimmed by overlay). Persistence is the
payoff, so this lands last rather than mid-walkthrough.

**Linked workspaces** (`multiProject:true`, e.g. `app` active + `app-2`): a real mmux feature the
renderer still supports — the sidebar shows **one clone at a time** (only the active clone's rows
render) with a bottom pager (`‹ name •∘ ›`) to switch (no stacked per-project headers), driven by
the chevrons, a horizontal swipe, or `[` / `]`. It is **not shown in the sandbox** — `#tw-how`
(`DEFAULT_STATE`) is intentionally single-project (`app`), so the bottom-left project switcher is
absent; the playable demo stays focused on spawning/typing and the git panel.

Footer hints (`.tw-status`, by focus): sidebar `↑↓ move   ⏎ open   x close   r restart   d detach`,
main/panel `keys → pane   drag = copy   ⌃b   h back   x close`, sandbox `↑↓ move   ⏎ open   x close   ·   click out to scroll`.

---

## 8. Realistic content rule (the "make it make sense" mandate)

The whole point: a visitor scrolling the demo sees **recognizable software**. The Claude scene must
open on Claude Code's real welcome banner (its block-glyph logo) and read like a real transcript
(the `●` tool bullets, `Read`/`Edit`/`Bash`, a `+/-` diff, a test result, a closing summary). The
Codex scene must open on Codex's real boxed banner (the `>_` mark) and read like a real Codex
session. The shell must read like a real zsh+cargo run, vite like a real vite banner, and the git
panel like the real native panel (`src/app/view/git.rs`). The agent banners are captured verbatim
by running `claude` / `codex` — don't invent them. Use the token model (§5.4) so it's syntax-colored.
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
  index.html    # all sections (§4) + the #tw skeleton (§5.1) + inline SVG icons. <head> also
                #   carries canonical + Open Graph/Twitter tags, a sitemap/llms.txt alternate,
                #   and a schema.org SoftwareApplication application/ld+json block. Loads:
                #   <link rel="stylesheet" href="styles.css">
                #   <script defer src="scenes.js"></script>   (before tui.js)
                #   <script defer src="tui.js"></script>
  styles.css    # the whole v2 system (§2,§3) + every class in §4/§5/§9. no @import/remote url().
  scenes.js     # window.MMUX_SCENES (§7) with realistic content (§8). pure data.
  tui.js        # renderTUI(state)+renderLine (§5.4) + scroll & sandbox drivers + copy + nav.
  robots.txt    # welcomes humans, search crawlers, and named AI bots; points to the sitemap.
  sitemap.xml   # the single canonical URL (https://mmux.org/).
  llms.txt      # llmstxt.org-format guide: summary + install + links to the GitHub docs.
  assets/
    og-image.png  # 1200×630 social card referenced by the og/twitter tags.
    og-image.svg  # editable source for og-image.png (regenerate: see the comment in the file).
  Dockerfile    # nginx:alpine. Build-time fingerprints css/js to <name>.<hash>.<ext>
                #   and rewrites the quoted refs in index.html (so the 1y immutable
                #   cache is safe — changed file ⇒ new URL ⇒ refetch, no purge on deploy).
                #   robots/sitemap/llms + assets/ are copied verbatim (stable URLs, not hashed).
  nginx.conf    # gzip + strict CSP + 1y immutable cache on the fingerprinted assets.
  DESIGN.md     # this file.
  README.md / fonts/README.md / .dockerignore  # unchanged.
```
- Copy buttons: `button.copy[data-copy]` → clipboard + textarea fallback; flash `copied` ~1s.
- No inline JS handlers (CSP `script-src 'self'`). Inline SVG icons, a `data:` favicon, and a
  single `application/ld+json` data block (CSP does not treat data blocks as script) are fine.
- One global namespace `window.MMUX` + `window.MMUX_SCENES`.

---

## 12. Definition of done (reviewers check all)

- [ ] Looks like a **premium real website**, not ASCII art. No box-drawing used as site structure
      (only as authentic content inside the terminal screen, if at all). Real borders/cards/diagram.
- [ ] The terminal is a **mac window hosting a TUI**: rounded chrome with traffic lights + a
      centered `mmux`, and inside, every region its own box-drawing frame with the title on the top
      border, one terminal bg, a status bar. The panes show **real, syntax-colored, legible**
      content per §8 (Claude banner + session, Codex banner + session, shell, vite, native git
      panel) — zero placeholder/abstract content.
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
