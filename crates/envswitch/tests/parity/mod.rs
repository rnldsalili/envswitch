mod adapter;
mod divergences;
mod json;
mod model;
mod ports;
mod transport;
use json::{J, differences};
use std::{
    collections::BTreeMap,
    fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
};
use transport::Bridge;

pub fn run() {
    if let Some(path) = std::env::var_os("ENVSWITCH_PARITY_WORKER") {
        worker(Path::new(&path));
    }

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let bridge = Bridge::new(repo);
    let filter = std::env::var("ENVSWITCH_PARITY_FILTER").unwrap_or_default();
    let mut summary: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut failures = Vec::new();
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let mut files: Vec<_> = fs::read_dir(repo.join("parity/scenarios"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    for scenario in files {
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&scenario).unwrap()).unwrap();
        let id = raw["id"].as_str().unwrap();
        let kind = raw["kind"].as_str().unwrap();
        if !filter.is_empty() && filter != kind && !id.starts_with(&filter) {
            continue;
        }
        if raw["platforms"]
            .as_array()
            .is_some_and(|list| !list.iter().any(|v| v == os))
        {
            continue;
        }
        let golden = repo
            .join("parity/goldens")
            .join(os)
            .join(format!("{id}.json"));
        let result =
            panic::catch_unwind(AssertUnwindSafe(|| one(&bridge, &scenario, &golden, kind)));
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(p) => Some(format!(
                "PANIC: {}",
                p.downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| p.downcast_ref::<&str>().copied())
                    .unwrap_or("non-string panic")
            )),
        };
        let tally = summary.entry(kind.into()).or_default();
        if let Some(error) = error {
            tally.1 += 1;
            eprintln!("FAIL {id}: {error}");
            failures.push(id.to_owned());
        } else {
            tally.0 += 1;
            eprintln!("PASS {id}");
        }
    }
    panic::set_hook(previous_hook);
    for (kind, (pass, fail)) in &summary {
        eprintln!("parity {kind}: {pass} passed, {fail} failed");
    }
    assert!(!summary.is_empty(), "no parity scenarios match {filter:?}");
    assert!(
        failures.is_empty(),
        "{} parity scenario(s) failed: {}",
        failures.len(),
        failures.join(", ")
    );
}
fn one(bridge: &Bridge, scenario: &Path, golden: &Path, kind: &str) -> Result<(), String> {
    if !golden.exists() {
        return Err(format!("missing committed golden: {}", golden.display()));
    }
    if kind == "blackbox" {
        let cli = PathBuf::from(env!("CARGO_BIN_EXE_envswitch"));
        let fake = cli
            .parent()
            .unwrap()
            .join(if cfg!(windows) { "op.exe" } else { "op" });
        if !fake.exists() {
            return Err("build fake-op first: cargo build --locked -p envswitch-fake-op".into());
        }
        let (_binaries, cli, fake) = isolated_binaries(&cli, &fake).map_err(|e| e.to_string())?;
        let actual = J::decode(&bridge.run(
            &[
                "blackbox".into(),
                scenario.display().to_string(),
                golden.display().to_string(),
                serde_json::json!([cli, fake]).to_string(),
            ],
            None,
        )?);
        let prepared = bridge.run(
            &[
                "prepare".into(),
                scenario.display().to_string(),
                golden.display().to_string(),
                serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"target":env!("TARGET")})
                    .to_string(),
            ],
            None,
        )?;
        let path = PathBuf::from(prepared["path"].as_str().unwrap());
        let bundle: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let s = J::decode(&bundle["scenario"]);
        let mut expected = J::decode(&bundle["expected"]);
        bridge.cleanup(&path);
        apply_divergences(&s, &mut expected, &actual, bridge)?;
        return compare(&expected, &actual);
    }
    let prepared = bridge.run(
        &[
            "prepare".into(),
            scenario.display().to_string(),
            golden.display().to_string(),
            serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"target":env!("TARGET")})
                .to_string(),
        ],
        None,
    )?;
    let path = PathBuf::from(prepared["path"].as_str().unwrap());
    let bundle: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let s = J::decode(&bundle["scenario"]);
    let mut expected = J::decode(&bundle["expected"]);
    // An isolated test subprocess captures stray process stdout/stderr as well as
    // Deps writers, and makes cwd/umask changes private even under parallel tests.
    let mut child = std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    child
        .args([
            "--ignored",
            "--exact",
            "scenario_parity",
            "--quiet",
            "--nocapture",
        ])
        .env("ENVSWITCH_PARITY_WORKER", &path)
        .env("ENVSWITCH_PARITY_BUN", &bridge.bun)
        .env("BUN_RUNTIME_TRANSPILER_CACHE_PATH", "")
        .env("PATH", path.parent().unwrap())
        .stdin(std::process::Stdio::null());
    for key in [
        "HOME",
        "USERPROFILE",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "LOCALAPPDATA",
        "APPDATA",
    ] {
        child.env(key, bundle["paths"]["HOME"].as_str().unwrap());
    }
    let output = child.output().map_err(|e| e.to_string())?;
    let partial_path = path.with_file_name("partial.json");
    if !output.status.success() || !partial_path.exists() {
        bridge.cleanup(&path);
        return Err(format!(
            "scenario worker failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let partial: serde_json::Value =
        serde_json::from_slice(&fs::read(&partial_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if let Some(message) = partial["harnessPanic"].as_str() {
        let message = message.to_owned();
        bridge.cleanup(&path);
        return Err(format!("PANIC: {message}"));
    }
    let stdout = output
        .stdout
        .strip_prefix(b"\nrunning 1 test\n")
        .or_else(|| output.stdout.strip_prefix(b"running 1 test\n"))
        .ok_or("unexpected Rust test-worker header")?;
    let actual = J::decode(&bridge.run(
        &[
            "finish".into(),
            path.display().to_string(),
            partial_path.display().to_string(),
        ],
        Some(&serde_json::json!({"extraStdout":stdout,"extraStderr":output.stderr}).to_string()),
    )?);
    apply_divergences(&s, &mut expected, &actual, bridge)?;
    compare(&expected, &actual)
}
fn isolated_binaries(
    cli: &Path,
    fake: &Path,
) -> std::io::Result<(tempfile::TempDir, PathBuf, PathBuf)> {
    let root = tempfile::Builder::new()
        .prefix("envswitch-parity-binaries-")
        .tempdir()?;
    let cli_dir = root.path().join("cli");
    let fake_dir = root.path().join("fake-op");
    fs::create_dir(&cli_dir)?;
    fs::create_dir(&fake_dir)?;
    let copied_cli = cli_dir.join(cli.file_name().unwrap());
    let copied_fake = fake_dir.join(fake.file_name().unwrap());
    fs::copy(cli, &copied_cli)?;
    fs::copy(fake, &copied_fake)?;
    Ok((root, copied_cli, copied_fake))
}

fn worker(path: &Path) -> ! {
    panic::set_hook(Box::new(|_| {}));
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let bundle: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("worker bundle"))
                .expect("valid worker bundle");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        adapter::execute(
            &J::decode(&bundle["scenario"]),
            &bundle,
            path,
            &Bridge::new(repo),
        )
    }));
    let text = serialize_worker_result(result);
    fs::write(path.with_file_name("partial.json"), text).expect("worker result");
    std::process::exit(0)
}

