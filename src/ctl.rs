//! The command side of the [control socket](crate::control): `mmux ls`, `status`,
//! `read`, `last`, `send`, `keys`, `new`, `start`, `stop`, `restart`, `close`, `wait`,
//! `ask`, `worktree new|rm`. Each parses its arguments, finds the right running session
//! ([`crate::control::locate`]), sends its request(s) and prints the answer — as
//! text for people, or JSON with `--json` for scripts and agents.
//!
//! Three verbs do more than one round trip, all on this side of the socket so the UI
//! thread never blocks or does file IO for them: `last` reads the agent's transcript
//! itself (the server only says where it is), `wait` polls `status`, and `ask` chains
//! `new`/`send` → `wait` → `last`.

use crate::agent::Turn;
use crate::control::{
    self, Cmd, Done, LastInfo, Listing, NewKind, ReadOut, Reply, Request, Response, SessionInfo,
    StatusInfo, WorktreeDone,
};
use anyhow::{bail, Result};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// How often `wait` asks for a session's state.
const POLL_EVERY: Duration = Duration::from_millis(250);
/// `wait`'s default: an agent quiet this long after working counts as done.
const DEFAULT_SETTLE: Duration = Duration::from_millis(1500);
/// `wait`/`ask` give up after this long by default.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
/// Input the agent never visibly started on (it answered too fast to be seen working,
/// or it ignored it) stops holding `wait` up after this long.
const INPUT_GRACE: Duration = Duration::from_secs(20);
/// How long a Claude agent's transcript may hold `wait` up after the screen went
/// quiet — past this, a turn that still looks open (an unrecognized record) yields to
/// the working signal rather than to the timeout.
const TRANSCRIPT_GRACE: Duration = Duration::from_secs(30);
/// Exit status for a `wait`/`ask` that ran out of time (errors are 1).
const EXIT_TIMEOUT: i32 = 2;

/// The verbs this module owns — checked by [`crate::cli`] before anything else.
pub const VERBS: &[&str] = &[
    "ls", "status", "read", "last", "send", "keys", "new", "start", "stop", "restart", "close",
    "wait", "ask", "worktree",
];

/// Whether `arg` is a control verb.
pub fn is_verb(arg: &str) -> bool {
    VERBS.contains(&arg)
}

/// Parsed command line: positionals plus the flags any verb may take.
#[derive(Default, Debug, PartialEq)]
struct Args {
    verb: String,
    pos: Vec<String>,
    json: bool,
    dir: Option<PathBuf>,
    lines: Option<usize>,
    no_enter: bool,
    force: bool,
    project: Option<String>,
    command: Option<String>,
    prompt: Option<String>,
    help: bool,
    /// `wait --exit`: until the session ends, not until it goes idle.
    exit: bool,
    timeout: Option<Duration>,
    settle: Option<Duration>,
    /// `ask --to <t>`: an existing agent instead of a new one.
    to: Option<String>,
    /// `ask`/`worktree new --agent <template>`.
    agent: Option<String>,
    /// `ask --close`: close the agent once it has answered.
    close: bool,
}

fn parse(args: &[String]) -> Result<Args> {
    let mut out = Args {
        verb: args.first().cloned().unwrap_or_default(),
        ..Args::default()
    };
    let mut it = args.iter().skip(1);
    let mut flags = true;
    while let Some(a) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))
        };
        match a.as_str() {
            "--" if flags => flags = false,
            "--json" if flags => out.json = true,
            "-C" | "--dir" if flags => out.dir = Some(PathBuf::from(value(a)?)),
            "-n" | "--lines" if flags => {
                let v = value(a)?;
                out.lines = Some(
                    v.parse()
                        .map_err(|_| anyhow::anyhow!("bad line count `{v}`"))?,
                );
            }
            "--no-enter" if flags => out.no_enter = true,
            "-f" | "--force" if flags => out.force = true,
            "-p" | "--project" if flags => out.project = Some(value(a)?),
            "--cmd" if flags => out.command = Some(value(a)?),
            "--prompt" if flags => out.prompt = Some(value(a)?),
            "--idle" if flags => out.exit = false,
            "--exit" if flags => out.exit = true,
            "-t" | "--timeout" if flags => out.timeout = Some(duration(&value(a)?)?),
            "--settle" if flags => out.settle = Some(duration(&value(a)?)?),
            "--to" if flags => out.to = Some(value(a)?),
            "--agent" if flags => out.agent = Some(value(a)?),
            "--close" if flags => out.close = true,
            "-h" | "--help" if flags => out.help = true,
            _ => out.pos.push(a.clone()),
        }
    }
    Ok(out)
}

