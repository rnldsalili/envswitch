//! Captured output streams, progress events, and environment variables.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use envswitch::model::ProgressEvent;
use envswitch::ports::EnvLookup;

/// A cloneable, thread-safe in-memory `Write` sink for `Deps::out`/`err`.
#[derive(Clone, Default)]
pub struct SharedBuf {
    inner: Arc<Mutex<Vec<u8>>>,
}

impl SharedBuf {
    pub fn new() -> Self {
        Self::default()
    }

    /// A boxed clone, ready for `Deps::out` / `Deps::err`.
    pub fn boxed(&self) -> Box<dyn Write> {
        Box::new(self.clone())
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.inner.lock().unwrap().clone()
    }

    /// Contents as UTF-8 (lossy).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }

    /// Lines without their `\n` terminators (a trailing empty line from a
    /// final `\n` is dropped; an embedded blank line is kept).
    pub fn lines(&self) -> Vec<String> {
        let text = self.text();
        let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        if lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Collects progress events (TS tests' injected `log`).
#[derive(Clone, Default)]
pub struct EventLog {
    inner: Arc<Mutex<Vec<ProgressEvent>>>,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// A sink for `Deps::log`.
    pub fn sink(&self) -> Box<dyn FnMut(ProgressEvent)> {
        let inner = Arc::clone(&self.inner);
        Box::new(move |event| inner.lock().unwrap().push(event))
    }

    /// Appends one event (for passing `&mut |e| log.push(e)` to library
    /// functions).
    pub fn push(&self, event: ProgressEvent) {
        self.inner.lock().unwrap().push(event);
    }

    pub fn events(&self) -> Vec<ProgressEvent> {
        self.inner.lock().unwrap().clone()
    }

    /// `"<status>:<message>"` per event, like the TS test assertions.
    pub fn lines(&self) -> Vec<String> {
        self.events()
            .iter()
            .map(|event| format!("{}:{}", event.status.as_str(), event.message))
            .collect()
    }
}

/// A mutable fake environment for `Deps::env`. Starts empty, so `CI` and
/// `NO_COLOR` are unset unless a test sets them.
#[derive(Clone, Default)]
pub struct EnvVars {
    inner: Arc<Mutex<HashMap<String, OsString>>>,
}

impl EnvVars {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, name: &str, value: &str) -> &Self {
        self.inner
            .lock()
            .unwrap()
            .insert(name.to_string(), OsString::from(value));
        self
    }

    pub fn remove(&self, name: &str) -> &Self {
        self.inner.lock().unwrap().remove(name);
        self
    }

    /// A lookup closure for `Deps::env`.
    pub fn lookup(&self) -> EnvLookup {
        let inner = Arc::clone(&self.inner);
        Box::new(move |name| inner.lock().unwrap().get(name).cloned())
    }
}
