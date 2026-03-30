use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::define_entity;
use crate::entity::PrimaryMap;

use super::block::{Block, BlockId};
use super::coroutine::CoroutineInfo;
use super::inst::{Inst, InstId, Op};
use super::module::SystemCallTypeRule;
use super::ty::{FunctionSig, Type};
use super::value::ValueId;

/// Inlining hint for a function.
///
/// Set to `Always` on runtime stubs that have IR bodies attached, so the
/// inliner can substitute the body at every call site.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum InlineHint {
    /// Always inline this function at every call site.
    Always,
    /// Use the default inlining heuristic (do not inline).
    #[default]
    Default,
}

/// The native backend behavior for an intrinsic function.
///
/// When a `Function` has `intrinsic: Some(kind)`, the IR-to-AST linear emitter
/// translates `Op::Call { func: name, args }` into `Expr::SystemCall { system, method, args }`
/// so that all existing engine-specific rewrite passes work unchanged.
///
/// The (system, method) mapping is defined by [`IntrinsicKind::system_method`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntrinsicKind {
    // GameMaker engine intrinsics — see `system_method()` for (system, method) mapping.
    GameMakerGetField,
    GameMakerSetField,
    GameMakerGetOn,
    GameMakerSetOn,
    GameMakerGetOther,
    GameMakerSetOther,
    GameMakerGetAll,
    GameMakerSetAll,
    GameMakerWithInstances,
    GameMakerGlobalGet,
    GameMakerGlobalSet,
    GameMakerArgumentGet,
    GameMakerDebugBreak,
}

impl IntrinsicKind {
    /// Return the `(system, method)` pair that this intrinsic maps to.
    ///
    /// Used by the linear emitter to lower `Op::Call` with an intrinsic function
    /// into `Expr::SystemCall { system, method, args }` so that all downstream
    /// engine-specific rewrite passes remain unchanged.
    pub fn system_method(&self) -> (&'static str, &'static str) {
        match self {
            IntrinsicKind::GameMakerGetField => ("GameMaker.Instance", "getField"),
            IntrinsicKind::GameMakerSetField => ("GameMaker.Instance", "setField"),
            IntrinsicKind::GameMakerGetOn => ("GameMaker.Instance", "getOn"),
            IntrinsicKind::GameMakerSetOn => ("GameMaker.Instance", "setOn"),
            IntrinsicKind::GameMakerGetOther => ("GameMaker.Instance", "getOther"),
            IntrinsicKind::GameMakerSetOther => ("GameMaker.Instance", "setOther"),
            IntrinsicKind::GameMakerGetAll => ("GameMaker.Instance", "getAll"),
            IntrinsicKind::GameMakerSetAll => ("GameMaker.Instance", "setAll"),
            IntrinsicKind::GameMakerWithInstances => ("GameMaker.Instance", "withInstances"),
            IntrinsicKind::GameMakerGlobalGet => ("GameMaker.Global", "get"),
            IntrinsicKind::GameMakerGlobalSet => ("GameMaker.Global", "set"),
            IntrinsicKind::GameMakerArgumentGet => ("GameMaker.Argument", "get"),
            IntrinsicKind::GameMakerDebugBreak => ("GameMaker.Debug", "break"),
        }
    }

    /// Return the canonical call name used in `Op::Call { func: name }`.
    ///
    /// Formed as `system + "." + method` where the system already uses dots,
    /// e.g. `"GameMaker.Instance.getField"`.
    pub fn call_name(&self) -> String {
        let (system, method) = self.system_method();
        format!("{system}.{method}")
    }
}

define_entity!(FuncId);

/// Visibility of a function or global.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
    Protected,
}

/// How a variable is captured in a closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureMode {
    /// Snapshot the value at closure-creation time.
    ByValue,
    /// Capture by reference (mutable binding shared with the outer scope).
    ByRef,
}

/// A capture parameter declared on a closure function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureParam {
    pub name: String,
    pub ty: Type,
    pub mode: CaptureMode,
}

/// What kind of method a function represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MethodKind {
    #[default]
    Free,
    Constructor,
    Instance,
    Static,
    /// AS3 class static initializer (`cinit`) — runs once when the class is
    /// first referenced.  Distinct from `Static` so the backend can identify
    /// it without falling back to a name-based "cinit" string check.
    StaticInit,
    Getter,
    Setter,
    Closure,
}