/// Run a control verb (`args[0]`) and exit non-zero on failure.
pub fn run(args: &[String]) -> Result<()> {
    let a = parse(args).unwrap_or_else(|e| {
        // A flag short of its value: still answer in JSON when it was asked for.
        let json = args.iter().any(|x| x == "--json");
        fail(
            &Args {
                json,
                ..Args::default()
            },
            e.to_string(),
        )
    });
    if a.help {
        print_help();
        return Ok(());
    }
    // The multi-step verbs check their own arguments; everything else is one request.
    let cmd = match a.verb.as_str() {
        "last" | "wait" | "ask" => None,
        // Through `fail`, so a usage error is still JSON under `--json`.
        _ => Some(build(&a).unwrap_or_else(|e| fail(&a, e.to_string()))),
    };
    if a.verb == "ask" && a.pos.is_empty() {
        fail(
            &a,
            "`mmux ask` needs a prompt (or `-` to read it from stdin)".into(),
        );
    }
    if matches!(a.verb.as_str(), "last" | "wait") && a.pos.is_empty() {
        fail(
            &a,
            format!(
                "`mmux {}` needs a target — an id (s3), a name (\"Claude #1\"), project/name, or self",
                a.verb
            ),
        );
    }
    let sock = match control::locate(a.dir.as_deref()) {
        Ok(s) => s,
        Err(e) => fail(&a, e),
    };
    let ctx = Ctx::new(sock, a.dir.as_deref());
    let Some(cmd) = cmd else {
        return match a.verb.as_str() {
            "last" => run_last(&a, &ctx),
            "wait" => run_wait(&a, &ctx),
            _ => run_ask(&a, &ctx),
        };
    };
    let resp = match control::call(&ctx.sock, &ctx.request(cmd)) {
        Ok(r) => r,
        Err(e) => fail(&a, e),
    };
    if a.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        if !resp.ok {
            std::process::exit(1);
        }
        return Ok(());
    }
    if !resp.ok {
        fail(&a, resp.error.unwrap_or_else(|| "failed".into()));
    }
    print_human(&a, resp)
}

/// Turn the parsed command line into the request's verb.
fn build(a: &Args) -> Result<Cmd> {
    let target = || -> Result<String> {
        match a.pos.first() {
            Some(t) => Ok(t.clone()),
            None => bail!("`mmux {}` needs a target — an id (s3), a name (\"Claude #1\"), project/name, or self", a.verb),
        }
    };
    Ok(match a.verb.as_str() {
        "ls" => Cmd::Ls,
        "status" => Cmd::Status { target: target()? },
        "read" => Cmd::Read {
            target: target()?,
            lines: a.lines,
        },
        "send" => {
            let mut text = a.pos[1.min(a.pos.len())..].join(" ");
            // `-` reads the text from stdin — the way to send a multi-line prompt.
            if text == "-" {
                text = stdin_text()?;
            }
            Cmd::Send {
                target: target()?,
                text,
                enter: !a.no_enter,
            }
        }
        "keys" => {
            if a.pos.len() < 2 {
                bail!(
                    "`mmux keys` needs a target and at least one key (Enter, C-c, Escape, Up, …)"
                );
            }
            Cmd::Keys {
                target: target()?,
                keys: a.pos[1..].to_vec(),
            }
        }
        "new" => {
            let kind = match a.pos.first().map(String::as_str) {
                Some("agent") => NewKind::Agent,
                Some("terminal" | "term") => NewKind::Terminal,
                _ => bail!("use `mmux new agent [template]` or `mmux new terminal [--cmd …]`"),
            };
            let prompt = match a.prompt.as_deref() {
                Some("-") => Some(stdin_text()?),
                other => other.map(str::to_string),
            };
            Cmd::New {
                kind,
                template: a.pos.get(1).cloned(),
                project: a.project.clone(),
                command: a.command.clone(),
                prompt,
            }
        }
        "start" => Cmd::Start { target: target()? },
        "stop" => Cmd::Stop {
            target: target()?,
            force: a.force,
        },
        "restart" => Cmd::Restart { target: target()? },
        "close" => Cmd::Close {
            target: target()?,
            force: a.force,
        },
        "worktree" => match a.pos.first().map(String::as_str) {
            Some("new" | "add") => {
                if a.command.is_some() {
                    bail!("--cmd is for terminals — start an agent in the worktree with --agent/--prompt");
                }
                let prompt = match a.prompt.as_deref() {
                    Some("-") => Some(stdin_text()?),
                    other => other.map(str::to_string),
                };
                Cmd::WorktreeNew {
                    branch: a.pos.get(1).cloned(),
                    project: a.project.clone(),
                    agent: a.agent.clone(),
                    prompt,
                }
            }
            Some("rm" | "remove") => match a.pos.get(1) {
                Some(t) => Cmd::WorktreeRm {
                    target: t.clone(),
                    force: a.force,
                },
                None => bail!("`mmux worktree rm` needs the worktree's branch (see `mmux ls`)"),
            },
            _ => bail!("use `mmux worktree new [branch]` or `mmux worktree rm <branch>`"),
        },
        other => bail!("unknown command `{other}`"),
    })
}

