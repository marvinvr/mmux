/* tui.js — the coupled core of mmux.org (v2).
 *
 * One global: window.MMUX. May read window.MMUX_SCENES (from scenes.js).
 * Responsibilities:
 *   1. renderTUI(state)  — idempotent DOM updater over the #tw skeleton (§5.3).
 *   2. renderLine(line)  — the realistic-content / token renderer (§5.4).
 *   3. scroll driver     — scrub window.MMUX_SCENES across the tall #demo (§6.1).
 *   4. sandbox driver    — make the finale keyboard-playable (§6.2).
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
   *   multiProject: bool,
   *   projects: [{ name, active }],
   *   sidebar: [ { kind:"AGENTS"|"TERMINAL"|"PROCESSES", rows: [
   *       { id, name, sub?, status:"running"|"exited"|"stopped",
   *         active?, attention?, project? },      // session row
   *       { id, launcher:true, name:"New Claude" } // launcher → "+ New Claude"
   *   ]}],
   *   main: { program:"claude"|"zsh"|"vite"|null, title, lines:[Line],
   *           placeholder:str|null, cursor:bool },
   *   panel: { visible, branch, lines:[Line] },
   *   focus: "sidebar"|"main"|"panel"|"sandbox",
   *   toast: { app, title, body } | null,
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
   * authors scene 8 to mirror this shape, so the field names here are the stable
   * contract. The content uses the token model so the pane looks real. */
  var DEFAULT_STATE = {
    title: "~/dev/app",
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
            sub: "refactoring auth",
            status: "running",
            active: true,
            attention: false,
            project: "app",
          },
          {
            id: "claude-2",
            name: "claude",
            sub: "running tests",
            status: "running",
            project: "app-2",
          },
          { id: "new-claude", launcher: true, name: "New Claude" },
        ],
      },
      {
        kind: "TERMINAL",
        rows: [
          { id: "zsh", name: "zsh", status: "running", project: "app" },
          { id: "new-terminal", launcher: true, name: "New Terminal" },
        ],
      },
      {
        kind: "PROCESSES",
        rows: [
          {
            id: "dev-server",
            name: "dev server",
            sub: "vite · :5173",
            status: "running",
            project: "app",
          },
          { id: "new-process", launcher: true, name: "New Process" },
        ],
      },
    ],
    main: {
      program: "claude",
      title: " claude — running ",
      lines: [
        { tokens: [{ t: "> ", c: "dim" }, { t: "refactor auth to use the new TokenService" }] },
        "",
        { tokens: [{ t: "●  ", c: "ai" }, { t: "Read", c: "fn" }, { t: "  src/auth.rs, src/token.rs", c: "path" }] },
        { tokens: [{ t: "●  ", c: "ai" }, { t: "Edit", c: "fn" }, { t: "  src/auth.rs", c: "path" }] },
        { text: "     -  let token = generate_token(user_id);", cls: "ln-del" },
        { text: "     +  let token = self.tokens.issue(user_id)?;", cls: "ln-add" },
        { tokens: [{ t: "●  ", c: "ai" }, { t: "Bash", c: "fn" }, { t: "  cargo test auth" }] },
        { tokens: [{ t: "     test result: " }, { t: "ok.", c: "ok" }, { t: " 12 passed; 0 failed", c: "dim" }] },
        { tokens: [{ t: "●  ", c: "ai" }, { t: "auth now delegates to TokenService. " }, { t: "✓", c: "ok" }] },
      ],
      placeholder: null,
      cursor: true,
    },
    panel: {
      visible: true,
      branch: "main",
      lines: [
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

  // Cache the #tw sub-elements once; re-resolve lazily so a missing skeleton is harmless.
  var TW = null;
  function twRefs() {
    if (TW && document.body.contains(TW.root)) return TW;
    var root = document.getElementById("tw");
    if (!root) return null;
    TW = {
      root: root,
      barName: $(".tw-titlebar-name", root),
      barPath: $(".tw-titlebar-path", root),
      barMeta: $(".tw-titlebar-meta", root),
      sidebar: $(".tw-sidebar", root),
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
    return TW;
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
  function renderTUI(state) {
    var t = twRefs();
    if (!t || !state) return;

    renderBar(t, state);
    renderSidebar(t.sidebar, state);
    renderMain(t, state.main || {});
    renderPanel(t, state.panel || {});
    renderStatus(t.status, state.focus);
    renderToast(t.toast, state.toast);
    renderOverlay(t.overlay, state.overlay);

    // Reflect engaged-focus on the root so CSS can paint pane accents.
    t.root.classList.toggle("tw--main-focus", state.focus === "main");
    t.root.classList.toggle("tw--panel-focus", state.focus === "panel");
    t.root.classList.toggle(
      "tw--sidebar-focus",
      state.focus === "sidebar" || state.focus === "sandbox"
    );
  }

  function renderBar(t, state) {
    if (t.barPath && state.title) t.barPath.textContent = state.title;
    // title bar meta optionally carries the branch; default "⌁ tmux" stays in HTML.
    if (t.barMeta && state.panel && state.panel.visible && state.panel.branch) {
      t.barMeta.textContent = "⎇ " + state.panel.branch;
    } else if (t.barMeta) {
      t.barMeta.textContent = "⌁ tmux";
    }
  }

  function renderSidebar(host, state) {
    if (!host) return;
    host.textContent = ""; // rebuild wholesale: small DOM, simpler than diffing.

    var byProject = !!state.multiProject && Array.isArray(state.projects);

    if (byProject) {
      state.projects.forEach(function (proj) {
        var head = el(
          "div",
          "sb-project" + (proj.active ? " sb-project--active" : ""),
          " " + proj.name + " "
        );
        host.appendChild(head);
        (state.sidebar || []).forEach(function (section) {
          appendSection(host, section, proj.name);
        });
      });
    } else {
      (state.sidebar || []).forEach(function (section) {
        appendSection(host, section, null);
      });
    }
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
    // tab: program label, or hidden when program is null
    if (t.tab) {
      if (main.program) {
        t.tab.textContent = main.program;
        t.tab.hidden = false;
      } else {
        t.tab.textContent = "";
        t.tab.hidden = true;
      }
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
  }

  function renderPanel(t, panel) {
    if (!t.panel) return;
    var visible = !!panel.visible;
    t.panel.hidden = !visible;
    if (!visible) return;

    if (t.panelHead) {
      t.panelHead.textContent = panel.branch ? " " + panel.branch + " " : " git ";
    }
    if (t.panelScreen) {
      t.panelScreen.textContent = "";
      (panel.lines || []).forEach(function (line) {
        t.panelScreen.appendChild(renderLine(line));
      });
    }
  }

  function renderStatus(host, focus) {
    if (!host) return;
    host.textContent = STATUS[focus] || STATUS.sidebar;
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
    var head = el("div", "toast-head");
    var dot = el("span", "toast-dot", "●");
    dot.setAttribute("aria-hidden", "true");
    head.appendChild(dot);
    head.appendChild(el("span", "toast-app", toast.app || ""));
    head.appendChild(el("span", "toast-title", toast.title || ""));
    host.appendChild(head);
    if (toast.body) host.appendChild(el("div", "toast-body", toast.body));
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
    var demo, stage, captionHost;
    var active = false; // #demo in viewport?
    var rafQueued = false;
    var currentScene = -1;
    var revealTimer = null;
    var sandbox = null;

    function init(opts) {
      scenes = (window.MMUX_SCENES && window.MMUX_SCENES.slice()) || [];
      sandbox = opts && opts.sandbox;
      demo = document.getElementById("demo");
      stage = $(".demo-stage", demo || document);
      captionHost = $(".demo-caption", demo || document);
      if (!demo || !stage) return; // nothing to drive

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
      if (sandbox) sandbox.enableStatic(); // sandbox playable immediately
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
      var idx = clamp(Math.floor(p * n), 0, n - 1);

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

      var isFinale = idx === scenes.length - 1;
      var base = sc.state || {};
      var reveal = sc.type; // optional reveal hint

      if (reveal && !reducedMotion()) {
        runReveal(base, reveal);
      } else if (
        !isFinale && // at the finale the sandbox owns the state; don't stream over it
        !reducedMotion() &&
        base.main &&
        base.main.program === "claude" &&
        Array.isArray(base.main.lines) &&
        base.main.lines.length > 1
      ) {
        // §6.1: stream the Claude scene's lines progressively so it reads as working.
        streamLines(base);
      } else {
        renderTUI(base);
      }

      if (isFinale && sandbox) {
        sandbox.enable(base);
      } else if (sandbox) {
        sandbox.disable();
      }
    }

    /* Reveal hint dispatch: { target:"main", text:"mmux" } → typing; otherwise
     * fall back to a line-stream of the base state's main.lines. */
    function runReveal(base, reveal) {
      if (reveal.target === "main" && reveal.text && reveal.text.length <= 16) {
        typeInto(base, reveal.text);
        return;
      }
      streamLines(base);
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
   * 4. Sandbox driver (§6.2) — the finale makes #tw keyboard-playable.
   * Owns a live mutable state cloned from the finale scene. Traps keys only
   * while engaged (clicked/focused in); Esc / click-out / focusout releases.
   * ===================================================================== */
  var sandboxDriver = (function () {
    var state = null; // live, mutable
    var enabled = false; // finale reached?
    var engaged = false; // keys trapped?
    var ready = false; // finale currently active → sandbox is interactive
    var root = null;
    var hintEl = null;
    var liveEl = null;
    var seq = { claude: 1, terminal: 1, process: 1 };

    function refs() {
      root = document.getElementById("tw");
      hintEl = root ? $(".tw-sandbox-hint", root) : null;
      liveEl = root ? $(".tw-a11y-live", root) : null;
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

    /* a11y: #tw is a decorative image until the finale (HTML ships it that way —
     * no tabindex). When playable it becomes focusable; while engaged it's an
     * application so AT passes keystrokes through (§6.2). */
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

    function enable(baseState) {
      refs();
      if (!root) return;
      if (!enabled) {
        enabled = true;
        state = cloneState(baseState || DEFAULT_STATE);
        state.focus = "sidebar"; // visitor must click in to engage
        ensureSelection();
        renderTUI(state);
        attachListeners();
      }
      ready = true;
      setA11y("ready");
      root.classList.add("tw--ready");
      if (!engaged) showHint(true);
    }

    function enableStatic() {
      refs();
      if (!root) return;
      enabled = true;
      if (!state) state = cloneState(DEFAULT_STATE);
      state.focus = "sidebar";
      ensureSelection();
      renderTUI(state);
      ready = true;
      setA11y("ready");
      root.classList.add("tw--ready");
      showHint(true);
      attachListeners();
    }

    function disable() {
      ready = false;
      if (engaged) release();
      if (hintEl) hintEl.hidden = true;
      if (root) root.classList.remove("tw--ready");
      setA11y("decorative");
    }

    function showHint(show) {
      if (!hintEl) return;
      hintEl.hidden = !show;
      if (show) hintEl.textContent = "click a row to play  ·  ↑↓ ⏎ x";
    }

    var listenersAttached = false;
    function attachListeners() {
      if (listenersAttached || !root) return;
      listenersAttached = true;
      root.addEventListener("click", onTwClick);
      root.addEventListener("focus", engage); // tab into it
      root.addEventListener("keydown", onKey);
      root.addEventListener("focusout", onFocusOut);
      document.addEventListener("click", onDocClick, true);
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
      // clicking a sidebar row plays it: launchers spawn, sessions focus (§6.2)
      var rowEl = e.target && e.target.closest ? e.target.closest(".sb-row[data-id]") : null;
      if (!rowEl) return;
      var id = rowEl.getAttribute("data-id");
      var rows = selectableRows();
      for (var i = 0; i < rows.length; i++) {
        if (rows[i].id === id) {
          selectRow(rows, i);
          activate(rows[i]);
          announce(describe(rows[i]));
          renderTUI(state);
          break;
        }
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
      state.focus = "sandbox";
      renderTUI(state);
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
      renderTUI(state);
      showHint(true);
    }

    /* --- flat list of selectable rows (sessions + launchers), in DOM order.
     * In multi-project mode the sidebar repeats sections per project, but the
     * underlying row objects are shared; we walk the data once, picking the
     * rows that belong to the active project (+ all launchers). --- */
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

    function onKey(e) {
      if (!engaged) return;
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
        case "Escape":
          if (state.focus === "main") {
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
        renderTUI(state);
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

    /* canned realistic content blocks for freshly-spawned sessions (§6.2). */
    var SPAWN = {
      claude: {
        program: "claude",
        lines: [
          { tokens: [{ t: "> ", c: "dim" }, { t: "scaffold a health-check endpoint" }] },
          "",
          { tokens: [{ t: "●  ", c: "ai" }, { t: "Write", c: "fn" }, { t: "  src/routes/health.rs", c: "path" }] },
          { text: "     +  pub async fn health() -> Json<Status> {", cls: "ln-add" },
          { text: "     +      Json(Status::ok())", cls: "ln-add" },
          { tokens: [{ t: "●  ", c: "ai" }, { t: "Bash", c: "fn" }, { t: "  cargo check" }] },
          { tokens: [{ t: "     Finished", c: "ok" }, { t: " in 1.84s", c: "dim" }] },
        ],
      },
      zsh: {
        program: "zsh",
        lines: [
          { tokens: [{ t: "~/dev/app", c: "path" }, { t: "  on  " }, { t: "main", c: "ai" }] },
          { tokens: [{ t: "❯ ", c: "prompt" }, { t: "git status -s" }] },
          { tokens: [{ t: " M ", c: "warn" }, { t: "src/auth.rs" }] },
          { tokens: [{ t: "?? ", c: "dim" }, { t: "src/routes/health.rs" }] },
        ],
      },
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

    // Launcher → append a new running row of the matching kind, focus main, stream.
    function spawnFrom(launcher) {
      var kind, name, sub = null, block;

      var lname = launcher.name || "";
      if (/claude/i.test(lname)) {
        kind = "AGENTS";
        var n = seq.claude++;
        name = "claude";
        sub = "scaffolding health-check";
        block = SPAWN.claude;
        if (n > 1 || hasRowNamed("claude")) {
          // keep the name "claude"; the sidebar can hold several (the app does too)
          sub = "new session";
        }
      } else if (/terminal/i.test(lname)) {
        kind = "TERMINAL";
        name = seq.terminal === 1 ? "zsh" : "zsh " + seq.terminal;
        seq.terminal++;
        block = SPAWN.zsh;
      } else {
        kind = "PROCESSES";
        name = "dev server";
        sub = "vite · :5173";
        seq.process++;
        block = SPAWN.vite;
      }

      var section = sectionByKind(kind);
      if (!section) return;

      var id = name.replace(/\s+/g, "-").toLowerCase() + "-" + Date.now();
      var newRow = {
        id: id,
        name: name,
        sub: sub,
        status: "running",
        active: false,
        attention: false,
        project: state.multiProject ? activeProjectName() : undefined,
      };

      // insert above the launcher within the section
      var li = section.rows.indexOf(launcher);
      if (li < 0) li = section.rows.length;
      section.rows.splice(li, 0, newRow);

      // select it, focus main, stream its realistic content
      selectRow(selectableRows(), selectableRows().indexOf(newRow));
      state.focus = "main";
      state.main = {
        program: block.program,
        title: titleFor(newRow),
        lines: block.lines,
        placeholder: null,
        cursor: true,
      };
      streamMain(block.lines);
      announce("new " + name + " spawned, running");
    }

    // x → stop the selected session row (keep the row; dot goes "stopped").
    function closeRow(row) {
      if (!row || row.launcher) return;
      row.status = "stopped";
      row.attention = false;
      if (state.focus === "main") {
        state.main = mainFor(row); // shows the stopped placeholder
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
      // re-focusing a running row: show a small live snippet for its kind
      var prog = programFor(row);
      var block = (prog && SPAWN[prog === "dev server" ? "vite" : prog]) || null;
      return {
        program: block ? block.program : prog,
        title: titleFor(row),
        lines: block ? block.lines : [{ tokens: [{ t: "❯ ", c: "prompt" }, { t: "" }] }],
        placeholder: null,
        cursor: true,
      };
    }
    function programFor(row) {
      if (/dev server/i.test(row.name)) return "vite";
      if (/claude|codex/i.test(row.name)) return "claude";
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
        renderTUI(state);
        return;
      }
      var step = Math.max(80, Math.floor(800 / Math.max(1, lines.length)));
      var count = 1;
      state.main.lines = lines.slice(0, count);
      renderTUI(state);
      function next() {
        count++;
        if (!engaged) return; // bail if visitor left
        state.main.lines = lines.slice(0, count);
        renderTUI(state);
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

    return {
      enable: enable,
      enableStatic: enableStatic,
      disable: disable,
    };
  })();

  /* =====================================================================
   * Generic helpers
   * ===================================================================== */
  function clamp(v, lo, hi) {
    return v < lo ? lo : v > hi ? hi : v;
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
    // Render an initial state so the window is never blank before scroll fires.
    var first =
      window.MMUX_SCENES && window.MMUX_SCENES.length
        ? window.MMUX_SCENES[0].state || DEFAULT_STATE
        : DEFAULT_STATE;
    renderTUI(first);

    scrollDriver.init({ sandbox: sandboxDriver });
    wireCopyButtons();
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
