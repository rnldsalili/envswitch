//! Layer merge and title collation (contract 02 §4.3; D8). Owner: C3.
//! Everything here is `pub(crate)`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::thread;

use icu_collator::options::{AlternateHandling, CaseLevel, CollatorOptions, Strength};
use icu_collator::preferences::{CollationCaseFirst, CollationNumericOrdering};
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use icu_locale::locale;

use crate::error::Result;
use crate::jsutil::compare_utf16;
use crate::model::{EnvLayer, EnvMap, ItemSummary, MergeResult, ProgressEvent, ResolvedTarget};
use crate::ports::OnePassword;

use super::matcher::{TagMatcher, classify_with_matcher};

/// The fixed `en-US` collator (D8), built once.
///
/// Options mirror what `String.prototype.localeCompare(other)` resolves to
/// under Bun 1.4.0 (`new Intl.Collator().resolvedOptions()`): locale `en-US`,
/// usage `sort`, sensitivity `variant` (tertiary strength), punctuation not
/// ignored (alternate handling non-ignorable), `numeric: false`,
/// `caseFirst: "false"`, collation `default`. Each is set explicitly so a
/// future change in locale defaults cannot move the order silently. ICU4X
/// always normalizes to NFD, which matches ECMA-402's requirement that
/// canonically equivalent strings compare equal.
fn collator() -> &'static CollatorBorrowed<'static> {
    static COLLATOR: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();
    COLLATOR.get_or_init(|| {
        let mut prefs = CollatorPreferences::from(locale!("en-US"));
        prefs.case_first = Some(CollationCaseFirst::False);
        prefs.numeric_ordering = Some(CollationNumericOrdering::False);
        let mut options = CollatorOptions::default();
        options.strength = Some(Strength::Tertiary);
        options.alternate_handling = Some(AlternateHandling::NonIgnorable);
        options.case_level = Some(CaseLevel::Off);
        Collator::try_new(prefs, options).expect("compiled en-US collation data is always present")
    })
}

/// `left.localeCompare(right)` with the fixed `en-US` ICU collator (D8).
///
/// Verified against the oracle in `tests.rs` (golden list) and by a
/// differential run over every BMP code point and 30,000 random strings.
/// Known gaps: Han ideographs (ICU4C 78 sorts them by radical and stroke,
/// ICU4X by code point) and code points that ICU 78.1 does not yet assign;
/// see `known_divergence_han_ideographs_use_code_point_order`.
pub(crate) fn compare_titles(left: &str, right: &str) -> Ordering {
    collator().compare(left, right)
}

/// Most `op item get` reads in flight at once (DIVERGENCES l1). Each `op`
/// call can take seconds, and a switch often reads one or two items per
/// target, so reads overlap across targets; the cap keeps the number of
/// concurrent `op` processes modest.
pub(crate) const READ_CONCURRENCY: usize = 8;

/// A worker's read: its result, or the panic payload of a panicking read.
type Outcome = std::thread::Result<Result<EnvMap>>;

/// Every env item a switch will read, deduplicated by `ItemSummary::key()`,
/// in the order the sequential TS loop reads them: targets in order,
/// candidates in API order within each target (the shared read cache means
/// a key is read only for the first target that classifies it).
pub(crate) fn planned_reads<'a>(
    candidates: &'a [ItemSummary],
    targets: &[ResolvedTarget],
    matcher: &TagMatcher,
) -> Vec<&'a ItemSummary> {
    let mut seen = HashSet::new();
    let mut reads = Vec::new();
    for resolved in targets {
        for item in candidates {
            if classify_with_matcher(item, &resolved.target.name, matcher).is_some()
                && seen.insert(item.key())
            {
                reads.push(item);
            }
        }
    }
    reads
}

/// State shared between the consumer and the read workers.
struct Shared {
    next: AtomicUsize,
    stop: AtomicBool,
    slots: Mutex<Vec<Option<Outcome>>>,
    filled: Condvar,
}

impl Shared {
    /// Claims indexes in increasing order until the plan is exhausted or
    /// anything stopped the run. Every index before the earliest failure has
    /// been claimed and will be filled, so the consumer, which takes slots in
    /// order and stops at the first failure, never waits for an unclaimed one.
    fn work(&self, op: &dyn OnePassword, items: &[&ItemSummary]) {
        while !self.stop.load(AtomicOrdering::SeqCst) {
            let index = self.next.fetch_add(1, AtomicOrdering::SeqCst);
            let Some(item) = items.get(index) else {
                break;
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| op.read_env_item(item)));
            if !matches!(outcome, Ok(Ok(_))) {
                self.stop.store(true, AtomicOrdering::SeqCst);
            }
            self.slots.lock().unwrap_or_else(PoisonError::into_inner)[index] = Some(outcome);
            self.filled.notify_all();
        }
    }

    fn take(&self, index: usize) -> Outcome {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(outcome) = slots[index].take() {
                return outcome;
            }
            slots = self
                .filled
                .wait(slots)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Ordered access to a plan of env reads that may run ahead on worker
/// threads (see [`with_reads`]).
pub(crate) struct Reads<'a> {
    op: &'a dyn OnePassword,
    plan: &'a [&'a ItemSummary],
    shared: Option<&'a Shared>,
    consumed: usize,
}

