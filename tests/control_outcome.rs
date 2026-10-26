use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Drive only the CLI against a fixture socket; no TUI or live session is involved.
fn run(args: &[&str], response: Option<Value>) -> Output {
    run_sequence(args, vec![response])
}

fn run_sequence(args: &[&str], responses: Vec<Option<Value>>) -> Output {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mmux-outcome-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ctl.sock");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let accept = || loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    break stream;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "missing CLI connection");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("{e}"),
            }
        };
        // Discovery first probes the socket with a bare connect-and-close.
        drop(accept());
        for response in responses {
            let mut stream = accept();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let _: Value = serde_json::from_str(&line).unwrap();
            if let Some(response) = response {
                writeln!(stream, "{response}").unwrap();
            }
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_mmux"))
        .args(args)
        .env("MMUX_SOCKET", &path)
        .env_remove("MMUX_SESSION")
        .env_remove("MMUX_DEPTH")
        .output()
        .unwrap();
    server.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    output
}

fn idle_agent() -> Value {
    json!({
        "id": "s1", "name": "Claude #1", "kind": "agent",
        "project": "fixture", "project_dir": "/tmp", "status": "running",
        "working": false, "attention": false, "idle_for_ms": 2000,
        "input_age_ms": 9000, "worked_since_input": true, "input_pending": false
    })
}

fn status_response() -> Option<Value> {
    let mut data = idle_agent();
    data["status_line"] = json!([]);
    Some(json!({"ok": true, "data": data}))
}

fn last_response() -> Option<Value> {
    Some(json!({"ok": true, "data": {
        "id": "s1", "name": "Claude #1", "cwd": "/tmp", "screen": "the answer"
    }}))
}

fn unknown_response() -> Option<Value> {
    Some(json!({
        "ok": false, "error": "request may still be applied",
        "outcome_unknown": true
    }))
}

fn assert_unknown(output: &Output, json: bool) {
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    if json {
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["ok"], false);
        assert_eq!(response["outcome_unknown"], true);
    } else {
        assert!(String::from_utf8_lossy(&output.stderr).contains("may still be applied"));
    }
}

#[test]
fn wait_reports_unknown_or_lost_transcript_replies() {
    for json in [false, true] {
        for reply in [unknown_response(), None] {
            let mut args = vec!["wait", "s1"];
            if json {
                args.push("--json");
            }
            let output = run_sequence(&args, vec![status_response(), reply]);
            assert_unknown(&output, json);
        }
    }
}

#[test]
fn ask_close_reports_unknown_or_lost_cleanup_replies_and_keeps_the_answer() {
    for json in [false, true] {
        for reply in [unknown_response(), None] {
            let mut args = vec!["ask", "hello", "--close"];
            if json {
                args.push("--json");
            }
            let mut done = idle_agent();
            done["message"] = json!("created");
            let output = run_sequence(
                &args,
                vec![
                    Some(json!({"ok": true, "data": done})),
                    status_response(),
                    last_response(),
                    last_response(),
                    reply,
                ],
            );
            assert_unknown(&output, json);
            if json {
                let response: Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(response["data"]["reply"], "the answer");
            }
        }
    }
}

#[test]
fn ordinary_transcript_and_cleanup_refusals_keep_their_successful_fallbacks() {
    let refused = Some(json!({"ok": false, "error": "fixture refusal"}));
    let wait = run_sequence(
        &["wait", "s1", "--json"],
        vec![status_response(), refused.clone()],
    );
    assert_eq!(wait.status.code(), Some(0), "{wait:?}");
    let response: Value = serde_json::from_slice(&wait.stdout).unwrap();
    assert_eq!(response["ok"], true);
    assert!(response.get("outcome_unknown").is_none());

    for close in [refused, Some(json!({"ok": true}))] {
        let mut done = idle_agent();
        done["message"] = json!("created");
        let ask = run_sequence(
            &["ask", "hello", "--close", "--json"],
            vec![
                Some(json!({"ok": true, "data": done})),
                status_response(),
                last_response(),
                last_response(),
                close,
            ],
        );
        assert_eq!(ask.status.code(), Some(0), "{ask:?}");
        let response: Value = serde_json::from_slice(&ask.stdout).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["data"]["reply"], "the answer");
        assert!(response.get("outcome_unknown").is_none());
    }
}

#[test]
fn uncertain_outcomes_have_a_distinct_exit_in_plain_json_and_chained_commands() {
    let unknown = json!({
        "ok": false,
        "error": "request may still be applied; check `mmux ls` before retrying",
        "outcome_unknown": true
    });
    for args in [
        vec!["worktree", "new", "test"],
        vec!["worktree", "rm", "test", "--json"],
        vec!["start", "s1", "--wait", "--json"],
        vec!["ask", "hello", "--json"],
        vec!["wait", "s1", "--exit", "--json"],
    ] {
        let output = run(&args, Some(unknown.clone()));
        assert_eq!(output.status.code(), Some(5), "{args:?}: {output:?}");
        if args.contains(&"--json") {
            let response: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response, unknown);
        } else {
            assert!(String::from_utf8_lossy(&output.stderr).contains("may still be applied"));
        }
    }
    // Losing the connection after submission also leaves the operation uncertain.
    let lost = run(&["worktree", "new", "test", "--json"], None);
    assert_eq!(lost.status.code(), Some(5));
    let response: Value = serde_json::from_slice(&lost.stdout).unwrap();
    assert_eq!(response["outcome_unknown"], true);
    assert!(response["error"].as_str().unwrap().contains("mmux ls"));

    let refused = json!({"ok": false, "error": "bad target"});
    let output = run(&["worktree", "rm", "test", "--json"], Some(refused.clone()));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        refused
    );
}
