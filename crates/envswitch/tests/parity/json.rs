//! Ordered JSON transport. Objects are vectors; serde_json::Value alone sorts keys.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Object(Vec<(String, J)>),
    Array(Vec<J>),
    String(String),
    Scalar(Value),
}
impl J {
    pub fn object(pairs: impl IntoIterator<Item = (impl Into<String>, J)>) -> Self {
        Self::Object(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
    pub fn array(values: impl IntoIterator<Item = J>) -> Self {
        Self::Array(values.into_iter().collect())
    }
    pub fn string(v: impl Into<String>) -> Self {
        Self::String(v.into())
    }
    pub fn null() -> Self {
        Self::Scalar(Value::Null)
    }
    pub fn bool(v: bool) -> Self {
        Self::Scalar(Value::Bool(v))
    }
    pub fn number(v: impl Into<serde_json::Number>) -> Self {
        Self::Scalar(Value::Number(v.into()))
    }
    pub fn decode(v: &Value) -> Self {
        match v[0].as_str().expect("ordered tag") {
            "object" => Self::object(
                v[1].as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (p[0].as_str().unwrap(), Self::decode(&p[1]))),
            ),
            "array" => Self::array(v[1].as_array().unwrap().iter().map(Self::decode)),
            // a7 replaces the unsupported lone-surrogate expected result explicitly.
            "string" => Self::string(String::from_utf16_lossy(
                &v[1]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_u64().unwrap() as u16)
                    .collect::<Vec<_>>(),
            )),
            "scalar" => Self::Scalar(v[1].clone()),
            _ => panic!("unknown ordered tag"),
        }
    }
    pub fn encode(&self) -> Value {
        match self {
            Self::Object(pairs) => serde_json::json!([
                "object",
                pairs
                    .iter()
                    .map(|(key, value)| serde_json::json!([key, value.encode()]))
                    .collect::<Vec<_>>()
            ]),
            Self::Array(values) => {
                serde_json::json!(["array", values.iter().map(Self::encode).collect::<Vec<_>>()])
            }
            Self::String(value) => {
                serde_json::json!(["string", value.encode_utf16().collect::<Vec<_>>()])
            }
            Self::Scalar(value) => serde_json::json!(["scalar", value]),
        }
    }
    pub fn get(&self, k: &str) -> &Self {
        if let Self::Object(p) = self {
            p.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v)
                .unwrap_or(&NULL)
        } else {
            &NULL
        }
    }
    pub fn at(&self, i: usize) -> &Self {
        self.items().get(i).unwrap_or(&NULL)
    }
    pub fn has(&self, k: &str) -> bool {
        self.pairs().iter().any(|(key, _)| key == k)
    }
    pub fn text(&self) -> &str {
        if let Self::String(s) = self { s } else { "" }
    }
    pub fn str_or<'a>(&'a self, fallback: &'a str) -> &'a str {
        if matches!(self, Self::String(_)) {
            self.text()
        } else {
            fallback
        }
    }
    pub fn boolean(&self) -> bool {
        matches!(self, Self::Scalar(Value::Bool(true)))
    }
    pub fn int(&self) -> i64 {
        if let Self::Scalar(v) = self {
            v.as_i64().unwrap_or(0)
        } else {
            0
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Scalar(Value::Null))
    }
    pub fn items(&self) -> &[Self] {
        if let Self::Array(a) = self { a } else { &[] }
    }
    pub fn pairs(&self) -> &[(String, Self)] {
        if let Self::Object(o) = self { o } else { &[] }
    }
    pub fn set(&mut self, k: &str, v: Self) {
        let Self::Object(o) = self else {
            panic!("set on non-object")
        };
        if let Some(p) = o.iter_mut().find(|(key, _)| key == k) {
            p.1 = v
        } else {
            o.push((k.into(), v))
        }
    }
    pub fn json(&self) -> String {
        match self {
            Self::Object(p) => format!(
                "{{{}}}",
                p.iter()
                    .map(|(k, v)| format!("{}:{}", serde_json::to_string(k).unwrap(), v.json()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Array(a) => format!(
                "[{}]",
                a.iter().map(Self::json).collect::<Vec<_>>().join(",")
            ),
            Self::String(s) => serde_json::to_string(s).unwrap(),
            Self::Scalar(v) => v.to_string(),
        }
    }
    pub fn strings(&self) -> Vec<String> {
        self.items().iter().map(|v| v.text().to_owned()).collect()
    }
}
static NULL: J = J::Scalar(Value::Null);
pub fn differences(e: &J, a: &J, path: &str, out: &mut Vec<String>) {
    if e == a {
        return;
    }
    match (e, a) {
        (J::Object(ep), J::Object(ap)) => {
            let ek: Vec<_> = ep.iter().map(|(k, _)| k).collect();
            let ak: Vec<_> = ap.iter().map(|(k, _)| k).collect();
            if ek != ak {
                out.push(format!("{path}: object key order {ek:?} != {ak:?}"))
            }
            for (k, v) in ep {
                differences(v, a.get(k), &format!("{path}.{k}"), out)
            }
        }
        (J::Array(ev), J::Array(av)) => {
            if ev.len() != av.len() {
                out.push(format!("{path}: length {} != {}", ev.len(), av.len()))
            }
            for (i, (e, a)) in ev.iter().zip(av).enumerate() {
                differences(e, a, &format!("{path}[{i}]"), out)
            }
        }
        _ => out.push(format!(
            "{path}: expected {}, actual {}",
            e.json(),
            a.json()
        )),
    }
}
