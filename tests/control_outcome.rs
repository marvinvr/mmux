use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// Drive only the CLI against a fixture socket; no TUI or live session is involved.
fn run(args: &[&str], response: Option<Value>) -> Output {
    let dir = std::env::temp_dir().join(format!("mmux-outcome-{}", std::process::id()));
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
