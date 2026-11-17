//! Layer merge and title collation (contract 02 §4.3; D8). Owner: C3.
//! Everything here is `pub(crate)`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
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

/// Every env item the selected targets will read, deduplicated by
/// `ItemSummary::key()`, in the order the sequential TS loop reads them:
/// targets in order, candidates in API order within each target.
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

/// Reads `items` with up to [`READ_CONCURRENCY`] concurrent `op` calls
/// (DIVERGENCES l1). Workers claim indexes in order and stop claiming after
/// the first failure, so every item before the lowest failing index has a
/// result: consuming results in plan order reports exactly the error the
/// sequential TS loop would hit first. Items after a failure may be missing
/// (never claimed) or already read (in flight); neither is ever consumed.
pub(crate) fn prefetch_reads(
    op: &dyn OnePassword,
    items: &[&ItemSummary],
) -> HashMap<String, Result<EnvMap>> {
    let workers = items.len().min(READ_CONCURRENCY);
    if workers <= 1 {
        // Nothing to overlap: merge_target reads lazily, exactly like TS.
        return HashMap::new();
    }
    let next = AtomicUsize::new(0);
    let first_failure = AtomicUsize::new(usize::MAX);
    let slots: Vec<OnceLock<Result<EnvMap>>> = items.iter().map(|_| OnceLock::new()).collect();
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, AtomicOrdering::SeqCst);
                    if index >= items.len() || index > first_failure.load(AtomicOrdering::SeqCst) {
                        break;
                    }
                    let result = op.read_env_item(items[index]);
                    if result.is_err() {
                        first_failure.fetch_min(index, AtomicOrdering::SeqCst);
                    }
                    let _ = slots[index].set(result);
                }
            });
        }
    });
    items
        .iter()
        .zip(slots)
        .filter_map(|(item, slot)| Some((item.key().to_string(), slot.into_inner()?)))
        .collect()
}

/// TS `mergeTarget(op, candidates, params)` (02 §4.3).
///
/// * Candidates are visited in API order. Each classified item is read at
///   most once per switch: `cache` is keyed by `ItemSummary::key()` and shared
///   across targets. A miss emits `info reading <title || id>`, then takes the
///   [`prefetch_reads`] result for that key (reading it now when absent); a
///   failed read aborts immediately (nothing is cached for it).
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
    prefetched: &mut HashMap<String, Result<EnvMap>>,
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
                let values = match prefetched.remove(key) {
                    Some(result) => result?,
                    None => op.read_env_item(item)?,
                };
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
