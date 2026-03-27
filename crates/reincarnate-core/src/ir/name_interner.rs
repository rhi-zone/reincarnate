use std::collections::HashSet;

/// Collision-free name generation utility.
///
/// Tracks which names are taken in a given namespace. Given a candidate name,
/// [`NameInterner::intern`] returns a unique name — either the candidate itself
/// (if not yet taken) or the candidate with `_2`, `_3`, … appended until a
/// free slot is found. The returned name is then marked taken.
///
/// Each name scope should use its own `NameInterner` instance. The interner has
/// no knowledge of any specific engine or target language — suffix strategies
/// belong at call sites.
///
/// # Determinism
///
/// The output of `intern` is deterministic given the same sequence of `reserve`
/// and `intern` calls. It is *not* order-independent: inserting names in a
/// different order may produce different suffixes. Callers that require
/// order-independent output should pre-`reserve` all known names before
/// calling `intern`.
#[derive(Debug, Default, Clone)]
pub struct NameInterner {
    taken: HashSet<String>,
}

impl NameInterner {
    /// Create an empty interner.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an interner with a set of pre-reserved names.
    ///
    /// Equivalent to calling `reserve` for each name in `reserved`.
    pub fn with_reserved(reserved: impl IntoIterator<Item = String>) -> Self {
        let taken = reserved.into_iter().collect();
        Self { taken }
    }

    /// Intern `candidate`.
    ///
    /// If `candidate` is not taken, marks it taken and returns it unchanged.
    /// Otherwise, tries `{candidate}_2`, `{candidate}_3`, … until a free name
    /// is found, marks it taken, and returns it.
    pub fn intern(&mut self, candidate: String) -> String {
        if !self.taken.contains(&candidate) {
            self.taken.insert(candidate.clone());
            return candidate;
        }
        let mut n: u64 = 2;
        loop {
            let attempt = format!("{candidate}_{n}");
            if !self.taken.contains(&attempt) {
                self.taken.insert(attempt.clone());
                return attempt;
            }
            n += 1;
        }
    }

    /// Mark `name` as taken without generating or returning a unique variant.
    ///
    /// Use this to pre-claim names that already exist in the namespace before
    /// calling [`intern`](Self::intern) for new candidates.
    pub fn reserve(&mut self, name: String) {
        self.taken.insert(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_name_returned_unchanged() {
        let mut interner = NameInterner::new();
        assert_eq!(interner.intern("Foo".to_string()), "Foo");
    }

    #[test]
    fn collision_appends_2() {
        let mut interner = NameInterner::new();
        interner.intern("Foo".to_string());
        assert_eq!(interner.intern("Foo".to_string()), "Foo_2");
    }

    #[test]
    fn second_collision_appends_3() {
        let mut interner = NameInterner::new();
        interner.intern("Foo".to_string());
        interner.intern("Foo".to_string());
        assert_eq!(interner.intern("Foo".to_string()), "Foo_3");
    }

    #[test]
    fn reserve_blocks_name() {
        let mut interner = NameInterner::new();
        interner.reserve("Bar".to_string());
        assert_eq!(interner.intern("Bar".to_string()), "Bar_2");
    }

    #[test]
    fn with_reserved_pre_claims_names() {
        let mut interner = NameInterner::with_reserved(["Alpha".to_string(), "Beta".to_string()]);
        assert_eq!(interner.intern("Alpha".to_string()), "Alpha_2");
        assert_eq!(interner.intern("Beta".to_string()), "Beta_2");
        assert_eq!(interner.intern("Gamma".to_string()), "Gamma");
    }

    #[test]
    fn skips_over_already_taken_suffix() {
        // If Foo_2 is already reserved, intern("Foo") after Foo is taken → Foo_3.
        let mut interner = NameInterner::with_reserved(["Foo_2".to_string()]);
        interner.intern("Foo".to_string()); // takes "Foo"
        assert_eq!(interner.intern("Foo".to_string()), "Foo_3");
    }
}
