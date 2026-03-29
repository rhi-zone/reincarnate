use serde::{Deserialize, Serialize};

use crate::define_entity;

use super::ty::Type;

define_entity!(ValueId);

/// A compile-time constant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Constant {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    String(String),
}

impl Eq for Constant {}

impl std::hash::Hash for Constant {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Use the discriminant to distinguish variants.
        std::mem::discriminant(self).hash(state);
        match self {
            Constant::Null => {}
            Constant::Bool(b) => b.hash(state),
            Constant::Int(i) => i.hash(state),
            Constant::UInt(u) => u.hash(state),
            // Hash floats by bit representation so the impl is consistent with
            // PartialEq for non-NaN values.  NaN constants are not valid in
            // function default signatures so this edge case does not arise.
            Constant::Float(f) => f.to_bits().hash(state),
            Constant::String(s) => s.hash(state),
        }
    }
}

impl Constant {
    /// Infer the type of this constant.
    pub fn ty(&self) -> Type {
        match self {
            Constant::Null => Type::Option(Box::new(Type::Unknown)),
            Constant::Bool(_) => Type::Bool,
            Constant::Int(_) => Type::Int(64),
            Constant::UInt(_) => Type::UInt(64),
            Constant::Float(_) => Type::Float(64),
            Constant::String(_) => Type::String,
        }
    }
}
