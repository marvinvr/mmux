/* tui.js — the coupled core of mmux.org.
 *
 * One global: window.MMUX. May read window.MMUX_SCENES (from scenes.js).
 * Three responsibilities:
 *   1. renderTUI(state)  — pure-ish, idempotent DOM updater over the #tui skeleton (§5.3).
 *   2. scroll driver     — scrub window.MMUX_SCENES across the tall #demo section (§6.1).
 *   3. sandbox driver    — make scene 8 keyboard-playable (§6.2).
 * Plus: generic copy buttons + smooth-scroll nav (§8).
 *
 * No modules/imports (must work over file://). Everything guards: missing scenes or
 * missing elements must not throw — zero console errors is part of the done-list (§9).
 */
(function () {
  "use strict";

  /* =====================================================================
   * State shape — the contract between tui.js, scenes.js and renderTUI (§5.3)
   * ---------------------------------------------------------------------
   * state = {
   *   multiProject: bool,                 // render project headers when true
   *   projects: [{ name, active }],       // sidebar project headers (multi only)
   *   sidebar: [                          // ordered sections, top→bottom
   *     { kind: "AGENTS"|"TERMINAL"|"PROCESSES", rows: [
   *         // a session row:
   *         { id, glyph: "●"|"○"|"·", name, sub?, status, selected?,
   *           attention?, project? },
   *         // a launcher row:
   *         { id, launcher: true, name: "+ New …", selected? },
   *     ]},
   *   ],
   *   main: {
   *     title,                            // " {name} — {status} " (em-dash, spaces)
   *     lines: [str],                     // screen rows (used when placeholder is null)
   *     placeholder: str|null,            // faint text; takes precedence over lines
   *     cursor: bool,                     // block cursor on last line
   *   },
   *   panel: { visible, branch, lines: [str] },
   *   focus: "sidebar"|"main"|"panel"|"sandbox",
   *   toast: { title, body } | null,      // .tui-toast (alert accent)
   *   overlay: "detached"|"reattached" | null,
   * }
   * ===================================================================== */

  /* DEFAULT_STATE — a scene-8-like state. Used when window.MMUX_SCENES is absent
   * (degrade to a static, playable finale per §8). scenes.js authors its scene-8
   * `state` to this exact shape, so the field names here are the stable contract. */
  var DEFAULT_STATE = {
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
      lines: ["● main", "  modified  src/auth.rs", "  staged    2 files"],
    },
    focus: "sidebar",
    toast: null,
    overlay: null,
  };

  /* Footer hint strings, keyed by focus (§5.2). The renderer picks one. */
  var FOOTERS = {
    sidebar: "↑↓ move   ⏎ open   s start   x close   r restart   d detach   q quit",
    main: "keys → pane   drag = copy   Ctrl-b   h back   x close",
    panel: "keys → pane   drag = copy   Ctrl-b   h back   x close",
    sandbox: "↑↓ move   ⏎ open   x close   —   click out to scroll",
  };

  var OVERLAY_TEXT = { detached: "detached", reattached: "reattached" };

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
  // Cache the #tui sub-elements once; re-resolve lazily so a missing skeleton is harmless.
  var TUI = null;
  function tuiRefs() {
    if (TUI && document.body.contains(TUI.root)) return TUI;
    var root = document.getElementById("tui");
    if (!root) return null;
    TUI = {
      root: root,
      sidebar: $(".tui-sidebar", root),
      mainTitle: $(".tui-main-title", root),
      mainScreen: $(".tui-main-screen", root),
      panel: $(".tui-panel", root),
      panelTitle: $(".tui-panel-title", root),
      panelScreen: $(".tui-panel-screen", root),
      footer: $(".tui-footer", root),
      toast: $(".tui-toast", root),
      overlay: $(".tui-overlay", root),
      sandboxHint: $(".tui-sandbox-hint", root),
    };
    return TUI;
  }

  /* =====================================================================
   * 1. renderTUI(state) — idempotent DOM updater. Does NOT animate (§5.3).
   * ===================================================================== */
  function renderTUI(state) {
    var t = tuiRefs();
    if (!t || !state) return;

    renderSidebar(t.sidebar, state);
    renderMain(t, state.main || {}, state);
    renderPanel(t, state.panel || {});
    renderFooter(t.footer, state.focus);
    renderToast(t.toast, state.toast);
    renderOverlay(t.overlay, state.overlay);

    // Reflect engaged-focus on the root so CSS can paint pane borders in --accent.
    t.root.classList.toggle("tui--main-focus", state.focus === "main");
    t.root.classList.toggle("tui--panel-focus", state.focus === "panel");
    t.root.classList.toggle("tui--sidebar-focus", state.focus === "sidebar" || state.focus === "sandbox");
  }

  function renderSidebar(host, state) {
    if (!host) return;
    // Rebuild wholesale: small DOM, simpler than diffing, still idempotent.
    host.textContent = "";

    var byProject = !!state.multiProject && Array.isArray(state.projects);

    if (byProject) {
      // Render each project as: header + the sections belonging to it.
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

  // Append one section; when `projectName` is set, only rows for that project.
  function appendSection(host, section, projectName) {
    var rows = (section.rows || []).filter(function (r) {
      if (projectName == null || r.launcher) return true; // launchers belong to every project block
      return r.project == null || r.project === projectName;
    });
    if (!rows.length) return;

    var wrap = el("div", "sb-section");
    wrap.appendChild(el("div", "sb-header", section.kind));
    rows.forEach(function (r) {
      wrap.appendChild(buildRow(r));
    });
    host.appendChild(wrap);
  }

  function buildRow(r) {
    var classes = "sb-row";
    if (r.launcher) classes += " sb-row--launcher";
    if (r.selected) classes += " sb-row--selected";
    var row = el("div", classes);
    if (r.id != null) row.setAttribute("data-id", r.id);

    // selection bar ▌ (cyan when selected, else a space to preserve column width)
    var bar = el("span", "sb-bar", r.selected ? "▌" : " ");
    bar.setAttribute("aria-hidden", "true");
    row.appendChild(bar);

    // status glyph (●/○/·) only on rows that carry one — i.e. PROCESSES. Agents and
    // terminals render name-only and convey status by text color, exactly like the app
    // (src/app/view/sidebar.rs nav_row: badge() is Kind::Process-only).
    if (!r.launcher && r.glyph) {
      var glyph = el("span", "sb-glyph", r.glyph);
      glyph.setAttribute("aria-hidden", "true");
      row.appendChild(glyph);
    }

    row.appendChild(el("span", "sb-name", r.name || ""));

    if (r.sub) row.appendChild(el("span", "sb-sub", r.sub));

    // attention bell ● (red); hidden unless attention is set
    if (!r.launcher) {
      var dot = el("span", "sb-dot", "●");
      dot.setAttribute("aria-hidden", "true");
      if (!r.attention) dot.hidden = true;
      row.appendChild(dot);
    }
    return row;
  }

  function renderMain(t, main, state) {
    if (t.mainTitle) t.mainTitle.textContent = main.title || " mmux ";
    if (!t.mainScreen) return;

    t.mainScreen.textContent = "";
    if (main.placeholder) {
      // placeholder takes precedence over lines, rendered faint (§5.3)
      var ph = el("div", "screen-placeholder", main.placeholder);
      t.mainScreen.appendChild(ph);
      return;
    }
    var lines = main.lines || [];
    lines.forEach(function (line, i) {
      var row = el("div", "screen-line", line);
      // block cursor on the last line when requested and the pane is focused-ish
      if (main.cursor && i === lines.length - 1) {
        var cur = el("span", "screen-cursor", "▮");
        cur.setAttribute("aria-hidden", "true");
        row.appendChild(cur);
      }
      t.mainScreen.appendChild(row);
    });
  }

  function renderPanel(t, panel) {
    if (!t.panel) return;
    var visible = !!panel.visible;
    t.panel.hidden = !visible;
    if (!visible) return;

    if (t.panelTitle) t.panelTitle.textContent = " git ";
    if (t.panelScreen) {
      t.panelScreen.textContent = "";
      // branch line first, then any panel lines
      if (panel.branch) {
        var b = el("div", "panel-branch", panel.branch);
        t.panelScreen.appendChild(b);
      }
      (panel.lines || []).forEach(function (line) {
        t.panelScreen.appendChild(el("div", "panel-line", line));
      });
    }
  }

  function renderFooter(host, focus) {
    if (!host) return;
    host.textContent = FOOTERS[focus] || FOOTERS.sidebar;
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
    var dot = el("span", "toast-dot", "●");
    dot.setAttribute("aria-hidden", "true");
    host.appendChild(dot);
    host.appendChild(el("span", "toast-title", toast.title || ""));
    if (toast.body) host.appendChild(el("span", "toast-body", toast.body));
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
   * 2. Scroll driver (§6.1) — scrub SCENES across #demo.
   * ===================================================================== */
  var scrollDriver = (function () {
    var scenes = [];
    var demo, stage, captionHost;
    var active = false; // #demo in viewport?
    var rafQueued = false;
    var currentScene = -1;
    var revealTimer = null; // for typing/streaming reveals
    var sandbox = null; // set by init() — scene 8 hands off to it

    function init(opts) {
      scenes = (window.MMUX_SCENES && window.MMUX_SCENES.slice()) || [];
      sandbox = opts && opts.sandbox;
      demo = document.getElementById("demo");
      stage = $(".demo-stage", demo || document);
      captionHost = $(".demo-caption", demo || document);
      if (!demo || !stage) return; // nothing to drive

      buildCaptions();

      if (reducedMotion() || !scenes.length) {
        // §6.1: no scrubbing. Land on the last scene statically; captions stacked.
        renderStatic();
        return;
      }

      // IntersectionObserver gates the scroll handler to when #demo is visible (§6.1).
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
        block.appendChild(el("div", "caption-title", cap.title || ""));
        block.appendChild(el("div", "caption-body", cap.body || ""));
        captionHost.appendChild(block);
      });
    }

    // Reduced-motion / no-scenes fallback: stacked captions + static final state.
    function renderStatic() {
      if (captionHost) {
        captionHost.classList.add("demo-caption--static");
        // show all captions as a plain stacked list (remove transient visibility gating)
        Array.prototype.forEach.call(
          captionHost.querySelectorAll(".caption"),
          function (c) {
            c.classList.add("caption--visible");
          }
        );
      }
      var last = scenes.length
        ? scenes[scenes.length - 1].state
        : DEFAULT_STATE;
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

      // progress p∈[0,1] across the scrollable travel of #demo.
      var rect = demo.getBoundingClientRect();
      var travel = demo.offsetHeight - window.innerHeight;
      var scrolled = -rect.top;
      var p = travel > 0 ? clamp(scrolled / travel, 0, 1) : 0;

      var n = scenes.length;
      if (!n) return;
      // map p → scene index. Use floor with a clamp so the last scene holds.
      var idx = clamp(Math.floor(p * n), 0, n - 1);

      if (idx !== currentScene) {
        showScene(idx);
        currentScene = idx;
      }
    }

    function showScene(idx) {
      var sc = scenes[idx];
      if (!sc) return;

      // cross-fade captions: only the active one is --visible (§6.1)
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

      // Clear any in-flight reveal from the previous scene.
      if (revealTimer) {
        clearTimeout(revealTimer);
        revealTimer = null;
      }

      // Last scene = sandbox hand-off (§6.2). Make #tui playable.
      var isFinale = idx === scenes.length - 1;

      // Render the scene's target state, then layer short time-based reveals.
      var base = sc.state || {};
      var reveal = sc.type; // { target, text } hint (§7)

      if (reveal && reveal.text && !reducedMotion()) {
        runReveal(base, reveal);
      } else {
        renderTUI(base);
      }

      if (isFinale && sandbox) {
        sandbox.enable(base);
      } else if (sandbox) {
        sandbox.disable();
      }
    }

    /* Short (<=700ms) reveal: type into main, or stream lines progressively.
     * `type.target` "main" + short text → typing 'mmux'; otherwise line-stream. */
    function runReveal(base, reveal) {
      var DURATION = 650; // keep under the 700ms budget (§6.1)

      if (reveal.target === "main" && reveal.text && reveal.text.length <= 12) {
        // typing effect: reveal the text char-by-char into main.lines[0]
        var full = reveal.text;
        var chars = full.split("");
        var step = Math.max(40, Math.floor(DURATION / chars.length));
        var shown = 0;
        var typeNext = function () {
          shown++;
          var partial = full.slice(0, shown);
          var s = cloneState(base);
          s.main = s.main || {};
          // null the placeholder so the typed text actually renders (renderMain
          // honors lines only when placeholder is falsy, §5.3).
          s.main.placeholder = null;
          s.main.lines = [partial];
          s.main.cursor = true;
          renderTUI(s);
          if (shown < chars.length) {
            revealTimer = setTimeout(typeNext, step);
          }
        };
        // start from empty then type
        var s0 = cloneState(base);
        s0.main = s0.main || {};
        s0.main.placeholder = null;
        s0.main.lines = [""];
        s0.main.cursor = true;
        renderTUI(s0);
        revealTimer = setTimeout(typeNext, step);
        return;
      }

      // line-stream: reveal main.lines one at a time
      var target = (base.main && base.main.lines) || [];
      if (!target.length) {
        renderTUI(base);
        return;
      }
      var step2 = Math.max(80, Math.floor(DURATION / target.length));
      var count = 0;
      var streamNext = function () {
        count++;
        var s = cloneState(base);
        s.main = s.main || {};
        s.main.placeholder = null; // lines take effect only when placeholder is falsy
        s.main.lines = target.slice(0, count);
        renderTUI(s);
        if (count < target.length) {
          revealTimer = setTimeout(streamNext, step2);
        }
      };
      // start with first line, then stream the rest
      var s1 = cloneState(base);
      s1.main = s1.main || {};
      s1.main.placeholder = null;
      s1.main.lines = target.slice(0, 1);
      renderTUI(s1);
      count = 1;
      if (target.length > 1) revealTimer = setTimeout(streamNext, step2);
    }

    return { init: init };
  })();

  /* =====================================================================
   * 3. Sandbox driver (§6.2) — scene 8 makes #tui keyboard-playable.
   * ---------------------------------------------------------------------
   * Owns a live mutable state cloned from the finale scene. Traps keys only
   * while engaged (clicked/focused in); Esc or click-out releases.
   * ===================================================================== */
  var sandboxDriver = (function () {
    var state = null; // live, mutable
    var enabled = false; // scene 8 reached?
    var engaged = false; // keys trapped?
    var root = null;
    var hintEl = null;
    var seq = { claude: 1, terminal: 1, process: 1 }; // for "claude 2" naming

    function refs() {
      root = document.getElementById("tui");
      hintEl = root ? $(".tui-sandbox-hint", root) : null;
    }

    /* a11y: #tui is a decorative image until scene 8 (the HTML ships it that way — no
     * tabindex). When playable it becomes a focusable group; while engaged it's an
     * application so AT passes keystrokes through. The driver upgrades/downgrades it (§6.2). */
    function setA11y(mode) {
      if (!root) return;
      if (mode === "active") {
        root.setAttribute("tabindex", "0");
        root.setAttribute("role", "application");
        root.setAttribute("aria-label", "mmux sandbox — active. ↑↓ move, Enter open, x close, Escape to leave.");
      } else if (mode === "ready") {
        root.setAttribute("tabindex", "0");
        root.setAttribute("role", "group");
        root.setAttribute("aria-label", "mmux sandbox — interactive demo. focus it, then ↑↓ to move, Enter to open, Escape to leave.");
      } else {
        root.removeAttribute("tabindex");
        root.setAttribute("role", "img");
        root.setAttribute("aria-label", "a simulated mmux terminal session");
      }
    }

    // Called by the scroll driver when scene 8 becomes active.
    function enable(baseState) {
      refs();
      if (!root) return;
      if (!enabled) {
        enabled = true;
        state = cloneState(baseState || DEFAULT_STATE);
        // start in sidebar focus, not engaged — visitor must click in
        state.focus = "sidebar";
        renderTUI(state);
        attachListeners();
      }
      setA11y("ready"); // focusable + interactive label (re-applied on every re-enter)
      if (!engaged) showHint(true); // re-show the hint when scrolling back to the finale
    }

    // Reduced-motion path: enable immediately, playable, with hint shown.
    function enableStatic() {
      refs();
      if (!root) return;
      enabled = true;
      if (!state) state = cloneState(DEFAULT_STATE);
      state.focus = "sidebar";
      renderTUI(state);
      setA11y("ready");
      showHint(true);
      attachListeners();
    }

    function disable() {
      // leaving scene 8 by scrolling up: release the trap, hide hint, go decorative.
      if (engaged) release();
      if (hintEl) hintEl.hidden = true;
      setA11y("decorative");
    }

    function showHint(show) {
      if (!hintEl) return;
      hintEl.hidden = !show;
      if (show) hintEl.textContent = "your turn — click in to play";
    }

    var listenersAttached = false;
    function attachListeners() {
      if (listenersAttached || !root) return;
      listenersAttached = true;
      root.addEventListener("click", onTuiClick);
      root.addEventListener("focus", engage); // tab into it
      root.addEventListener("keydown", onKey);
      root.addEventListener("focusout", onFocusOut); // tab/blur out releases the trap
      document.addEventListener("click", onDocClick, true);
    }

    // Focus leaving #tui (e.g. Tab to the next page link) releases the trap so the
    // tui--engaged styling never stays stuck on a widget that lost the keyboard (§6.2).
    function onFocusOut(e) {
      if (!engaged) return;
      // ignore focus moving to a descendant of #tui (still inside the widget)
      if (root && e.relatedTarget && root.contains(e.relatedTarget)) return;
      release();
    }

    function onTuiClick(e) {
      if (!enabled) return;
      e.stopPropagation();
      engage();
    }

    function onDocClick(e) {
      if (!engaged) return;
      if (root && !root.contains(e.target)) release(); // click-out releases (§6.2)
    }

    function engage() {
      if (!enabled || engaged) return;
      engaged = true;
      root.classList.add("tui--engaged");
      if (hintEl) hintEl.hidden = true;
      setA11y("active");
      state.focus = "sandbox";
      renderTUI(state);
      try {
        root.focus({ preventScroll: true });
      } catch (_) {
        /* older browsers: focus() with no options */
        root.focus();
      }
    }

    function release() {
      if (!engaged) return;
      engaged = false;
      root.classList.remove("tui--engaged");
      setA11y("ready");
      state.focus = "sidebar";
      renderTUI(state);
      showHint(true);
    }

    /* --- flat list of selectable rows (sessions + launchers), in DOM order --- */
    function selectableRows() {
      var out = [];
      (state.sidebar || []).forEach(function (section) {
        (section.rows || []).forEach(function (r) {
          out.push(r);
        });
      });
      return out;
    }
    function selectedIndex(rows) {
      for (var i = 0; i < rows.length; i++) if (rows[i].selected) return i;
      return 0;
    }
    function selectRow(rows, idx) {
      rows.forEach(function (r) {
        r.selected = false;
      });
      if (rows[idx]) rows[idx].selected = true;
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
            selectRow(rows, (i - 1 + rows.length) % rows.length); // wrap (§6.2)
          } else handled = false;
          break;
        case "ArrowDown":
        case "j":
          if (state.focus === "sandbox" || state.focus === "sidebar") {
            selectRow(rows, (i + 1) % rows.length); // wrap
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
            state.focus = "sandbox"; // main → back to sidebar list (§6.2)
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
        // running session row → focus its pane (§6.2)
        state.focus = "main";
        state.main = mainFor(row);
      } else {
        // stopped/exited → start it (Enter to start)
        row.status = "running";
        if (row.glyph) row.glyph = "●"; // only process rows carry a glyph
        state.focus = "main";
        state.main = mainFor(row);
      }
    }

    // Launcher → append a new row of the matching kind, focus main, stream lines.
    function spawnFrom(launcher) {
      var kind, name, sub = null, lines;
      var section;

      if (/claude/i.test(launcher.name)) {
        kind = "AGENTS";
        var n = seq.claude++;
        name = n === 1 ? "claude" : "claude " + n;
        // avoid colliding with an existing "claude"
        if (n === 1 && hasRowNamed("claude")) name = "claude " + (n + 1);
        sub = "starting…";
        lines = ["⏵ launching claude", "✓ ready", "$ "];
      } else if (/terminal/i.test(launcher.name)) {
        kind = "TERMINAL";
        name = seq.terminal === 1 ? "zsh" : "zsh " + seq.terminal;
        seq.terminal++;
        lines = ["$ "];
      } else {
        kind = "PROCESSES";
        name = "dev server";
        sub = "listening :5173";
        lines = ["⏵ vite dev", "  ready in 240ms", "  http://localhost:5173"];
      }

      section = sectionByKind(kind);
      if (!section) return;
      var id = name.replace(/\s+/g, "-").toLowerCase() + "-" + Date.now();
      var newRow = {
        id: id,
        glyph: kind === "PROCESSES" ? "●" : undefined, // glyph only on processes
        name: name,
        sub: sub,
        status: "running",
        selected: false,
        attention: false,
        project: state.multiProject ? activeProjectName() : undefined,
      };
      // insert above the launcher within the section
      var li = section.rows.indexOf(launcher);
      if (li < 0) li = section.rows.length;
      section.rows.splice(li, 0, newRow);

      // select it, focus main, stream
      selectRow(selectableRows(), selectableRows().indexOf(newRow));
      state.focus = "main";
      state.main = {
        title: titleFor(newRow),
        lines: lines,
        placeholder: null,
        cursor: true,
      };
      streamMain(lines);
    }

    // x → close/stop the selected session row → stopped '·' (we keep the row).
    function closeRow(row) {
      if (!row || row.launcher) return;
      row.status = "stopped";
      if (row.glyph) row.glyph = "·"; // only process rows carry a glyph
      row.attention = false;
      if (state.focus === "main") {
        state.main = mainFor(row); // shows the "stopped" placeholder
        state.focus = "sandbox";
      }
    }

    /* --- main-pane helpers --- */
    function titleFor(row) {
      // the app's main_title for a live session is " name — status " with NO project
      // suffix — the · project suffix appears only on the + New launcher titles (pane.rs).
      return " " + row.name + " — " + row.status + " ";
    }
    function mainFor(row) {
      if (row.status !== "running") {
        return {
          title: titleFor(row),
          lines: [],
          // app uses a blank-line break, not a double space (pane.rs placeholder_text).
          placeholder: row.name + " is stopped.\n\nPress Enter or 's' to start it.",
          cursor: false,
        };
      }
      return {
        title: titleFor(row),
        lines: ["$ "],
        placeholder: null,
        cursor: true,
      };
    }

    // progressive line stream into main (<=700ms; only while engaged & on this row)
    var streamTimer = null;
    function streamMain(lines) {
      if (streamTimer) clearTimeout(streamTimer);
      var step = Math.max(90, Math.floor(600 / Math.max(1, lines.length)));
      var count = 1;
      state.main.lines = lines.slice(0, count);
      renderTUI(state);
      var next = function () {
        count++;
        if (!engaged) return; // bail if visitor left
        state.main.lines = lines.slice(0, count);
        renderTUI(state);
        if (count < lines.length) streamTimer = setTimeout(next, step);
      };
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
  // Deep-ish clone of a state object (plain data only; structuredClone if present).
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
   * Copy buttons (§8): button.copy[data-copy] → clipboard, flash 'copied'.
   * ===================================================================== */
  function wireCopyButtons() {
    var buttons = document.querySelectorAll("button.copy[data-copy]");
    Array.prototype.forEach.call(buttons, function (btn) {
      btn.addEventListener("click", function () {
        var text = btn.getAttribute("data-copy") || "";
        copyText(text).then(function () {
          flash(btn);
        }, function () {
          flash(btn); // still flash; nothing else we can do offline
        });
      });
    });
  }
  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text);
    }
    // textarea fallback (file://, older browsers)
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
    btn.textContent = "[copied]";
    btn.classList.add("copy--copied");
    setTimeout(function () {
      btn.textContent = prev;
      btn.classList.remove("copy--copied");
      btn.dataset.flashing = "0";
    }, 1000);
  }

  /* =====================================================================
   * Smooth-scroll nav anchors (§8). Respect reduced-motion.
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
    // Render an initial state so the TUI is never blank before scroll fires.
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

  /* Single global namespace (§9): renderer + state contract, for scenes/debug. */
  window.MMUX = {
    renderTUI: renderTUI,
    DEFAULT_STATE: DEFAULT_STATE,
    FOOTERS: FOOTERS,
  };
})();