fn serialize_worker_result(result: std::thread::Result<envswitch::Result<J>>) -> String {
    match result {
        Ok(Ok(partial)) => partial.encode().to_string(),
        Ok(Err(error)) => serde_json::json!({"harnessPanic": error.to_string()}).to_string(),
        Err(panic) => serde_json::json!({"harnessPanic": panic.downcast_ref::<String>().map(String::as_str).or_else(|| panic.downcast_ref::<&str>().copied()).unwrap_or("non-string panic")}).to_string(),
    }
}

fn apply_divergences(s: &J, expected: &mut J, actual: &J, bridge: &Bridge) -> Result<(), String> {
    divergences::apply(s, expected, actual, bridge).map_err(|error| {
        let stdout = bridge.decode_base64(actual.get("stdoutBase64").text());
        let stderr = bridge.decode_base64(actual.get("stderrBase64").text());
        format!("{error}\nactual.exitCode: {}\nactual.stdout: {:?}\nactual.stderr: {:?}\nactual.result: {}", actual.get("exitCode").int(), String::from_utf8_lossy(&stdout), String::from_utf8_lossy(&stderr), actual.get("result").json())
    })
}

fn compare(expected: &J, actual: &J) -> Result<(), String> {
    let mut diff = Vec::new();
    differences(expected, actual, "$", &mut diff);
    if diff.is_empty() {
        Ok(())
    } else {
        Err(diff.join("\n"))
    }
}
/// Exercise a real implemented S1 export through the entire transcript transport,
/// without pretending the still-stubbed cli::run is implemented.
pub fn scaffold_version_smoke() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let bridge = Bridge::new(repo);
    let prepared = bridge
        .run(
            &[
                "prepare".into(),
                repo.join("parity/scenarios/cli-version.json")
                    .display()
                    .to_string(),
                repo.join(format!("parity/goldens/{os}/cli-version.json"))
                    .display()
                    .to_string(),
            ],
            None,
        )
        .unwrap();
    let path = PathBuf::from(prepared["path"].as_str().unwrap());
    let bundle: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let partial = J::object([
        (
            "stdout",
            J::array(envswitch::cli::version_line().bytes().map(J::number)),
        ),
        ("stderr", J::array([])),
        ("exitCode", J::number(0)),
        ("result", J::null()),
    ]);
    let partial_path = path.with_file_name("partial.json");
    fs::write(&partial_path, partial.encode().to_string()).unwrap();
    let actual = J::decode(
        &bridge
            .run(
                &[
                    "finish".into(),
                    path.display().to_string(),
                    partial_path.display().to_string(),
                ],
                Some("{\"extraStdout\":[],\"extraStderr\":[]}"),
            )
            .unwrap(),
    );
    let mut expected = J::decode(&bundle["expected"]);
    divergences::apply(
        &J::decode(&bundle["scenario"]),
        &mut expected,
        &actual,
        &bridge,
    )
    .unwrap();
    compare(&expected, &actual).unwrap();
}