/// All of stdin, minus trailing newlines — a `-` prompt.
fn stdin_text() -> Result<String> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let trimmed = text.trim_end_matches('\n').len();
    text.truncate(trimmed);
    Ok(text)
}

/// Report an error the way the caller asked for output, and exit 1.
fn fail(a: &Args, msg: String) -> ! {
    fail_with(a, msg, 1)
}

/// [`fail`], with a chosen exit status (a timeout is 2).
fn fail_with(a: &Args, msg: String, code: i32) -> ! {
    if a.json {
        let resp = Response::err(msg);
        println!(
            "{}",
            serde_json::to_string_pretty(&resp).unwrap_or_default()
        );
    } else {
        eprintln!("mmux: {msg}");
    }
    std::process::exit(code);
}

/// Everything a request carries besides its verb, fixed for the whole command.
struct Ctx {
    sock: PathBuf,
    caller: Option<String>,
    cwd: Option<String>,
    depth: u32,
}

impl Ctx {
    fn new(sock: PathBuf, dir: Option<&Path>) -> Ctx {
        // Only a caller talking to *its own* mmux names itself: an `s3` means nothing
        // in another session, and would resolve `self` to a stranger there.
        let own = std::env::var_os("MMUX_SOCKET").is_some_and(|s| PathBuf::from(s) == sock);
        // `-C <dir>` stands in for the caller's directory, so a bare name or `new`
        // resolves in the project it points at. Made absolute here: the server would
        // resolve a relative path against its own cwd.
        let cwd = std::env::current_dir().ok().map(|here| match dir {
            Some(d) => here.join(d),
            None => here,
        });
        Ctx {
            caller: own
                .then(|| std::env::var("MMUX_SESSION").ok())
                .flatten()
                .filter(|s| !s.is_empty()),
            cwd: cwd.map(|d| d.to_string_lossy().into_owned()),
            depth: std::env::var("MMUX_DEPTH")
                .ok()
                .and_then(|d| d.parse().ok())
                .unwrap_or(0),
            sock,
        }
    }

    fn request(&self, cmd: Cmd) -> Request {
        Request {
            caller: self.caller.clone(),
            cwd: self.cwd.clone(),
            depth: self.depth,
            cmd,
        }
    }

    /// One round trip; a refusal comes back as `Err` with mmux's own message.
    fn call(&self, cmd: Cmd) -> Result<Value, String> {
        let resp = control::call(&self.sock, &self.request(cmd))?;
        match resp.ok {
            true => Ok(resp.data),
            false => Err(resp.error.unwrap_or_else(|| "failed".into())),
        }
    }

    fn status(&self, target: &str) -> Result<StatusInfo, String> {
        let data = self.call(Cmd::Status {
            target: target.to_string(),
        })?;
        serde_json::from_value(data).map_err(|e| e.to_string())
    }

    /// The agent's last reply: from its transcript when mmux knows which one, else
    /// its screen.
    fn last(&self, target: &str) -> Result<Reply, String> {
        let data = self.call(Cmd::Last {
            target: target.to_string(),
        })?;
        let info: LastInfo = serde_json::from_value(data).map_err(|e| e.to_string())?;
        let transcript = match (info.tool, info.session_id.as_deref(), &info.transcripts) {
            (Some(tool), Some(id), Some(root)) => {
                crate::agent::last_reply(tool, id, Path::new(&info.cwd), Path::new(root))
            }
            _ => None,
        };
        let (reply, source) = match transcript {
            Some(text) => (text, "transcript"),
            None => (info.screen, "screen"),
        };
        Ok(Reply {
            id: info.id,
            name: info.name,
            reply,
            source: source.to_string(),
        })
    }
}

