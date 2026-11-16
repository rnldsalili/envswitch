use serde_json::{Value, json};
use std::{
    env, fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn native_protocol_traces_handshake_response_and_unknown_argv() {
    let dir = env::temp_dir().join(format!(
        "envswitch-fake-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    let fixture = dir.join("fixture.json");
    let trace = dir.join("trace.jsonl");
    fs::write(&fixture, json!({"responses":[{"argv":["whoami","--format","json"],"stdout":"demo-suite-private","stderr":"safe failure\r\n","exitCode":7}]}).to_string()).unwrap();
    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_op"))
            .args(args)
            .env("ENVSWITCH_FAKE_OP_FIXTURE", &fixture)
            .env("ENVSWITCH_FAKE_OP_TRACE", &trace)
            .output()
            .unwrap()
    };
    let handshake = invoke(&["__envswitch_fake_handshake"]);
    assert!(handshake.status.success());
    assert_eq!(handshake.stdout, b"envswitch-fake-op-v1\n");
    assert!(handshake.stderr.is_empty());
    let response = invoke(&["whoami", "--format", "json"]);
    assert_eq!(response.status.code(), Some(7));
    assert_eq!(response.stdout, b"demo-suite-private");
    assert_eq!(response.stderr, b"safe failure\r\n");
    let unknown = invoke(&["account", "list"]);
    assert_eq!(unknown.status.code(), Some(97));
    assert!(unknown.stdout.is_empty());
    let calls: Vec<Value> = fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        calls,
        vec![
            json!(["__envswitch_fake_handshake"]),
            json!(["whoami", "--format", "json"]),
            json!(["account", "list"])
        ]
    );
    fs::remove_dir_all(dir).unwrap();
}