#[cfg(test)]
mod checks {
    use super::*;
    #[test]
    fn blackbox_binaries_have_separate_private_directories() {
        let original = tempfile::tempdir().unwrap();
        let cli = original.path().join("envswitch.exe");
        let fake = original.path().join("op.exe");
        fs::write(&cli, b"synthetic CLI").unwrap();
        fs::write(&fake, b"synthetic op").unwrap();
        let (_private, copied_cli, copied_fake) = isolated_binaries(&cli, &fake).unwrap();
        assert_ne!(copied_cli.parent(), copied_fake.parent());
        assert_ne!(copied_cli.parent(), cli.parent());
        assert!(!copied_cli.parent().unwrap().join("op.exe").exists());
        assert!(!copied_cli.parent().unwrap().join("op").exists());
        assert_eq!(fs::read(copied_cli).unwrap(), b"synthetic CLI");
        assert_eq!(fs::read(copied_fake).unwrap(), b"synthetic op");
    }
    #[test]
    fn a8_fixtures_are_rejected_by_serde_json() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        for suffix in ["overflow", "depth-129", "lone-surrogate"] {
            let scenario: serde_json::Value = serde_json::from_slice(
                &fs::read(repo.join(format!("parity/scenarios/config-a8-{suffix}.json"))).unwrap(),
            )
            .unwrap();
            assert!(
                serde_json::from_str::<serde_json::Value>(scenario["configRaw"].as_str().unwrap())
                    .is_err(),
                "a8 {suffix} must require a replacement assertion"
            );
        }
    }
    #[test]
    fn release_fake_records_unneeded_lookup_without_panicking() {
        use envswitch::ports::ReleaseSource;
        let calls = std::sync::Arc::new(std::sync::Mutex::new(ports::Calls::default()));
        let release = ports::Release {
            fake: crate::support::FakeReleaseSource::new(None),
            calls: calls.clone(),
        };
        assert_eq!(release.latest_version(), None);
        assert_eq!(
            calls.lock().unwrap().updates[0].get("method").text(),
            "fetchLatestVersion"
        );
        for status in [4, 143] {
            assert_eq!(
                envswitch::Error::status(status, "synthetic installer failure").exit_code(),
                status
            );
        }
    }
    #[test]
    fn successful_worker_result_uses_ordered_transport() {
        let partial = J::object([
            (
                "result",
                J::object([("z", J::number(1)), ("1", J::number(2))]),
            ),
            ("stdout", J::array([J::number(255), J::number(0)])),
        ]);
        let encoded: serde_json::Value =
            serde_json::from_str(&serialize_worker_result(Ok(Ok(partial.clone())))).unwrap();
        assert_eq!(encoded[0], "object");
        assert_eq!(J::decode(&encoded), partial);
    }
    #[test]
    fn object_order_is_observable() {
        let e = J::object([("b", J::number(1)), ("a", J::number(2))]);
        let a = J::object([("a", J::number(2)), ("b", J::number(1))]);
        assert!(compare(&e, &a).unwrap_err().contains("key order"));
        let integer_keys = J::object([("z", J::bool(true)), ("1", J::string("a"))]);
        assert_eq!(J::decode(&integer_keys.encode()), integer_keys);
    }
    #[test]
    fn ordered_transport_roundtrips_nested_maps() {
        let encoded = serde_json::json!([
            "object",
            [
                [
                    "z",
                    ["object", [["b", ["scalar", 1]], ["a", ["scalar", 2]]]]
                ],
                ["a", ["scalar", null]]
            ]
        ]);
        assert_eq!(
            J::decode(&encoded).json(),
            "{\"z\":{\"b\":1,\"a\":2},\"a\":null}"
        );
    }
}