/// How a `wait` ended.
enum WaitEnd {
    /// The condition held; the session's last state (`None`: it's gone).
    Done(Option<StatusInfo>),
    TimedOut(Option<StatusInfo>),
}

/// Whether an agent has finished with what it was given: quiet (not working, nothing
/// queued for it) for at least `settle`, and it either worked since the last input,
/// was never sent any, or ignored it for longer than [`INPUT_GRACE`]. A session that
/// is no longer running is finished too — it won't do anything more.
fn is_idle(s: &SessionInfo, settle: Duration) -> bool {
    if s.status != "running" {
        return true;
    }
    let quiet =
        !s.working && !s.input_pending && s.idle_for_ms.unwrap_or(0) >= settle.as_millis() as u64;
    let answered = match s.input_age_ms {
        None => true,
        Some(age) => s.worked_since_input || age >= INPUT_GRACE.as_millis() as u64,
    };
    quiet && answered
}

/// The transcript half of "done", for a Claude agent that was sent input: its record
/// of the turn must have come to rest too (see [`crate::agent::Turn`]). The working
/// signal alone can be fooled — a boot-time title flash reads as "worked since the
/// input" before a launch-argument prompt is even underway. Anything that can't be
/// judged (another tool, no transcript tree, no input) passes, as does an agent quiet
/// for [`TRANSCRIPT_GRACE`]. Only called once [`is_idle`] holds, so the file is read
/// only then.
fn turn_settled(info: Option<&LastInfo>, s: &SessionInfo) -> bool {
    let Some(info) = info else { return true };
    let (Some(crate::agent::Tool::Claude), Some(id), Some(root), Some(age)) = (
        info.tool,
        info.session_id.as_deref(),
        info.transcripts.as_deref(),
        s.input_age_ms,
    ) else {
        return true;
    };
    if s.status != "running" || s.idle_for_ms.unwrap_or(0) >= TRANSCRIPT_GRACE.as_millis() as u64 {
        return true;
    }
    let Some(since) = SystemTime::now().checked_sub(Duration::from_millis(age)) else {
        return true;
    };
    match crate::agent::claude_turn(id, Path::new(&info.cwd), Path::new(root), since) {
        Turn::Unknown | Turn::Settled => true,
        // Nothing recorded since: not started yet — or the input was no prompt at all.
        Turn::Pending => age >= INPUT_GRACE.as_millis() as u64,
        Turn::Open => false,
    }
}

/// Poll `target` until it is idle (or, with `exit`, until it stops running or goes
/// away), or `timeout` passes. The target is pinned to its id first, so a name can't
/// drift to another session midway.
fn wait_for(
    ctx: &Ctx,
    target: &str,
    exit: bool,
    timeout: Duration,
    settle: Duration,
) -> Result<WaitEnd, String> {
    // An absurd `-t` overflows the clock: that is simply no deadline.
    let deadline = Instant::now().checked_add(timeout);
    let first = ctx.status(target)?;
    let id = first.session.id.clone();
    // Where an agent's transcript is: fixed for the session, so asked once. Only a
    // Claude agent's is read (see `turn_settled`); anything else ignores it.
    let transcript = match (exit, first.session.kind.as_str()) {
        (false, "agent") => ctx
            .call(Cmd::Last { target: id.clone() })
            .ok()
            .and_then(|d| serde_json::from_value::<LastInfo>(d).ok()),
        _ => None,
    };
    let mut last = Some(first);
    loop {
        if let Some(s) = &last {
            let done = match exit {
                true => s.session.status != "running",
                false => {
                    is_idle(&s.session, settle) && turn_settled(transcript.as_ref(), &s.session)
                }
            };
            if done {
                return Ok(WaitEnd::Done(last));
            }
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Ok(WaitEnd::TimedOut(last));
        }
        std::thread::sleep(POLL_EVERY);
        last = match ctx.status(&id) {
            Ok(s) => Some(s),
            // Closed, or mmux itself went away: for `--exit` that is the answer.
            Err(_) if exit => return Ok(WaitEnd::Done(None)),
            Err(e) => return Err(format!("{id}: {e}")),
        };
    }
}