/// A function in the IR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    /// Function name. Kept on the struct for backward compatibility during
    /// the NameTable migration. The authoritative name source is
    /// `Module::name_table`; callers with a `FuncId` should prefer
    /// `module.func_name(id)`.
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
    /// Capture parameters for closure functions (empty for non-closures).
    ///
    /// In the entry block, capture params are appended after the regular
    /// `sig.params`. Use [`FunctionBuilder::capture_param`] to access them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capture_params: Vec<CaptureParam>,
    /// ValueIds of Mem2Reg null-sentinel constants — one per promoted alloc.
    ///
    /// Mem2Reg inserts a `Const(Null)` at the function entry as the initial
    /// "reaching definition" for each promoted alloc (the conservative SSA
    /// value for the pre-first-store case). These are not real null values from
    /// user code; they represent an uninitialized slot. Block-arg assignments
    /// from these sentinels should be skipped during code emission so that
    /// TypeScript's definite-assignment analysis can verify correct initialization.
    ///
    /// Populated by Mem2Reg. Skipped during serialization — this data is only valid
    /// within a single pipeline run and cannot be reconstructed from the serialized IR.
    /// If IR is ever serialized post-Mem2Reg (e.g. for incremental builds), this field
    /// would need to be either serialized or the sentinel logic made reconstructable.
    #[serde(skip)]
    pub null_sentinel_values: HashSet<ValueId>,
    /// Type rule for syscall-replacement functions.
    ///
    /// When set, the constraint collector and HM solver use this rule to infer
    /// the result type of `Op::Call` invocations targeting this function,
    /// exactly as they would for `Op::SystemCall` entries in `system_call_type_rules`.
    ///
    /// `None` for regular (non-intrinsic) functions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_rule: Option<SystemCallTypeRule>,
    /// Intrinsic kind — how the backend should emit this function.
    ///
    /// When `Some`, the linear emitter lowers `Op::Call { func: name }` to
    /// `Expr::SystemCall { system, method, args }` using [`IntrinsicKind::system_method`],
    /// so that all existing engine-specific rewrite passes work unchanged.
    ///
    /// `None` for regular functions (emitted as a plain call).
    ///
    /// Not serialized — rebuilt by the frontend on every run.
    #[serde(skip)]
    pub intrinsic: Option<IntrinsicKind>,
    /// Typed overload specializations for polymorphic `_any` builtins.
    ///
    /// Maps a concrete argument-type signature (ordered list of [`Type`] values,
    /// one per argument) to the [`FuncId`] of the typed variant that should be
    /// called when those argument types are known at compile time.
    ///
    /// Non-empty only on `builtin.xxx_any` stubs registered by
    /// [`Module::register_arithmetic_any_builtins`].  All other functions leave this empty.
    ///
    /// The `BuiltinOverloadSelect` transform reads this table to replace
    /// `_any` calls with their concrete typed counterparts without any
    /// string manipulation or hardcoded type maps.
    ///
    /// Not serialized — rebuilt by `register_arithmetic_any_builtins` on every run.
    #[serde(skip)]
    pub specializations: HashMap<Vec<Type>, FuncId>,
    /// Inlining hint — whether the inliner should substitute this function's
    /// body at call sites.
    ///
    /// Set to `InlineHint::Always` by `register_runtime_bodies` after attaching
    /// an IR body to a stub so the inline pass can substitute it everywhere.
    ///
    /// Not serialized — rebuilt by the frontend on every run.
    #[serde(skip)]
    pub inline_hint: InlineHint,
}

impl Function {
    /// Remove dead instructions from the arena.
    ///
    /// After transforms like Mem2Reg and DCE, instructions removed from blocks
    /// remain in the `insts` arena. This compacts the arena so only live
    /// instructions remain, allowing downstream consumers to safely iterate it.
    pub fn compact_insts(&mut self) {
        let mut live: HashSet<InstId> = HashSet::new();
        for block in self.blocks.values() {
            for &inst_id in &block.insts {
                live.insert(inst_id);
            }
        }

        if live.len() == self.insts.len() {
            return;
        }

        let mut new_insts = PrimaryMap::new();
        let mut remap: HashMap<InstId, InstId> = HashMap::new();
        for (old_id, inst) in self.insts.iter() {
            if live.contains(&old_id) {
                let new_id = new_insts.push(inst.clone());
                remap.insert(old_id, new_id);
            }
        }

        for block in self.blocks.values_mut() {
            for inst_id in &mut block.insts {
                *inst_id = remap[inst_id];
            }
        }

        self.insts = new_insts;
    }

    /// Move all `Op::Alloc` instructions to the entry block.
    ///
    /// In most supported source languages (GML, Harlowe, Flash AS3), variable
    /// declarations have function scope — a variable set inside an `if` branch
    /// is visible for the rest of the function. But when a frontend emits
    /// `Op::Alloc` inside a nested block (e.g. inside an if-branch), the
    /// emitted TypeScript `let _x` is block-scoped, causing TS2304 errors for
    /// references outside that block.
    ///
    /// This pass corrects that by moving every `Op::Alloc` to the front of
    /// the entry block, where it emits as a function-level declaration.
    /// Downstream passes (Mem2Reg, structurizer, emitter) are unaffected:
    /// Mem2Reg only cares about Alloc/Store/Load relationships, not position,
    /// and moving an Alloc earlier never breaks data-flow since it produces a
    /// slot rather than consuming a value.
    pub fn hoist_allocs(&mut self) {
        let entry = self.entry;
        let block_ids: Vec<BlockId> = self.blocks.keys().collect();
        let mut allocs_to_hoist: Vec<InstId> = vec![];

        for bid in block_ids {
            if bid == entry {
                continue;
            }
            // Collect alloc positions from this block (in order).
            let positions: Vec<usize> = self.blocks[bid]
                .insts
                .iter()
                .enumerate()
                .filter_map(|(i, &iid)| matches!(self.insts[iid].op, Op::Alloc(_)).then_some(i))
                .collect();
            // Remove in reverse to preserve indices.
            for pos in positions.into_iter().rev() {
                allocs_to_hoist.push(self.blocks[bid].insts.remove(pos));
            }
        }

        if allocs_to_hoist.is_empty() {
            return;
        }

        // allocs_to_hoist is reverse-order (last block's last alloc first).
        allocs_to_hoist.reverse();

        // Prepend all hoisted allocs to the entry block.
        let entry_insts = &mut self.blocks[entry].insts;
        let existing = std::mem::take(entry_insts);
        entry_insts.extend(allocs_to_hoist);
        entry_insts.extend(existing);
    }
}
