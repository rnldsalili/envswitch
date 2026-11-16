//! `ScriptedPrompter`: records every prompt and replays scripted answers
//! (the Rust counterpart of the TS `FakePrompt` / `CancelPrompt`).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use envswitch::error::{Error, Result};
use envswitch::ports::{InputSpec, Prompter, SelectSpec};

/// One recorded prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptRecord {
    Select {
        message: String,
        labels: Vec<String>,
        default_index: Option<usize>,
    },
    Input {
        message: String,
        default: Option<String>,
        required: bool,
    },
}

impl PromptRecord {
    pub fn message(&self) -> &str {
        match self {
            PromptRecord::Select { message, .. } | PromptRecord::Input { message, .. } => message,
        }
    }
}

/// A scripted answer, consumed in order by `select` and `input`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// `select`: choose the choice with this exact label.
    Label(String),
    /// `select`: choose this index.
    Index(usize),
    /// `input`: return this text verbatim.
    Text(String),
    /// Accept the default (select `default_index`, input `default`); an
    /// error if there is none.
    Default,
    /// Return `Error::Cancelled` (Ctrl-C / Esc).
    Cancel,
    /// Return this error.
    Fail(Error),
}

impl Answer {
    pub fn label(label: &str) -> Self {
        Answer::Label(label.to_string())
    }

    pub fn text(text: &str) -> Self {
        Answer::Text(text.to_string())
    }
}

struct PromptState {
    answers: VecDeque<Answer>,
    records: Vec<PromptRecord>,
}

/// Thread-safe scripted [`Prompter`]. Clones share state. When the script
/// runs out, it behaves like [`Answer::Default`] (as the TS `FakePrompt`).
#[derive(Clone)]
pub struct ScriptedPrompter {
    inner: Arc<Mutex<PromptState>>,
}

impl ScriptedPrompter {
    pub fn new(answers: impl IntoIterator<Item = Answer>) -> Self {
        ScriptedPrompter {
            inner: Arc::new(Mutex::new(PromptState {
                answers: answers.into_iter().collect(),
                records: Vec::new(),
            })),
        }
    }

    /// Cancels at the first prompt (TS `CancelPrompt`).
    pub fn cancelling() -> Self {
        Self::new([Answer::Cancel])
    }

    /// A boxed clone, ready for `Deps::prompter`.
    pub fn boxed(&self) -> Box<dyn Prompter> {
        Box::new(self.clone())
    }

    pub fn records(&self) -> Vec<PromptRecord> {
        self.inner.lock().unwrap().records.clone()
    }

    /// Messages of every prompt shown, in order.
    pub fn messages(&self) -> Vec<String> {
        self.records()
            .iter()
            .map(|record| record.message().to_string())
            .collect()
    }

    /// Answers not consumed yet.
    pub fn remaining(&self) -> usize {
        self.inner.lock().unwrap().answers.len()
    }

    fn next_answer(&self, record: PromptRecord) -> Answer {
        let mut state = self.inner.lock().unwrap();
        state.records.push(record);
        state.answers.pop_front().unwrap_or(Answer::Default)
    }
}

fn script_error(message: String) -> Error {
    Error::internal(format!("ScriptedPrompter: {message}"))
}

impl Prompter for ScriptedPrompter {
    fn select(&self, spec: &SelectSpec) -> Result<usize> {
        let labels: Vec<String> = spec
            .choices
            .iter()
            .map(|choice| choice.label.clone())
            .collect();
        let answer = self.next_answer(PromptRecord::Select {
            message: spec.message.clone(),
            labels: labels.clone(),
            default_index: spec.default_index,
        });
        match answer {
            Answer::Label(label) => labels
                .iter()
                .position(|candidate| *candidate == label)
                .ok_or_else(|| {
                    script_error(format!(
                        "{}: no choice labeled {label:?} in {labels:?}",
                        spec.message
                    ))
                }),
            Answer::Index(index) if index < labels.len() => Ok(index),
            Answer::Index(index) => Err(script_error(format!(
                "{}: index {index} out of range",
                spec.message
            ))),
            Answer::Default => spec
                .default_index
                .ok_or_else(|| script_error(format!("{}: no answer and no default", spec.message))),
            Answer::Cancel => Err(Error::Cancelled),
            Answer::Fail(error) => Err(error),
            Answer::Text(text) => Err(script_error(format!(
                "{}: text answer {text:?} given to a select",
                spec.message
            ))),
        }
    }

    fn input(&self, spec: &InputSpec) -> Result<String> {
        let answer = self.next_answer(PromptRecord::Input {
            message: spec.message.clone(),
            default: spec.default.clone(),
            required: spec.required,
        });
        match answer {
            Answer::Text(text) => Ok(text),
            Answer::Default => spec
                .default
                .clone()
                .ok_or_else(|| script_error(format!("{}: no answer and no default", spec.message))),
            Answer::Cancel => Err(Error::Cancelled),
            Answer::Fail(error) => Err(error),
            other => Err(script_error(format!(
                "{}: {other:?} given to an input",
                spec.message
            ))),
        }
    }
}