fn run_last(a: &Args, ctx: &Ctx) -> Result<()> {
    let r = match ctx.last(&a.pos[0]) {
        Ok(r) => r,
        Err(e) => fail(a, e),
    };
    match a.json {
        true => print_json(&Response::ok(serde_json::to_value(&r)?)),
        false => println!("{}", r.reply),
    }
    Ok(())
}

fn run_wait(a: &Args, ctx: &Ctx) -> Result<()> {
    let timeout = a.timeout.unwrap_or(DEFAULT_TIMEOUT);
    let settle = a.settle.unwrap_or(DEFAULT_SETTLE);
    let (state, timed_out) = match wait_for(ctx, &a.pos[0], a.exit, timeout, settle) {
        Ok(WaitEnd::Done(s)) => (s, false),
        Ok(WaitEnd::TimedOut(s)) => (s, true),
        Err(e) => fail(a, e),
    };
    if timed_out {
        let who = state
            .as_ref()
            .map(|s| format!("{} {}", s.session.id, s.session.name))
            .unwrap_or_else(|| a.pos[0].clone());
        fail_with(
            a,
            format!("timed out after {} waiting on {who}", human(timeout)),
            EXIT_TIMEOUT,
        );
    }
    match (a.json, state) {
        (true, Some(s)) => print_json(&Response::ok(serde_json::to_value(&s)?)),
        (true, None) => print_json(&Response::ok(serde_json::json!({ "status": "gone" }))),
        (false, Some(s)) => println!("{} {} — {}", s.session.id, s.session.name, state_word(&s)),
        (false, None) => println!("{} — gone", a.pos[0]),
    }
    Ok(())
}

/// `claude -p`, but visible: start (or reuse) an agent, hand it the prompt, wait for it
/// to finish, print its reply. The agent stays in the sidebar — watch it, take over,
/// or `ask --to` it again — unless `--close`.
fn run_ask(a: &Args, ctx: &Ctx) -> Result<()> {
    let mut prompt = a.pos.join(" ");
    if prompt == "-" {
        prompt = stdin_text()?;
    }
    if prompt.trim().is_empty() {
        fail(a, "the prompt is empty".into());
    }
    // Typing a prompt into a shell would run it: check before sending anything, and
    // send to the id that check saw, so a name can't resolve differently in between.
    let to = a.to.as_ref().map(|target| match ctx.status(target) {
        Ok(s) if s.session.kind == "agent" => s.session.id,
        Ok(s) => fail(
            a,
            format!("{} {} is not an agent", s.session.id, s.session.name),
        ),
        Err(e) => fail(a, e),
    });
    let started = match &to {
        Some(target) => ctx.call(Cmd::Send {
            target: target.clone(),
            text: prompt,
            enter: true,
        }),
        None => ctx.call(Cmd::New {
            kind: NewKind::Agent,
            template: a.agent.clone(),
            project: a.project.clone(),
            command: None,
            prompt: Some(prompt),
        }),
    };
    let done: Done =
        match started.and_then(|d| serde_json::from_value(d).map_err(|e| e.to_string())) {
            Ok(d) => d,
            Err(e) => fail(a, e),
        };
    let id = done.session.id.clone();
    if done.session.kind != "agent" {
        fail(a, format!("{id} {} is not an agent", done.session.name));
    }
    eprintln!(
        "mmux: asked {id} {} — waiting for its reply",
        done.session.name
    );
    let timeout = a.timeout.unwrap_or(DEFAULT_TIMEOUT);
    let settle = a.settle.unwrap_or(DEFAULT_SETTLE);
    match wait_for(ctx, &id, false, timeout, settle) {
        Ok(WaitEnd::Done(_)) => {}
        Ok(WaitEnd::TimedOut(_)) => fail_with(
            a,
            format!(
                "{id} is still working after {} — `mmux wait {id}` then `mmux last {id}`",
                human(timeout)
            ),
            EXIT_TIMEOUT,
        ),
        Err(e) => fail(a, e),
    }
    let reply = match ctx.last(&id) {
        Ok(r) => r,
        Err(e) => fail(a, e),
    };
    if a.close {
        if let Err(e) = ctx.call(Cmd::Close {
            target: id.clone(),
            force: true,
        }) {
            eprintln!("mmux: couldn't close {id}: {e}");
        }
    }
    match a.json {
        true => print_json(&Response::ok(serde_json::to_value(&reply)?)),
        false => println!("{}", reply.reply),
    }
    Ok(())
}

fn print_json(resp: &Response) {
    println!("{}", serde_json::to_string_pretty(resp).unwrap_or_default());
}

