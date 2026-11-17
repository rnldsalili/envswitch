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
/// DIVERGENCES l1, applied to every scenario: a target's env reads overlap,
/// so within a run of consecutive env reads the order is free. Only runs that
/// directly follow the candidate `op item list` (not the `envswitch-config`
/// scan) qualify, and every read in them must be the exact env-read argv for
/// that list's vault and an item the scenario's fake `op` serves. Everything
/// else stays exact: every other call in order, the same runs, the same read
/// set (each item once), and `readCounts` per key. Extra reads (already in
/// flight) are allowed only when the oracle failed on an env read: its last
/// call ends this final run and its last progress event is that `reading`.
/// Accepted transcripts adopt the actual order.
fn overlapped_reads(s: &J, e: &mut J, a: &J) -> Result<(), String> {
    fn is_get(call: &J) -> bool {
        call.at(0).text() == "item" && call.at(1).text() == "get"
    }
    fn runs(calls: &[J]) -> Vec<Vec<J>> {
        let mut out: Vec<Vec<J>> = Vec::new();
        for call in calls {
            match out.last_mut() {
                Some(run) if is_get(call) && is_get(&run[0]) => run.push(call.clone()),
                _ => out.push(vec![call.clone()]),
            }
        }
        out
    }
    let (expected_calls, actual_calls) = (e.get("opCalls").items(), a.get("opCalls").items());
    if expected_calls == actual_calls {
        return Ok(());
    }
    let (expected_runs, actual_runs) = (runs(expected_calls), runs(actual_calls));
    if expected_runs.len() != actual_runs.len() {
        return Ok(()); // a structural difference: let the exact compare report it
    }
    let op = s.get("op");
    let served: Vec<&str> = std::iter::once(op.get("items"))
        .chain(
            op.get("byVault")
                .pairs()
                .iter()
                .map(|(_, v)| v.get("items")),
        )
        .flat_map(J::items)
        .flat_map(|item| [item.get("id").text(), item.get("title").text()])
        .filter(|key| !key.is_empty())
        .collect();
    let last = expected_runs.len() - 1;
    let mut extras = false;
    for (index, (want, got)) in expected_runs.iter().zip(&actual_runs).enumerate() {
        if want == got {
            continue;
        }
        let list = index
            .checked_sub(1)
            .and_then(|i| expected_runs[i].first())
            .filter(|call| {
                expected_runs[index - 1].len() == 1
                    && call.at(0).text() == "item"
                    && call.at(1).text() == "list"
                    && call.items().last().map(J::text) != Some("envswitch-config")
            });
        let Some(list) = list else {
            return Ok(()); // not an env-read run: let the exact compare report it
        };
        let vault = list.at(3).text();
        let env_read = |call: &J| {
            let item = call.at(2).text();
            *call
                == J::array(
                    [
                        "item", "get", item, "--vault", vault, "--format", "json", "--reveal",
                    ]
                    .map(J::string),
                )
                && served.contains(&item)
        };
        require(
            want.iter().chain(got).all(env_read),
            "l1 reorders only env reads of served items in the listed vault",
        )?;
        let distinct = |run: &[J]| run.iter().enumerate().all(|(i, c)| !run[..i].contains(c));
        require(
            distinct(want) && distinct(got),
            "l1 reads each item once per run",
        )?;
        require(
            want.iter().all(|call| got.contains(call)),
            "l1 overlapped run keeps every oracle read",
        )?;
        if got.len() > want.len() {
            let progress = e.get("progress").items();
            require(
                index == last
                    && e.get("exitCode").int() != 0
                    && progress
                        .last()
                        .is_some_and(|event| event.get("message").text().starts_with("reading ")),
                "l1 extra in-flight reads only after a failed env read",
            )?;
            extras = true;
        }
    }
    let (want, got) = (e.get("readCounts").pairs(), a.get("readCounts").pairs());
    for (key, count) in want {
        require(
            a.get("readCounts").get(key) == count,
            "l1 keeps per-item read counts",
        )?;
    }
    for (key, count) in got {
        if !want.iter().any(|(k, _)| k == key) {
            require(
                extras && count.int() == 1,
                "l1 extra read counts come from in-flight reads",
            )?;
        }
    }
    e.set("opCalls", a.get("opCalls").clone());
    e.set("readCounts", a.get("readCounts").clone());
    Ok(())
}

