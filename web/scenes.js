/* scenes.js — the data for the mmux.org scroll walkthrough (v3).
 *
 * One global: window.MMUX_SCENES — pure data (plus a couple of tiny content
 * builders so the box-drawing banners stay aligned). No logic, no console output.
 *
 * The state shape, the Line/token model, and the field names are the contract
 * defined in DESIGN.md §5.3 / §5.4 and consumed verbatim by tui.js's
 * renderTUI / renderLine. The standalone, always-playable "how it works" sandbox
 * (#tw-how) is seeded from tui.js's DEFAULT_STATE, not from a scene here.
 *
 * Every pane shows REAL, recognizable software (DESIGN.md §8): the actual Claude
 * Code welcome banner (its block-glyph logo, captured from `claude`), the actual
 * OpenAI Codex banner (captured from `codex`), a real zsh + cargo run, a real vite
 * banner, and the real native mmux git panel (Changes / Branches / Commits boxes).
 */

(function () {
  "use strict";

  /* ----- reusable content blocks (authored once, shared across scenes) ----- */

  // The real Claude Code welcome banner — its block-glyph logo (captured verbatim
  // from `claude`), then the version / model / cwd column. The logo column is
  // padded so the text lines up. `claude` tone = Claude's warm brand orange.
  // cls:"art" → tight leading so the three glyph rows tile into one mark instead
  // of tearing apart at the screen's 1.7 line-height.
  // (the model line is trimmed to fit the pane even with the git panel open —
  // an over-long art row clips with an ellipsis, which reads as a glitch here)
  var CLAUDE_BANNER = [
    { tokens: [{ t: " ▐▛███▜▌  ", c: "claude" }, { t: "Claude Code " }, { t: "v2.1.193", c: "dim" }], cls: "art" },
    { tokens: [{ t: "▝▜█████▛▘ ", c: "claude" }, { t: "Opus 4.8 (1M context) · Claude Max", c: "dim" }], cls: "art" },
    { tokens: [{ t: "  ▘▘ ▝▝   ", c: "claude" }, { t: "~/dev/app", c: "path" }], cls: "art" },
  ];

  // A real Claude Code session: the welcome banner, the prompt, a line of Claude
  // prose, then ⏺ tool calls each with their ⎿ result, a +/- diff, a closing line —
  // the exact shape Claude Code prints. (DESIGN.md §7 / §8.)
  var CLAUDE_LINES = CLAUDE_BANNER.concat([
    "",
    { tokens: [{ t: "> ", c: "dim" }, { t: "refactor auth to use the new TokenService" }] },
    "",
    { tokens: [{ t: "⏺ ", c: "ai" }, { t: "I'll route token creation through " }, { t: "TokenService", c: "type" }, { t: " and update the call site." }] },
    "",
    { tokens: [{ t: "⏺ ", c: "ai" }, { t: "Read", c: "fn" }, { t: "(", c: "dim" }, { t: "src/auth.rs", c: "path" }, { t: ")", c: "dim" }] },
    { tokens: [{ t: "  ⎿  ", c: "dim" }, { t: "Read 248 lines", c: "dim" }] },
    "",
    { tokens: [{ t: "⏺ ", c: "ai" }, { t: "Update", c: "fn" }, { t: "(", c: "dim" }, { t: "src/auth.rs", c: "path" }, { t: ")", c: "dim" }] },
    { tokens: [{ t: "  ⎿  ", c: "dim" }, { t: "Updated with 1 addition and 1 removal", c: "dim" }] },
    { text: "       -  let token = generate_token(user_id);", cls: "ln-del" },
    { text: "       +  let token = self.tokens.issue(user_id)?;", cls: "ln-add" },
    "",
    { tokens: [{ t: "⏺ ", c: "ai" }, { t: "Bash", c: "fn" }, { t: "(", c: "dim" }, { t: "cargo test auth", c: "dim" }, { t: ")", c: "dim" }] },
    { tokens: [{ t: "  ⎿  ", c: "dim" }, { t: "test result: " }, { t: "ok.", c: "ok" }, { t: " 12 passed; 0 failed", c: "dim" }] },
    "",
    { tokens: [{ t: "⏺ ", c: "ai" }, { t: "Done — auth now issues tokens through " }, { t: "TokenService", c: "type" }, { t: "." }] },
  ]);

  // Build the real Codex rounded banner box from row segments, padding each row so
  // the right border lines up. `rows` is an array of token-segment arrays.
  function codexBox(rows) {
    var W = 48;
    var dash = "─".repeat(W - 2);
    // cls:"art" → tight leading so the box borders (│ ╭ ╰) connect into one frame.
    var out = [{ tokens: [{ t: "╭" + dash + "╮", c: "dim" }], cls: "art" }];
    rows.forEach(function (segs) {
      var len = segs.reduce(function (a, s) { return a + (s.t || "").length; }, 0);
      var pad = Math.max(0, W - 4 - len);
      out.push({
        tokens: [{ t: "│ ", c: "dim" }].concat(segs).concat([{ t: " ".repeat(pad) + " │", c: "dim" }]),
        cls: "art",
      });
    });
    out.push({ tokens: [{ t: "╰" + dash + "╯", c: "dim" }], cls: "art" });
    return out;
  }

  // The real OpenAI Codex banner (captured verbatim from `codex`): the >_ mark,
  // the version, the model and directory. `codex` tone = Codex's teal accent.
  var CODEX_BANNER = codexBox([
    [{ t: ">_ ", c: "codex" }, { t: "OpenAI Codex " }, { t: "(v0.142.2)", c: "dim" }],
    [{ t: "" }],
    [{ t: "model:       ", c: "dim" }, { t: "gpt-5.5 high" }, { t: "   /model to change", c: "dim" }],
    [{ t: "directory:   ", c: "dim" }, { t: "~/dev/app", c: "path" }],
    [{ t: "permissions: ", c: "dim" }, { t: "YOLO mode", c: "warn" }],
  ]);

  // A real Codex session: the banner, a prompt (›), • action lines each with a └
  // result, a small diff, a closing summary — the shape the Codex CLI prints.
  var CODEX_LINES = CODEX_BANNER.concat([
    "",
    { tokens: [{ t: "› ", c: "codex" }, { t: "add a /health route and a test for it" }] },
    "",
    { tokens: [{ t: "• ", c: "codex" }, { t: "Read " }, { t: "src/routes.rs", c: "path" }] },
    { tokens: [{ t: "• ", c: "codex" }, { t: "Added " }, { t: "src/routes/health.rs", c: "path" }] },
    { text: "    +  pub async fn health() -> Json<Status> {", cls: "ln-add" },
    { text: "    +      Json(Status::ok())", cls: "ln-add" },
    { tokens: [{ t: "• ", c: "codex" }, { t: "Ran " }, { t: "cargo test", c: "dim" }] },
    { tokens: [{ t: "  └ ", c: "dim" }, { t: "ok", c: "ok" }, { t: " — 8 passed", c: "dim" }] },
    "",
    { tokens: [{ t: "• ", c: "codex" }, { t: "Added a " }, { t: "/health", c: "path" }, { t: " route returning " }, { t: "Status::ok()", c: "type" }, { t: " plus a test." }] },
  ]);

  // A real zsh session: prompt ❯, cargo run, Compiling/Finished/Running, the
  // server's "listening on" line. (DESIGN.md §7.)
  var ZSH_LINES = [
    { tokens: [{ t: "~/dev/app", c: "path" }, { t: "  on  " }, { t: "main", c: "ai" }] },
    { tokens: [{ t: "❯ ", c: "prompt" }, { t: "cargo run" }] },
    { tokens: [{ t: "   Compiling", c: "dim" }, { t: " app v0.2.0", c: "dim" }] },
    { tokens: [{ t: "    Finished", c: "ok" }, { t: " `dev` profile in 3.41s", c: "dim" }] },
    { tokens: [{ t: "     Running", c: "dim" }, { t: " `target/debug/app`", c: "dim" }] },
    { tokens: [{ t: "  ➜  ", c: "info" }, { t: "listening on " }, { t: "http://localhost:3000", c: "path" }] },
    { tokens: [{ t: "❯ ", c: "prompt" }, { t: "" }] },
  ];

  // A real vite dev server: the banner, Local/Network, the hmr update line.
  var VITE_LINES = [
    { tokens: [{ t: "  VITE v5.2.0", c: "brand" }, { t: "  ready in 412 ms", c: "ok" }] },
    "",
    { tokens: [{ t: "  ➜  ", c: "info" }, { t: "Local:    " }, { t: "http://localhost:5173/", c: "path" }] },
    { tokens: [{ t: "  ➜  ", c: "info" }, { t: "Network:  " }, { t: "http://192.168.1.4:5173/", c: "path" }] },
    { text: "  ➜  press h + enter to show help", cls: "ln-dim" },
    "",
    { tokens: [{ t: " 10:42:01 ", c: "dim" }, { t: "[vite]", c: "brand" }, { t: " hmr update " }, { t: "/src/App.tsx", c: "path" }] },
  ];

  // Claude paused awaiting input (attention scene) — the real edit-approval prompt
  // Claude Code shows below the diff it wants to apply.
  var CLAUDE_WAITING_LINES = CLAUDE_BANNER.concat([
    "",
    { tokens: [{ t: "> ", c: "dim" }, { t: "refactor auth to use the new TokenService" }] },
    "",
    { tokens: [{ t: "⏺ ", c: "ai" }, { t: "Update", c: "fn" }, { t: "(", c: "dim" }, { t: "src/auth.rs", c: "path" }, { t: ")", c: "dim" }] },
    { text: "       -  let token = generate_token(user_id);", cls: "ln-del" },
    { text: "       +  let token = self.tokens.issue(user_id)?;", cls: "ln-add" },
    "",
    { tokens: [{ t: "  Do you want to make this edit to ", c: "warn" }, { t: "src/auth.rs", c: "path" }, { t: "?", c: "warn" }] },
    { tokens: [{ t: "  ❯ ", c: "ai" }, { t: "1. Yes" }] },
    { tokens: [{ t: "    2. Yes, and don't ask again this session", c: "dim" }] },
    { tokens: [{ t: "    3. No, and tell Claude what to do differently", c: "dim" }] },
  ]);

  /* ----- the native git panel: three bordered boxes (DESIGN.md §5 / §9) -----
   * Mirrors src/app/view/git.rs: Changes (a file tree with [✓]/[~]/[ ] staging
   * checkboxes, names colored by change type, the cursor row on a magenta bar),
   * Branches (current ● in green), Commits (short hash + summary). */
  function gitSections(active) {
    return [
      {
        title: "Changes · main ↑1",
        active: active,
        lines: [
          { tokens: [{ t: " " }, { t: "[~]", c: "warn" }, { t: " src/", c: "info" }] },
          { tokens: [{ t: "▌", c: "ai" }, { t: "  [✓]", c: "ok" }, { t: " auth.rs", c: "warn" }], cls: "git-sel" },
          { tokens: [{ t: " " }, { t: "  [ ]", c: "dim" }, { t: " token.rs", c: "warn" }] },
          { tokens: [{ t: " " }, { t: "  [✓]", c: "ok" }, { t: " lib.rs", c: "ok" }] },
          { tokens: [{ t: " " }, { t: "[ ]", c: "dim" }, { t: " Cargo.toml", c: "warn" }] },
        ],
      },
      {
        title: "Branches",
        lines: [
          { tokens: [{ t: " ● ", c: "ok" }, { t: "main", c: "ok" }, { t: "   origin/main", c: "dim" }] },
          { tokens: [{ t: "   feat/tokens" }] },
        ],
      },
      {
        title: "Commits",
        lines: [
          { tokens: [{ t: "e2e6087 ", c: "dim" }, { t: "add token service" }] },
          { tokens: [{ t: "fce46df ", c: "dim" }, { t: "drag-select scrollback" }] },
          { tokens: [{ t: "a1b9c34 ", c: "dim" }, { t: "native git panel" }] },
        ],
      },
    ];
  }

  // A second project's Claude session (the workspace-manifest scene): same real
  // Claude Code shape, but running in ~/dev/api.
  var CLAUDE_API_LINES = [
    { tokens: [{ t: " ▐▛███▜▌  ", c: "claude" }, { t: "Claude Code " }, { t: "v2.1.193", c: "dim" }], cls: "art" },
    { tokens: [{ t: "▝▜█████▛▘ ", c: "claude" }, { t: "Opus 4.8 (1M context) · Claude Max", c: "dim" }], cls: "art" },
    { tokens: [{ t: "  ▘▘ ▝▝   ", c: "claude" }, { t: "~/dev/api", c: "path" }], cls: "art" },
    "",
    { tokens: [{ t: "> ", c: "dim" }, { t: "add rate limiting to the login route" }] },
    "",
    { tokens: [{ t: "⏺ ", c: "ai" }, { t: "I'll add a limiter middleware and wire it into " }, { t: "/login", c: "path" }, { t: "." }] },
    "",
    { tokens: [{ t: "⏺ ", c: "ai" }, { t: "Read", c: "fn" }, { t: "(", c: "dim" }, { t: "src/routes/login.rs", c: "path" }, { t: ")", c: "dim" }] },
    { tokens: [{ t: "  ⎿  ", c: "dim" }, { t: "Read 96 lines", c: "dim" }] },
    { tokens: [{ t: "⏺ ", c: "ai" }, { t: "Update", c: "fn" }, { t: "(", c: "dim" }, { t: "src/middleware.rs", c: "path" }, { t: ")", c: "dim" }] },
    { tokens: [{ t: "  ⎿  ", c: "dim" }, { t: "Updated with 14 additions", c: "dim" }] },
  ];

  /* --------------------------- sidebar rows -------------------------------- */
  // Launchers come FIRST in every section, matching the real sidebar order
  // (src/app/nav.rs build_nav: NewAgent → agents, NewTerminal → terminals, …).
  var L_CLAUDE = { id: "new-claude", launcher: true, name: "New Claude" };
  var L_CODEX = { id: "new-codex", launcher: true, name: "New Codex" };
  // Gemini is one of the built-in presets too (alongside Amp and opencode) — shown here
  // as a launcher so the demo hints that mmux runs whatever agent you configure, not just
  // Claude/Codex. Kept to one extra harness so the sidebar stays readable.
  var L_GEMINI = { id: "new-gemini", launcher: true, name: "New Gemini" };
  var L_TERMINAL = { id: "new-terminal", launcher: true, name: "New Terminal" };
  var L_PROCESS = { id: "new-process", launcher: true, name: "New Process" };

  // Fresh row objects (so per-scene `active`/`status` never bleed across scenes).
  function claude(o) {
    return Object.assign(
      { id: "claude", name: "Claude", sub: "refactoring auth", status: "running" },
      o || {}
    );
  }
  function codex(o) {
    return Object.assign(
      { id: "codex", name: "Codex", sub: "scaffolding /health", status: "running" },
      o || {}
    );
  }

  /* =====================================================================
   * The scenes. Captions terse, lowercase, confident; `status` is the
   * bottom-bar HINT (what's happening) — not key bindings, since the
   * scroll demo isn't interactive (the playable sandbox below it is).
   * ===================================================================== */
  window.MMUX_SCENES = [
    /* 0 — it starts with one command. A PLAIN terminal: someone types `mmux`, and
     * that's it. The mmux layout itself "pops up" in scene 1 (which boots in) — this
     * scene stays an ordinary shell so the before/after is unmistakable. Given extra
     * `weight` so it holds the stage long enough to read while scrolling down. ----- */
    {
      id: 0,
      weight: 1.7,
      caption: {
        kicker: "// one command",
        title: "it starts with one command.",
        body: "open any ordinary terminal, in any directory, and type mmux. one binary — nothing else to set up.",
      },
      // The reveal (tui.js typeIntoBare): type `mmux` into the bare terminal and
      // rest there. No takeover here — the layout appears in the next scene.
      type: { target: "main", text: "mmux" },
      term: {
        bare: true,
        title: "~/dev/app",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [],
        main: {
          program: "zsh",
          title: " zsh ",
          lines: [{ tokens: [{ t: "❯ ", c: "prompt" }, { t: "" }] }],
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", sections: [] },
        focus: "main",
        toast: null,
        overlay: null,
      },
      // reduced-motion / non-reveal resting frame: the same plain terminal, `mmux`
      // typed and waiting at the prompt.
      state: {
        bare: true,
        title: "~/dev/app",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [],
        main: {
          program: "zsh",
          title: " zsh ",
          lines: [{ tokens: [{ t: "❯ ", c: "prompt" }, { t: "mmux" }] }],
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", sections: [] },
        focus: "main",
        toast: null,
        overlay: null,
      },
    },

    /* 1 — …and the whole thing pops up. (boot:true → the chrome slides in) ---- */
    {
      id: 1,
      weight: 1.3,
      boot: true,
      caption: {
        kicker: "// your work",
        title: "everything in one window.",
        body: "agents you spawn, terminals you open, processes you watch — each a row; the focused one fills the pane.",
      },
      state: {
        title: "~/dev/app",
        status: "claude, codex, gemini and more come configured — spawn any",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [L_CLAUDE, L_CODEX, L_GEMINI, claude({ sub: "idle", status: "exited", active: true })],
          },
          { kind: "TERMINAL", rows: [L_TERMINAL, { id: "zsh", name: "zsh", status: "running" }] },
          { kind: "PROCESSES", rows: [L_PROCESS] },
        ],
        main: {
          program: null,
          title: " ",
          lines: [],
          placeholder: "Select a session on the left,\nor spawn one with + New Claude.",
          cursor: false,
        },
        panel: { visible: false, branch: "main", sections: [] },
        focus: "sidebar",
        toast: null,
        overlay: null,
      },
    },

    /* 2 — spawn an agent. (the real Claude Code welcome + session) --------- */
    {
      id: 2,
      caption: {
        kicker: "// agents",
        title: "spawn an agent.",
        body: 'pick "+ New Claude" and the real Claude Code goes to work in its own pane, right beside everything else.',
      },
      state: {
        title: "~/dev/app",
        status: "claude is refactoring auth in its own pane",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [L_CLAUDE, L_CODEX, L_GEMINI, claude({ status: "running", active: true })],
          },
          { kind: "TERMINAL", rows: [L_TERMINAL, { id: "zsh", name: "zsh", status: "running" }] },
          { kind: "PROCESSES", rows: [L_PROCESS] },
        ],
        main: {
          program: "claude",
          title: " Claude — running ",
          lines: CLAUDE_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", sections: [] },
        focus: "main",
        toast: null,
        overlay: null,
      },
    },

    /* 3 — or codex. or whatever you run. (the real Codex banner) ---------- */
    {
      id: 3,
      caption: {
        kicker: "// any agent",
        title: "or codex. or whatever you run.",
        body: "claude and codex come configured out of the box — and any command-line agent is one line of yaml away.",
      },
      state: {
        title: "~/dev/app",
        status: "codex running right beside claude — same window",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [
              L_CLAUDE,
              L_CODEX,
              L_GEMINI,
              claude({ status: "running" }),
              codex({ status: "running", active: true }),
            ],
          },
          { kind: "TERMINAL", rows: [L_TERMINAL, { id: "zsh", name: "zsh", status: "running" }] },
          { kind: "PROCESSES", rows: [L_PROCESS] },
        ],
        main: {
          program: "codex",
          title: " Codex — running ",
          lines: CODEX_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", sections: [] },
        focus: "main",
        toast: null,
        overlay: null,
      },
    },

    /* 4 — a terminal when you need one. (zsh + cargo run) ----------------- */
    {
      id: 4,
      caption: {
        kicker: "// terminals",
        title: "a terminal when you need one.",
        body: "drop into a shell in the same window — your build, your scripts, your one-off commands.",
      },
      state: {
        title: "~/dev/app",
        status: "a shell, in the same window as everything else",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [L_CLAUDE, L_CODEX, L_GEMINI, claude({ status: "running" }), codex({ status: "running" })],
          },
          {
            kind: "TERMINAL",
            rows: [L_TERMINAL, { id: "zsh", name: "zsh", status: "running", active: true }],
          },
          { kind: "PROCESSES", rows: [L_PROCESS] },
        ],
        main: {
          program: "zsh",
          title: " zsh — running ",
          lines: ZSH_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", sections: [] },
        focus: "main",
        toast: null,
        overlay: null,
      },
    },

    /* 5 — every process in one place. (dev server + tests + watcher) ------ */
    {
      id: 5,
      caption: {
        kicker: "// processes",
        title: "every process in one place.",
        body: "your dev server, your tests, your watcher — start, stop and tail them all here, just like your agents.",
      },
      state: {
        title: "~/dev/app",
        status: "dev server, tests and typecheck — all watched here",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [L_CLAUDE, L_CODEX, L_GEMINI, claude({ status: "running" }), codex({ status: "running" })],
          },
          {
            kind: "TERMINAL",
            rows: [L_TERMINAL, { id: "zsh", name: "zsh", status: "running" }],
          },
          {
            kind: "PROCESSES",
            rows: [
              L_PROCESS,
              { id: "dev-server", name: "dev server", sub: "vite · :5173", status: "running", active: true },
              { id: "tests", name: "tests", sub: "vitest · watch", status: "running" },
              { id: "typecheck", name: "typecheck", sub: "tsc --watch", status: "stopped" },
            ],
          },
        ],
        main: {
          program: "vite",
          title: " dev server — running ",
          lines: VITE_LINES,
          placeholder: null,
          cursor: false,
        },
        panel: { visible: false, branch: "main", sections: [] },
        focus: "main",
        toast: null,
        overlay: null,
      },
    },

    /* 6 — a git panel, pinned. (native git panel visible) ----------------- */
    {
      id: 6,
      caption: {
        kicker: "// the panel",
        title: "a git panel, pinned.",
        body: "a native git panel right where you work — changes, branches and history, following whichever project is active.",
      },
      state: {
        title: "~/dev/app",
        status: "native git panel — stage a file, a folder, or [a] for all",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [L_CLAUDE, L_CODEX, L_GEMINI, claude({ status: "running", active: true }), codex({ status: "running" })],
          },
          {
            kind: "TERMINAL",
            rows: [L_TERMINAL, { id: "zsh", name: "zsh", status: "running" }],
          },
          {
            kind: "PROCESSES",
            rows: [
              L_PROCESS,
              { id: "dev-server", name: "dev server", sub: "vite · :5173", status: "running" },
            ],
          },
        ],
        main: {
          program: "claude",
          title: " Claude — running ",
          lines: CLAUDE_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: true, branch: "main", sections: gitSections(true) },
        focus: "panel",
        toast: null,
        overlay: null,
      },
    },

    /* 7 — workspace manifest: one window, many projects. -- */
    {
      id: 7,
      caption: {
        kicker: "// workspaces",
        title: "your whole workspace, one window.",
        body: "bundle related projects in one manifest — the sidebar, git panel and pane all follow whichever project is active.",
      },
      state: {
        title: "~/dev/private",
        workspaceName: "private",
        status: "workspace manifest · [ and ] switch projects",
        multiProject: true,
        projects: [
          { name: "app", branch: "feature/auth", gitChanges: 3 },
          { name: "api", active: true, branch: "main", gitChanges: 1 },
        ],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [
              L_CLAUDE,
              L_CODEX,
              L_GEMINI,
              claude({ project: "app" }),
              { id: "claude-api", name: "Claude", sub: "rate limiting", status: "running", active: true, project: "api" },
            ],
          },
          {
            kind: "TERMINAL",
            rows: [
              L_TERMINAL,
              { id: "zsh", name: "zsh", status: "running", project: "app" },
              { id: "zsh-api", name: "zsh", status: "running", project: "api" },
            ],
          },
          {
            kind: "PROCESSES",
            rows: [
              L_PROCESS,
              { id: "dev-server", name: "dev server", sub: "vite · :5173", status: "running", project: "app" },
              { id: "api-server", name: "api", sub: "uvicorn · :8000", status: "running", project: "api" },
            ],
          },
        ],
        main: {
          program: "claude",
          title: " Claude — running ",
          lines: CLAUDE_API_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", sections: [] },
        focus: "main",
        toast: null,
        overlay: null,
      },
    },

    /* 8 — it taps you on the shoulder. (attention + toast) ---------------- */
    {
      id: 8,
      caption: {
        kicker: "// attention",
        title: "it taps you on the shoulder.",
        body: "a bell becomes a dot — and a real desktop notification. even over ssh.",
      },
      state: {
        title: "~/dev/app",
        status: "claude needs you — a dot, and a desktop notification",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [
              L_CLAUDE,
              L_CODEX,
              L_GEMINI,
              claude({ sub: "waiting for you", status: "running", active: true, attention: true }),
              codex({ status: "running" }),
            ],
          },
          {
            kind: "TERMINAL",
            rows: [L_TERMINAL, { id: "zsh", name: "zsh", status: "running" }],
          },
          {
            kind: "PROCESSES",
            rows: [
              L_PROCESS,
              { id: "dev-server", name: "dev server", sub: "vite · :5173", status: "running" },
            ],
          },
        ],
        main: {
          program: "claude",
          title: " Claude — waiting ",
          lines: CLAUDE_WAITING_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: true, branch: "main", sections: gitSections(false) },
        focus: "main",
        toast: {
          app: "mmux",
          title: "Claude needs your input",
          body: "approve the edit to src/auth.rs?",
        },
        overlay: null,
      },
    },

    /* 9 — it survives you. The closer, in two beats: the amber "ssh
     * disconnected" scrim first, then — the payoff — the green "reattached"
     * frame with every session still running (tui.js runReconnect). --------- */
    {
      id: 9,
      weight: 1.4,
      type: { reconnect: true },
      caption: {
        kicker: "// persistence",
        title: "it survives you.",
        body: "the whole thing lives in a per-directory tmux session. close the terminal, drop ssh, come back — everything's still running.",
      },
      state: {
        title: "~/dev/app",
        status: "reattached — every session still alive",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [L_CLAUDE, L_CODEX, L_GEMINI, claude({ status: "running", active: true }), codex({ status: "running" })],
          },
          {
            kind: "TERMINAL",
            rows: [L_TERMINAL, { id: "zsh", name: "zsh", status: "running" }],
          },
          {
            kind: "PROCESSES",
            rows: [
              L_PROCESS,
              { id: "dev-server", name: "dev server", sub: "vite · :5173", status: "running" },
            ],
          },
        ],
        main: {
          program: "claude",
          title: " Claude — running ",
          lines: CLAUDE_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", sections: [] },
        focus: "main",
        toast: null,
        overlay: "reattached",
      },
    },
  ];
})();