/// Parse a CLI duration: `500ms`, `1.5s`, `90s`, `10m`, `2h`, or a bare number of
/// seconds.
fn duration(raw: &str) -> Result<Duration> {
    let raw = raw.trim().to_lowercase();
    let split = raw
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(raw.len());
    let (num, unit) = raw.split_at(split);
    let n: f64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("bad duration `{raw}` — try 90s, 10m, 1.5s"))?;
    let secs = match unit.trim() {
        "ms" => n / 1000.0,
        "" | "s" | "sec" | "secs" => n,
        "m" | "min" | "mins" => n * 60.0,
        "h" | "hr" | "hrs" => n * 3600.0,
        _ => bail!("bad duration `{raw}` — try 90s, 10m, 1.5s"),
    };
    if !secs.is_finite() || secs < 0.0 {
        bail!("bad duration `{raw}`");
    }
    Duration::try_from_secs_f64(secs).map_err(|_| anyhow::anyhow!("bad duration `{raw}`"))
}

/// A duration the way a person would say it: `90s`, `10m`, `1h30m`.
fn human(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        s if s < 90 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => match (s / 3600, s % 3600 / 60) {
            (h, 0) => format!("{h}h"),
            (h, m) => format!("{h}h{m}m"),
        },
    }
}

fn print_human(a: &Args, resp: Response) -> Result<()> {
    match a.verb.as_str() {
        "ls" => {
            let l: Listing = serde_json::from_value(resp.data)?;
            println!("{} · {}", l.name, l.root);
            for p in &l.projects {
                let mut head = format!("\n{}  {}", p.name, p.dir);
                if p.active {
                    head.push_str("  (in view)");
                }
                println!("{head}");
                if !p.agents.is_empty() {
                    println!("  new agent: {}", p.agents.join(", "));
                }
                for s in l.sessions.iter().filter(|s| s.project_dir == p.dir) {
                    println!("  {}", row(s));
                }
            }
        }
        "status" => {
            let s: StatusInfo = serde_json::from_value(resp.data)?;
            let i = &s.session;
            println!("{} {} ({} · {})", i.id, i.name, i.kind, i.project);
            println!("state: {}", state_word(&s));
            if let Some(t) = &i.title {
                println!("title: {t}");
            }
            if let Some(e) = &i.error {
                println!("error: {e}");
            }
            if !s.status_line.is_empty() {
                println!("---");
                for l in &s.status_line {
                    println!("{l}");
                }
            }
        }
        "read" => {
            let r: ReadOut = serde_json::from_value(resp.data)?;
            println!("{}", r.text);
        }
        "worktree" => {
            let d: WorktreeDone = serde_json::from_value(resp.data)?;
            println!("{}", d.message);
            if matches!(a.pos.first().map(String::as_str), Some("new" | "add")) {
                println!("project: {}  {}", d.project.name, d.project.dir);
            }
            if let Some(s) = &d.agent {
                println!("agent: {} {}", s.id, s.name);
            }
        }
        _ => {
            let d: Done = serde_json::from_value(resp.data)?;
            println!("{} {} — {}", d.session.id, d.session.name, d.message);
        }
    }
    Ok(())
}

/// One `ls` row: id, kind, name, state, title.
fn row(s: &SessionInfo) -> String {
    let mut title: String = s.title.clone().unwrap_or_default();
    if title.chars().count() > 60 {
        title = title.chars().take(59).collect::<String>() + "…";
    }
    format!(
        "{:<5} {:<8} {:<20} {:<9} {}",
        s.id,
        s.kind,
        s.name,
        state(s),
        title
    )
    .trim_end()
    .to_string()
}

/// A session's state, with how long an agent has been quiet: `idle 42s`.
fn state_word(s: &StatusInfo) -> String {
    let i = &s.session;
    let word = state(i);
    match (i.kind.as_str(), i.working, i.idle_for_ms) {
        ("agent", false, Some(ms)) if i.status == "running" => {
            format!("{word} {}", human(Duration::from_millis(ms)))
        }
        _ => word,
    }
}

/// A session's state in one word: an agent is `working` or `idle`; `!` marks one
/// asking for attention.
fn state(s: &SessionInfo) -> String {
    let word = match (s.status.as_str(), s.kind.as_str()) {
        ("running", "agent") if s.working => "working",
        ("running", "agent") => "idle",
        (status, _) => status,
    };
    match s.attention {
        true => format!("{word}!"),
        false => word.to_string(),
    }
}

