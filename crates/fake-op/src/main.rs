use serde_json::Value;
use std::{
    env, fs,
    fs::OpenOptions,
    io::{self, Write},
    process,
};

const HANDSHAKE: &str = "envswitch-fake-op-v1";

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut trace = OpenOptions::new()
        .create(true)
        .append(true)
        .open(env::var("ENVSWITCH_FAKE_OP_TRACE")?)?;
    let mut line = serde_json::to_vec(&args)?;
    line.push(b'\n');
    trace.write_all(&line)?;
    if args == ["__envswitch_fake_handshake"] {
        println!("{HANDSHAKE}");
        return Ok(0);
    }
    let fixture: Value =
        serde_json::from_slice(&fs::read(env::var("ENVSWITCH_FAKE_OP_FIXTURE")?)?)?;
    let response = fixture["responses"]
        .as_array()
        .ok_or("fixture.responses must be an array")?
        .iter()
        .find(|entry| entry["argv"] == serde_json::json!(args))
        .ok_or_else(|| format!("unconfigured fake-op argv: {}", serde_json::json!(args)))?;
    io::stdout().write_all(response["stdout"].as_str().unwrap_or("").as_bytes())?;
    io::stderr().write_all(response["stderr"].as_str().unwrap_or("").as_bytes())?;
    let code = response["exitCode"].as_i64().unwrap_or(0);
    if !(0..=255).contains(&code) {
        return Err("fixture exitCode must be 0..255".into());
    }
    Ok(code as i32)
}

fn main() {
    match run() {
        Ok(code) => process::exit(code),
        Err(error) => {
            eprintln!("fake-op: {error}");
            process::exit(97);
        }
    }
}
