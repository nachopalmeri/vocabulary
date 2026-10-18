//! Built-in starter collection.
//!
//! These entries are original WordDock material written for this application:
//! intermediate English vocabulary (judgement, decisions, statistics and
//! everyday use) with short Spanish glosses and example sentences composed for
//! this file. They are deliberately *not* presented as quotations or as
//! extraction from any book, and no book title is used to generate vocabulary.

use crate::model::{Collection, Entry, Rng};

pub const SAMPLE_COLLECTION_NAME: &str = "Sample: Judgment & Everyday English";
pub const SAMPLE_NOTE: &str = "Original sample entries (English → Spanish) written for WordDock. \
Rename this collection after your own book or topic and replace the entries with your own words.";

const SAMPLE_SOURCE: &str = "WordDock sample";

/// `(word, meaning, example)`
const SAMPLE_ENTRIES: &[(&str, &str, &str)] = &[
    (
        "adroit",
        "Hábil, diestro (especialmente para manejar situaciones).",
        "She was adroit at steering the meeting away from a pointless argument.",
    ),
    (
        "hindsight",
        "Retrospectiva: entender algo solo después de que ocurrió.",
        "In hindsight, waiting one more week would have been cheaper.",
    ),
    (
        "hindsight bias",
        "Sesgo retrospectivo: creer que ya se sabía lo que iba a pasar.",
        "The hindsight bias makes old forecasts look more obvious than they were.",
    ),
    (
        "to weigh options",
        "Sopesar opciones, comparar alternativas antes de decidir.",
        "We weighed our options for a week before signing anything.",
    ),
    (
        "trade-off",
        "Compensación: ceder algo para ganar otra cosa.",
        "Working shorter hours is a trade-off between income and free time.",
    ),
    (
        "deliberate",
        "Deliberado, intencional; también deliberar (pensar con cuidado).",
        "The decision was slow but deliberate.",
    ),
    (
        "to stall",
        "Dar largas, postergar una decisión.",
        "He stalled until the deadline solved the problem for him.",
    ),
    (
        "premature",
        "Prematuro: hecho antes del momento adecuado.",
        "Closing the file this early feels premature.",
    ),
    (
        "compelling",
        "Convincente, atractivo (un argumento, una idea).",
        "Her summary was short and compelling.",
    ),
    (
        "negligible",
        "Insignificante: tan pequeño que no cambia el resultado.",
        "The difference between the two estimates was negligible.",
    ),
    (
        "outlier",
        "Valor atípico: un dato muy alejado del resto.",
        "One outlier pulled the average up.",
    ),
    (
        "to skew",
        "Sesgar, distorsionar un resultado.",
        "A small sample can skew the whole comparison.",
    ),
    (
        "sample size",
        "Tamaño de la muestra: cantidad de casos observados.",
        "With such a small sample size, the ranking means little.",
    ),
    (
        "threshold",
        "Umbral: límite a partir del cual algo cambia.",
        "Sales crossed the threshold we had set for the quarter.",
    ),
    (
        "to nudge",
        "Empujar suavemente; influir de forma sutil.",
        "A plain reminder nudged people to finish the survey.",
    ),
    (
        "gut feeling",
        "Corazonada, intuición visceral.",
        "My gut feeling said the numbers were too neat.",
    ),
    (
        "to second-guess",
        "Cuestionar una decisión ya tomada, revisarla de más.",
        "Stop second-guessing every choice you make.",
    ),
    (
        "incidental",
        "Accidental, secundario: no es lo principal.",
        "The extra cost was incidental to the plan.",
    ),
    (
        "to keep track of",
        "Llevar registro de algo, no perderle la pista.",
        "I keep track of new words in a single notebook.",
    ),
    (
        "to put off",
        "Aplazar, dejar algo para después.",
        "Reading one card at a time is easier than putting it off entirely.",
    ),
];

/// Builds the starter collection shown on the very first run.
pub fn sample_collection(rng: &mut Rng) -> Collection {
    let mut collection = Collection::new(SAMPLE_COLLECTION_NAME, rng);
    collection.note = SAMPLE_NOTE.to_string();
    for (word, meaning, example) in SAMPLE_ENTRIES {
        collection.entries.push(Entry {
            id: crate::model::new_id("e", rng),
            word: (*word).to_string(),
            meaning: (*meaning).to_string(),
            example: (*example).to_string(),
            source: SAMPLE_SOURCE.to_string(),
            source_language: "English".to_string(),
            target_language: "Spanish".to_string(),
            learned: false,
        });
    }
    collection
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Library, Rng};

    #[test]
    fn sample_collection_has_about_twenty_complete_entries() {
        let mut rng = Rng::new(1);
        let collection = sample_collection(&mut rng);
        assert!(
            (18..=24).contains(&collection.entries.len()),
            "expected about 20 samples, got {}",
            collection.entries.len()
        );
        for entry in &collection.entries {
            assert!(!entry.word.trim().is_empty());
            assert!(!entry.meaning.trim().is_empty());
            assert_eq!(entry.source_language, "English");
            assert_eq!(entry.target_language, "Spanish");
            assert!(!entry.learned);
        }
        // No fabricated book attribution: the source label is the app's own.
        assert!(collection.entries.iter().all(|e| e.source == SAMPLE_SOURCE));
        assert!(SAMPLE_NOTE.contains("Original"));
    }

    #[test]
    fn sample_words_are_unique() {
        let mut rng = Rng::new(2);
        let mut library = Library::default();
        library.collections.push(sample_collection(&mut rng));
        let collection = &library.collections[0];
        let keys: std::collections::HashSet<String> =
            collection.entries.iter().map(|e| e.key()).collect();
        assert_eq!(keys.len(), collection.entries.len());
    }
}