pub fn print_help() {
    println!(
        r#"mmux control — drive a running mmux from scripts and agents

Every command finds the mmux session for the current directory (a project, a
workspace member, or a worktree), or the one it runs inside ($MMUX_SOCKET).
-C <dir> picks another. --json prints the raw response instead of text.

    mmux ls                         Projects and sessions (id, kind, name, state, title)
    mmux status <t>                 One session: state, title, last lines of its screen
    mmux read <t> [-n N]            Last N lines of output (default 200, 0 = all)
    mmux send <t> <text…>           Type text, then press Enter (--no-enter: don't).
                                    `-` as the text reads it from stdin.
    mmux keys <t> <key>…            Press keys: Enter Escape Tab BTab BSpace Space
                                    Up Down Left Right Home End PageUp PageDown
                                    Delete Insert F1-F12, C-x (Ctrl), M-x (Alt),
                                    S-x (Shift). Other words are typed.
    mmux new agent [template] [-p project] [--prompt "<first prompt>"]
    mmux new terminal [-p project] [--cmd "<command>"]
    mmux start <t>                  Start a session that isn't running (processes too)
    mmux restart <t>                (Re)start a session whether or not it's running
    mmux stop <t> [--force]         Stop a process in place (runs its stop: command);
                                    on an agent/terminal, the same as close
    mmux close <t> [--force]        Close an agent/terminal (refused while busy
                                    unless --force); stop a process
    mmux last <t>                   An agent's last reply (Claude/Codex: from its
                                    transcript; others: the end of its screen)
    mmux wait <t> [--idle|--exit] [-t 10m] [--settle 1.5s]
                                    Block until the agent has finished working on
                                    what it was sent (--idle, the default), or
                                    with --exit until it ends. Exit 2 on timeout.
    mmux ask [--agent <template>] [-p project] [--to <t>] [-t 10m] [--close] <prompt…>
                                    Start an agent (or use --to), give it the
                                    prompt, wait, print its reply. The agent stays
                                    in the sidebar unless --close. `-` = stdin.
    mmux worktree new [branch] [-p project] [--agent <template>] [--prompt "…"]
                                    Cut a git worktree (a generated branch name if
                                    none) and open it as a project; with --agent or
                                    --prompt, start an agent in it. Address it later
                                    by its branch: -p <branch>, <branch>/<name>.
    mmux worktree rm <branch> [--force]
                                    Remove a worktree: its sessions close, the
                                    checkout goes, the branch is deleted only if
                                    merged. Refused while it has uncommitted changes,
                                    an agent at work, or is in view, unless --force
                                    (which discards uncommitted changes).

Targets <t>: an id (s12), a name ("Claude #2", claude#2, or a unique prefix),
project/name, or `self` (the pane you run in). Programs inside mmux get
MMUX_SOCKET, MMUX_SESSION, MMUX_PROJECT and MMUX_DEPTH in their environment;
detected agents are also told at launch that they run inside mmux."#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_flags_anywhere_and_keeps_positionals_in_order() {
        let a = parse(&args(&[
            "read",
            "--json",
            "Claude #1",
            "-n",
            "50",
            "-C",
            "/tmp",
        ]))
        .unwrap();
        assert!(a.json);
        assert_eq!(a.pos, vec!["Claude #1"]);
        assert_eq!(a.lines, Some(50));
        assert_eq!(a.dir, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn double_dash_lets_text_start_with_a_dash() {
        let a = parse(&args(&["send", "s3", "--", "--help", "me"])).unwrap();
        assert!(!a.help);
        assert_eq!(a.pos, vec!["s3", "--help", "me"]);
    }

    #[test]
    fn send_joins_the_rest_as_text() {
        let a = parse(&args(&["send", "s3", "fix", "the", "tests", "--no-enter"])).unwrap();
        match build(&a).unwrap() {
            Cmd::Send {
                target,
                text,
                enter,
            } => {
                assert_eq!(target, "s3");
                assert_eq!(text, "fix the tests");
                assert!(!enter);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn new_takes_kind_template_and_project() {
        let a = parse(&args(&["new", "agent", "codex", "-p", "api"])).unwrap();
        assert_eq!(
            build(&a).unwrap(),
            Cmd::New {
                kind: NewKind::Agent,
                template: Some("codex".into()),
                project: Some("api".into()),
                command: None,
                prompt: None,
            }
        );
        assert!(build(&parse(&args(&["new", "robot"])).unwrap()).is_err());
    }

    #[test]
    fn new_agent_carries_a_first_prompt() {
        let a = parse(&args(&["new", "agent", "--prompt", "fix the tests"])).unwrap();
        assert!(matches!(
            build(&a).unwrap(),
            Cmd::New { prompt: Some(p), .. } if p == "fix the tests"
        ));
    }

    #[test]
    fn durations_take_units_fractions_and_bare_seconds() {
        assert_eq!(duration("1.5s").unwrap(), Duration::from_millis(1500));
        assert_eq!(duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(duration("10m").unwrap(), Duration::from_secs(600));
        assert_eq!(duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(duration("90").unwrap(), Duration::from_secs(90));
        assert!(duration("soon").is_err());
        assert!(duration("5y").is_err());
    }

    #[test]
    fn wait_and_ask_flags_parse() {
        let a = parse(&args(&[
            "ask", "--to", "s4", "-t", "2m", "--settle", "3s", "--close", "hi", "there",
        ]))
        .unwrap();
        assert_eq!(a.to.as_deref(), Some("s4"));
        assert_eq!(a.timeout, Some(Duration::from_secs(120)));
        assert_eq!(a.settle, Some(Duration::from_secs(3)));
        assert!(a.close);
        assert_eq!(a.pos, vec!["hi", "there"]);
        assert!(parse(&args(&["wait", "s1", "--exit"])).unwrap().exit);
    }

    fn agent(working: bool, idle: u64, input_age: Option<u64>, worked: bool) -> SessionInfo {
        SessionInfo {
            id: "s1".into(),
            kind: "agent".into(),
            name: "Claude #1".into(),
            project: "p".into(),
            project_dir: "/p".into(),
            status: "running".into(),
            working,
            attention: false,
            title: None,
            error: None,
            idle_for_ms: Some(idle),
            input_age_ms: input_age,
            worked_since_input: worked,
            input_pending: false,
        }
    }

    #[test]
    fn idle_means_quiet_after_working_on_the_input() {
        let settle = Duration::from_millis(1500);
        // Worked on it, now quiet long enough: done.
        assert!(is_idle(&agent(false, 2000, Some(9000), true), settle));
        // Still working, or not quiet for long enough yet.
        assert!(!is_idle(&agent(true, 0, Some(9000), true), settle));
        assert!(!is_idle(&agent(false, 500, Some(9000), true), settle));
        // Sent input it hasn't started on: not done — until the grace runs out.
        assert!(!is_idle(&agent(false, 60_000, Some(3000), false), settle));
        assert!(is_idle(&agent(false, 60_000, Some(25_000), false), settle));
        // Never given input: quiet is enough.
        assert!(is_idle(&agent(false, 2000, None, false), settle));
        // Queued input still counts as busy.
        let mut pending = agent(false, 5000, None, false);
        pending.input_pending = true;
        assert!(!is_idle(&pending, settle));
        // An agent that exited won't do more.
        let mut gone = agent(false, 0, Some(10), false);
        gone.status = "exited".into();
        assert!(is_idle(&gone, settle));
    }

    #[test]
    fn verbs_needing_a_target_refuse_without_one() {
        assert!(build(&parse(&args(&["status"])).unwrap()).is_err());
        assert!(build(&parse(&args(&["keys", "s1"])).unwrap()).is_err());
        assert!(build(&parse(&args(&["ls"])).unwrap()).is_ok());
        assert!(build(&parse(&args(&["worktree", "rm"])).unwrap()).is_err());
        assert!(build(&parse(&args(&["worktree"])).unwrap()).is_err());
    }

    #[test]
    fn worktree_new_takes_branch_project_and_agent() {
        let a = parse(&args(&[
            "worktree", "new", "fix-auth", "-p", "api", "--agent", "codex", "--prompt", "go",
        ]))
        .unwrap();
        assert_eq!(
            build(&a).unwrap(),
            Cmd::WorktreeNew {
                branch: Some("fix-auth".into()),
                project: Some("api".into()),
                agent: Some("codex".into()),
                prompt: Some("go".into()),
            }
        );
        let a = parse(&args(&["worktree", "new", "--cmd", "ls"])).unwrap();
        assert!(build(&a).is_err());
        let a = parse(&args(&["worktree", "rm", "fix-auth", "--force"])).unwrap();
        assert_eq!(
            build(&a).unwrap(),
            Cmd::WorktreeRm {
                target: "fix-auth".into(),
                force: true,
            }
        );
    }
}
