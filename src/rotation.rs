//! Rotation engine: sequential or shuffled-without-repetition traversal of the
//! eligible entries of the active collection.
//!
//! The engine only ever works with entry ids. The caller passes the list of
//! *eligible* ids (the active collection minus learned entries unless the user
//! asked to keep them), which keeps the engine free of policy decisions and
//! easy to test.

use crate::model::{Entry, Order, Rng};

/// How many previously shown entries are remembered for the "previous" action.
const HISTORY_LIMIT: usize = 64;

#[derive(Debug, Default)]
pub struct Rotation {
    order: Order,
    /// Remaining ids of the current shuffle cycle (unused in sequential mode).
    cycle: Vec<String>,
    /// Recently shown ids, newest last.
    history: Vec<String>,
    last: Option<String>,
}

impl Rotation {
    pub fn new(order: Order) -> Self {
        Rotation {
            order,
            cycle: Vec::new(),
            history: Vec::new(),
            last: None,
        }
    }

    pub fn order(&self) -> Order {
        self.order
    }

    /// Changes the order mode and drops the current cycle so the next draw
    /// starts a fresh one.
    pub fn set_order(&mut self, order: Order) {
        if self.order != order {
            self.order = order;
            self.cycle.clear();
        }
    }

    /// Forgets current position, cycle and history (used when the active
    /// collection changes or its contents are edited).
    pub fn reset(&mut self) {
        self.cycle.clear();
        self.history.clear();
        self.last = None;
    }

    /// Keeps the last shown id only if it is still eligible; otherwise the next
    /// draw starts over.
    pub fn retain_current(&mut self, eligible: &[String]) {
        match &self.last {
            Some(id) if eligible.iter().any(|e| e == id) => {}
            _ => {
                self.last = None;
                self.history.clear();
            }
        }
        self.cycle.retain(|id| eligible.iter().any(|e| e == id));
    }

    pub fn current(&self) -> Option<&str> {
        self.last.as_deref()
    }

    /// Sets the current entry (e.g. restoring the card after a restart).
    pub fn set_current(&mut self, id: Option<String>) {
        self.last = id;
    }

    /// Next id to show. `None` when there is nothing eligible to show.
    pub fn next(&mut self, eligible: &[String], rng: &mut Rng) -> Option<String> {
        if eligible.is_empty() {
            self.last = None;
            self.cycle.clear();
            self.history.clear();
            return None;
        }

        let current: Option<String> = self.last.clone();
        let next = match (self.order, current.as_deref()) {
            (_, None) => match self.order {
                // Deterministic start: first eligible entry.
                Order::Sequential => eligible.first().cloned(),
                Order::Shuffled => {
                    self.refill_cycle(eligible, rng);
                    self.cycle.pop()
                }
            },
            (Order::Sequential, Some(current)) => {
                let index = eligible
                    .iter()
                    .position(|id| id == current)
                    .unwrap_or(eligible.len() - 1);
                Some(eligible[(index + 1) % eligible.len()].clone())
            }
            (Order::Shuffled, Some(current)) => {
                self.cycle.retain(|id| id != current);
                if self.cycle.is_empty() {
                    self.refill_cycle(eligible, rng);
                    // Avoid showing the same word twice in a row at a cycle boundary.
                    if self.cycle.len() > 1
                        && self.cycle.last().map(|s| s.as_str()) == Some(current)
                    {
                        let n = self.cycle.len();
                        self.cycle.swap(n - 1, n - 2);
                    }
                }
                self.cycle.pop()
            }
        };

        if let Some(id) = &next {
            if let Some(current) = self.last.take() {
                if current != *id {
                    self.history.push(current);
                    if self.history.len() > HISTORY_LIMIT {
                        self.history.remove(0);
                    }
                }
            }
            self.last = Some(id.clone());
        }
        next
    }

    /// Previous id to show: walks the recent history when possible and falls
    /// back to the deterministic predecessor in the eligible list.
    pub fn prev(&mut self, eligible: &[String], _rng: &mut Rng) -> Option<String> {
        if eligible.is_empty() {
            self.last = None;
            self.history.clear();
            return None;
        }
        while let Some(candidate) = self.history.pop() {
            if eligible.iter().any(|id| id == &candidate) {
                self.last = Some(candidate.clone());
                return Some(candidate);
            }
        }
        let current = self.last.as_deref()?;
        let index = eligible.iter().position(|id| id == current).unwrap_or(0);
        let prev = eligible[(index + eligible.len() - 1) % eligible.len()].clone();
        self.last = Some(prev.clone());
        Some(prev)
    }

    /// Fills the cycle with a shuffled copy of the eligible ids. The cycle is
    /// consumed from the end, so the first drawn item is `cycle.last()`.
    fn refill_cycle(&mut self, eligible: &[String], rng: &mut Rng) {
        self.cycle.clear();
        self.cycle.extend_from_slice(eligible);
        rng.shuffle(&mut self.cycle);
    }
}

