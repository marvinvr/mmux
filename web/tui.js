/* tui.js — the coupled core of mmux.org (v2).
 *
 * One global: window.MMUX. May read window.MMUX_SCENES (from scenes.js).
 * Responsibilities:
 *   1. renderTUI(state)  — idempotent DOM updater over the #tw skeleton (§5.3).
 *   2. renderLine(line)  — the realistic-content / token renderer (§5.4).
 *   3. scroll driver     — scrub window.MMUX_SCENES across the tall #demo (§6.1).
 *   4. sandbox driver    — make the standalone #tw-how terminal playable (§6.2).
 *   Plus: copy buttons + smooth-scroll nav (§11).
 *
 * No modules/imports (must work over file://). Everything guards: missing scenes
 * or missing elements must not throw — zero console errors is a done-list item (§12).
 */
(function () {
  "use strict";

  /* =====================================================================
   * state shape (§5.3) — the contract between tui.js / scenes.js / renderTUI
   * ---------------------------------------------------------------------
   * state = {
   *   title: "~/dev/app",                  // path shown in the title bar
   *   bare: bool,                          // true → plain terminal (mmux chrome hidden)
   *   status: str,                         // bottom-bar hint; falls back to STATUS[focus]
   *   multiProject: bool,
   *   projects: [{ name, active, branch?, gitChanges? }],
   *   workspaceName?: str,                 // manifest/session name (sidebar title)
   *   sidebar: [ { kind:"AGENTS"|"TERMINAL"|"PROCESSES", rows: [    // launchers FIRST
   *       { id, launcher:true, name:"New Claude" }, // launcher → "+ New Claude"
   *       { id, name, sub?, status:"running"|"exited"|"stopped",
   *         active?, attention?, project? },        // session row
   *   ]}],
   *   main: { program:"claude"|"codex"|"zsh"|"vite"|null, title, lines:[Line],
   *           placeholder:str|null, cursor:bool },
   *   panel: { visible, branch, sections:[{ title, active?, lines:[Line] }] },
   *   focus: "sidebar"|"main"|"panel"|"sandbox",
   *   toast: { app, title, body, time? } | null,   // macOS notification; time defaults to "now"
   *   overlay: "disconnected"|"reattached" | null,
   * }
   *
   * Line (§5.4) is one of:
   *   "plain string"
   *   { text, cls? }                          cls: ln-add|ln-del|ln-cmd|ln-dim
   *   { tokens:[{t,c}], cls? }                c (tone): kw fn str num comment type
   *                                           path op add del ok warn info ai dim
   *                                           prompt brand
   * ===================================================================== */

  /* DEFAULT_STATE — a realistic, populated finale (§5.3 / §8). Used when
   * window.MMUX_SCENES is absent (degrade to a static, playable finale). scenes.js
   * authors the finale scene to mirror this shape, so the field names here are the
   * stable contract. The content uses the token model so the pane looks real. */
  var DEFAULT_STATE = {
    title: "~/dev/app",
    multiProject: false,
    projects: [{ name: "app", active: true }],
    // Launchers come FIRST in every section, matching the real sidebar order
    // (src/app/nav.rs build_nav). Claude, Codex and Gemini are configured agents here
    // (Amp and opencode ship as presets too — see `mmux agents`).
    sidebar: [
      {
        kind: "AGENTS",
        rows: [
          { id: "new-claude", launcher: true, name: "New Claude" },
          { id: "new-codex", launcher: true, name: "New Codex" },
          { id: "new-gemini", launcher: true, name: "New Gemini" },
          {
            id: "claude",
            name: "Claude",
            sub: "refactoring auth",
            status: "running",
            active: true,
            attention: false,
          },
          {
            id: "codex",
            name: "Codex",
            sub: "running tests",
            status: "running",
          },
        ],
      },
      {
        kind: "TERMINAL",
        rows: [
          { id: "new-terminal", launcher: true, name: "New Terminal" },
          { id: "zsh", name: "zsh", status: "running" },
        ],
      },
      {
        kind: "PROCESSES",
        rows: [
          { id: "new-process", launcher: true, name: "New Process" },
          {
            id: "dev-server",
            name: "dev server",
            sub: "vite · :5173",
            status: "running",
          },
        ],
      },
    ],
    // A ready-for-input Claude preview. Clicking the row makes it live (you type a
    // prompt; Claude then "works" — see the sandbox driver's freshPane/startWorking).
    main: {
      program: "claude",
      title: " Claude — ready ",
      lines: [
        { tokens: [{ t: " ▐▛███▜▌  ", c: "claude" }, { t: "Claude Code " }, { t: "v2.1.193", c: "dim" }], cls: "art" },
        { tokens: [{ t: "▝▜█████▛▘ ", c: "claude" }, { t: "Opus 4.8 (1M context) · Claude Max", c: "dim" }], cls: "art" },
        { tokens: [{ t: "  ▘▘ ▝▝   ", c: "claude" }, { t: "~/dev/app", c: "path" }], cls: "art" },
        "",
        { tokens: [{ t: "  Ask Claude to do something — type a prompt and press enter.", c: "dim" }] },
        "",
        { tokens: [{ t: "❯ ", c: "prompt" }, { t: "" }] },
      ],
      placeholder: null,
      cursor: true,
    },
    // The native mmux git panel: three bordered boxes (see src/app/view/git.rs).
    panel: {
      visible: true,
      branch: "main",
      sections: [
        {
          title: "Changes · main ↑1",
          active: true,
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
      ],
    },
    focus: "sidebar",
    toast: null,
    overlay: null,
  };

  /* Footer hint strings, keyed by focus (§7). The renderer picks one. */
  var STATUS = {
    sidebar: "↑↓ move   ⏎ open   x close   r restart   d detach",
    main: "keys → pane   drag = copy   ⌃b   h back   x close",
    panel: "keys → pane   drag = copy   ⌃b   h back   x close",
    sandbox: "click a row · ↑↓ move · ⏎ open · x close · esc to leave",
  };

  var OVERLAY_TEXT = {
    disconnected: "ssh disconnected — session kept alive",
    reattached: "reattached — nothing lost",
  };

  /* ---- tiny DOM helpers (guarded) ---- */
  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }
  function $(sel, root) {
    return (root || document).querySelector(sel);
  }

  // Cache each terminal's sub-elements by root id; re-resolve lazily so a missing
  // skeleton is harmless. Two terminals exist: #tw (the scrubbed demo) and #tw-how
  // (the standalone, always-playable sandbox in the "how it works" section).
  var TW_CACHE = {};
  function twRefs(id) {
    id = id || "tw";
    var cached = TW_CACHE[id];
    if (cached && document.body.contains(cached.root)) return cached;
    var root = document.getElementById(id);
    if (!root) return null;
    var refs = {
      root: root,
      sidebar: $(".tw-sidebar", root),
      sidebarTitle: $(".tw-sidebar-title", root), // the project name cut into the sidebar box
      mainTitle: $(".tw-main-title", root),        // " Claude — running " cut into the main box
      tab: $(".tw-tab", root),
      screen: $(".tw-screen", root),
      panel: $(".tw-panel", root),
      panelHead: $(".tw-panel-head", root),
      panelScreen: $(".tw-panel-screen", root),
      status: $(".tw-status", root),
      toast: $(".tw-toast", root),
      overlay: $(".tw-overlay", root),
      sandboxHint: $(".tw-sandbox-hint", root),
    };
    TW_CACHE[id] = refs;
    return refs;
  }

  /* =====================================================================
   * 2. renderLine(line) — the realistic-content renderer (§5.4).
   * Returns a .screen-line div (+ optional cls). Whitespace preserved by CSS
   * (white-space: pre-wrap). Never throws on malformed input.
   * ===================================================================== */
  function renderLine(line) {
    var div = el("div", "screen-line");
    if (line == null) {
      return div; // blank line
    }

    // plain string
    if (typeof line === "string") {
      div.appendChild(document.createTextNode(line));
      return div;
    }

    if (line.cls) {
      div.className = "screen-line " + line.cls;
    }

    // token list → one span.tok-<c> per token
    if (Array.isArray(line.tokens)) {
      line.tokens.forEach(function (tok) {
        if (!tok) return;
        var t = tok.t != null ? tok.t : "";
        if (tok.c) {
          var span = el("span", "tok-" + tok.c);
          span.appendChild(document.createTextNode(t));
          div.appendChild(span);
        } else {
          div.appendChild(document.createTextNode(t));
        }
      });
      return div;
    }

    // { text, cls }
    if (line.text != null) {
      div.appendChild(document.createTextNode(line.text));
      return div;
    }

    return div;
  }

  /* =====================================================================
   * 1. renderTUI(state) — idempotent DOM updater. Does NOT animate (§5.3).
   * ===================================================================== */
  function renderTUI(state, id) {
    var t = twRefs(id);
    if (!t || !state) return;

    renderBar(t, state);
    renderSidebar(t.sidebar, state);
    // A manifest keeps its own workspace identity; a plain session uses the
    // active/single project's name.
    if (t.sidebarTitle) {
      var projs = state.projects || [];
      var act = projs.filter(function (p) { return p.active; })[0] || projs[0];
      t.sidebarTitle.textContent = state.workspaceName || (act && act.name) || "app";
    }
    renderMain(t, state.main || {});
    renderPanel(t, state.panel || {});
    renderStatus(t.status, state.focus, state.status);
    renderToast(t.toast, state.toast);
    renderOverlay(t.overlay, state.overlay);

    // Bare mode: a plain terminal with the mmux chrome (sidebar, panel, status,
    // tab, titlebar name/meta) hidden — used for scene 0's "before mmux" terminal.
    t.root.classList.toggle("tw--bare", !!state.bare);

    // Reflect engaged-focus on the root so CSS can paint pane accents.
    t.root.classList.toggle("tw--main-focus", state.focus === "main");
    t.root.classList.toggle("tw--panel-focus", state.focus === "panel");
    t.root.classList.toggle(
      "tw--sidebar-focus",
      state.focus === "sidebar" || state.focus === "sandbox"
    );
  }

  // The window bar is static macOS-Terminal chrome — traffic lights + a centered
  // "mmux" — so there's nothing data-driven to update here. Kept for renderTUI's
  // call shape (and any future per-scene title).
  function renderBar() {}

  function renderSidebar(host, state) {
    if (!host) return;
    host.textContent = ""; // rebuild wholesale: small DOM, simpler than diffing.

    // A manifest workspace stacks one compact box per member, matching the real
    // TUI. Plain sessions keep the original single set of sections.
    var projects =
      state.multiProject && Array.isArray(state.projects) && state.projects.length
        ? state.projects
        : null;
    if (projects) {
      var sections = state.sidebar || [];
      var ordered = projects.filter(function (project) {
        return projectAgentRows(sections, project.name).length > 0;
      }).concat(projects.filter(function (project) {
        return projectAgentRows(sections, project.name).length === 0;
      }));
      ordered.sort(function (a, b) {
        var aHasAgents = projectAgentRows(sections, a.name).length > 0;
        var bHasAgents = projectAgentRows(sections, b.name).length > 0;
        if (aHasAgents !== bHasAgents) return aHasAgents ? -1 : 1;
        return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
      });
      ordered.forEach(function (project) {
        var box = el("div", "sb-project" + (project.active ? " sb-project--active" : ""));
        box.appendChild(el("div", "sb-project-name", project.name));
        if (project.active) {
          sections.forEach(function (section) {
            appendSection(box, section, project.name);
          });
        } else {
          box.appendChild(buildProjectSummary(sections, project));
        }
        host.appendChild(box);
      });
    } else {
      (state.sidebar || []).forEach(function (section) {
        appendSection(host, section, null);
      });
    }
  }

  // The real TUI gives an inactive project a compact agent row (plus a git row
  // for repositories) instead of clipping its first AGENTS heading into view.
  function projectAgentRows(sections, projectName) {
    var rows = [];
    sections.forEach(function (section) {
      if (section.kind !== "AGENTS") return;
      (section.rows || []).forEach(function (row) {
        if (!row.launcher && row.project === projectName) rows.push(row);
      });
    });
    return rows;
  }

  function buildProjectSummary(sections, project) {
    var agents = projectAgentRows(sections, project.name);
    var working = agents.filter(function (row) {
      return row.status === "running" && !row.attention;
    }).length;
    var ready = agents.filter(function (row) {
      return row.status === "running" && row.attention;
    }).length;
    var failed = agents.filter(function (row) {
      return row.status === "failed" || row.status === "crashed";
    }).length;
    var collapsed = el("div", "sb-project-collapsed");
    if (working || ready || failed) {
      var summary = el("div", "sb-project-summary");
      if (working) summary.appendChild(el("span", "sb-project-working", "⠲ " + working + " working"));
      if (ready) summary.appendChild(el("span", "sb-project-ready", "● " + ready + " ready"));
      if (failed) summary.appendChild(el("span", "sb-project-failed", "○ " + failed + " failed"));
      collapsed.appendChild(summary);
    }
    if (project.gitChanges != null) {
      var gitRow = el("div", "sb-project-git-row");
      gitRow.appendChild(el("span", "sb-project-branch", project.branch || "HEAD"));
      var git = project.gitChanges ? "git ±" + project.gitChanges : "git ✓";
      gitRow.appendChild(el("span", project.gitChanges ? "sb-project-git sb-project-git--dirty" : "sb-project-git", git));
      collapsed.appendChild(gitRow);
    }
    return collapsed;
  }

  // Append one section; when `projectName` is set, only rows for that project
  // (launchers belong to every project block).
  function appendSection(host, section, projectName) {
    var rows = (section.rows || []).filter(function (r) {
      if (projectName == null || r.launcher) return true;
      return r.project == null || r.project === projectName;
    });
    if (!rows.length) return;

    var wrap = el("div", "sb-section");
    wrap.appendChild(el("div", "sb-head", section.kind));
    rows.forEach(function (r) {
      wrap.appendChild(buildRow(r));
    });
    host.appendChild(wrap);
  }

  function buildRow(r) {
    if (r.launcher) {
      var lr = el("div", "sb-row sb-row--launcher" + (r.active ? " sb-row--active" : ""));
      if (r.id != null) lr.setAttribute("data-id", r.id);
      var plus = el("span", "sb-plus", "+");
      plus.setAttribute("aria-hidden", "true");
      lr.appendChild(plus);
      // accept either "New Claude" or "+ New Claude" — normalise to "New Claude"
      var nm = (r.name || "").replace(/^\s*\+\s*/, "");
      lr.appendChild(el("span", "sb-name", nm));
      return lr;
    }

    var classes = "sb-row";
    if (r.active) classes += " sb-row--active";
    var row = el("div", classes);
    if (r.id != null) row.setAttribute("data-id", r.id);

    // colored status dot — data-status drives the color in CSS
    var dot = el("span", "sb-dot");
    dot.setAttribute("data-status", r.status || "stopped");
    dot.setAttribute("aria-hidden", "true");
    row.appendChild(dot);

    row.appendChild(el("span", "sb-name", r.name || ""));

    if (r.sub) row.appendChild(el("span", "sb-sub", r.sub));

    // attention bell ● (coral); hidden unless attention is set
    var bell = el("span", "sb-bell", "●");
    bell.setAttribute("aria-hidden", "true");
    if (!r.attention) bell.hidden = true;
    row.appendChild(bell);

    return row;
  }

  function renderMain(t, main) {
    // The pane is labelled the way the real mmux labels it — the session name +
    // status cut into the box's top border (main_title in src/app/view/pane.rs),
    // NOT a separate tab strip. So the title goes on the border chip and the old
    // tab element stays empty/hidden (kept only for layout stability).
    if (t.tab) {
      t.tab.textContent = "";
      t.tab.hidden = true;
    }
    if (t.mainTitle) {
      var title = (main.title || "").trim();
      t.mainTitle.textContent = title;
      t.mainTitle.hidden = !title; // an empty pane shows an unbroken top border
    }
    if (!t.screen) return;

    t.screen.textContent = "";

    // placeholder beats lines (§5.3)
    if (main.placeholder) {
      var ph = el("div", "screen-placeholder", main.placeholder);
      t.screen.appendChild(ph);
      return;
    }

    var lines = main.lines || [];
    var lastEl = null;
    lines.forEach(function (line) {
      lastEl = renderLine(line);
      t.screen.appendChild(lastEl);
    });

    // block cursor ▮ appended to the last line when cursor is set (§5.4)
    if (main.cursor) {
      if (!lastEl) {
        lastEl = el("div", "screen-line");
        t.screen.appendChild(lastEl);
      }
      var cur = el("span", "screen-cursor", "▮");
      cur.setAttribute("aria-hidden", "true");
      lastEl.appendChild(cur);
    }

    // keep the newest line in view (a live terminal scrolls to the bottom)
    if (typeof t.screen.scrollTop === "number") {
      t.screen.scrollTop = t.screen.scrollHeight;
    }
  }

  // The native git panel: a column of bordered boxes (Changes / Branches / Commits),
  // matching src/app/view/git.rs. Legacy `panel.lines` still renders as a flat list.
  function renderPanel(t, panel) {
    if (!t.panel) return;
    var visible = !!panel.visible;
    t.panel.hidden = !visible;
    if (!visible) return;

    var sections = Array.isArray(panel.sections) ? panel.sections : null;
    if (t.panelHead) {
      // The boxed panel carries its branch in the Changes box title; the old " git "
      // chip only shows for the legacy flat-list shape.
      t.panelHead.hidden = !!sections;
      if (!sections) t.panelHead.textContent = panel.branch ? " " + panel.branch + " " : " git ";
    }
    if (!t.panelScreen) return;
    t.panelScreen.textContent = "";

    if (sections) {
      sections.forEach(function (sec) {
        var box = el("div", "git-box" + (sec.active ? " git-box--active" : ""));
        box.appendChild(el("div", "git-box-title", sec.title || ""));
        var body = el("div", "git-box-body");
        (sec.lines || []).forEach(function (line) {
          var ld = renderLine(line);
          // a stageable tree node (file or folder) → clickable hook (sandbox)
          if (line && line.gitNode) {
            ld.classList.add("git-row");
            ld.setAttribute("data-git-node", line.gitNode);
          }
          body.appendChild(ld);
        });
        box.appendChild(body);
        t.panelScreen.appendChild(box);
      });
      return;
    }

    (panel.lines || []).forEach(function (line) {
      t.panelScreen.appendChild(renderLine(line));
    });
  }

  // The bottom bar: an explicit per-state hint (the scroll demo, which isn't
  // interactive) wins; otherwise the focus-keyed key hints (the playable sandbox).
  function renderStatus(host, focus, hint) {
    if (!host) return;
    host.textContent = hint || STATUS[focus] || STATUS.sidebar;
  }

  function renderToast(host, toast) {
    if (!host) return;
    if (!toast) {
      host.hidden = true;
      host.textContent = "";
      return;
    }
    host.hidden = false;
    host.textContent = "";

    // macOS notification layout: app icon, then app name + time, bold title, body.
    var icon = el("div", "toast-icon");
    icon.setAttribute("aria-hidden", "true");
    host.appendChild(icon);

    var main = el("div", "toast-main");
    var head = el("div", "toast-head");
    head.appendChild(el("span", "toast-app", toast.app || ""));
    head.appendChild(el("span", "toast-time", toast.time || "now"));
    main.appendChild(head);
    if (toast.title) main.appendChild(el("div", "toast-title", toast.title));
    if (toast.body) main.appendChild(el("div", "toast-body", toast.body));
    host.appendChild(main);
  }

  function renderOverlay(host, overlay) {
    if (!host) return;
    if (!overlay) {
      host.hidden = true;
      host.textContent = "";
      return;
    }
    host.hidden = false;
    host.textContent = OVERLAY_TEXT[overlay] || overlay;
    host.setAttribute("data-overlay", overlay);
  }

  /* =====================================================================
   * Reduced-motion check (live; users can toggle it).
   * ===================================================================== */
  function reducedMotion() {
    return (
      window.matchMedia &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    );
  }

  /* =====================================================================
   * 3. Scroll driver (§6.1) — scrub SCENES across #demo.
   * ===================================================================== */
  var scrollDriver = (function () {
    var scenes = [];
    var bounds = []; // cumulative scroll edges per scene (weighted), in [0,1]
    var demo, stage, captionHost;
    var active = false; // #demo in viewport?
    var rafQueued = false;
    var currentScene = -1;
    var revealTimer = null;

    function init() {
      scenes = (window.MMUX_SCENES && window.MMUX_SCENES.slice()) || [];
      demo = document.getElementById("demo");
      stage = $(".demo-stage", demo || document);
      captionHost = $(".demo-caption", demo || document);
      if (!demo || !stage) return; // nothing to drive

      buildBounds();
      buildCaptions();

      if (reducedMotion() || !scenes.length) {
        // §6.1: no scrub. Render the last scene statically; captions stacked.
        renderStatic();
        return;
      }

      if ("IntersectionObserver" in window) {
        var io = new IntersectionObserver(
          function (entries) {
            active = entries[0].isIntersecting;
            if (active) onScroll();
          },
          { threshold: 0 }
        );
        io.observe(demo);
      } else {
        active = true;
      }

      window.addEventListener("scroll", onScroll, { passive: true });
      window.addEventListener("resize", onScroll, { passive: true });
      onScroll();
    }

    // Each scene gets scroll proportional to its `weight` (default 1) — so the
    // opening "type mmux" scene can hold the stage a little longer than the rest.
    // bounds[i] is the cumulative upper edge of scene i in [0,1]; tick() finds the
    // scene that the scroll-progress p falls into.
    function sceneWeight(s) {
      return s && typeof s.weight === "number" && s.weight > 0 ? s.weight : 1;
    }
    function buildBounds() {
      var total = 0, i;
      for (i = 0; i < scenes.length; i++) total += sceneWeight(scenes[i]);
      bounds = [];
      var acc = 0;
      for (i = 0; i < scenes.length; i++) {
        acc += sceneWeight(scenes[i]);
        bounds.push(total > 0 ? acc / total : (i + 1) / Math.max(1, scenes.length));
      }
    }

    // Pre-render every caption as a hidden child; the driver toggles --visible.
    function buildCaptions() {
      if (!captionHost) return;
      captionHost.textContent = "";
      scenes.forEach(function (sc, i) {
        var cap = sc.caption || {};
        var block = el("div", "caption");
        block.dataset.scene = String(i);
        if (cap.kicker) block.appendChild(el("div", "caption-kicker", cap.kicker));
        block.appendChild(el("div", "caption-title", cap.title || ""));
        block.appendChild(el("div", "caption-body", cap.body || ""));
        captionHost.appendChild(block);
      });
    }

    // Reduced-motion / no-scenes fallback: stacked captions + static final state.
    function renderStatic() {
      if (captionHost) {
        captionHost.classList.add("demo-caption--static");
        Array.prototype.forEach.call(
          captionHost.querySelectorAll(".caption"),
          function (c) {
            c.classList.add("caption--visible");
          }
        );
      }
      var last = scenes.length ? scenes[scenes.length - 1].state : DEFAULT_STATE;
      renderTUI(last || DEFAULT_STATE);
    }

    function onScroll() {
      if (!active || rafQueued) return;
      rafQueued = true;
      window.requestAnimationFrame(tick);
    }

    function tick() {
      rafQueued = false;
      if (!demo) return;

      var rect = demo.getBoundingClientRect();
      var travel = demo.offsetHeight - window.innerHeight;
      var scrolled = -rect.top;
      var p = travel > 0 ? clamp(scrolled / travel, 0, 1) : 0;

      var n = scenes.length;
      if (!n) return;
      // weighted bands: walk to the first scene whose cumulative edge p hasn't passed.
      var idx = 0;
      while (idx < n - 1 && p >= bounds[idx]) idx++;

      if (idx !== currentScene) {
        showScene(idx);
        currentScene = idx;
      }
    }

    function showScene(idx) {
      var sc = scenes[idx];
      if (!sc) return;

      if (captionHost) {
        Array.prototype.forEach.call(
          captionHost.querySelectorAll(".caption"),
          function (c) {
            c.classList.toggle(
              "caption--visible",
              c.dataset.scene === String(idx)
            );
          }
        );
      }

      if (revealTimer) {
        clearTimeout(revealTimer);
        revealTimer = null;
      }

      // Clear any prior boot animation so it only replays when a boot scene re-enters.
      var tw = twRefs("tw");
      if (tw && tw.root) tw.root.classList.remove("tw--boot");

      var base = sc.state || {};
      var reveal = sc.type; // optional reveal hint

      if (reveal && !reducedMotion()) {
        runReveal(sc);
      } else if (
        !reducedMotion() &&
        base.main &&
        (base.main.program === "claude" || base.main.program === "codex") &&
        Array.isArray(base.main.lines) &&
        base.main.lines.length > 1
      ) {
        // §6.1: stream the agent scene's lines progressively so it reads as working.
        streamLines(base);
      } else {
        renderTUI(base);
      }

      // A scene flagged `boot` pops the mmux layout in — the sidebar/panel slide,
      // the screen fades (scene 1, right after `mmux` is typed in scene 0).
      if (sc.boot && !reducedMotion()) applyBoot();
    }

    // The one-shot "mmux takes over the window" animation: replay tw--boot from the
    // top by toggling the class with a reflow in between.
    function applyBoot() {
      var tw = twRefs("tw");
      if (!tw || !tw.root) return;
      tw.root.classList.remove("tw--boot");
      void tw.root.offsetWidth; // reflow so the animation restarts on re-entry
      tw.root.classList.add("tw--boot");
    }

    /* Reveal hint dispatch. Scene 0 carries a bare `term`: type `mmux` into it and
     * rest there (no takeover — the layout boots in on the next scene). A
     * `reconnect` scene plays its two-beat drop/reattach sequence. Otherwise:
     * typing reveal, then a line-stream fallback. */
    function runReveal(sc) {
      var base = sc.state || {};
      var reveal = sc.type || {};
      if (reveal.reconnect) {
        runReconnect(sc);
        return;
      }
      if (sc.term && reveal.target === "main" && reveal.text) {
        typeIntoBare(sc.term, reveal.text);
        return;
      }
      if (reveal.target === "main" && reveal.text && reveal.text.length <= 16) {
        typeInto(base, reveal.text);
        return;
      }
      streamLines(base);
    }

    // The closer, in two beats: paint the scene under the amber "ssh disconnected"
    // scrim first, then flip to its real state — the green "reattached" frame with
    // every session intact. Reduced-motion (and scrolling past) lands straight on
    // the reattached state, which is the scene's authored `state`.
    function runReconnect(sc) {
      var base = sc.state || {};
      var drop = cloneState(base);
      drop.overlay = "disconnected";
      drop.status = "ssh dropped — mmux keeps every session alive";
      renderTUI(drop);
      revealTimer = setTimeout(function () {
        renderTUI(base);
      }, 1600);
    }

    // Scene 0: type `text` into the bare terminal, char by char, and stay there. The
    // mmux UI does NOT take over here — scene 1 boots the layout in. Resting on the
    // plain terminal is what keeps this step visible while scrolling down (not just
    // when scrolling back up into it).
    function typeIntoBare(term, full) {
      var chars = full.split("");
      var step = Math.max(75, Math.floor(560 / chars.length));
      // The scene's LAST line is the live prompt `full` types into; anything above
      // it is committed scrollback (a prior `git pull`, say) that stays put.
      var lines = (term.main && term.main.lines) || [];
      var history = lines.slice(0, Math.max(0, lines.length - 1));
      var prompt = "❯ ";
      var last = lines[lines.length - 1];
      if (last && last.tokens && last.tokens[0] && last.tokens[0].t) prompt = last.tokens[0].t;

      function paint(n) {
        var s = cloneState(term);
        s.main = s.main || {};
        s.main.placeholder = null;
        s.main.lines = history.concat([{ tokens: [{ t: prompt, c: "prompt" }, { t: full.slice(0, n) }] }]);
        s.main.cursor = true;
        renderTUI(s);
      }

      var shown = 0;
      paint(0);
      function frame() {
        shown++;
        paint(shown);
        if (shown < chars.length) revealTimer = setTimeout(frame, step);
      }
      revealTimer = setTimeout(frame, step);
    }

    // Typing reveal: type `text` char-by-char into the main pane (scene 0 'mmux').
    function typeInto(base, full) {
      var chars = full.split("");
      var DURATION = 600;
      var step = Math.max(45, Math.floor(DURATION / chars.length));
      var shown = 0;

      // use the prompt glyph the scene authored (e.g. "❯ "), not a hardcoded "$ "
      var prompt = "$ ";
      var l0 = base.main && base.main.lines && base.main.lines[0];
      if (l0 && l0.tokens && l0.tokens[0] && l0.tokens[0].t) prompt = l0.tokens[0].t;
      else if (base.main && base.main.placeholder) prompt = base.main.placeholder;

      function frame() {
        shown++;
        var s = cloneState(base);
        s.main = s.main || {};
        s.main.placeholder = null;
        s.main.lines = [
          { tokens: [{ t: prompt, c: "prompt" }, { t: full.slice(0, shown) }] },
        ];
        s.main.cursor = true;
        renderTUI(s);
        if (shown < chars.length) revealTimer = setTimeout(frame, step);
      }

      var s0 = cloneState(base);
      s0.main = s0.main || {};
      s0.main.placeholder = null;
      s0.main.lines = [{ tokens: [{ t: prompt, c: "prompt" }, { t: "" }] }];
      s0.main.cursor = true;
      renderTUI(s0);
      revealTimer = setTimeout(frame, step);
    }

    // Progressive line-stream: reveal main.lines one at a time (≤ ~900ms total).
    function streamLines(base) {
      var target = (base.main && base.main.lines) || [];
      if (target.length <= 1) {
        renderTUI(base);
        return;
      }
      var DURATION = 900;
      var step = Math.max(70, Math.floor(DURATION / target.length));
      var count = 1;

      function frame() {
        count++;
        var s = cloneState(base);
        s.main = s.main || {};
        s.main.placeholder = null;
        s.main.lines = target.slice(0, count);
        renderTUI(s);
        if (count < target.length) revealTimer = setTimeout(frame, step);
      }

      var s1 = cloneState(base);
      s1.main = s1.main || {};
      s1.main.placeholder = null;
      s1.main.lines = target.slice(0, 1);
      renderTUI(s1);
      revealTimer = setTimeout(frame, step);
    }

    return { init: init };
  })();

  /* =====================================================================
   * 4. Sandbox driver (§6.2) — makes the standalone #tw-how terminal in the
   * "how it works" section click/keyboard-playable. Owns a live mutable state
   * cloned from DEFAULT_STATE; it's a separate terminal from the scrubbed demo
   * (#tw), so the two never fight over one element. Traps keys only while engaged
   * (clicked/focused in); Esc / click-out / focusout releases.
   * ===================================================================== */
  var sandboxDriver = (function () {
    var ROOT_ID = "tw-how"; // the standalone, always-playable terminal in #how
    var state = null; // live, mutable
    var engaged = false; // keys trapped?
    var ready = false; // sandbox initialized → interactive
    var root = null;
    var hintEl = null;
    var liveEl = null;
    var seq = { claude: 1, codex: 1, terminal: 1, process: 1 };

    function refs() {
      root = document.getElementById(ROOT_ID);
      hintEl = root ? $(".tw-sandbox-hint", root) : null;
      liveEl = root ? $(".tw-a11y-live", root) : null;
    }

    // every sandbox render targets its own root (#tw-how), never the demo's #tw.
    // Keep the bottom-bar hint honest about what the pane is doing (these keys work).
    function render() {
      state.status = sandboxStatus();
      renderTUI(state, ROOT_ID);
    }
    function sandboxStatus() {
      var m = state.main;
      if (state.focus === "main" && m) {
        if (m.working) return (m.kind === "codex" ? "codex" : "claude") + " is working · esc to interrupt";
        if (m.typeable) return m.kind === "zsh"
          ? "type a command · ⏎ run · esc back"
          : "type a prompt · ⏎ send · esc back";
        return "live output · esc back";
      }
      return "click a row · ↑↓ move · ⏎ open · x close · esc to leave";
    }

    /* Announce sandbox changes to AT: the navigable rows live in the aria-hidden
     * sidebar, so without this an engaged screen-reader user gets no feedback. */
    function announce(msg) {
      if (liveEl) liveEl.textContent = msg || "";
    }
    function describe(row) {
      if (!row) return "";
      if (row.launcher) return (row.name || "").replace(/^\+\s*/, "") + ", launcher";
      return row.name + ", " + (row.status || "running");
    }

    /* a11y: #tw-how ships as a decorative image (no tabindex). start() makes it a
     * focusable "ready" group; while engaged it's an application so AT passes
     * keystrokes through (§6.2). The "decorative" branch is kept for completeness. */
    function setA11y(mode) {
      if (!root) return;
      if (mode === "active") {
        root.setAttribute("tabindex", "0");
        root.setAttribute("role", "application");
        root.setAttribute(
          "aria-label",
          "mmux sandbox — active. ↑↓ move, Enter open, x close, Escape to leave."
        );
      } else if (mode === "ready") {
        root.setAttribute("tabindex", "0");
        root.setAttribute("role", "group");
        root.setAttribute(
          "aria-label",
          "mmux sandbox — interactive demo. focus it, then ↑↓ to move, Enter to open, Escape to leave."
        );
      } else {
        root.removeAttribute("tabindex");
        root.setAttribute("role", "img");
        root.setAttribute("aria-label", "a simulated mmux terminal session");
      }
    }

    // Initialize the standalone sandbox once: populate it, make it playable, wire
    // its listeners. Called at boot — #tw-how is interactive whenever it's on
    // screen (no scroll-finale gate anymore).
    function start() {
      refs();
      if (!root) return;
      state = cloneState(DEFAULT_STATE);
      state.focus = "sidebar"; // visitor clicks/tabs in to engage
      ensureSelection();
      initGit();         // the Changes box becomes clickable (stage/unstage files)
      openActivePane();  // the open session is a LIVE, typeable pane right away
      render();
      ready = true;
      setA11y("ready");
      root.classList.add("tw--ready");
      showHint(true);
      attachListeners();
    }

    // Make the initially-shown pane a real freshPane (typeable), so you can type into
    // the open Claude/Codex/terminal without first switching to another row and back.
    function openActivePane() {
      var rows = selectableRows();
      var sel = rows[selectedIndex(rows)];
      if (sel && !sel.launcher && sel.status === "running") state.main = mainFor(sel);
    }

    // The "click a row to play" chip is intentionally disabled — keep it hidden
    // regardless of caller. (Restore the body below to bring the hint back.)
    function showHint(show) {
      if (!hintEl) return;
      hintEl.hidden = true;
    }

    var listenersAttached = false;
    function attachListeners() {
      if (listenersAttached || !root) return;
      listenersAttached = true;
      root.addEventListener("click", onTwClick);
      root.addEventListener("focus", engage); // tab into it
      root.addEventListener("keydown", onKey);
      root.addEventListener("focusout", onFocusOut);
      root.addEventListener("touchstart", onTouchStart, { passive: true });
      root.addEventListener("touchend", onTouchEnd, { passive: true });
      document.addEventListener("click", onDocClick, true);
    }

    /* horizontal swipe over the sandbox switches the active project (mobile).
     * vertical-dominant or short gestures fall through so page scrolling is intact. */
    var swipeX = null, swipeY = null;
    function onTouchStart(e) {
      var t = e.touches && e.touches[0];
      swipeX = t ? t.clientX : null;
      swipeY = t ? t.clientY : null;
    }
    function onTouchEnd(e) {
      if (swipeX == null) return;
      var t = e.changedTouches && e.changedTouches[0];
      var x0 = swipeX, y0 = swipeY;
      swipeX = swipeY = null;
      if (!t) return;
      var dx = t.clientX - x0, dy = t.clientY - y0;
      if (Math.abs(dx) < 44 || Math.abs(dx) <= Math.abs(dy)) return;
      if (!state.multiProject || (state.projects || []).length < 2) return;
      if (!engaged) engage();
      switchProject(dx < 0 ? "next" : "prev");
    }

    function onFocusOut(e) {
      if (!engaged) return;
      if (root && e.relatedTarget && root.contains(e.relatedTarget)) return;
      release();
    }

    function onTwClick(e) {
      if (!ready) return;
      e.stopPropagation();
      if (!engaged) engage();
      var tgt = e.target;
      var closest = tgt && tgt.closest ? function (s) { return tgt.closest(s); } : function () { return null; };

      // clicking a node in the git Changes box stages / unstages it (file → itself;
      // folder → all its children, with the checkbox going [✓]/[~]/[ ])
      var gnEl = closest(".git-row[data-git-node]");
      if (gnEl) { toggleGitNode(gnEl.getAttribute("data-git-node")); return; }

      // clicking a sidebar row plays it: launchers spawn, sessions focus (§6.2)
      var rowEl = closest(".sb-row[data-id]");
      if (rowEl) {
        var id = rowEl.getAttribute("data-id");
        var rows = selectableRows();
        for (var i = 0; i < rows.length; i++) {
          if (rows[i].id === id) {
            selectRow(rows, i);
            activate(rows[i]);
            announce(describe(rows[i]));
            render();
            break;
          }
        }
        return;
      }

      // clicking inside the main pane focuses it, so a typeable session takes input
      var mainEl = closest(".tw-main");
      if (mainEl && state.main && state.main.typeable) {
        state.focus = "main";
        render();
      }
    }

    function onDocClick(e) {
      if (!engaged) return;
      if (root && !root.contains(e.target)) release(); // click-out releases
    }

    function engage() {
      if (!ready || engaged) return;
      engaged = true;
      root.classList.add("tw--engaged");
      if (hintEl) hintEl.hidden = true;
      setA11y("active");
      // Land in the open pane if it's a typeable session (type right away); otherwise
      // land on the sidebar list to navigate/spawn. Esc steps main → sidebar → out.
      state.focus = (state.main && state.main.typeable) ? "main" : "sandbox";
      render();
      try {
        root.focus({ preventScroll: true });
      } catch (_) {
        root.focus();
      }
    }

    function release() {
      if (!engaged) return;
      engaged = false;
      root.classList.remove("tw--engaged");
      setA11y("ready");
      state.focus = "sidebar";
      render();
      showHint(true);
    }

    /* --- flat list of selectable rows for the active project. The rendered
     * manifest sidebar shows every member box; keyboard actions stay scoped to
     * whichever project [ / ] made active. --- */
    function selectableRows() {
      var out = [];
      var activeProj = state.multiProject ? activeProjectName() : null;
      (state.sidebar || []).forEach(function (section) {
        (section.rows || []).forEach(function (r) {
          if (r.launcher) {
            out.push(r);
          } else if (activeProj == null || r.project == null || r.project === activeProj) {
            out.push(r);
          }
        });
      });
      return out;
    }
    function selectedIndex(rows) {
      for (var i = 0; i < rows.length; i++) if (rows[i].active) return i;
      return 0;
    }
    function selectRow(rows, idx) {
      // clear active across ALL rows (data is shared across project blocks)
      (state.sidebar || []).forEach(function (s) {
        (s.rows || []).forEach(function (r) {
          r.active = false;
        });
      });
      if (rows[idx]) rows[idx].active = true;
    }
    function ensureSelection() {
      var rows = selectableRows();
      if (!rows.some(function (r) { return r.active; }) && rows.length) {
        rows[0].active = true;
      }
    }

    /* Manifest projects: switch the active member (swipe or [ / ]).
     * Re-points the active project, re-selects that clone's first row, follows it
     * in the title bar, and previews its session in main. */
    function switchProject(dir) {
      var projs = state.projects || [];
      if (!state.multiProject || projs.length < 2) return;
      var cur = 0;
      for (var n = 0; n < projs.length; n++) if (projs[n].active) cur = n;
      var to = dir === "prev"
        ? (cur - 1 + projs.length) % projs.length
        : (cur + 1) % projs.length;
      if (to === cur) return;
      projs.forEach(function (p, idx) { p.active = idx === to; });

      var name = projs[to].name;
      state.title = "~/dev/" + name; // pane/git context follows the active member
      state.focus = "sandbox";

      var rows = selectableRows();
      if (rows.length) selectRow(rows, 0); // first row of the now-active member
      var first = rows[0];
      if (first && !first.launcher) state.main = mainFor(first);

      render();
      announce("project " + name);
    }

    function onKey(e) {
      if (!engaged) return;

      // A focused typeable pane (terminal / Claude / Codex) takes keystrokes as input;
      // Escape is the one key that falls through (to exit / interrupt below).
      if (state.focus === "main" && state.main && state.main.typeable) {
        if (handleTyping(e)) return;
      }

      var rows = selectableRows();
      var i = selectedIndex(rows);
      var handled = true;

      switch (e.key) {
        case "ArrowUp":
        case "k":
          if (state.focus === "sandbox" || state.focus === "sidebar") {
            var up = (i - 1 + rows.length) % rows.length;
            selectRow(rows, up);
            announce(describe(rows[up]));
          } else handled = false;
          break;
        case "ArrowDown":
        case "j":
          if (state.focus === "sandbox" || state.focus === "sidebar") {
            var dn = (i + 1) % rows.length;
            selectRow(rows, dn);
            announce(describe(rows[dn]));
          } else handled = false;
          break;
        case "Enter":
          activate(rows[i]);
          break;
        case "x":
          closeRow(rows[i]);
          break;
        case "[":
          if (state.multiProject) { switchProject("prev"); return; }
          handled = false;
          break;
        case "]":
          if (state.multiProject) { switchProject("next"); return; }
          handled = false;
          break;
        case "Escape":
          if (state.focus === "main") {
            if (state.main && state.main.working) {
              interruptWork(); // Esc interrupts a working agent (stays in the pane)
              return; // interruptWork already rendered
            }
            state.focus = "sandbox"; // main → back to the sidebar list
          } else {
            release(); // Esc from sidebar releases the trap
            return; // release() already rendered
          }
          break;
        default:
          handled = false;
      }

      if (handled) {
        e.preventDefault();
        render();
      }
    }

    function activate(row) {
      if (!row) return;
      if (row.launcher) {
        spawnFrom(row);
      } else if (row.status === "running") {
        state.focus = "main";
        state.main = mainFor(row);
        announce(row.name + " pane focused");
      } else {
        // stopped/exited → start it
        row.status = "running";
        state.focus = "main";
        state.main = mainFor(row);
        announce(row.name + " started");
      }
    }

    /* ----------------------------------------------------------------------
     * Live, typeable panes. A focused terminal / Claude / Codex pane takes
     * keystrokes: the terminal runs a few hardcoded commands; Claude and Codex,
     * once you submit a prompt, "work" forever (a rotating gerund + spinner with
     * the odd tool line) so they look and feel real. A process pane is output-
     * only — no input cursor.
     * -------------------------------------------------------------------- */

    // Output-only dev server (the one non-typeable spawn).
    var SPAWN = {
      vite: {
        program: "vite",
        lines: [
          { tokens: [{ t: "  VITE v5.2.0", c: "brand" }, { t: "  ready in 412 ms", c: "ok" }] },
          { tokens: [{ t: "  ➜  ", c: "info" }, { t: "Local:    " }, { t: "http://localhost:5173/", c: "path" }] },
          { tokens: [{ t: "  ➜  ", c: "info" }, { t: "Network:  " }, { t: "http://192.168.1.4:5173/", c: "path" }] },
          { text: "  ➜  press h + enter to show help", cls: "ln-dim" },
          "",
          { tokens: [{ t: " 10:42:01 ", c: "dim" }, { t: "[vite]", c: "brand" }, { t: " hmr update " }, { t: "/src/App.tsx", c: "path" }] },
        ],
      },
    };

    // A fresh, interactive pane of `kind` ("zsh" | "claude" | "codex"), ready for
    // input. `history` is the committed scrollback; the live prompt line is composed
    // on top by paintPane(). Only these kinds are typeable / get an input cursor.
    function freshPane(kind, title) {
      var glyph = "❯ ", tone = "prompt", program = "zsh", history;
      if (kind === "claude") {
        program = "claude";
        history = claudeBanner().concat([
          "",
          { tokens: [{ t: "  Ask Claude to do something — type a prompt and press enter.", c: "dim" }] },
          "",
        ]);
      } else if (kind === "codex") {
        program = "codex"; glyph = "› "; tone = "codex";
        history = codexBox([
          [{ t: ">_ ", c: "codex" }, { t: "OpenAI Codex " }, { t: "(v0.142.2)", c: "dim" }],
          [{ t: "" }],
          [{ t: "model:       ", c: "dim" }, { t: "gpt-5.5 high" }, { t: "   /model to change", c: "dim" }],
          [{ t: "directory:   ", c: "dim" }, { t: "~/dev/app", c: "path" }],
          [{ t: "permissions: ", c: "dim" }, { t: "YOLO mode", c: "warn" }],
        ]).concat([
          "",
          { tokens: [{ t: "  Ask Codex to do something — type a prompt and press enter.", c: "dim" }] },
          "",
        ]);
      } else {
        history = [{ tokens: [{ t: "~/dev/app", c: "path" }, { t: "  on  " }, { t: "main", c: "ai" }] }];
      }
      var m = {
        program: program, kind: kind, typeable: true,
        promptGlyph: glyph, promptTone: tone,
        input: "", working: false, title: title, cursor: true, history: history,
      };
      m.lines = history.concat([{ tokens: [{ t: glyph, c: tone }, { t: "" }] }]);
      return m;
    }

    // Compose the visible pane = committed history + the live prompt line, then render.
    function paintPane() {
      var m = state.main;
      if (!m) return;
      capHistory(m);
      if (m.working) return; // the work loop paints its own status tail
      m.lines = (m.history || []).concat([
        { tokens: [{ t: m.promptGlyph, c: m.promptTone }, { t: m.input || "" }] },
      ]);
      m.cursor = true;
      render();
    }
    function capHistory(m) {
      var MAX = 24;
      if (m.history && m.history.length > MAX) m.history = m.history.slice(m.history.length - MAX);
    }

    // Key handling for a focused typeable pane. Returns true if the key was consumed
    // (the nav handler is then skipped); Escape falls through so it still exits/interrupts.
    function handleTyping(e) {
      var m = state.main;
      if (m.working) {
        if (e.key === "Escape") return false; // Esc interrupts (handled by nav switch)
        e.preventDefault();
        return true; // swallow everything else while working
      }
      if (e.key === "Enter") { e.preventDefault(); submitInput(); return true; }
      if (e.key === "Backspace") {
        m.input = (m.input || "").slice(0, -1);
        paintPane();
        e.preventDefault();
        return true;
      }
      if (e.key && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
        m.input = (m.input || "") + e.key;
        paintPane();
        e.preventDefault();
        return true;
      }
      if (/^Arrow|^Home$|^End$/.test(e.key || "")) { e.preventDefault(); return true; }
      return false; // Escape & friends fall through to nav
    }

    // Enter: commit the typed line, then act on it.
    function submitInput() {
      var m = state.main;
      var cmd = (m.input || "").trim();
      m.history = (m.history || []).concat([
        { tokens: [{ t: m.promptGlyph, c: m.promptTone }, { t: m.input || "" }] },
      ]);
      m.input = "";
      if (m.kind === "zsh") {
        var out = runCommand(cmd);
        if (out === null) m.history = [{ tokens: [{ t: "~/dev/app", c: "path" }, { t: "  on  " }, { t: "main", c: "ai" }] }];
        else m.history = m.history.concat(out);
        paintPane();
        announce(cmd ? "ran " + cmd : "");
      } else if (!cmd) {
        paintPane(); // empty prompt to an agent → just a new prompt
      } else {
        startWorking(m.kind);
        announce(m.kind + " is working");
      }
    }

    // A few hardcoded shell commands so the terminal feels alive. Returns output lines
    // (no trailing prompt), or null for `clear`.
    function runCommand(c) {
      if (!c) return [];
      var argv = c.split(/\s+/);
      switch (argv[0]) {
        case "clear": return null;
        case "ls":
          return [{ tokens: [{ t: "Cargo.toml  README.md  mmux.yaml  " }, { t: "src  docs  dist", c: "info" }] }];
        case "pwd": return ["/Users/you/dev/app"];
        case "whoami": return ["you"];
        case "echo": return [c.replace(/^echo\s*/, "")];
        case "date": return ["Fri Jun 26 12:00:00 CEST 2026"];
        case "git":
          if (argv[1] === "status")
            return [
              { tokens: [{ t: "On branch ", c: "dim" }, { t: "main", c: "ai" }] },
              { text: "Changes not staged for commit:" },
              { tokens: [{ t: "  modified:   ", c: "warn" }, { t: "src/auth.rs" }] },
              { tokens: [{ t: "  modified:   ", c: "warn" }, { t: "src/token.rs" }] },
            ];
          if (argv[1] === "branch")
            return [{ tokens: [{ t: "* ", c: "ok" }, { t: "main", c: "ok" }] }, { tokens: [{ t: "  feat/tokens", c: "dim" }] }];
          if (argv[1] === "log")
            return [{ tokens: [{ t: "e2e6087 ", c: "dim" }, { t: "add token service" }] }, { tokens: [{ t: "fce46df ", c: "dim" }, { t: "drag-select scrollback" }] }];
          return [{ tokens: [{ t: "git: '" + (argv[1] || "") + "' is not a git command", c: "dim" }] }];
        case "cargo":
          if (argv[1] === "run" || argv[1] === "build")
            return [
              { tokens: [{ t: "   Compiling", c: "ok" }, { t: " app v0.2.0", c: "dim" }] },
              { tokens: [{ t: "    Finished", c: "ok" }, { t: " `dev` profile in 2.91s", c: "dim" }] },
            ];
          if (argv[1] === "test")
            return [{ tokens: [{ t: "test result: " }, { t: "ok.", c: "ok" }, { t: " 12 passed; 0 failed", c: "dim" }] }];
          return [{ tokens: [{ t: "error: no such subcommand `" + (argv[1] || "") + "`", c: "del" }] }];
        case "mmux": return [{ tokens: [{ t: "you're already in mmux — this is it.", c: "dim" }] }];
        case "help":
        case "?":
          return [{ tokens: [{ t: "try: ", c: "dim" }, { t: "ls · pwd · echo · date · git status · cargo run · clear" }] }];
        default:
          return [{ tokens: [{ t: "zsh: command not found: " + argv[0], c: "del" }] }];
      }
    }

    /* ---- the "working forever" loop for Claude / Codex ----
     * Each EVENT is a small group of lines (a tool call + its result) that gets
     * appended as a unit, walked in order, so the scrollback reads like a real
     * session — Read → Search → Edit → Bash → a line of prose, then around again.
     * The live tail is the spinner + status line each agent actually prints. */
    var CLAUDE_WORDS = ["Pondering", "Pontificating", "Noodling", "Percolating", "Finagling",
      "Ruminating", "Schlepping", "Conjuring", "Marinating", "Galvanizing", "Spelunking",
      "Transmuting", "Coalescing", "Wrangling", "Tinkering", "Cogitating", "Simmering",
      "Vibing", "Brewing", "Hatching", "Mulling", "Puzzling"];
    var CLAUDE_STARS = ["✻", "✶", "✳", "✺", "✦"];
    var DOTS = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    var CLAUDE_EVENTS = [
      [{ tokens: [{ t: "⏺ ", c: "ai" }, { t: "Read", c: "fn" }, { t: "(", c: "dim" }, { t: "src/auth.rs", c: "path" }, { t: ")", c: "dim" }] },
       { tokens: [{ t: "  ⎿  ", c: "dim" }, { t: "Read 248 lines", c: "dim" }] }],
      [{ tokens: [{ t: "⏺ ", c: "ai" }, { t: "Search", c: "fn" }, { t: "(", c: "dim" }, { t: 'pattern: "TokenService"', c: "path" }, { t: ")", c: "dim" }] },
       { tokens: [{ t: "  ⎿  ", c: "dim" }, { t: "Found 4 files", c: "dim" }] }],
      [{ tokens: [{ t: "⏺ ", c: "ai" }, { t: "Update", c: "fn" }, { t: "(", c: "dim" }, { t: "src/token.rs", c: "path" }, { t: ")", c: "dim" }] },
       { tokens: [{ t: "  ⎿  ", c: "dim" }, { t: "Updated with 8 additions and 2 removals", c: "dim" }] },
       { text: "       +  pub fn issue(&self, uid: UserId) -> Result<Token> {", cls: "ln-add" }],
      [{ tokens: [{ t: "⏺ ", c: "ai" }, { t: "Bash", c: "fn" }, { t: "(", c: "dim" }, { t: "cargo test auth", c: "dim" }, { t: ")", c: "dim" }] },
       { tokens: [{ t: "  ⎿  ", c: "dim" }, { t: "test result: " }, { t: "ok.", c: "ok" }, { t: " 12 passed; 0 failed", c: "dim" }] }],
      [{ tokens: [{ t: "⏺ ", c: "ai" }, { t: "Wiring the call site to delegate to " }, { t: "TokenService", c: "type" }, { t: " now." }] }],
    ];
    var CODEX_EVENTS = [
      [{ tokens: [{ t: "• ", c: "codex" }, { t: "Explored " }, { t: "src/auth.rs", c: "path" }] }],
      [{ tokens: [{ t: "• ", c: "codex" }, { t: "Edited " }, { t: "src/token.rs", c: "path" }] },
       { tokens: [{ t: "  └ ", c: "dim" }, { t: "+8 −2", c: "dim" }] }],
      [{ tokens: [{ t: "• ", c: "codex" }, { t: "Ran " }, { t: "cargo test", c: "dim" }] },
       { tokens: [{ t: "  └ ", c: "dim" }, { t: "ok", c: "ok" }, { t: " — 12 passed", c: "dim" }] }],
      [{ tokens: [{ t: "• ", c: "codex" }, { t: "Routing token issuance through " }, { t: "TokenService::issue", c: "type" }, { t: "." }] }],
    ];

    // 1234 → "1.2k", 540 → "540" — for the live token counter in the status line.
    function fmtTokens(n) {
      return n >= 1000 ? (n / 1000).toFixed(1).replace(/\.0$/, "") + "k" : String(n);
    }

    var workTimer = null, workTok = 0, workStart = 0, workTick = 0;
    // The closing line each agent prints when it's done. Generation is FINITE: the
    // agent walks its event list once, prints this, then hands the prompt back — so
    // the message you sent reads as past scrollback and you can send another.
    function doneLine(kind) {
      return kind === "codex"
        ? { tokens: [{ t: "• ", c: "codex" }, { t: "Done — routed token issuance through " }, { t: "TokenService::issue", c: "type" }, { t: "." }] }
        : { tokens: [{ t: "⏺ ", c: "ai" }, { t: "Done — auth now issues tokens through " }, { t: "TokenService", c: "type" }, { t: "." }] };
    }
    function startWorking(kind) {
      var m = state.main;
      m.working = true;
      m.cursor = false;
      workTok++;
      m._tok = workTok;
      workTick = 0;
      workStart = nowMs();
      var pool = kind === "codex" ? CODEX_EVENTS : CLAUDE_EVENTS;
      if (reducedMotion()) {
        // static: drop the whole transcript at once, then return the prompt
        var all = pool.reduce(function (a, e) { return a.concat(e); }, []);
        m.history = (m.history || []).concat(all, [doneLine(kind), ""]);
        finishWorking(kind, workTok);
        return;
      }
      var tok = workTok;
      var BEAT = 4; // spinner frames between appended events
      var beat = 0;
      paintWorking(kind);
      (function loop() {
        if (!engaged || workTok !== tok || !state.main || state.main._tok !== tok) return;
        workTick++;
        if (workTick % BEAT === 0) {
          if (beat < pool.length) {
            state.main.history = (state.main.history || []).concat(pool[beat]);
            capHistory(state.main);
            beat++;
          } else {
            // every event walked → print the closing line and return the prompt
            state.main.history = (state.main.history || []).concat([doneLine(kind), ""]);
            finishWorking(kind, tok);
            return;
          }
        }
        paintWorking(kind);
        workTimer = setTimeout(loop, kind === "codex" ? 430 : 360);
      })();
    }

    // Generation finished (or was applied statically): re-enable the composer and
    // repaint with a live, typeable prompt sitting at the bottom, ready for the next.
    function finishWorking(kind, tok) {
      if (workTok !== tok) return;
      if (workTimer) { clearTimeout(workTimer); workTimer = null; }
      var m = state.main;
      if (!m) return;
      m.working = false;
      m._tok = null;
      m.input = "";
      paintPane();
      announce((kind || "agent") + " finished — type another message");
    }

    // Render history + the live spinner/status tail + a dimmed, disabled composer.
    // The tail is what each agent really prints: Claude's colour-cycling ✻ + a gerund
    // + (Ns · ↓ N tokens · esc to interrupt); Codex's braille spinner + Working. The
    // composer stays pinned at the bottom (greyed, cursor-less) so it's clearly "your
    // input, just disabled while it works" — not gone.
    function paintWorking(kind) {
      var m = state.main;
      if (!m) return;
      var elapsed = Math.max(0, Math.round((nowMs() - workStart) / 1000));
      var toks = fmtTokens(Math.max(0, Math.round(elapsed * 215)));
      var tail;
      if (kind === "codex") {
        tail = { tokens: [
          { t: DOTS[workTick % DOTS.length] + " ", c: "codex" },
          { t: "Working ", c: "codex" },
          { t: "(" + elapsed + "s · " + toks + " tokens · esc to interrupt)", c: "dim" },
        ] };
      } else {
        var star = CLAUDE_STARS[workTick % CLAUDE_STARS.length];
        var word = CLAUDE_WORDS[Math.floor(workTick / 6) % CLAUDE_WORDS.length];
        tail = { tokens: [
          { t: star + " ", c: "ai" },
          { t: word + "… ", c: "ai" },
          { t: "(" + elapsed + "s · ↓ " + toks + " tokens · esc to interrupt)", c: "dim" },
        ] };
      }
      var composer = { tokens: [{ t: m.promptGlyph, c: "dim" }, { t: "", c: "dim" }] };
      m.lines = (m.history || []).concat([tail, "", composer]);
      m.cursor = false;
      render();
    }

    // Esc while working → interrupt: stop the loop, drop a note, restore the prompt.
    function interruptWork() {
      if (workTimer) { clearTimeout(workTimer); workTimer = null; }
      workTok++; // invalidate any pending loop frame
      var m = state.main;
      if (!m) return;
      m.working = false;
      m._tok = null;
      m.input = "";
      m.history = (m.history || []).concat([
        { tokens: [{ t: "  ⎿ ", c: "dim" }, { t: "interrupted by user", c: "dim" }] },
        "",
      ]);
      paintPane();
    }

    function nowMs() { return Date.now(); }

    // Launcher → append a new running row of the matching kind, focus and open it.
    function spawnFrom(launcher) {
      var kind, name, sub = null, paneKind = null;

      var lname = launcher.name || "";
      if (/codex/i.test(lname)) {
        kind = "AGENTS"; seq.codex++; name = "Codex"; sub = "ready"; paneKind = "codex";
      } else if (/claude/i.test(lname)) {
        kind = "AGENTS"; seq.claude++; name = "Claude"; sub = "ready"; paneKind = "claude";
      } else if (/terminal/i.test(lname)) {
        kind = "TERMINAL"; name = seq.terminal === 1 ? "zsh" : "zsh " + seq.terminal; seq.terminal++; paneKind = "zsh";
      } else {
        kind = "PROCESSES"; name = "dev server"; sub = "vite · :5173"; seq.process++;
      }

      var section = sectionByKind(kind);
      if (!section) return;

      var id = name.replace(/\s+/g, "-").toLowerCase() + "-" + Date.now();
      var newRow = {
        id: id, name: name, sub: sub, status: "running",
        active: false, attention: false,
        project: state.multiProject ? activeProjectName() : undefined,
      };

      // Launchers sit at the top of a section; a new session goes right below them.
      var insertAt = 0;
      for (var ri = 0; ri < section.rows.length; ri++) {
        if (section.rows[ri].launcher) insertAt = ri + 1;
      }
      section.rows.splice(insertAt, 0, newRow);

      selectRow(selectableRows(), selectableRows().indexOf(newRow));
      state.focus = "main";
      if (paneKind) {
        state.main = freshPane(paneKind, titleFor(newRow)); // interactive, ready for input
        render();
        announce("new " + name + " spawned — ready for input");
      } else {
        state.main = { program: "vite", title: titleFor(newRow), lines: SPAWN.vite.lines, placeholder: null, cursor: false };
        streamMain(SPAWN.vite.lines);
        announce("new " + name + " spawned, running");
      }
    }

    // x → stop the selected session row (keep the row; dot goes "stopped").
    function closeRow(row) {
      if (!row || row.launcher) return;
      row.status = "stopped";
      row.attention = false;
      if (state.focus === "main") {
        state.main = mainFor(row); // shows the stopped placeholder (stops any work loop)
        state.focus = "sandbox";
      }
      announce(row.name + " stopped");
    }

    /* --- main-pane helpers --- */
    function titleFor(row) {
      return " " + row.name + " — " + row.status + " ";
    }
    function mainFor(row) {
      if (row.status !== "running") {
        return {
          program: row.name,
          title: titleFor(row),
          lines: [],
          placeholder: row.name + " is stopped.\n\nPress Enter or 's' to start it.",
          cursor: false,
        };
      }
      var prog = programFor(row);
      if (prog === "vite") {
        return { program: "vite", title: titleFor(row), lines: SPAWN.vite.lines, placeholder: null, cursor: false };
      }
      return freshPane(prog, titleFor(row)); // claude / codex / zsh → interactive, ready
    }
    function programFor(row) {
      if (/dev server/i.test(row.name)) return "vite";
      if (/codex/i.test(row.name)) return "codex";
      if (/claude/i.test(row.name)) return "claude";
      return "zsh";
    }

    // progressive line stream into main (≤ ~900ms; only while engaged & on this row)
    var streamTimer = null;
    function streamMain(lines) {
      if (streamTimer) clearTimeout(streamTimer);
      // reduced-motion (§6.1/§10): no setTimeout reveal — land statically with all
      // lines visible. CSS can't catch this JS content-stream, so guard it here.
      if (reducedMotion()) {
        state.main.lines = lines;
        render();
        return;
      }
      var step = Math.max(80, Math.floor(800 / Math.max(1, lines.length)));
      var count = 1;
      state.main.lines = lines.slice(0, count);
      render();
      function next() {
        count++;
        if (!engaged) return; // bail if visitor left
        state.main.lines = lines.slice(0, count);
        render();
        if (count < lines.length) streamTimer = setTimeout(next, step);
      }
      if (lines.length > 1) streamTimer = setTimeout(next, step);
    }

    /* --- small lookups --- */
    function sectionByKind(kind) {
      return (state.sidebar || []).filter(function (s) {
        return s.kind === kind;
      })[0];
    }
    function hasRowNamed(name) {
      return selectableRows().some(function (r) {
        return r.name === name && !r.launcher;
      });
    }
    function activeProjectName() {
      var p = (state.projects || []).filter(function (x) {
        return x.active;
      })[0];
      return p ? p.name : undefined;
    }

    /* --- the native git panel, made interactive ---------------------------
     * The Changes box is a real tree: files AND folders are clickable. Clicking a
     * file stages / unstages it; clicking a folder stages / unstages all of its
     * descendants at once. Every folder's checkbox is computed from its leaves —
     * a green `[✓]` when all are staged, `[ ]` when none are, and a partial `[~]`
     * in between — so the selection ripples up the tree like the real panel. */
    function initGit() {
      state._git = {
        roots: [
          { id: "src", name: "src/", kind: "dir", children: [
            { id: "src/auth.rs", name: "auth.rs", kind: "file", change: "modified", staged: true },
            { id: "src/token.rs", name: "token.rs", kind: "file", change: "modified", staged: false },
            { id: "src/lib.rs", name: "lib.rs", kind: "file", change: "added", staged: true },
          ] },
          { id: "assets", name: "assets/", kind: "dir", children: [
            { id: "assets/logo.png", name: "logo.png", kind: "file", change: "modified", staged: false },
          ] },
          { id: "Cargo.toml", name: "Cargo.toml", kind: "file", change: "modified", staged: false },
        ],
        cursor: "src/auth.rs",
      };
      rebuildGit();
    }
    function rebuildGit() {
      if (state.panel && Array.isArray(state.panel.sections) && state.panel.sections.length) {
        state.panel.sections[0] = buildGitSection();
      }
    }
    // descendant leaf files of a node (a file is its own only leaf)
    function gitLeaves(node) {
      if (node.kind === "file") return [node];
      var out = [];
      (node.children || []).forEach(function (c) { out = out.concat(gitLeaves(c)); });
      return out;
    }
    // tri-state: "checked" (all staged) | "none" (none staged) | "partial" (some)
    function gitState(node) {
      if (node.kind === "file") return node.staged ? "checked" : "none";
      var leaves = gitLeaves(node);
      if (!leaves.length) return "none";
      var n = leaves.filter(function (l) { return l.staged; }).length;
      return n === 0 ? "none" : n === leaves.length ? "checked" : "partial";
    }
    function findGitNode(id) {
      var found = null;
      (function walk(n) {
        if (!n || found) return;
        if (n.id === id) { found = n; return; }
        (n.children || []).forEach(walk);
      })({ children: state._git.roots });
      return found;
    }
    function buildGitSection() {
      var rows = [];
      function walk(node, depth) {
        var st = gitState(node);
        var box = st === "checked" ? "[✓]" : st === "partial" ? "[~]" : "[ ]";
        var boxTone = st === "checked" ? "ok" : st === "partial" ? "warn" : "dim";
        var nameTone = node.kind === "dir" ? "info" : node.change === "added" ? "ok" : "warn";
        var isCursor = state._git.cursor === node.id;
        var line = {
          tokens: [
            { t: isCursor ? "▌" : " ", c: "ai" },
            { t: "  ".repeat(depth) + box, c: boxTone },
            { t: " " + node.name, c: nameTone },
          ],
          gitNode: node.id,
        };
        if (isCursor) line.cls = "git-sel";
        rows.push(line);
        if (node.kind === "dir") (node.children || []).forEach(function (c) { walk(c, depth + 1); });
      }
      (state._git.roots || []).forEach(function (r) { walk(r, 0); });
      return { title: "Changes · main ↑1", active: true, lines: rows };
    }
    // Click → toggle. A file flips itself; a folder stages all its leaves (or, if
    // they're already all staged, unstages them). The cursor moves to the clicked
    // node, and every ancestor's checkbox recomputes via gitState().
    function toggleGitNode(id) {
      var node = findGitNode(id);
      if (!node) return;
      state._git.cursor = id;
      if (node.kind === "file") {
        node.staged = !node.staged;
        announce(node.name + (node.staged ? " staged" : " unstaged"));
      } else {
        var leaves = gitLeaves(node);
        var allStaged = leaves.length && leaves.every(function (l) { return l.staged; });
        leaves.forEach(function (l) { l.staged = !allStaged; });
        announce(node.name + (allStaged ? " all unstaged" : " all staged"));
      }
      rebuildGit();
      render();
    }

    return {
      start: start,
    };
  })();

  /* =====================================================================
   * Generic helpers
   * ===================================================================== */
  function clamp(v, lo, hi) {
    return v < lo ? lo : v > hi ? hi : v;
  }

  // The real Claude Code welcome banner (its block-glyph logo), captured from `claude`.
  function claudeBanner() {
    return [
      { tokens: [{ t: " ▐▛███▜▌  ", c: "claude" }, { t: "Claude Code " }, { t: "v2.1.193", c: "dim" }], cls: "art" },
      { tokens: [{ t: "▝▜█████▛▘ ", c: "claude" }, { t: "Opus 4.8 (1M context) · Claude Max", c: "dim" }], cls: "art" },
      { tokens: [{ t: "  ▘▘ ▝▝   ", c: "claude" }, { t: "~/dev/app", c: "path" }], cls: "art" },
    ];
  }

  // Build the real Codex rounded banner from row segments, padding each row so the
  // right border lines up. `rows` is an array of token-segment arrays. (Mirrors the
  // builder in scenes.js; the sandbox spawns its own canned content.)
  function codexBox(rows) {
    var W = 48;
    var dash = "─".repeat(W - 2);
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
  function cloneState(s) {
    if (typeof structuredClone === "function") {
      try {
        return structuredClone(s);
      } catch (_) {
        /* fall through */
      }
    }
    return JSON.parse(JSON.stringify(s));
  }

  /* =====================================================================
   * Copy buttons (§11): button.copy[data-copy] → clipboard, flash 'copied'.
   * ===================================================================== */
  function wireCopyButtons() {
    var buttons = document.querySelectorAll("button.copy[data-copy]");
    Array.prototype.forEach.call(buttons, function (btn) {
      btn.addEventListener("click", function () {
        var text = btn.getAttribute("data-copy") || "";
        copyText(text).then(
          function () {
            flash(btn);
          },
          function () {
            flash(btn); // still flash; nothing else we can do offline
          }
        );
      });
    });
  }
  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text);
    }
    return new Promise(function (resolve, reject) {
      try {
        var ta = document.createElement("textarea");
        ta.value = text;
        ta.setAttribute("readonly", "");
        ta.style.position = "fixed";
        ta.style.top = "-9999px";
        document.body.appendChild(ta);
        ta.select();
        var ok = document.execCommand("copy");
        document.body.removeChild(ta);
        ok ? resolve() : reject();
      } catch (e) {
        reject(e);
      }
    });
  }
  function flash(btn) {
    if (btn.dataset.flashing === "1") return;
    btn.dataset.flashing = "1";
    var prev = btn.textContent;
    btn.textContent = "copied";
    btn.classList.add("copy--copied");
    setTimeout(function () {
      btn.textContent = prev;
      btn.classList.remove("copy--copied");
      btn.dataset.flashing = "0";
    }, 1000);
  }

  /* =====================================================================
   * Install-method tabs (§11): .install-tab[data-tab] toggles which
   * .install-row[data-panel] is shown. Script is the default; only one
   * panel is ever visible, so the section stays a single command line.
   * ===================================================================== */
  function wireInstallTabs() {
    var tabs = document.querySelectorAll(".install-tab[data-tab]");
    if (!tabs.length) return;
    Array.prototype.forEach.call(tabs, function (tab) {
      tab.addEventListener("click", function () {
        var name = tab.getAttribute("data-tab");
        Array.prototype.forEach.call(tabs, function (t) {
          var on = t === tab;
          t.classList.toggle("is-active", on);
          t.setAttribute("aria-selected", on ? "true" : "false");
        });
        var panels = document.querySelectorAll(".install-row[data-panel]");
        Array.prototype.forEach.call(panels, function (p) {
          p.hidden = p.getAttribute("data-panel") !== name;
        });
      });
    });
  }

  /* =====================================================================
   * Smooth-scroll nav anchors (§11). Respect reduced-motion.
   * ===================================================================== */
  function wireNavAnchors() {
    var links = document.querySelectorAll('a[href^="#"]');
    Array.prototype.forEach.call(links, function (a) {
      a.addEventListener("click", function (e) {
        var id = a.getAttribute("href").slice(1);
        if (!id) return;
        var target = document.getElementById(id);
        if (!target) return;
        e.preventDefault();
        target.scrollIntoView({
          behavior: reducedMotion() ? "auto" : "smooth",
          block: "start",
        });
      });
    });
  }

  /* =====================================================================
   * Boot
   * ===================================================================== */
  function boot() {
    // Render an initial state so the demo window is never blank before scroll fires.
    // Scene 0 opens on its bare "before mmux" terminal; the reveal types `mmux` there
    // and rests, and scene 1 boots the mmux layout in once #demo scrolls into view.
    var sc0 = window.MMUX_SCENES && window.MMUX_SCENES[0];
    var first = sc0 ? sc0.term || sc0.state || DEFAULT_STATE : DEFAULT_STATE;
    renderTUI(first);

    scrollDriver.init();    // scrubs the scenes across #tw
    sandboxDriver.start();  // makes #tw-how (the "how it works" terminal) playable
    wireCopyButtons();
    wireInstallTabs();
    wireNavAnchors();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }

  /* Single global namespace (§11): renderer + line renderer + state contract. */
  window.MMUX = {
    renderTUI: renderTUI,
    renderLine: renderLine,
    DEFAULT_STATE: DEFAULT_STATE,
    STATUS: STATUS,
  };
})();
