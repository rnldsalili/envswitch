//! Narrow, executable replacement assertions; an approved marker never disables a scenario.
use super::{json::J, transport::Bridge};
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(format!("divergence assertion: {message}"))
    }
}
fn text(t: &J, key: &str, b: &Bridge) -> String {
    String::from_utf8_lossy(&b.decode_base64(t.get(key).text())).into_owned()
}
pub fn apply(s: &J, expected: &mut J, actual: &J, b: &Bridge) -> Result<(), String> {
    let markers = match s.get("divergence") {
        J::Array(v) => v.iter().map(|v| v.text()).collect(),
        J::String(v) => vec![v.as_str()],
        _ => Vec::new(),
    };
    for row in &markers {
        match *row {
            "a1" => {
                require(
                    s.get("id").text() == "tags-prototype",
                    "a1 needs a precise template replacement",
                )?;
                expected.set(
                    "result",
                    J::object([
                        ("tags", J::object([("base", J::array([J::string("x:")]))])),
                        (
                            "candidates",
                            J::array([J::string("x:"), J::string("env:local")]),
                        ),
                    ]),
                );
            }
            "a2" => {
                require(
                    s.get("id").text() == "op-env-proto",
                    "a2 is only the __proto__ env field",
                )?;
                expected.set(
                    "result",
                    J::object([("__proto__", J::string("alpha")), ("A", J::string("1"))]),
                );
            }
            "a3" => {
                if s.get("kind").text() == "parseArgs" {
                    let mut r = expected.get("result").clone();
                    r.set("flags", J::object([("__proto__", J::string("x"))]));
                    expected.set("result", r)
                } else {
                    require(actual.get("exitCode").int() == 1, "a3 exit must be 1")?;
                    require(
                        text(actual, "stderrBase64", b) == "envswitch: unknown flag: --__proto__\n",
                        "a3 unknown flag diagnostic",
                    )?;
                    expected.set("exitCode", J::number(1));
                    expected.set("stderrBase64", actual.get("stderrBase64").clone());
                }
            }
            "a4" => {
                if s.get("kind").text() == "parseArgs" {
                    let mut r = expected.get("result").clone();
                    r.set(
                        "flags",
                        J::object([("z", J::bool(true)), ("1", J::string("a"))]),
                    );
                    expected.set("result", r)
                } else {
                    require(
                        actual.get("exitCode").int() == 1
                            && text(actual, "stderrBase64", b) == "envswitch: unknown flag: --z\n",
                        "a4 insertion-order first error",
                    )?;
                    expected.set("stderrBase64", actual.get("stderrBase64").clone());
                }
            }
            "a5" => {
                let old = text(expected, "stderrBase64", b);
                let prefix = if let Some(index) = old.rfind(" is not valid JSON: ") {
                    &old[..index + " is not valid JSON: ".len()]
                } else if let Some(index) =
                    old.find("failed to read package.json for project root validation: ")
                {
                    &old[..index
                        + "failed to read package.json for project root validation: ".len()]
                } else {
                    return Err("a5 marker has no approved JSON diagnostic".into());
                };
                let new = text(actual, "stderrBase64", b);
                require(
                    new.starts_with(prefix) && new.ends_with('\n') && new.len() > prefix.len() + 1,
                    "a5 preserves complete diagnostic prefix",
                )?;
                expected.set("stderrBase64", actual.get("stderrBase64").clone());
            }
            "a8" => {
                require(
                    actual.get("exitCode").int() == 1,
                    "a8 config/package parse failures are exit 1",
                )?;
                require(
                    text(actual, "stdoutBase64", b).is_empty(),
                    "a8 emits no success output",
                )?;
                let prefix = if s.get("kind").text() == "cli" {
                    let progress = expected.get("progress").items();
                    let end = progress
                        .iter()
                        .position(|e| e.get("message").text().starts_with("reading config item "))
                        .ok_or("a8 CLI config-read event missing")?
                        + 1;
                    let retained = J::array(progress[..end].iter().cloned());
                    require(
                        actual.get("progress") == &retained,
                        "a8 stops at config parsing",
                    )?;
                    expected.set("progress", retained);
                    let old = text(expected, "stderrBase64", b);
                    let header: String = old.split_inclusive('\n').take(end).collect();
                    format!("{header}envswitch: config field is not valid JSON: ")
                } else if s.get("kind").text() == "inspect" {
                    "envswitch: failed to read package.json for project root validation: "
                        .to_owned()
                } else {
                    "envswitch: config field is not valid JSON: ".to_owned()
                };
                let error = text(actual, "stderrBase64", b);
                require(
                    error.starts_with(&prefix)
                        && error.ends_with('\n')
                        && error.len() > prefix.len() + 1,
                    "a8 preserves JSON parse diagnostic prefix",
                )?;
                expected.set("exitCode", J::number(1));
                expected.set("stdoutBase64", J::string(""));
                expected.set("stderrBase64", actual.get("stderrBase64").clone());
                expected.set("result", J::null());
            }
            "a6" => {
                require(actual.get("exitCode").int() == 2, "a6 exit 2")?;
                let e = text(actual, "stderrBase64", b);
                require(e.starts_with("envswitch [info] checking 1Password authentication\nenvswitch: failed to run op: ")&&e.ends_with('\n'),"a6 spawn error prefix")?;
                expected.set("stderrBase64", actual.get("stderrBase64").clone());
            }
            "a7" => {
                // Same path as TS's own invalid-JSON handling (parseJson → fail): exit 1.
                require(
                    actual.get("exitCode").int() == 1,
                    "a7 lone surrogate is exit 1",
                )?;
                let e = text(actual, "stderrBase64", b);
                require(
                    e.starts_with("envswitch: op item get ")
                        && e.contains(" output is not valid JSON: ")
                        && e.ends_with('\n'),
                    "a7 JSON diagnostic prefix",
                )?;
                expected.set("exitCode", J::number(1));
                expected.set("result", J::null());
                expected.set("stderrBase64", actual.get("stderrBase64").clone());
            }
            "d1" => {
                let e = text(expected, "stderrBase64", b);
                let a = text(actual, "stderrBase64", b);
                let listed = ["ENOENT", "ENOTDIR", "EACCES", "EPERM", "EEXIST", "EISDIR"];
                if !listed.iter().any(|code| e.contains(code)) {
                    let prefix = e.split(": ").next().unwrap_or("envswitch");
                    require(
                        a.starts_with(prefix) && actual.get("exitCode") == expected.get("exitCode"),
                        "d1 other-errno prefix and exit",
                    )?;
                    expected.set("stderrBase64", actual.get("stderrBase64").clone());
                }
            }
            "e1" | "e2" | "e3" | "h1" => {}
            "e6" => {
                let status = s.get("update").get("exitCode").int();
                require(
                    status != 0 && actual.get("exitCode").int() == status,
                    "e6 forwards the fake installer's status",
                )?;
                require(
                    text(actual, "stderrBase64", b).ends_with(&format!(
                        "envswitch: update failed: installer exited with code {status}\n"
                    )),
                    "e6 preserves installer failure message",
                )?;
                require(
                    text(actual, "stdoutBase64", b).is_empty(),
                    "e6 prints no success",
                )?;
                expected.set("exitCode", J::number(status));
            }
            "e4" => {
                require(
                    actual.get("exitCode").int() == 1
                        && text(actual, "stderrBase64", b)
                            == "envswitch: cannot locate a valid envswitch install receipt for this binary; reinstall envswitch with the install script from the README\n",
                    "e4 copied-binary guidance",
                )?;
                expected.set("exitCode", J::number(1));
                expected.set("stderrBase64", actual.get("stderrBase64").clone());
            }
            "f1" => {
                require(actual.get("exitCode").int() == 0, "f1 exit 0")?;
                require(
                    text(actual, "stdoutBase64", b) == envswitch::cli::version_line(),
                    "f1 compiled version and target line",
                )?;
                expected.set("exitCode", J::number(0));
                expected.set("stdoutBase64", actual.get("stdoutBase64").clone());
                expected.set("stderrBase64", J::string(""));
            }
            "c1" | "c2" | "c3" | "b1" | "e5" | "g1" => {
                return Err(format!(
                    "{row} requires its dedicated PTY/installer/locale lane; no blanket waiver"
                ));
            }
            unknown => return Err(format!("unknown divergence marker: {unknown}")),
        }
    }
    if markers.iter().any(|r| ["e1", "e2", "e3"].contains(r)) {
        update(s, expected, actual, b, &markers)?;
    }
    Ok(())
}
fn update(s: &J, e: &mut J, a: &J, b: &Bridge, rows: &[&str]) -> Result<(), String> {
    if s.get("id").text() == "cli-update-source-checkout" {
        return require(
            a.get("exitCode") == e.get("exitCode") && a.get("updateCalls").items().is_empty(),
            "source-checkout guard still runs first",
        );
    }
    // Every fixture here supplies a managed receipt in the adapter. The production
    // updater must use its compiled version and embedded installer, never the TS lookup.
    require(
        rows.contains(&"e1") && rows.contains(&"e2") && rows.contains(&"e3"),
        "native update fixtures require e1/e2/e3 together",
    )?;
    let calls = a.get("updateCalls").items();
    require(
        calls.len() == 1 && calls[0].get("method").text() == "runInstaller",
        "D3 always invokes installer once and performs no separate latest lookup",
    )?;
    let call = &calls[0];
    require(
        call.get("source").text() == "embedded",
        "e2 installer is embedded",
    )?;
    let plan = call.get("plan");
    require(
        plan.get("currentVersion").text().trim_start_matches('v') == env!("CARGO_PKG_VERSION"),
        "e1 current version is compiled",
    )?;
    require(
        plan.get("repo").text() == "demo-suite/alpha"
            && plan.get("appRoot").text() == "{TMP}/app"
            && plan.get("installDir").text() == "{TMP}/launcher",
        "e3 persisted receipt settings forwarded",
    )?;
    require(
        plan.get("version").is_null(),
        "ordinary D3 installer resolves latest once",
    )?;
    if s.get("update").get("exitCode").int() != 0 {
        require(
            rows.contains(&"e6"),
            "installer exit-status replacement requires e6",
        )?;
    }
    let status = if s.get("update").has("installerError") {
        1
    } else {
        s.get("update").get("exitCode").int()
    };
    require(
        a.get("exitCode").int() == status,
        "installer status/preflight error mapping",
    )?;
    let compiled = format!("v{}", env!("CARGO_PKG_VERSION"));
    let progress = format!("envswitch [info] updating envswitch {compiled} -> latest\n");
    let stderr = text(a, "stderrBase64", b);
    require(
        stderr.starts_with(&progress),
        "native update progress uses compiled version and latest",
    )?;
    if status != 0 {
        let suffix = if s.get("update").has("installerError") {
            format!(
                "envswitch: update failed: could not run installer: {}\n",
                s.get("update").get("installerError").text()
            )
        } else {
            format!("envswitch: update failed: installer exited with code {status}\n")
        };
        require(
            stderr == format!("{progress}{suffix}"),
            "native update failure diagnostic",
        )?;
        require(
            text(a, "stdoutBase64", b).is_empty(),
            "no success after installer failure",
        )?;
    } else {
        require(stderr == progress, "success stderr contains only progress")?;
        let tag = s
            .get("update")
            .get("updatedVersion")
            .str_or(s.get("update").get("latest").str_or("v0.0.4"))
            .trim();
        let tag = if tag.starts_with('v') {
            tag.to_owned()
        } else {
            format!("v{tag}")
        };
        require(
            text(a, "stdoutBase64", b) == format!("updated envswitch {compiled} -> {tag}\n"),
            "native update validated result summary",
        )?;
    }
    for key in [
        "exitCode",
        "stdoutBase64",
        "stderrBase64",
        "progress",
        "updateCalls",
    ] {
        e.set(key, a.get(key).clone());
    }
    // Files and cleanup still compare exactly; only fake-controlled native fixture
    // entries are added to the expected baseline by the shared transport.
    Ok(())
}
