//! Dummy tool binary for e2e tests: speaks the one-JSON-line-per-line protocol
//! on stdin/stdout. Behavior selected by the request's `input` field:
//! - `echo <text>`  -> ok, stdout = <text>
//! - `fail`         -> ok=false, code=1
//! - `sleep`        -> answers only after 60 s (timeout tests)
//! - `garbage`      -> answers with a non-JSON line (malformed-response tests)
//! - `printenv`     -> ok, stdout = the tool's whole environment (env leak tests)
//! - `leak`         -> ok, stdout contains a fixed secret value (masking tests)
//! - anything else  -> ok, stdout = the input verbatim

use std::io::{BufRead, BufWriter, Write};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<serde_json::Value>(&line) else {
            break;
        };
        let input = req["input"].as_str().unwrap_or("").to_string();
        if input == "garbage" {
            let _ = writeln!(out, "this is not json");
            let _ = out.flush();
            continue;
        }
        if input == "sleep" {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
        let (ok, code, stdout) = if let Some(text) = input.strip_prefix("echo ") {
            (true, 0, text.to_string())
        } else if input == "fail" {
            (false, 1, String::new())
        } else if input == "leak" {
            (
                true,
                0,
                "the token is s3cr3t-value, do not print it".to_string(),
            )
        } else if input == "printenv" {
            let dump = std::env::vars()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("\n");
            (true, 0, dump)
        } else {
            (true, 0, input)
        };
        let resp = serde_json::json!({
            "ok": ok,
            "code": code,
            "stdout": stdout,
            "stderr": "",
        });
        let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap());
        let _ = out.flush();
    }
}