impl Reads<'_> {
    /// The result of reading `item`, which must be the next planned item.
    /// Returns exactly what `op.read_env_item(item)` would; a read that
    /// panicked is resumed here. Off-plan items (and sequential mode) read
    /// directly.
    pub(crate) fn read(&mut self, item: &ItemSummary) -> Result<EnvMap> {
        let planned = self.plan.get(self.consumed).map(|planned| planned.key());
        let Some(shared) = self.shared.filter(|_| planned == Some(item.key())) else {
            debug_assert!(self.shared.is_none(), "read out of plan order");
            return self.op.read_env_item(item);
        };
        let index = self.consumed;
        self.consumed += 1;
        match shared.take(index) {
            Ok(Ok(values)) => Ok(values),
            Ok(Err(error)) => {
                shared.stop.store(true, AtomicOrdering::SeqCst);
                Err(error)
            }
            Err(panic) => {
                shared.stop.store(true, AtomicOrdering::SeqCst);
                resume_unwind(panic)
            }
        }
    }
}

/// Runs `body` while up to [`READ_CONCURRENCY`] workers read `plan` ahead
/// (DIVERGENCES l1). `body` gets each result through [`Reads::read`], in plan
/// order, so its events, errors, and writes keep the sequential TS order;
/// only the `op` calls run early. When `body` returns (or fails) no new reads
/// start, and returning waits for reads already in flight. A plan of at most
/// one item, or a host that cannot start threads, reads on demand on the
/// calling thread.
pub(crate) fn with_reads<T>(
    op: &dyn OnePassword,
    plan: &[&ItemSummary],
    body: impl FnOnce(&mut Reads<'_>) -> T,
) -> T {
    if plan.len() <= 1 {
        return body(&mut Reads {
            op,
            plan,
            shared: None,
            consumed: 0,
        });
    }
    let shared = Shared {
        next: AtomicUsize::new(0),
        stop: AtomicBool::new(false),
        slots: Mutex::new(plan.iter().map(|_| None).collect()),
        filled: Condvar::new(),
    };
    struct StopOnExit<'s>(&'s Shared);
    impl Drop for StopOnExit<'_> {
        fn drop(&mut self) {
            self.0.stop.store(true, AtomicOrdering::SeqCst);
        }
    }
    thread::scope(|scope| {
        let _stop = StopOnExit(&shared);
        let worker = || shared.work(op, plan);
        let started = (0..plan.len().min(READ_CONCURRENCY))
            .filter(|_| thread::Builder::new().spawn_scoped(scope, worker).is_ok())
            .count();
        body(&mut Reads {
            op,
            plan,
            shared: (started > 0).then_some(&shared),
            consumed: 0,
        })
    })
}

/// Reads `items` in order through [`with_reads`]: `on_read(item)` runs right
/// before each result is awaited, and the first failure in order is returned.
#[cfg(test)]
pub(crate) fn read_in_order(
    op: &dyn OnePassword,
    items: &[&ItemSummary],
    on_read: &mut dyn FnMut(&ItemSummary),
) -> Result<Vec<EnvMap>> {
    with_reads(op, items, |reads| {
        items
            .iter()
            .map(|item| {
                on_read(item);
                reads.read(item)
            })
            .collect()
    })
}

/// TS `mergeTarget(op, candidates, params)` (02 §4.3).
///
/// * Candidates are visited in API order. Each classified item is read at
///   most once per switch: `cache` is keyed by `ItemSummary::key()` and shared
///   across targets. A miss emits `info reading <title || id>` *before* its
///   result is taken from `reads`; a failed read aborts immediately (nothing
///   is cached for it). With [`planned_reads`] as the plan, misses arrive in
///   plan order.
/// * Layers are stably sorted by rank, then by title collation; ties keep
///   API order (no id tie-break).
/// * Later layers overwrite earlier values. A key is reported as overwritten
///   only when it already existed with a *different* value.
pub(crate) fn merge_target(
    reads: &mut Reads<'_>,
    candidates: &[ItemSummary],
    resolved: &ResolvedTarget,
    matcher: &TagMatcher,
    cache: &mut HashMap<String, EnvMap>,
    log: &mut dyn FnMut(ProgressEvent),
) -> Result<MergeResult> {
    let mut layers = Vec::new();
    for item in candidates {
        let Some(name) = classify_with_matcher(item, &resolved.target.name, matcher) else {
            continue;
        };

        let key = item.key();
        let values = match cache.get(key) {
            Some(values) => values.clone(),
            None => {
                log(ProgressEvent::info(format!(
                    "reading {}",
                    item.display_name()
                )));
                let values = reads.read(item)?;
                cache.insert(key.to_string(), values.clone());
                values
            }
        };
        layers.push(EnvLayer {
            name,
            rank: name.rank(),
            item: item.clone(),
            values,
        });
    }

    Ok(merge_layers(layers))
}

/// Sorts `layers` and applies them in order (`switcher.ts:242-265`).
pub(crate) fn merge_layers(mut layers: Vec<EnvLayer>) -> MergeResult {
    // `slice::sort_by` is stable, like `Array.prototype.sort`.
    layers.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| compare_titles(&left.item.title, &right.item.title))
    });

    let mut values = EnvMap::new();
    let mut overwritten = HashSet::new();
    for layer in &layers {
        for (key, value) in &layer.values {
            if values.get(key).is_some_and(|previous| previous != value) {
                overwritten.insert(key.clone());
            }
            // Re-inserting keeps the key's first position, like assigning an
            // existing JS property.
            values.insert(key.clone(), value.clone());
        }
    }

    let mut overwritten_keys: Vec<String> = overwritten.into_iter().collect();
    overwritten_keys.sort_by(|left, right| compare_utf16(left, right));

    MergeResult {
        values,
        overwritten_keys,
        layers,
    }
}
