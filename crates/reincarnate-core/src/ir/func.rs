use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::define_entity;
use crate::entity::PrimaryMap;

use super::block::{Block, BlockId};
use super::coroutine::CoroutineInfo;
use super::inst::{Inst, InstId};
use super::ty::{FunctionSig, Type};
use super::value::ValueId;

define_entity!(FuncId);

/// Visibility of a function or global.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
    Protected,
}

/// What kind of method a function represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MethodKind {
    #[default]
    Free,
    Constructor,
    Instance,
    Static,
    Getter,
    Setter,
}

/// A function in the IR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub sig: FunctionSig,
    pub visibility: Visibility,
    /// Namespace segments (e.g. `["classes", "Scenes", "Areas", "Bog"]`).
    #[serde(default)]
    pub namespace: Vec<String>,
    /// Owning class short name (e.g. `"Phouka"`).
    #[serde(default)]
    pub class: Option<String>,
    /// What kind of method this function represents.
    #[serde(default)]
    pub method_kind: MethodKind,
    pub blocks: PrimaryMap<BlockId, Block>,
    pub insts: PrimaryMap<InstId, Inst>,
    pub value_types: PrimaryMap<ValueId, Type>,
    /// Entry block — always the first block.
    pub entry: BlockId,
    /// If this function is a coroutine, metadata about it.
    pub coroutine: Option<CoroutineInfo>,
    /// Optional debug names for values (from source-level variable names).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub value_names: HashMap<ValueId, String>,
}
