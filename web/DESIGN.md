# mmux.org — design & build contract (v4)

Single source of truth for the static site in `web/`. v4 is the current look: **flat, sharp,
terminal-honest**, in the product's own colors — tmux-green over neutral terminal greys — with the
**pixel identity** carried by the green brand tile and a pixel display face. No gradients, no
glows, minimal radius. (v2's blue→magenta gradient system is dead; v3 stripped it back to the
green; v4 adds the pixel identity, the spec sheet, the try-it framing, a linked-projects scene
and the reconnect closer.)

The centerpiece is unchanged in spirit since v2: ONE thing on the page looks like a terminal — a
single high-fidelity **macOS Terminal window hosting the mmux TUI** — and inside it you watch the
**real Claude Code session, the real Codex banner, a real shell, a real vite server, and the real
native mmux git panel**, syntax-colored and legible. Below the tour, a second instance of the same
window is a **live, typeable sandbox**.

If two files must agree on a name (class, id, state field, scene key, token tone), it is defined
here and nowhere else.

---

## 0. Non-negotiables

- **Static only.** Plain HTML + CSS + vanilla JS. No framework, no bundler, no build step. Plain
  `<script defer>` + globals (NOT ES modules). Works over `file://`.
- **One external origin, exactly.** The self-hosted umami instance (`stats.marvinvr.ch`) is loaded
  from `index.html` and allow-listed in the CSP's `script-src` + `connect-src` (nginx.conf). It is
  analytics only — the site must render perfectly with it blocked or offline. **Nothing else** may
  be remote: no CDNs, no remote fonts, no remote images. The Departure Mono woff2 is self-hosted
  in `fonts/`.
