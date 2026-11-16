use super::json::J;
use envswitch::cli::{FlagValue, ParsedArgs};
use envswitch::model::*;

pub fn strings(v: &[String]) -> J {
    J::array(v.iter().map(J::string))
}
pub fn env(v: &EnvMap) -> J {
    J::object(v.iter().map(|(k, v)| (k, J::string(v))))
}
pub fn env_input(v: &J) -> EnvMap {
    v.pairs()
        .iter()
        .map(|(k, v)| (k.clone(), v.text().to_owned()))
        .collect()
}
pub fn target(t: &TargetConfig) -> J {
    let mut o = J::object([
        ("name", J::string(&t.name)),
        ("output", J::string(&t.output)),
    ]);
    if let Some(g) = &t.groups {
        o.set("groups", strings(g))
    }
    o
}
pub fn item(i: &ItemSummary) -> J {
    J::object([
        ("id", J::string(&i.id)),
        ("title", J::string(&i.title)),
        ("tags", strings(&i.tags)),
    ])
}
pub fn item_input(v: &J) -> ItemSummary {
    ItemSummary {
        id: v.get("id").text().into(),
        title: v.get("title").text().into(),
        tags: v.get("tags").strings(),
    }
}
pub fn config(c: &Config) -> J {
    let mut o = J::object([("targets", J::array(c.targets.iter().map(target)))]);
    if let Some(p) = &c.project {
        let mut j = J::object(Vec::<(String, J)>::new());
        if let Some(id) = &p.id {
            j.set("id", J::string(id))
        }
        if let Some(markers) = &p.root_markers {
            j.set(
                "rootMarkers",
                J::array(markers.iter().map(|m| {
                    let mut o = J::object(Vec::<(String, J)>::new());
                    if let Some(v) = &m.path {
                        o.set("path", J::string(v))
                    }
                    if let Some(v) = &m.package_json_name {
                        o.set("packageJsonName", J::string(v))
                    }
                    o
                })),
            )
        }
        o.set("project", j)
    }
    if let Some(v) = &c.clients {
        o.set("clients", strings(v))
    }
    if let Some(v) = &c.environments {
        o.set("environments", strings(v))
    }
    if let Some(t) = &c.tags {
        let mut j = J::object(Vec::<(String, J)>::new());
        if let Some(v) = &t.base {
            j.set("base", strings(v))
        }
        for (k, v) in [
            ("project", &t.project),
            ("env", &t.env),
            ("client", &t.client),
            ("target", &t.target),
            ("group", &t.group),
            ("layer", &t.layer),
        ] {
            if let Some(v) = v {
                j.set(k, J::string(v))
            }
        }
        o.set("tags", j)
    }
    o
}
pub fn tag_values(t: &TagValues) -> J {
    let mut j = J::object([("base", strings(&t.base))]);
    for (k, v) in [
        ("project", &t.project),
        ("env", &t.env),
        ("client", &t.client),
        ("target", &t.target),
        ("group", &t.group),
        ("layer", &t.layer),
    ] {
        if let Some(v) = v {
            j.set(k, J::string(v))
        }
    }
    j
}
pub fn args(a: &ParsedArgs) -> J {
    let mut j = J::object(Vec::<(String, J)>::new());
    if let Some(v) = &a.command {
        j.set("command", J::string(v))
    }
    j.set("positional", strings(&a.positional));
    j.set(
        "flags",
        J::object(a.flags.iter().map(|(k, v)| {
            (
                k,
                match v {
                    FlagValue::True => J::bool(true),
                    FlagValue::Value(v) => J::string(v),
                },
            )
        })),
    );
    j
}
pub fn progress(e: &ProgressEvent) -> J {
    J::object([
        ("status", J::string(e.status.as_str())),
        ("message", J::string(&e.message)),
    ])
}
pub fn status(v: &str) -> ProgressStatus {
    match v {
        "info" => ProgressStatus::Info,
        "done" => ProgressStatus::Done,
        "dry-run" => ProgressStatus::DryRun,
        _ => panic!("invalid progress status"),
    }
}
pub fn layer(v: &str) -> Option<LayerName> {
    LayerName::ALL.into_iter().find(|l| l.as_str() == v)
}
pub fn switch(r: &SwitchResult) -> J {
    J::array(r.targets.iter().map(|t| {
        J::object([
            ("target", target(&t.target)),
            ("outputPath", J::string(t.output_path.to_string_lossy())),
            ("written", J::bool(t.written)),
            ("keyCount", J::number(t.key_count as u64)),
            (
                "layers",
                J::array(t.layers.iter().map(|l| {
                    J::object([
                        ("name", J::string(l.name.as_str())),
                        ("rank", J::number(l.rank)),
                        ("item", item(&l.item)),
                        ("values", env(&l.values)),
                    ])
                })),
            ),
            ("overwrittenKeys", strings(&t.overwritten_keys)),
        ])
    }))
}
pub fn inspect(r: &InspectReport) -> J {
    J::array(r.targets.iter().map(|t| {
        J::object([
            ("target", target(&t.target)),
            ("outputPath", J::string(t.output_path.to_string_lossy())),
            ("parentExists", J::bool(t.parent_exists)),
        ])
    }))
}
pub fn default_config() -> J {
    J::object([(
        "targets",
        J::array([J::object([
            ("name", J::string("api")),
            ("output", J::string("apps/api/.env")),
            ("groups", J::array([J::string("apis")])),
        ])]),
    )])
}
