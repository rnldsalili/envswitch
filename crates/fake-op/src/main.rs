use serde_json::Value;
use std::{
    env, fs,
    fs::OpenOptions,
    io::{self, Write},
    process,
};

const HANDSHAKE: &str = "envswitch-fake-op-v1";

#[cfg(windows)]
mod ctrl_break {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
        fn GetStdHandle(kind: u32) -> *mut c_void;
        fn WriteFile(
            handle: *mut c_void,
            bytes: *const u8,
            len: u32,
            written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn ExitProcess(code: u32) -> !;
    }

    unsafe extern "system" fn handler(kind: u32) -> i32 {
        if kind != 1 {
            return 0;
        }
        let message = b"fake-op: caught Ctrl-Break\n";
        let mut written = 0;
        // SAFETY: synchronous write to this process's stderr, followed by exit.
        unsafe {
            WriteFile(
                GetStdHandle(-12_i32 as u32),
                message.as_ptr(),
                message.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            );
            ExitProcess(1);
        }
    }

    pub fn install() -> std::io::Result<()> {
        // SAFETY: the callback has the required Windows ABI and static lifetime.
        if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

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
    #[cfg(windows)]
    if let Some(mode) = response.get("catchCtrlBreak") {
        let ready = mode["readyFile"]
            .as_str()
            .ok_or("catchCtrlBreak.readyFile must be a path")?;
        ctrl_break::install()?;
        fs::write(ready, process::id().to_string())?;
    }
    if let Some(delay) = response.get("delayMs") {
        let millis = delay
            .as_u64()
            .ok_or("fixture delayMs must be an unsigned integer")?;
        std::thread::sleep(std::time::Duration::from_millis(millis));
    }
    let mut stdout = io::stdout().lock();
    if let Some(encoded) = response["stdoutBase64"].as_str() {
        stdout.write_all(&decode_base64(encoded)?)?;
    } else if let Some(size) = response["stdoutSize"]["size"].as_u64() {
        let byte = response["stdoutSize"]["byte"]
            .as_u64()
            .filter(|n| *n <= 255)
            .ok_or("invalid stdoutSize byte")? as u8;
        let block = [byte; 8192];
        let mut remaining = size;
        while remaining > 0 {
            let count = remaining.min(block.len() as u64) as usize;
            stdout.write_all(&block[..count])?;
            remaining -= count as u64;
        }
    } else {
        stdout.write_all(response["stdout"].as_str().unwrap_or("").as_bytes())?;
    }
    stdout.flush()?;
    io::stderr().write_all(response["stderr"].as_str().unwrap_or("").as_bytes())?;
    let code = response["exitCode"].as_i64().unwrap_or(0);
    if !(0..=255).contains(&code) {
        return Err("fixture exitCode must be 0..255".into());
    }
    Ok(code as i32)
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    if !encoded.len().is_multiple_of(4) {
        return Err("invalid stdoutBase64 length".into());
    }
    let mut output = Vec::new();
    for (index, chunk) in encoded.as_bytes().as_chunks::<4>().0.iter().enumerate() {
        let last = index + 1 == encoded.len() / 4;
        let padding = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if padding > 2 || (padding > 0 && !last) {
            return Err("invalid stdoutBase64 padding".into());
        }
        let mut bits = 0_u32;
        for (i, byte) in chunk.iter().enumerate() {
            let value = if i >= 4 - padding {
                0
            } else {
                table
                    .iter()
                    .position(|value| value == byte)
                    .ok_or("invalid stdoutBase64 character")? as u32
            };
            bits = (bits << 6) | value;
        }
        output.extend_from_slice(&bits.to_be_bytes()[1..4 - padding]);
    }
    Ok(output)
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