/// Ids that may be shown: every entry, or only the ones not marked as learned.
pub fn eligible_ids(entries: &[Entry], exclude_learned: bool) -> Vec<String> {
    entries
        .iter()
        .filter(|e| !exclude_learned || !e.learned)
        .map(|e| e.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Entry;

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("e{i}")).collect()
    }

    #[test]
    fn sequential_walks_in_order_and_wraps() {
        let eligible = ids(3);
        let mut rng = Rng::new(1);
        let mut rotation = Rotation::new(Order::Sequential);
        let seen: Vec<String> = (0..5)
            .map(|_| rotation.next(&eligible, &mut rng).unwrap())
            .collect();
        assert_eq!(seen, vec!["e0", "e1", "e2", "e0", "e1"]);
        assert_eq!(rotation.current(), Some("e1"));
    }

    #[test]
    fn shuffled_shows_every_entry_once_per_cycle() {
        let eligible = ids(8);
        let mut rng = Rng::new(99);
        let mut rotation = Rotation::new(Order::Shuffled);
        let mut cycle: Vec<String> = (0..8)
            .map(|_| rotation.next(&eligible, &mut rng).unwrap())
            .collect();
        let mut sorted = cycle.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(cycle.len(), 8, "one cycle should draw every entry once");
        assert_eq!(sorted.len(), 8, "no repetition inside a cycle: {cycle:?}");

        // The next cycle must not repeat the entry that closed the previous one.
        let first_of_next = rotation.next(&eligible, &mut rng).unwrap();
        assert_ne!(Some(&first_of_next), cycle.last());

        cycle.clear();
        cycle.extend((0..8).map(|_| rotation.next(&eligible, &mut rng).unwrap()));
        assert!(!cycle.contains(&first_of_next) && !cycle.is_empty());
    }

    #[test]
    fn shuffled_two_entries_never_repeat_back_to_back() {
        let eligible = ids(2);
        let mut rng = Rng::new(5);
        let mut rotation = Rotation::new(Order::Shuffled);
        let mut last: Option<String> = None;
        for _ in 0..20 {
            let next = rotation.next(&eligible, &mut rng).unwrap();
            assert_ne!(last.as_deref(), Some(next.as_str()));
            last = Some(next);
        }
    }

    #[test]
    fn previous_prefers_history_then_falls_back() {
        let eligible = ids(4);
        let mut rng = Rng::new(11);
        let mut rotation = Rotation::new(Order::Sequential);
        for _ in 0..3 {
            rotation.next(&eligible, &mut rng);
        }
        assert_eq!(rotation.current(), Some("e2"));
        assert_eq!(rotation.prev(&eligible, &mut rng).as_deref(), Some("e1"));
        assert_eq!(rotation.prev(&eligible, &mut rng).as_deref(), Some("e0"));
        // History is exhausted, so it wraps to the deterministic predecessor.
        assert_eq!(rotation.prev(&eligible, &mut rng).as_deref(), Some("e3"));
    }

    #[test]
    fn empty_eligibility_returns_none_and_clears_state() {
        let mut rng = Rng::new(1);
        let mut rotation = Rotation::new(Order::Shuffled);
        assert!(rotation.next(&ids(2), &mut rng).is_some());
        assert_eq!(rotation.next(&[], &mut rng), None);
        assert_eq!(rotation.current(), None);
        assert_eq!(rotation.prev(&[], &mut rng), None);
    }

    #[test]
    fn learned_entries_are_filtered_out_of_eligibility() {
        let mut rng = Rng::new(21);
        let mut entries = vec![
            Entry::new("adroit", "hábil", &mut rng),
            Entry::new("hindsight", "retrospectiva", &mut rng),
            Entry::new("outlier", "valor atípico", &mut rng),
            Entry::new("threshold", "umbral", &mut rng),
        ];
        entries[1].learned = true;
        entries[3].learned = true;

        let eligible = eligible_ids(&entries, true);
        assert_eq!(eligible, vec![entries[0].id.clone(), entries[2].id.clone()]);

        // With the exclusion turned off every entry is eligible again.
        let all = eligible_ids(&entries, false);
        assert_eq!(all.len(), 4);

        // The engine never returns a learned entry.
        let mut rotation = Rotation::new(Order::Shuffled);
        for _ in 0..12 {
            let id = rotation.next(&eligible, &mut rng).unwrap();
            assert_ne!(id, entries[1].id);
            assert_ne!(id, entries[3].id);
        }

        // Every entry learned: nothing eligible, card must show the "all learned" state.
        for entry in &mut entries {
            entry.learned = true;
        }
        assert!(eligible_ids(&entries, true).is_empty());
        assert_eq!(rotation.next(&eligible_ids(&entries, true), &mut rng), None);
    }

    #[test]
    fn retain_current_drops_stale_selection() {
        let mut rng = Rng::new(4);
        let mut rotation = Rotation::new(Order::Sequential);
        rotation.set_current(Some("e1".into()));
        rotation.retain_current(&ids(3));
        assert_eq!(rotation.current(), Some("e1"));
        rotation.retain_current(&vec!["e0".to_string()]);
        assert_eq!(rotation.current(), None);
        let _ = &mut rng;
    }
}
