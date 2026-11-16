use envswitch::cli::{FlagValue, parse_args};

fn parse(argv: &[&str]) -> envswitch::cli::ParsedArgs {
    parse_args(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

#[test]
fn omitted_optional_switch_flags_stay_absent() {
    let args = parse(&[
        "switch",
        "--vault",
        "vault",
        "--config-item",
        "cfg",
        "--client",
        "alpha",
    ]);
    assert_eq!(args.command.as_deref(), Some("switch"));
    assert!(args.positional.is_empty());
    assert_eq!(
        args.flags.into_iter().collect::<Vec<_>>(),
        vec![
            ("vault".into(), FlagValue::Value("vault".into())),
            ("config-item".into(), FlagValue::Value("cfg".into())),
            ("client".into(), FlagValue::Value("alpha".into())),
        ]
    );
}

#[test]
fn first_token_is_always_the_command() {
    assert_eq!(parse(&[]).command, None);
    assert_eq!(parse(&[""]).command.as_deref(), Some(""));
    let args = parse(&["--vault", "x", "switch"]);
    assert_eq!(args.command.as_deref(), Some("--vault"));
    assert_eq!(args.positional, ["x", "switch"]);
    assert!(args.flags.is_empty());
}

#[test]
fn empty_next_token_is_not_consumed() {
    let args = parse(&["switch", "--help", "", "--dry-run", "--vault="]);
    assert_eq!(args.positional, [""]);
    assert_eq!(args.flags["help"], FlagValue::True);
    assert_eq!(args.flags["dry-run"], FlagValue::True);
    assert_eq!(args.flags["vault"], FlagValue::Value(String::new()));
}

#[test]
fn values_are_greedy_and_equals_splits_only_once() {
    let args = parse(&[
        "switch",
        "-h",
        "--x=-a=b",
        "--dry-run",
        "foo",
        "--client",
        "-y",
        "--",
        "tail",
    ]);
    assert_eq!(args.positional, ["-h"]);
    assert_eq!(args.flags["x"], FlagValue::Value("-a=b".into()));
    assert_eq!(args.flags["dry-run"], FlagValue::Value("foo".into()));
    assert_eq!(args.flags["client"], FlagValue::Value("-y".into()));
    assert_eq!(args.flags[""], FlagValue::Value("tail".into()));
    assert_eq!(parse(&["switch", "--"]).flags[""], FlagValue::True);
}

#[test]
fn last_value_wins_without_reordering_including_approved_js_divergences() {
    let args = parse(&[
        "switch",
        "--z",
        "a",
        "--1",
        "b",
        "--__proto__",
        "x",
        "--z=c",
    ]);
    assert_eq!(
        args.flags.keys().map(String::as_str).collect::<Vec<_>>(),
        ["z", "1", "__proto__"]
    );
    assert_eq!(args.flags["z"], FlagValue::Value("c".into()));
    assert_eq!(args.flags["__proto__"], FlagValue::Value("x".into()));
}
