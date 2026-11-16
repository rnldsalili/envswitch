//! Layer merge and title collation (contract 02 §4.3; D8). Owner: C3.
//! Everything here is `pub(crate)`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

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

/// TS `mergeTarget(op, candidates, params)` (02 §4.3).
///
/// * Candidates are visited in API order. Each classified item is read at
///   most once per switch: `cache` is keyed by `ItemSummary::key()` and shared
///   across targets. A miss emits `info reading <title || id>` *before* the
///   read; a failed read aborts immediately (nothing is cached for it).
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
                let values = op.read_env_item(item)?;
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