pub fn apply(s: &J, expected: &mut J, actual: &J, b: &Bridge) -> Result<(), String> {
    overlapped_reads(s, expected, actual)?;
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
            "e5" if s.get("kind").text() == "cli" && s.get("argv").at(0).text() == "update" => {}
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
        rows.contains(&"e1")
            && rows.contains(&"e2")
            && rows.contains(&"e3")
            && rows.contains(&"e5"),
        "native update fixtures require e1/e2/e3/e5 together",
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
        plan.get("currentVersion").text() == format!("v{}", env!("CARGO_PKG_VERSION")),
        "e1 current version is compiled",
    )?;
    require(
        plan.get("repo").text() == "demo-suite/alpha"
            && plan.get("appRoot").text() == "{TMP}/app"
            && plan.get("installDir").text() == "{TMP}/launcher",
        "e3 persisted receipt settings forwarded",
    )?;
    require(
        plan.get("version").is_null() && plan.get("verifyAttestations") == &J::bool(false),
        "ordinary D3 installer resolves latest once",
    )?;
    if s.get("update").get("exitCode").int() != 0 {
        require(
            rows.contains(&"e6"),
            "installer exit-status replacement requires e6",
        )?;
    }
    let status = if s.get("update").has("installerError") {
        2
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
        require(
            text(a, "stdoutBase64", b)
                == format!(
                    "{}{}",
                    s.get("rustUpdate").get("installerStdout").text(),
                    s.get("rustUpdate").get("summary").text()
                ),
            "native installer terminal/suffix and CLI disposition summary",
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

#[cfg(test)]
mod overlapped_reads_tests {
    use super::{J, overlapped_reads};

    fn call(parts: &[&str]) -> J {
        J::array(parts.iter().map(|p| J::string(*p)))
    }
    fn get_in(vault: &str, id: &str) -> J {
        call(&[
            "item", "get", id, "--vault", vault, "--format", "json", "--reveal",
        ])
    }
    fn get(id: &str) -> J {
        get_in("v", id)
    }
    fn list() -> J {
        call(&[
            "item",
            "list",
            "--vault",
            "v",
            "--format",
            "json",
            "--tags",
            "env:local",
        ])
    }
    fn scenario() -> J {
        let items = ["a", "b", "c"].map(|id| J::object([("id", J::string(id))]));
        J::object([("op", J::object([("items", J::array(items))]))])
    }
    fn transcript(exit: i64, last_progress: &str, calls: Vec<J>) -> J {
        let mut counts: Vec<(String, J)> = Vec::new();
        for c in &calls {
            if c.at(1).text() == "get" {
                counts.push((
                    format!("{}/{}", c.at(4).text(), c.at(2).text()),
                    J::number(1),
                ));
            }
        }
        J::object([
            ("exitCode", J::number(exit)),
            (
                "progress",
                J::array([J::object([("message", J::string(last_progress))])]),
            ),
            ("opCalls", J::array(calls)),
            ("readCounts", J::object(counts)),
        ])
    }

    #[test]
    fn accepts_a_reordered_env_read_run_and_adopts_it() {
        let mut e = transcript(
            0,
            "done",
            vec![get("cfg"), list(), get("a"), get("b"), get("c")],
        );
        let a = transcript(
            0,
            "done",
            vec![get("cfg"), list(), get("c"), get("a"), get("b")],
        );
        overlapped_reads(&scenario(), &mut e, &a).unwrap();
        assert_eq!(e, a);
    }

    #[test]
    fn accepts_in_flight_extras_only_after_a_failed_env_read() {
        let mut e = transcript(2, "reading a", vec![list(), get("a")]);
        let a = transcript(2, "reading a", vec![list(), get("b"), get("a")]);
        overlapped_reads(&scenario(), &mut e, &a).unwrap();
        assert_eq!(e, a);

        // A success, or a failure that was not an env read (e.g. a write).
        for (exit, progress) in [(0, "reading a"), (2, "writing x")] {
            let mut e = transcript(exit, progress, vec![list(), get("a")]);
            let a = transcript(exit, progress, vec![list(), get("b"), get("a")]);
            assert!(overlapped_reads(&scenario(), &mut e, &a).is_err());
        }
    }

    #[test]
    fn rejects_a_missing_repeated_unserved_or_foreign_read() {
        let mut e = transcript(2, "reading b", vec![list(), get("a"), get("b")]);
        let a = transcript(2, "reading b", vec![list(), get("b"), get("c")]);
        assert!(overlapped_reads(&scenario(), &mut e, &a).is_err());

        let mut e = transcript(0, "done", vec![list(), get("a"), get("b")]);
        let a = transcript(0, "done", vec![list(), get("b"), get("a"), get("a")]);
        assert!(overlapped_reads(&scenario(), &mut e, &a).is_err());

        let mut e = transcript(2, "reading a", vec![list(), get("a")]);
        let a = transcript(2, "reading a", vec![list(), get("SECRET"), get("a")]);
        assert!(overlapped_reads(&scenario(), &mut e, &a).is_err());

        let mut e = transcript(2, "reading a", vec![list(), get("a")]);
        let a = transcript(2, "reading a", vec![list(), get_in("other", "b"), get("a")]);
        assert!(overlapped_reads(&scenario(), &mut e, &a).is_err());
    }

    #[test]
    fn leaves_non_env_read_runs_to_the_exact_compare() {
        // A config read that fails (no candidate list before it).
        let original = transcript(2, "reading config item cfg", vec![get("cfg")]);
        let mut e = original.clone();
        let a = transcript(
            2,
            "reading config item cfg",
            vec![get("SECRET"), get("cfg")],
        );
        overlapped_reads(&scenario(), &mut e, &a).unwrap();
        assert_eq!(e, original);

        // Reordered non-read calls.
        let original = transcript(0, "done", vec![get("cfg"), list(), get("a"), get("b")]);
        let mut e = original.clone();
        let a = transcript(0, "done", vec![list(), get("cfg"), get("a"), get("b")]);
        overlapped_reads(&scenario(), &mut e, &a).unwrap();
        assert_eq!(e, original);
    }
}
