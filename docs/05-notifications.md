# Notifications

When a session needs you, mmux raises a **native desktop notification** — and it works over SSH.
This is on by default.

## What Triggers a Notification

mmux watches each program's output for two kinds of attention signal:

- the terminal **bell** (`BEL`) — for example, Claude Code emits one via
  `preferredNotifChannel terminal_bell`;
- a program-emitted **notification escape**: `OSC 9`, `OSC 777`, or (best-effort) kitty's
  `OSC 99`.

Either one emits a desktop notification. A rich escape (with a title/body) carries that text
through; a bare bell shows `<session> — needs attention`. No notification fires for the pane you
are currently looking at (see `only_when_unfocused`).

The bell also lights a **terminal** or **process** row **green** until you focus it. **Agent** rows
use a different, more reliable cue — they go green when their terminal title goes quiet (i.e. the
agent stopped working), independent of the bell. See
[Status and Attention](03-usage.md#status-and-attention).

## How It Reaches Your Desktop — Even Over SSH

The notification is delivered as a **terminal escape sequence** written to mmux's own output
stream. Your terminal emulator sees it and renders the popup. Because it's just bytes on the
normal output stream, it works **over SSH**: the popup lands on whatever machine your terminal is
running on, not on the remote box where mmux lives.

mmux's tmux session is configured with `allow-passthrough on`, and inside tmux the escape is
wrapped in a DCS passthrough so it survives the jail and reaches the outer terminal.

> The one exception is `mechanism: command`, which runs a local program — that cannot cross an
> SSH hop.

## Configuration

```yaml
notifications:
  enabled: true
  mechanism: osc9          # how to deliver the popup (see below)
  only_when_unfocused: true # don't notify for the pane you're already looking at
  throttle_secs: 5         # min seconds between popups from the same session
  # command: 'terminal-notifier -title "$MMUX_NOTIFY_TITLE" -message "$MMUX_NOTIFY_BODY"'
```

| Field | Default | Notes |
| --- | --- | --- |
| `enabled` | `true` | Master switch. |
| `mechanism` | `osc9` | Delivery method — see the table below. |
| `only_when_unfocused` | `true` | Suppress notifications for the session you're currently focused on. |
| `throttle_secs` | `5` | Minimum seconds between popups from the same session. |
| `command` | *(unset)* | Shell command for `mechanism: command`. |

Notification settings are read from the **launch directory's** config only. In a workspace
manifest that means the manifest config, not an individual member project's override. As with the
other whole-block fields, a project or manifest that sets `notifications:` replaces the global
block entirely.

## Mechanisms

`osc9` is the default because it has the widest reach. Switch if your terminal isn't covered:

| `mechanism` | Escape | Terminals (all work over SSH) |
| --- | --- | --- |
| `osc9` | `OSC 9` | iTerm2, kitty, ghostty, WezTerm (default) |
| `osc777` | `OSC 777` | ghostty, foot, WezTerm, urxvt/VTE |
| `bell` | `BEL` | anything that notifies on a bell |
| `command` | runs a command | local only — can't cross an SSH hop |

`osc9` carries a single message with no separate title, so mmux folds the session name into the
body (`Claude — needs attention`). iTerm2 and kitty understand **only** `osc9`; `osc777` adds a
bold title on terminals that support it.

## The `command` Mechanism

With `mechanism: command`, the command runs through the shell with the notification available as
`$MMUX_NOTIFY_TITLE` and `$MMUX_NOTIFY_BODY`. If `command` is left unset, mmux falls back to
`osascript` on macOS and `notify-send` on Linux. This path is **local only** — use an OSC
mechanism if you need notifications to reach you over SSH.
