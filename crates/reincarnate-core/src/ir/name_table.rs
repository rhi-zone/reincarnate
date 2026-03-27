use serde::{Deserialize, Serialize};

use crate::entity::PrimaryMap;

use super::func::FuncId;
use super::ty::TypeId;

/// Centralized name storage for IR symbols.
///
/// Names are rendering hints, not identity — two symbols are the same iff they
/// have the same ID, regardless of name. The `NameTable` maps typed IDs to
/// their display names. All name lookups go through this table; IR structs
/// carry only opaque IDs.
///
/// Currently stores function names and type names (`TypeDecl::Object` and
/// `TypeDecl::Enum`). Globals, fields, and enum variants lack typed IDs and
/// cannot migrate until `GlobalId`/`FieldId` are introduced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NameTable {
    /// Function names, indexed by `FuncId`.
    pub func_names: PrimaryMap<FuncId, String>,
    /// Type names, indexed by `TypeId`. `None` means the type is anonymous.
    ///
    /// Parallel to `Module::types`. Every push to `Module::types` must also
    /// push to this map to keep the two in sync.
    #[serde(default)]
    pub type_names: PrimaryMap<TypeId, Option<String>>,
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

    /// Look up the optional name of a type by its ID.
    ///
    /// Returns `None` if the type is anonymous or the ID is not in the table.
    pub fn type_name(&self, id: TypeId) -> Option<&str> {
        self.type_names.get(id).and_then(|n| n.as_deref())
    }

    /// Look up the name of a type by its ID, panicking if unnamed.
    ///
    /// # Panics
    /// Panics if the `TypeId` is not in the table or the type has no name.
    pub fn type_name_expect(&self, id: TypeId) -> &str {
        self.type_name(id).expect("TypeId has no name in NameTable")
    }
}
