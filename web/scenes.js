/* scenes.js — data for the mmux.org scroll walkthrough (§7).
 *
 * One global: window.MMUX_SCENES. Pure data, no logic. The scroll driver in
 * tui.js consumes each scene's `state` (shape per §5.3 / DEFAULT_STATE) and
 * `caption` / optional `type` reveal hint. Field names line up 1:1 with the
 * renderTUI contract — do not rename without updating tui.js.
 *
 * 9 scenes, ids 0..8:
 *   0 bare shell (typing `mmux`)        5 right git panel pinned
 *   1 boot: sidebar + launchers         6 attention: red dot + toast
 *   2 spawn claude + stream             7 linked projects regroup
 *   3 add terminal + process            8 finale (matches DEFAULT_STATE)
 *   4 detach / reattach overlay
 */
window.MMUX_SCENES = [
  /* ---- 0 — bare shell ------------------------------------------------- */
  {
    id: 0,
    caption: {
      title: "it starts as one command.",
      body: "mmux lives in your terminal — one binary, one directory.",
    },
    type: { target: "main", text: "mmux" },
    state: {
      multiProject: false,
      projects: [{ name: "app", active: true }],
      sidebar: [],
      main: {
        title: " mmux ",
        lines: [],
        placeholder: "$ ",
        cursor: true,
      },
      panel: { visible: false, branch: "main", lines: [] },
      focus: "main",
      toast: null,
      overlay: null,
    },
  },

  /* ---- 1 — boot: sidebar + launchers --------------------------------- */
  {
    id: 1,
    caption: {
      title: "a sidebar of things, one pane for the focused one.",
      body: "agents you spawn, terminals you open, processes you watch.",
    },
    state: {
      multiProject: false,
      projects: [{ name: "app", active: true }],
      sidebar: [
        {
          kind: "AGENTS",
          rows: [
            { id: "new-claude", launcher: true, name: "+ New Claude", selected: true },
          ],
        },
        {
          kind: "TERMINAL",
          rows: [
            { id: "new-terminal", launcher: true, name: "+ New Terminal" },
          ],
        },
        {
          kind: "PROCESSES",
          rows: [
            { id: "new-process", launcher: true, name: "+ New Process" },
          ],
        },
      ],
      main: {
        title: " mmux ",
        lines: [],
        placeholder: "Press Enter to launch a new Claude.",
        cursor: false,
      },
      panel: { visible: false, branch: "main", lines: [] },
      focus: "sidebar",
      toast: null,
      overlay: null,
    },
  },

  /* ---- 2 — spawn claude + stream ------------------------------------- */
  {
    id: 2,
    caption: {
      title: "spawn an agent on demand.",
      body: 'pick "+ New Claude", hit enter — it runs in its own pane.',
    },
    state: {
      multiProject: false,
      projects: [{ name: "app", active: true }],
      sidebar: [
        {
          kind: "AGENTS",
          rows: [
            {
              id: "claude",
              name: "claude",
              sub: "writing src/auth.rs",
              status: "running",
              selected: true,
              attention: false,
            },
            { id: "new-claude", launcher: true, name: "+ New Claude" },
          ],
        },
        {
          kind: "TERMINAL",
          rows: [
            { id: "new-terminal", launcher: true, name: "+ New Terminal" },
          ],
        },
        {
          kind: "PROCESSES",
          rows: [
            { id: "new-process", launcher: true, name: "+ New Process" },
          ],
        },
      ],
      main: {
        title: " claude — running ",
        lines: ["✓ wrote src/auth.rs", "✓ cargo build", "$ "],
        placeholder: null,
        cursor: true,
      },
      panel: { visible: false, branch: "main", lines: [] },
      focus: "main",
      toast: null,
      overlay: null,
    },
  },

  /* ---- 3 — add terminal + process ------------------------------------ */
  {
    id: 3,
    caption: {
      title: "everything in one list.",
      body: "a terminal here, a dev server there — all side by side.",
    },
    state: {
      multiProject: false,
      projects: [{ name: "app", active: true }],
      sidebar: [
        {
          kind: "AGENTS",
          rows: [
            {
              id: "claude",
              name: "claude",
              sub: "writing src/auth.rs",
              status: "running",
              selected: false,
              attention: false,
            },
            { id: "new-claude", launcher: true, name: "+ New Claude" },
          ],
        },
        {
          kind: "TERMINAL",
          rows: [
            {
              id: "zsh",
              name: "zsh",
              status: "running",
              selected: false,
            },
            { id: "new-terminal", launcher: true, name: "+ New Terminal" },
          ],
        },
        {
          kind: "PROCESSES",
          rows: [
            {
              id: "dev-server",
              glyph: "●",
              name: "dev server",
              sub: "listening :5173",
              status: "running",
              selected: true,
            },
            { id: "new-process", launcher: true, name: "+ New Process" },
          ],
        },
      ],
      main: {
        title: " dev server — running ",
        lines: ["⏵ vite dev", "  ready in 240ms", "  http://localhost:5173", "$ "],
        placeholder: null,
        cursor: true,
      },
      panel: { visible: false, branch: "main", lines: [] },
      focus: "main",
      toast: null,
      overlay: null,
    },
  },

  /* ---- 4 — detach / reattach overlay (persistence) ------------------- */
  {
    id: 4,
    caption: {
      title: "it doesn't die when you do.",
      body: "it lives in a per-directory tmux session. detach, drop ssh, come back — nothing lost.",
    },
    state: {
      multiProject: false,
      projects: [{ name: "app", active: true }],
      sidebar: [
        {
          kind: "AGENTS",
          rows: [
            {
              id: "claude",
              name: "claude",
              sub: "writing src/auth.rs",
              status: "running",
              selected: true,
              attention: false,
            },
            { id: "new-claude", launcher: true, name: "+ New Claude" },
          ],
        },
        {
          kind: "TERMINAL",
          rows: [
            {
              id: "zsh",
              name: "zsh",
              status: "running",
              selected: false,
            },
            { id: "new-terminal", launcher: true, name: "+ New Terminal" },
          ],
        },
        {
          kind: "PROCESSES",
          rows: [
            {
              id: "dev-server",
              glyph: "●",
              name: "dev server",
              sub: "listening :5173",
              status: "running",
              selected: false,
            },
            { id: "new-process", launcher: true, name: "+ New Process" },
          ],
        },
      ],
      main: {
        title: " claude — running ",
        lines: ["✓ wrote src/auth.rs", "✓ cargo build", "$ "],
        placeholder: null,
        cursor: true,
      },
      panel: { visible: false, branch: "main", lines: [] },
      focus: "sidebar",
      toast: null,
      overlay: "reattached",
    },
  },

  /* ---- 5 — right git panel pinned ------------------------------------ */
  {
    id: 5,
    caption: {
      title: "keep a panel pinned.",
      body: "lazygit beside your work, following whichever project is active.",
    },
    state: {
      multiProject: false,
      projects: [{ name: "app", active: true }],
      sidebar: [
        {
          kind: "AGENTS",
          rows: [
            {
              id: "claude",
              name: "claude",
              sub: "writing src/auth.rs",
              status: "running",
              selected: true,
              attention: false,
            },
            { id: "new-claude", launcher: true, name: "+ New Claude" },
          ],
        },
        {
          kind: "TERMINAL",
          rows: [
            {
              id: "zsh",
              name: "zsh",
              status: "running",
              selected: false,
            },
            { id: "new-terminal", launcher: true, name: "+ New Terminal" },
          ],
        },
        {
          kind: "PROCESSES",
          rows: [
            {
              id: "dev-server",
              glyph: "●",
              name: "dev server",
              sub: "listening :5173",
              status: "running",
              selected: false,
            },
            { id: "new-process", launcher: true, name: "+ New Process" },
          ],
        },
      ],
      main: {
        title: " claude — running ",
        lines: ["✓ wrote src/auth.rs", "✓ cargo build", "$ "],
        placeholder: null,
        cursor: true,
      },
      panel: {
        visible: true,
        branch: "main",
        lines: ["  modified  src/auth.rs", "  staged    src/lib.rs", "  staged    Cargo.toml"],
      },
      focus: "sidebar",
      toast: null,
      overlay: null,
    },
  },

  /* ---- 6 — attention: red dot + toast (the payoff) ------------------- */
  {
    id: 6,
    caption: {
      title: "when something needs you, you'll know.",
      body: "a bell becomes a dot — and a real desktop notification. even over ssh.",
    },
    state: {
      multiProject: false,
      projects: [{ name: "app", active: true }],
      sidebar: [
        {
          kind: "AGENTS",
          rows: [
            {
              id: "claude",
              name: "claude",
              sub: "done — needs review",
              status: "exited",
              selected: true,
              attention: true,
            },
            { id: "new-claude", launcher: true, name: "+ New Claude" },
          ],
        },
        {
          kind: "TERMINAL",
          rows: [
            {
              id: "zsh",
              name: "zsh",
              status: "running",
              selected: false,
            },
            { id: "new-terminal", launcher: true, name: "+ New Terminal" },
          ],
        },
        {
          kind: "PROCESSES",
          rows: [
            {
              id: "dev-server",
              glyph: "●",
              name: "dev server",
              sub: "listening :5173",
              status: "running",
              selected: false,
            },
            { id: "new-process", launcher: true, name: "+ New Process" },
          ],
        },
      ],
      main: {
        title: " claude — exited ",
        lines: ["✓ wrote src/auth.rs", "✓ cargo build", "✓ done — waiting on you", "$ "],
        placeholder: null,
        cursor: true,
      },
      panel: {
        visible: true,
        branch: "main",
        lines: ["  modified  src/auth.rs", "  staged    src/lib.rs", "  staged    Cargo.toml"],
      },
      focus: "sidebar",
      toast: { title: "claude", body: "finished in app — needs your review" },
      overlay: null,
    },
  },

  /* ---- 7 — linked projects regroup ----------------------------------- */
  {
    id: 7,
    caption: {
      title: "many clones, one sidebar.",
      body: "link sibling projects; each gets its own section.",
    },
    state: {
      multiProject: true,
      projects: [
        { name: "app", active: true },
        { name: "app-2", active: false },
      ],
      sidebar: [
        {
          kind: "AGENTS",
          rows: [
            {
              id: "claude",
              name: "claude",
              sub: "writing src/auth.rs",
              status: "running",
              selected: true,
              attention: false,
              project: "app",
            },
            {
              id: "claude-2",
              name: "claude",
              sub: "running tests",
              status: "running",
              selected: false,
              attention: false,
              project: "app-2",
            },
            { id: "new-claude", launcher: true, name: "+ New Claude" },
          ],
        },
        {
          kind: "TERMINAL",
          rows: [
            {
              id: "zsh",
              name: "zsh",
              status: "running",
              selected: false,
              project: "app",
            },
            { id: "new-terminal", launcher: true, name: "+ New Terminal" },
          ],
        },
        {
          kind: "PROCESSES",
          rows: [
            {
              id: "dev-server",
              glyph: "●",
              name: "dev server",
              sub: "listening :5173",
              status: "running",
              selected: false,
              project: "app",
            },
            { id: "new-process", launcher: true, name: "+ New Process" },
          ],
        },
      ],
      main: {
        title: " claude — running ",
        lines: ["✓ wrote src/auth.rs", "✓ cargo build", "$ "],
        placeholder: null,
        cursor: true,
      },
      panel: {
        visible: true,
        branch: "main",
        lines: ["  modified  src/auth.rs", "  staged    src/lib.rs", "  staged    Cargo.toml"],
      },
      focus: "sidebar",
      toast: null,
      overlay: null,
    },
  },

  /* ---- 8 — finale (matches DEFAULT_STATE; hand-off to sandbox) ------- */
  {
    id: 8,
    caption: {
      title: "your turn.",
      body: "↑↓ move · ⏎ open · x close. spawn an agent from a + New row.",
    },
    state: {
      multiProject: true,
      projects: [
        { name: "app", active: true },
        { name: "app-2", active: false },
      ],
      sidebar: [
        {
          kind: "AGENTS",
          rows: [
            {
              id: "claude",
              name: "claude",
              sub: "writing src/auth.rs",
              status: "running",
              selected: true,
              attention: false,
              project: "app",
            },
            { id: "new-claude", launcher: true, name: "+ New Claude" },
          ],
        },
        {
          kind: "TERMINAL",
          rows: [
            {
              id: "zsh",
              name: "zsh",
              status: "running",
              project: "app",
            },
            { id: "new-terminal", launcher: true, name: "+ New Terminal" },
          ],
        },
        {
          kind: "PROCESSES",
          rows: [
            {
              id: "dev-server",
              glyph: "●",
              name: "dev server",
              sub: "listening :5173",
              status: "running",
              project: "app",
            },
            { id: "new-process", launcher: true, name: "+ New Process" },
          ],
        },
      ],
      main: {
        title: " claude — running ",
        lines: ["✓ wrote src/auth.rs", "✓ cargo build", "$ "],
        placeholder: null,
        cursor: true,
      },
      panel: {
        visible: true,
        branch: "main",
        lines: ["  modified  src/auth.rs", "  staged    2 files"],
      },
      focus: "sidebar",
      toast: null,
      overlay: null,
    },
  },
];
