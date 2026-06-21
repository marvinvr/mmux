/* scenes.js — the data for the mmux.org scroll walkthrough (v2).
 *
 * One global: window.MMUX_SCENES — pure data, no logic, no console output.
 * Seven scenes (id 0..6). Each: { id, caption:{kicker?,title,body}, type?, state }.
 *
 * The state shape, the Line/token model, and the field names are the contract
 * defined in DESIGN.md §5.3 / §5.4 and consumed verbatim by tui.js's
 * renderTUI / renderLine. The standalone, always-playable "how it works" sandbox
 * (#tw-how) is seeded from tui.js's DEFAULT_STATE, not from a scene here.
 *
 * Every pane shows REAL, recognizable, syntax-colored software (DESIGN.md §8):
 * a real Claude Code session, a real zsh + cargo run, a real vite banner, and
 * real lazygit. Authored with the token model so the panes are colorful.
 */

(function () {
  "use strict";

  /* ----- reusable content blocks (authored once, shared across scenes) ----- */

  // A real Claude Code transcript: prompt, ● tool lines, a +/- diff, a test
  // result, a closing summary. (DESIGN.md §7 scene 2 / §8.)
  var CLAUDE_LINES = [
    { tokens: [{ t: "> ", c: "dim" }, { t: "refactor auth to use the new TokenService" }] },
    "",
    { tokens: [{ t: "●  ", c: "ai" }, { t: "Read", c: "fn" }, { t: "  src/auth.rs, src/token.rs", c: "path" }] },
    { tokens: [{ t: "●  ", c: "ai" }, { t: "Edit", c: "fn" }, { t: "  src/auth.rs", c: "path" }] },
    { text: "     -  let token = generate_token(user_id);", cls: "ln-del" },
    { text: "     +  let token = self.tokens.issue(user_id)?;", cls: "ln-add" },
    { tokens: [{ t: "●  ", c: "ai" }, { t: "Bash", c: "fn" }, { t: "  cargo test auth" }] },
    { tokens: [{ t: "     test result: " }, { t: "ok.", c: "ok" }, { t: " 12 passed; 0 failed", c: "dim" }] },
    { tokens: [{ t: "●  ", c: "ai" }, { t: "auth now delegates to TokenService. " }, { t: "✓", c: "ok" }] },
  ];

  // A real zsh session: prompt ❯, cargo run, Compiling/Finished/Running, the
  // server's "listening on" line. (DESIGN.md §7 scene 3.)
  var ZSH_LINES = [
    { tokens: [{ t: "~/dev/app", c: "path" }, { t: "  on  " }, { t: "main", c: "ai" }] },
    { tokens: [{ t: "❯ ", c: "prompt" }, { t: "cargo run" }] },
    { tokens: [{ t: "   Compiling", c: "dim" }, { t: " app v0.2.0", c: "dim" }] },
    { tokens: [{ t: "    Finished", c: "ok" }, { t: " `dev` profile in 3.41s", c: "dim" }] },
    { tokens: [{ t: "     Running", c: "dim" }, { t: " `target/debug/app`", c: "dim" }] },
    { tokens: [{ t: "  ➜  ", c: "info" }, { t: "listening on " }, { t: "http://localhost:3000", c: "path" }] },
    { tokens: [{ t: "❯ ", c: "prompt" }, { t: "" }] },
  ];

  // Real lazygit panel: Files (M/A), Branches (✓ main), Commits (hash + msg).
  // (DESIGN.md §7 scene 5 / §8.)
  var LAZYGIT_LINES = [
    { text: " Files", cls: "ln-dim" },
    { tokens: [{ t: "  M ", c: "warn" }, { t: "src/auth.rs" }] },
    { tokens: [{ t: "  M ", c: "warn" }, { t: "src/token.rs" }] },
    { tokens: [{ t: "  A ", c: "add" }, { t: "src/lib.rs" }] },
    { text: " Branches", cls: "ln-dim" },
    { tokens: [{ t: "  ✓ ", c: "ok" }, { t: "main" }] },
    { text: "    feat/tokens", cls: "ln-dim" },
    { text: " Commits", cls: "ln-dim" },
    { tokens: [{ t: "  e2e6087 ", c: "info" }, { t: "add token service" }] },
    { tokens: [{ t: "  fce46df ", c: "info" }, { t: "drag-select scrollback" }] },
  ];

  // Claude paused awaiting input (scene 6) — a real approval prompt.
  var CLAUDE_WAITING_LINES = [
    { tokens: [{ t: "> ", c: "dim" }, { t: "refactor auth to use the new TokenService" }] },
    "",
    { tokens: [{ t: "●  ", c: "ai" }, { t: "Read", c: "fn" }, { t: "  src/auth.rs, src/token.rs", c: "path" }] },
    { tokens: [{ t: "●  ", c: "ai" }, { t: "Edit", c: "fn" }, { t: "  src/auth.rs", c: "path" }] },
    { text: "     -  let token = generate_token(user_id);", cls: "ln-del" },
    { text: "     +  let token = self.tokens.issue(user_id)?;", cls: "ln-add" },
    "",
    { tokens: [{ t: "  Do you want to make this edit to ", c: "warn" }, { t: "src/auth.rs", c: "path" }, { t: "?", c: "warn" }] },
    { tokens: [{ t: "  ❯ ", c: "ai" }, { t: "1. Yes" }] },
    { tokens: [{ t: "    2. No, tell Claude what to do differently", c: "dim" }] },
  ];

  /* --------------------------- sidebar launchers --------------------------- */

  var L_CLAUDE = { id: "new-claude", launcher: true, name: "New Claude" };
  var L_TERMINAL = { id: "new-terminal", launcher: true, name: "New Terminal" };
  var L_PROCESS = { id: "new-process", launcher: true, name: "New Process" };

  /* =====================================================================
   * The 7 scenes.
   * ===================================================================== */
  window.MMUX_SCENES = [
    /* 0 — it's one command. ------------------------------------------------ */
    {
      id: 0,
      caption: {
        kicker: "// the demo",
        title: "it's one command.",
        body: "mmux runs in any terminal — one binary, one directory.",
      },
      type: { target: "main", text: "mmux" }, // typing reveal (tui.js §6.1)
      state: {
        title: "~/dev/app",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          { kind: "AGENTS", rows: [L_CLAUDE] },
          { kind: "TERMINAL", rows: [L_TERMINAL] },
          { kind: "PROCESSES", rows: [L_PROCESS] },
        ],
        main: {
          program: "zsh",
          title: " zsh ",
          lines: [{ tokens: [{ t: "❯ ", c: "prompt" }, { t: "mmux" }] }],
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", lines: [] },
        focus: "sidebar",
        toast: null,
        overlay: null,
      },
    },

    /* 1 — your work, in a sidebar. ---------------------------------------- */
    {
      id: 1,
      caption: {
        kicker: "// the sidebar",
        title: "your work, in a sidebar.",
        body: "agents you spawn, terminals you open, processes you watch — one pane for the focused one.",
      },
      state: {
        title: "~/dev/app",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [
              { id: "claude", name: "claude", sub: "idle", status: "exited", active: true },
              L_CLAUDE,
            ],
          },
          {
            kind: "TERMINAL",
            rows: [
              { id: "zsh", name: "zsh", status: "running" },
              L_TERMINAL,
            ],
          },
          { kind: "PROCESSES", rows: [L_PROCESS] },
        ],
        main: {
          program: null,
          title: " ",
          lines: [],
          placeholder: "Select a session on the left,\nor spawn one with + New Claude.",
          cursor: false,
        },
        panel: { visible: false, branch: "main", lines: [] },
        focus: "sidebar",
        toast: null,
        overlay: null,
      },
    },

    /* 2 — spawn an agent. (real Claude Code session, streamed) ------------- */
    {
      id: 2,
      caption: {
        kicker: "// agents",
        title: "spawn an agent.",
        body: 'pick "+ New Claude" and it goes to work in its own pane, right beside everything else.',
      },
      state: {
        title: "~/dev/app",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [
              {
                id: "claude",
                name: "claude",
                sub: "refactoring auth",
                status: "running",
                active: true,
                attention: false,
              },
              L_CLAUDE,
            ],
          },
          {
            kind: "TERMINAL",
            rows: [{ id: "zsh", name: "zsh", status: "running" }, L_TERMINAL],
          },
          { kind: "PROCESSES", rows: [L_PROCESS] },
        ],
        main: {
          program: "claude",
          title: " claude — running ",
          lines: CLAUDE_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", lines: [] },
        focus: "main",
        toast: null,
        overlay: null,
      },
    },

    /* 3 — terminals and processes too. (zsh + cargo run; dev server) ------ */
    {
      id: 3,
      caption: {
        kicker: "// terminals & processes",
        title: "terminals and processes too.",
        body: "a shell here, your dev server there — started, watched, never lost in tabs.",
      },
      state: {
        title: "~/dev/app",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [
              { id: "claude", name: "claude", sub: "refactoring auth", status: "running" },
              L_CLAUDE,
            ],
          },
          {
            kind: "TERMINAL",
            rows: [
              { id: "zsh", name: "zsh", status: "running", active: true },
              L_TERMINAL,
            ],
          },
          {
            kind: "PROCESSES",
            rows: [
              { id: "dev-server", name: "dev server", sub: "vite · :5173", status: "running" },
              L_PROCESS,
            ],
          },
        ],
        // §7 scene 3: main shows the zsh `cargo run` session; the dev-server
        // row is the vite process (sub "vite · :5173"). Its real vite banner —
        // "VITE v5.2.0 ready", Local/Network, the hmr line — is rendered by
        // tui.js (SPAWN.vite) when the process pane is focused in the sandbox.
        main: {
          program: "zsh",
          title: " zsh — running ",
          lines: ZSH_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", lines: [] },
        focus: "main",
        toast: null,
        overlay: null,
      },
    },

    /* 4 — it survives you. (ssh disconnect overlay over a live state) ----- */
    {
      id: 4,
      caption: {
        kicker: "// persistence",
        title: "it survives you.",
        body: "the whole thing lives in a per-directory tmux session. close the terminal, drop ssh, come back — nothing lost.",
      },
      state: {
        title: "~/dev/app",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [
              { id: "claude", name: "claude", sub: "refactoring auth", status: "running", active: true },
              L_CLAUDE,
            ],
          },
          {
            kind: "TERMINAL",
            rows: [{ id: "zsh", name: "zsh", status: "running" }, L_TERMINAL],
          },
          {
            kind: "PROCESSES",
            rows: [
              { id: "dev-server", name: "dev server", sub: "vite · :5173", status: "running" },
              L_PROCESS,
            ],
          },
        ],
        main: {
          program: "claude",
          title: " claude — running ",
          lines: CLAUDE_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: false, branch: "main", lines: [] },
        focus: "main",
        toast: null,
        overlay: "disconnected",
      },
    },

    /* 5 — keep a panel pinned. (lazygit panel visible) -------------------- */
    {
      id: 5,
      caption: {
        kicker: "// the panel",
        title: "keep a panel pinned.",
        body: "a built-in git panel, right where you work, following whichever project is active.",
      },
      state: {
        title: "~/dev/app",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [
              { id: "claude", name: "claude", sub: "refactoring auth", status: "running", active: true },
              L_CLAUDE,
            ],
          },
          {
            kind: "TERMINAL",
            rows: [{ id: "zsh", name: "zsh", status: "running" }, L_TERMINAL],
          },
          {
            kind: "PROCESSES",
            rows: [
              { id: "dev-server", name: "dev server", sub: "vite · :5173", status: "running" },
              L_PROCESS,
            ],
          },
        ],
        main: {
          program: "claude",
          title: " claude — running ",
          lines: CLAUDE_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: true, branch: "main", lines: LAZYGIT_LINES },
        focus: "main",
        toast: null,
        overlay: null,
      },
    },

    /* 6 — it taps you on the shoulder. (attention + toast) ---------------- */
    {
      id: 6,
      caption: {
        kicker: "// attention",
        title: "it taps you on the shoulder.",
        body: "a bell becomes a dot — and a real desktop notification. even over ssh.",
      },
      state: {
        title: "~/dev/app",
        multiProject: false,
        projects: [{ name: "app", active: true }],
        sidebar: [
          {
            kind: "AGENTS",
            rows: [
              {
                id: "claude",
                name: "claude",
                sub: "waiting for you",
                status: "running",
                active: true,
                attention: true,
              },
              L_CLAUDE,
            ],
          },
          {
            kind: "TERMINAL",
            rows: [{ id: "zsh", name: "zsh", status: "running" }, L_TERMINAL],
          },
          {
            kind: "PROCESSES",
            rows: [
              { id: "dev-server", name: "dev server", sub: "vite · :5173", status: "running" },
              L_PROCESS,
            ],
          },
        ],
        main: {
          program: "claude",
          title: " claude — waiting ",
          lines: CLAUDE_WAITING_LINES,
          placeholder: null,
          cursor: true,
        },
        panel: { visible: true, branch: "main", lines: LAZYGIT_LINES },
        focus: "main",
        toast: {
          app: "claude",
          title: "needs your input",
          body: "approve the edit to src/auth.rs?",
        },
        overlay: null,
      },
    },
  ];
})();
