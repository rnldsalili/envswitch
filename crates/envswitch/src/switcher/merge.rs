//! Layer merge and title collation (contract 02 §4.3; D8). Owner: C3.
//! Everything here is `pub(crate)`.

use std::any::Any;
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

/// Most `op item get` reads in flight at once (DIVERGENCES l1). Small enough
/// to stay clear of 1Password rate limits, large enough for typical configs.
pub(crate) const READ_CONCURRENCY: usize = 6;

/// A worker's read: its result, or the panic payload of a panicking read.
type Outcome = std::thread::Result<Result<EnvMap>>;

/// Reads `items` and returns their values in order, as if read one at a
/// time: `on_read(item)` runs right before each item's result is awaited, and
/// the first failure in order is returned (a panicking read is resumed when
/// reached). Up to [`READ_CONCURRENCY`] reads overlap (DIVERGENCES l1).
///
/// Workers claim indexes in increasing order and stop claiming after any
/// failure, so every index before the earliest failing one has been claimed
/// and will be filled; results after a failure are never consumed. Returning
/// waits for reads already in flight. A single item, or a host that cannot
/// start threads, reads sequentially on the calling thread.
pub(crate) fn read_in_order(
    op: &dyn OnePassword,
    items: &[&ItemSummary],
    on_read: &mut dyn FnMut(&ItemSummary),
) -> Result<Vec<EnvMap>> {
    let sequential = |on_read: &mut dyn FnMut(&ItemSummary)| {
        items
            .iter()
            .map(|item| {
                on_read(item);
                op.read_env_item(item)
            })
            .collect()
    };
    if items.len() <= 1 {
        return sequential(on_read);
    }
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let slots: Mutex<Vec<Option<Outcome>>> = Mutex::new(items.iter().map(|_| None).collect());
    let filled = Condvar::new();
    let worker = || {
        loop {
            if stop.load(AtomicOrdering::SeqCst) {
                break;
            }
            let index = next.fetch_add(1, AtomicOrdering::SeqCst);
            let Some(item) = items.get(index) else {
                break;
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| op.read_env_item(item)));
            if !matches!(outcome, Ok(Ok(_))) {
                stop.store(true, AtomicOrdering::SeqCst);
            }
            slots.lock().unwrap_or_else(PoisonError::into_inner)[index] = Some(outcome);
            filled.notify_all();
        }
    };
    let consumed: std::result::Result<Result<Vec<EnvMap>>, Box<dyn Any + Send>> =
        thread::scope(|scope| {
            let started = (0..items.len().min(READ_CONCURRENCY))
                .filter(|_| thread::Builder::new().spawn_scoped(scope, worker).is_ok())
                .count();
            if started == 0 {
                return Ok(sequential(on_read));
            }
            let mut values = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                on_read(item);
                let mut slots = slots.lock().unwrap_or_else(PoisonError::into_inner);
                let outcome = loop {
                    if let Some(outcome) = slots[index].take() {
                        break outcome;
                    }
                    slots = filled.wait(slots).unwrap_or_else(PoisonError::into_inner);
                };
                drop(slots);
                match outcome {
                    Ok(Ok(env)) => values.push(env),
                    Ok(Err(error)) => {
                        stop.store(true, AtomicOrdering::SeqCst);
                        return Ok(Err(error));
                    }
                    Err(panic) => {
                        stop.store(true, AtomicOrdering::SeqCst);
                        return Err(panic);
                    }
                }
            }
            Ok(Ok(values))
        });
    consumed.unwrap_or_else(|panic| resume_unwind(panic))
}

/// TS `mergeTarget(op, candidates, params)` (02 §4.3).
///
/// * Candidates are visited in API order. Each classified item is read at
///   most once per switch: `cache` is keyed by `ItemSummary::key()` and shared
///   across targets. A miss emits `info reading <title || id>` *before* the
///   read result is used; a failed read aborts immediately (nothing is cached
///   for it). This target's misses are read with [`read_in_order`]: they
///   overlap, but only within this target, so earlier targets are written
///   before any later target's read starts (DIVERGENCES l1).
/// * Layers are stably sorted by rank, then by title collation; ties keep
///   API order (no id tie-break).
/// * Later layers overwrite earlier values. A key is reported as overwritten
///   only when it already existed with a *different* value.
pub(crate) fn merge_target(
    op: &dyn OnePassword,
    candidates: &[ItemSummary],
    resolved: &ResolvedTarget,
    matcher: &TagMatcher,
    cache: &mut HashMap<String, EnvMap>,
    log: &mut dyn FnMut(ProgressEvent),
) -> Result<MergeResult> {
    let classified: Vec<_> = candidates
        .iter()
        .filter_map(|item| {
            classify_with_matcher(item, &resolved.target.name, matcher).map(|name| (name, item))
        })
        .collect();
    let mut pending = HashSet::new();
    let misses: Vec<&ItemSummary> = classified
        .iter()
        .map(|(_, item)| *item)
        .filter(|item| !cache.contains_key(item.key()) && pending.insert(item.key()))
        .collect();
    let values = read_in_order(op, &misses, &mut |item| {
        log(ProgressEvent::info(format!(
            "reading {}",
            item.display_name()
        )))
    })?;
    for (item, values) in misses.iter().zip(values) {
        cache.insert(item.key().to_string(), values);
    }

    let layers = classified
        .into_iter()
        .map(|(name, item)| EnvLayer {
            name,
            rank: name.rank(),
            item: item.clone(),
            values: cache[item.key()].clone(),
        })
        .collect();
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
