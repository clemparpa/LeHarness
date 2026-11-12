//! E2e tests for the subprocess launcher against the committed dummy tool
//! binary (`CARGO_BIN_EXE_dummy-tool`).

use std::time::Duration;

use harness::process::{parse_line, Tool, ToolRequest, DEFAULT_ENV_ALLOWLIST};
use harness::secrets::SecretStore;

fn dummy_tool() -> &'static str {
    env!("CARGO_BIN_EXE_dummy-tool")
}

fn store_with_secret() -> SecretStore {
    let mut store = SecretStore::new();
    store.insert("token", "s3cr3t-value", None);
    store
}

fn spawn_dummy(store: &SecretStore) -> Tool {
    Tool::spawn(dummy_tool(), &[], store, DEFAULT_ENV_ALLOWLIST).unwrap()
}

#[test]
fn request_response_roundtrip() {
    let store = SecretStore::new();
    let mut tool = spawn_dummy(&store);
    let resp = tool
        .call(
            &ToolRequest {
                id: 1,
                input: "echo hello world".into(),
            },
            Duration::from_secs(5),
        )
        .unwrap();
    assert!(resp.ok);
    assert_eq!(resp.code, 0);
    assert_eq!(resp.stdout, "hello world");
    assert!(tool.is_alive()); // the dummy keeps waiting for more lines
}

#[test]
fn tool_output_is_masked() {
    let store = store_with_secret();
    let mut tool = spawn_dummy(&store);
    let resp = tool
        .call(
            &ToolRequest {
                id: 1,
                input: "leak".into(),
            },
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(resp.stdout, "the token is ***, do not print it");
}

#[test]
fn child_env_excludes_non_allowlisted_vars() {
    std::env::set_var("LEHARNESS_E2E_SECRET", "s3cr3t-value");
    let store = store_with_secret();
    let mut tool = spawn_dummy(&store);
    let resp = tool
        .call(
            &ToolRequest {
                id: 1,
                input: "printenv".into(),
            },
            Duration::from_secs(5),
        )
        .unwrap();
    assert!(!resp.stdout.contains("s3cr3t-value"));
    assert!(!resp.stdout.contains("LEHARNESS_E2E_SECRET"));
}

#[test]
fn spawn_refuses_secret_in_passed_env_value() {
    std::env::set_var("LEHARNESS_E2E_SECRET", "s3cr3t-value");
    let store = store_with_secret();
    let result = Tool::spawn(dummy_tool(), &[], &store, &["PATH", "LEHARNESS_E2E_SECRET"]);
    assert!(matches!(
        result,
        Err(harness::process::ProcessError::SecretLeak)
    ));
}

#[test]
fn malformed_response_is_rejected_and_child_killed() {
    let store = SecretStore::new();
    let mut tool = spawn_dummy(&store);
    let result = tool.call(
        &ToolRequest {
            id: 1,
            input: "garbage".into(),
        },
        Duration::from_secs(5),
    );
    assert!(matches!(
        result,
        Err(harness::process::ProcessError::Malformed(_))
    ));
    assert!(!tool.is_alive());
}

#[test]
fn timeout_kills_the_child() {
    let store = SecretStore::new();
    let mut tool = spawn_dummy(&store);
    let result = tool.call(
        &ToolRequest {
            id: 1,
            input: "sleep".into(),
        },
        Duration::from_millis(300),
    );
    assert!(matches!(
        result,
        Err(harness::process::ProcessError::Timeout(_))
    ));
    assert!(!tool.is_alive());
}

#[test]
fn request_containing_a_secret_is_refused() {
    let store = store_with_secret();
    let mut tool = spawn_dummy(&store);
    let result = tool.call(
        &ToolRequest {
            id: 1,
            input: "echo s3cr3t-value".into(),
        },
        Duration::from_secs(5),
    );
    assert!(matches!(
        result,
        Err(harness::process::ProcessError::SecretLeak)
    ));
}

#[test]
fn parse_line_is_exported_for_fuzzing() {
    let resp = parse_line(r#"{"ok":true,"code":0,"stdout":"x","stderr":""}"#).unwrap();
    assert_eq!(resp.stdout, "x");
}