- **NO ASCII-ART CHROME.** Borders, frames, cards, the terminal window are **real CSS** — never
  box-drawing characters used as site layout. Box-drawing appears ONLY as authentic *content*
  inside the terminal screen (the Codex banner box, the git boxes' ratatui look) — and in
  `banner.txt`, which is literally a text file for curl.
- **Panes show REAL, legible content.** The Claude scene is the real Claude Code welcome + session
  shape; Codex the real boxed banner; the shell a real zsh + cargo run; vite the real banner; the
  git panel mirrors `src/app/view/git.rs`. No placeholder/abstract content, ever (§8).
- **Install is ONE command:** `curl -fsSL https://mmux.org/install.sh | sh` (the hero). The
  `#install` section adds a Homebrew tab as the macOS alternative — no cargo, no from-source.
- **`prefers-reduced-motion`** fully honored (no scrub/typing/streaming; land on finished states).
- **Accessible:** landmarks, heading order, keyboard operable, visible focus, AA contrast,
  decorative bits `aria-hidden`, a visually-hidden live region for the sandbox.

---

## 1. Color philosophy — flat, terminal-honest

Two roles, both quiet:

1. **Brand** = tmux green over neutral dark greys. A muted sage (`--green`) for accents — the
   status bar, active-row edge, links, kickers, the cursor — plus the brand tile's brighter pixel
   greens (`#86efac / #4ade80 / #16a34a`) reserved for the *mark itself* (nav, hero, favicon,
   toast icon). No gradients, no glows, radius ≤ 4px outside the mac window. The user explicitly
   rejected gradient branding: **do not reintroduce it.**
2. **Terminal semantics** = de-neoned ANSI-ish colors, INSIDE the window only: green running/add,
   red attention/del, amber warn, blue info, magenta focus/ai, cyan section heads.

Backgrounds stay dark and neutral; most prose is grey. The page reads designed because of type,
spacing and one committed accent — not because of color quantity.

## 2. Design tokens (`:root` in styles.css — styles.css is normative if they drift)

Surfaces `--bg #0d0e10 · --bg-2 #131416 · --surface #17181b · --surface-2 #1d1f22 ·
--surface-3 #26282c`; borders `--border #2a2d31 · --border-2 #3a3e44 · --tui-border #4a4f56`
(the ratatui box-line grey). Ink `--text #d7d9dc · --text-2 #abafb5 · --muted #7c8087 ·
--faint #4f535a`. Accent `--green #6e9e72 · --green-bright #8bbf8f · --green-dim #4f7355`.
ANSI-ish `--red #cf8080 · --yellow #c6a96c · --blue #7ea7c9 · --magenta #a98cc4 · --cyan #6fa8ad`,
mapped to `--running/--attention/--warn/--info/--add/--del/--ai`. Syntax tones `--tok-*`.
Shape `--radius 4px · --radius-sm 3px`; one soft shadow, no glows. Scale `--page-max 1120px ·
--measure 60ch`. Breakpoints: 900px (git panel hides, single-column demo), 640px (phone nav,
stacked spec rows), 460px (narrower sidebar).

The **selection color `#2d2d3c`** (active sidebar row / git cursor row) looks off-palette on
purpose: it is the product's real selection color (`Rgb(45,45,60)` in `src/app/view/theme.rs`).
Authenticity beats palette purity inside the window.

## 3. Typography

- **Two voices.** Body + all terminal content: the system mono stack (`--font`). Identity +
  headings: **Departure Mono** (`--font-display`) — a self-hosted pixel mono (OFL, `fonts/`,
  preloaded in `<head>`) that matches the squared-off tile mark.
- `--font-display` applies to: `.brand`, `.hero-wordmark`, `.section-h2`, `.caption-title`,
  `.footer-brand`, `.tw-overlay`. It is **single-weight**: always `font-weight: 400`,
  `letter-spacing: 0` (faux-bold smears pixels). The terminal window NEVER uses it.
- Body: 0.98rem, line-height ~1.6–1.75, `--text-2`, prose capped at `--measure`.
- Kickers: small green mono `// comments` above each section.
- Nav: sticky, translucent dark + blur, 1px bottom border.

## 4. Page sections (in order)

### 4.1 `<header class="site-nav">`
`.brand` (the pixel tile SVG + `mmux`) · links: `the demo · what you get · try it · github`.
Phones keep only the brand + github.

### 4.2 `<section id="hero">`
The **`.hero-mark`** — the brand tile SVG at `clamp(60px, 8.5vw, 92px)`, `crispEdges` — then
kicker `a terminal multiplexer for AI agents`, the wordmark **mmux** (display face) + blinking
block caret, tagline **persistent terminals for your coding agents.**, the sub line, the single
install row (`$ curl -fsSL https://mmux.org/install.sh | sh` + copy) with a quiet backlink to the
`#install` section, three chips, and the scroll cue. Kept compact so the demo window's title bar
peeks above the fold on a ~900px-tall viewport.

### 4.3 `<section id="demo">` — the centerpiece
A tall scroll track (`min-height: 950vh`); a sticky stage pins the terminal window while captions
cross-fade beside it. **Ten scenes** (§7), weighted.

### 4.4 `<section id="features">` — the spec sheet
kicker `// what you get`, h2 **everything in one place.** NOT cards: a flat, man-page-style
`<dl class="spec">` (green term column + grey description, hairline rows) beside a
**`.yaml-card`** — a little "file" (name-tab chip `mmux.yaml` + hand-tokenized code) showing the
real per-project config: `name`, a `processes` entry (`cmd`/`args`/`autostart`), and
`linked-projects`, exactly per `docs/04-configuration.md`. `.spec-note` under it says agents live
globally in `~/.mmux/config.yaml` and links the schema docs. Five spec rows: persistent ·
processes · native git · notifications · one binary.

### 4.5 `<section id="how">` — try it (the live sandbox)
kicker `// try it`, h2 **go on, type into it.** The lede tells the visitor it's live. Then
`#tw-how`, a second `.tw` skeleton driven by the sandbox driver (§6.2), and the three
`.how-points` (one window / always there / anywhere).

### 4.6 `<section id="install">`
kicker `// get it`, h2 **install in one line.** A centered `.install-tabs` strip (`script` default
+ `Homebrew`) sits on its own line above the command and toggles which `.install-row[data-panel]`
shows — one command line at a time, reusing the copy chip (toggle in tui.js §11). Then `then, in any
project directory: mmux`, ghost buttons → github + the docs. Script default, Homebrew alternative;
no cargo.

### 4.7 `<footer class="site-footer">`
`mmux · GPLv3 · github · docs · built by marvinvr`.

---

## 5. The terminal window — DOM, chrome, state, rendering

A single reusable, high-fidelity component. **`index.html` ships the static skeleton**;
**`tui.js`'s `renderTUI(state)`** fills it. CSS styles exactly the classes below.

### 5.1 DOM skeleton (ids/classes are LAW)

```html
<div id="tw" class="tw" role="img" aria-label="a simulated mmux terminal session">
  <div class="tw-bar">                       <!-- macOS Terminal window chrome -->
    <span class="tw-lights" aria-hidden="true"><i></i><i></i><i></i></span>
    <span class="tw-titlebar-name">mmux</span>
  </div>
  <div class="tw-body">
    <!-- each region is its own box-drawing frame (ratatui Block); its title is cut
         into the top border by an absolutely-positioned .tw-region-title chip. -->
    <div class="tw-region tw-region--sidebar">
      <span class="tw-region-title tw-sidebar-title" aria-hidden="true">app</span>
      <div class="tw-sidebar" aria-hidden="true"><!-- JS: sections + rows --></div>
    </div>
    <div class="tw-region tw-region--main">
      <span class="tw-region-title tw-main-title" aria-hidden="true"></span>
      <div class="tw-main">
        <div class="tw-tab"></div>            <!-- hidden; the label lives on the border -->
        <div class="tw-screen"></div>
      </div>
    </div>
    <div class="tw-panel" hidden>            <!-- native git panel: the .git-box stack -->
      <div class="tw-panel-head"> git </div>
      <div class="tw-panel-screen"></div>
    </div>
  </div>
  <div class="tw-status"></div>
  <div class="tw-a11y-live" aria-live="polite" aria-atomic="true"></div>
  <div class="tw-toast" hidden></div>
  <div class="tw-overlay" hidden></div>
  <div class="tw-sandbox-hint" hidden></div>
</div>
```

Sidebar rows (JS-produced): `.sb-section > .sb-head` + `.sb-row` (`.sb-dot[data-status]`,
`.sb-name`, `.sb-sub`, `.sb-bell`), launchers as `.sb-row--launcher` (`.sb-plus` + name), active
row `.sb-row--active`. With linked projects, ONE project's rows render at a time and a
`.sb-switch` pager (`‹ name •∘ ›` — chevrons, active name, position dots) pins to the sidebar
bottom; inactive dots use `--border-2` so they stay visible.

### 5.2 Chrome details

The model is **a macOS Terminal window hosting a real TUI**: outer mac chrome (rounded 10px,
traffic lights that reveal `✕ – +` on hover, centered `mmux` title, one soft drop shadow); inside,
every region is its own box-drawing frame (1px `--tui-border`, title chip masking the top edge —
a ratatui `Block`) on ONE terminal background. Key rules:

- **Sidebar** ~190px; launchers come FIRST in each section (matching `src/app/nav.rs build_nav`);
  section heads cyan; active row = the product's `#2d2d3c` selection + a green cursor edge;
  launcher rows green.
- **Main pane** fills its box; the session label (` Claude — running `) is cut into the top
  border via `.tw-main-title`, exactly like `main_title` in `src/app/view/pane.rs`. Focused main
  pane border = magenta. The block cursor is a real CSS block.
- **Git panel** ~200px: no outer frame — three bordered `.git-box`es (Changes tree with
  `[✓]/[~]/[ ]` staging checkboxes and a magenta cursor bar, Branches, Commits) ARE the column,
  mirroring `src/app/view/git.rs`; the first box takes the slack; long lines clip with an
  ellipsis; the focused box is bordered magenta.
- **Status bar**: the tmux-green signature — green bar, dark text. Scroll demo: a per-scene hint;
  sandbox: real key hints.
- **Toast**: a faithful macOS notification banner (SF Pro, translucent blur, the mmux tile as app
  icon) floating top-right.
- **Overlay**: a scrim with one line set in the display face at 1.15rem — amber when
  `disconnected`, green when `reattached`.
- **Bare mode** (`state.bare`): scene 0's "before mmux" plain shell — all mmux chrome hidden, the
  main region borderless. **Boot** (`boot:true` scene): the chrome slides/fades in (`.tw--boot`).

### 5.3 `state` shape (contract between tui.js / scenes.js / renderTUI)

```js
state = {
  title, bare, status,
  multiProject, projects: [{ name, active }],   // pager built when >1
  sidebar: [ { kind:"AGENTS"|"TERMINAL"|"PROCESSES", rows: [   // launchers FIRST
      { id, launcher:true, name:"New Claude" },
      { id, name, sub?, status:"running"|"exited"|"stopped", active?, attention?, project? },
  ]}],
  main:  { program, title, lines:[Line], placeholder, cursor },
  panel: { visible, branch, sections: [ { title, active?, lines:[Line] } ] },
  focus: "sidebar"|"main"|"panel"|"sandbox",
  toast: { app, title, body, time? } | null,
  overlay: "disconnected"|"reattached" | null,
}
// Scene extras: `term` (scene 0's bare terminal — the reveal types `mmux` into its
// LAST line; earlier lines are kept scrollback), `weight` (scroll share, default 1),
// `boot:true` (replay the chrome pop), `type:{reconnect:true}` (the two-beat closer).
```

### 5.4 Line / token model

`Line` is `"plain string"` | `{ text, cls? }` | `{ tokens:[{t,c}], cls? }`; git tree rows carry
`gitNode` for the sandbox's clickable staging. Tones (`c`): `kw fn str num comment type path op
add del ok warn info ai dim prompt brand`, plus `claude` (warm-orange logo) and `codex` (teal).
Line `cls`:
- `art` — banner/box rows: tight 1.05 leading, `white-space: pre`, **clips with an
  ellipsis** when the pane is too narrow (one clean fold, not a scrollbar).
- `ln-add` / `ln-del` — diff wash + colored gutter; **never wraps** (`pre` + ellipsis — a wrapped
  wash reads as a glitch). `ln-cmd`, `ln-dim` as before.

`renderLine` appends a `.screen-cursor` block to the last line when `main.cursor`.

---

## 6. Drivers (in tui.js) — one renderer, two drivers

### 6.1 Scroll driver
rAF-throttled scroll→progress over `#demo`, weighted bands per scene (`buildBounds`),
IntersectionObserver-gated; captions cross-fade. Reveals:
- **Bare reveal** (scene 0): `typeIntoBare` types `mmux` char-by-char into the `term`'s last line
  (any lines above it are kept as scrollback) and rests — the takeover happens next scene.
- **Boot pop** (`boot:true`): replays `.tw--boot`.
- **Streaming**: Claude/Codex scenes reveal `main.lines` progressively (≤ ~900ms).
- **Reconnect** (`type:{reconnect:true}`, the closer): paints the scene's state under an amber
  `disconnected` overlay first, then after ~1.6s renders the authored state — the green
  `reattached — nothing lost` frame with every session intact. Reduced-motion and the static
  fallback land directly on the authored (reattached) state.
- Reduced-motion: no scrub; captions become a stacked list; the last scene renders statically.

### 6.2 Sandbox driver (`#tw-how` — live & typeable)
Unchanged contract: decorative `role="img"` until engaged, then `role="application"`; keys
trapped only while engaged; Esc/click-out releases. Clicking in lands in the open typeable pane.
Sidebar nav (`↑/k ↓/j`, Enter, `x`), spawning via the `+ New …` launchers only. Typeable panes:
zsh runs canned commands (`runCommand`: ls/pwd/echo/date/git/cargo/clear/help — and `mmux`
answers "you're already in mmux — this is it."); Claude/Codex run a **finite** worked turn (live
status tail with each agent's real spinner + tool events, dimmed composer while working, Esc
interrupts) and hand the prompt back. The git Changes box is a live tri-state tree (files +
folders stage/unstage on click; `assets/logo.png` is in it). Single project — no pager.

---

## 7. scenes.js contract — the 10 scenes

`window.MMUX_SCENES`, pure data. Captions terse, lowercase, confident (verbatim):

- 0 — `weight 1.7` — **it starts with one command.** / open any ordinary terminal, in any
  directory, and type mmux. one binary — nothing else to set up. *(a bare shell; the reveal
  types `mmux` and rests)*
- 1 — `weight 1.3, boot` — **everything in one window.** / agents you spawn, terminals you open,
  processes you watch — each a row; the focused one fills the pane.
- 2 — **spawn an agent.** / pick "+ New Claude" and the real Claude Code goes to work in its own
  pane, right beside everything else. *(the real welcome banner — model line trimmed to
  `Opus 4.8 (1M context) · Claude Max` so it fits even beside the git panel — then the real
  ⏺ Read/Update/Bash session with a diff and a test result)*
- 3 — **or codex. or whatever you run.** / claude and codex come configured out of the box — and
  any command-line agent is one line of yaml away. *(the real Codex box + session)*
- 4 — **a terminal when you need one.** / drop into a shell in the same window — your build, your
  scripts, your one-off commands. *(zsh + cargo run)*
- 5 — **every process in one place.** / your dev server, your tests, your watcher — start, stop
  and tail them all here, just like your agents. *(vite banner; PROCESSES rows)*
- 6 — **a git panel, pinned.** / a native git panel right where you work — changes, branches and
  history, following whichever project is active. *(the three boxes, cursor on a staged file)*
- 7 — **your whole workspace, one window.** / link related projects and flip between them — the
  sidebar, the git panel and the pane all follow whichever one is active. *(`multiProject`: `app`
  + `api`, pager at the sidebar bottom, api's Claude working in `~/dev/api`)*
- 8 — **it taps you on the shoulder.** / a bell becomes a dot — and a real desktop notification.
  even over ssh. *(attention bell + the macOS toast + Claude's real edit-approval prompt)*
- 9 — `weight 1.4, type:{reconnect:true}` — **it survives you.** / the whole thing lives in a
  per-directory tmux session. close the terminal, drop ssh, come back — everything's still
  running. *(two beats: amber `ssh disconnected — session kept alive`, then the authored state:
  green `reattached — nothing lost`, sessions intact — the payoff frame)*

The bottom bar shows a per-scene hint (`state.status`) — the scroll demo isn't interactive.

## 8. Realistic content rule (the "make it make sense" mandate)

A visitor scrolling the demo sees **recognizable software**. Agent banners are captured from the
real `claude` / `codex` binaries — don't invent them (trimming a too-wide line is allowed;
inventing content is not). The git panel, selection colors and sidebar order mirror the actual
source files named above. Reviewers reject abstract/placeholder content.

## 9. Motion

Scroll scene cross-fades; agent output streams; dots/bell pulse gently; the boot pop; the
two-beat reconnect. Everything gated by `prefers-reduced-motion` (then: static end states,
stacked captions, no pulses).

---

## 10. File manifest & wiring

```
web/
  index.html    # sections (§4) + both #tw skeletons + head: canonical, OG/Twitter,
                #   sitemap/llms alternates, schema.org JSON-LD, data: favicon,
                #   font preload, and the ONE external script (umami, deferred).
  styles.css    # the whole v4 system: @font-face (Departure Mono), tokens, every
                #   class in §4/§5. No remote url()/@import.
  scenes.js     # window.MMUX_SCENES (§7) — pure data + tiny builders (codexBox,
                #   gitSections).
  tui.js        # renderTUI/renderLine + scroll & sandbox drivers + copy + nav.
  fonts/        # DepartureMono-Regular.woff2 + its OFL LICENSE + README.
  banner.txt    # the plain-text card nginx serves to curl/wget/httpie on `/`.
  robots.txt / sitemap.xml / llms.txt / assets/ (og-image, icon)   # stable URLs.
                # og-image.png's editable source is ../assets/og-image.src.html —
                #   kept OUTSIDE web/ so it never ships; regen recipe in its comment.
  Dockerfile    # nginx:alpine; fingerprints css/js to <name>.<hash>.<ext> and
                #   rewrites index.html refs (1y immutable cache stays safe).
                #   fonts/, assets/ and the txt/xml files are copied verbatim.
  nginx.conf    # gzip + charset utf-8 + strict CSP (self + the umami origin in
                #   script-src/connect-src) + the curl→banner.txt rewrite on `/`
                #   + no-cache index.html + 1y immutable fingerprinted assets.
  DESIGN.md     # this file.  README.md — the web/ readme.
```

- Copy buttons: `button.copy[data-copy]` → clipboard + textarea fallback; flash `copied` ~1s.
- No inline JS handlers (CSP has no `unsafe-inline`). One global namespace `window.MMUX` +
  `window.MMUX_SCENES`.
- `node --check` must pass on scenes.js + tui.js; zero console errors (a blocked analytics
  request is not an error — but a CSP *violation* is, so keep nginx.conf's allow-list in sync
  with index.html).

## 11. Definition of done (reviewers check all)

- [ ] Looks like a **designed terminal-native site**: flat surfaces, sharp corners, tmux green,
      the pixel tile + Departure Mono carrying the identity. NO gradients, NO glows, no
      box-drawing as site structure.
- [ ] The terminal is a mac window hosting a faithful ratatui TUI; every pane shows real,
      syntax-colored content per §8 — including the linked-projects scene.
- [ ] Hero shows the one-line script install; the `#install` section tabs it (script default,
      Homebrew alternative). No cargo / from-source on the page.
- [ ] Scroll demo scrubs 10 scenes; the closer plays
      disconnect → reattach and rests on the green frame.
- [ ] Sandbox is click/keyboard-playable, focus-trapped only while engaged, escapable, announced
      to AT via the live region.
- [ ] The spec sheet reads like a man page; the yaml card matches the real schema
      (docs/04-configuration.md).
- [ ] reduced-motion: no scrub/stream/typing; static end states; stacked captions.
- [ ] Responsive: git panel hides <900px, single column <640px, no x-overflow; art rows clip
      with an ellipsis instead of scrolling or tearing.
- [ ] Landmarks, heading order, visible focus, AA contrast, decorative bits aria-hidden.
- [ ] Works over `file://` with zero network; the only remote request in production is umami,
      and the CSP allow-lists exactly that origin. `curl https://mmux.org/` returns banner.txt.
- [ ] `node --check` passes; no console errors; no globals beyond MMUX + MMUX_SCENES.
