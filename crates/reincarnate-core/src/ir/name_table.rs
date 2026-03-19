use serde::{Deserialize, Serialize};

use crate::entity::PrimaryMap;

use super::func::FuncId;

/// Centralized name storage for IR symbols.
///
/// Names are rendering hints, not identity — two symbols are the same iff they
/// have the same ID, regardless of name. The `NameTable` maps typed IDs to
/// their display names. All name lookups go through this table; IR structs
/// carry only opaque IDs.
///
/// Currently stores function names only. Other name fields (struct names, class
/// names, global names, field names, enum names) will migrate here in a future
/// phase.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NameTable {
    /// Function names, indexed by `FuncId`.
    pub func_names: PrimaryMap<FuncId, String>,
}

impl NameTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the name of a function by its ID.
    ///
    /// # Panics
    /// Panics if the `FuncId` is not in the table.
    pub fn func_name(&self, id: FuncId) -> &str {
        &self.func_names[id]
    }

    /// Mutably borrow a function name by its ID.
    ///
    /// # Panics
    /// Panics if the `FuncId` is not in the table.
    pub fn func_name_mut(&mut self, id: FuncId) -> &mut String {
        &mut self.func_names[id]
    }
}
