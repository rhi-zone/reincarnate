use serde::{Deserialize, Serialize};

use crate::define_entity;

use super::block::BlockId;
use super::func::FuncId;
use super::ty::Type;
use super::value::{Constant, ValueId};

define_entity!(InstId);

/// Source location span for diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub file: String,
    pub line: u32,
    pub col: u32,
}

/// An IR instruction: an operation with an optional result value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inst {
    pub op: Op,
    /// The value produced by this instruction, if any.
    pub result: Option<ValueId>,
    /// Source location for diagnostics.
    pub span: Option<Span>,
}

/// Distinguishes the two semantics of `Op::Cast`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CastKind {
    /// Nullable cast — returns null if the value is not an instance of the target type
    /// (e.g. AS3 `as`, Kotlin `as?`, C# `as`).
    NullableCoerce,
    /// Runtime coercion (Coerce/Convert opcodes).
    Coerce,
}

/// Comparison kind for relational operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CmpKind {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpKind {
    /// Return the inverse comparison (e.g. Lt ↔ Ge, Eq ↔ Ne).
    pub fn inverse(self) -> Self {
        match self {
            CmpKind::Eq => CmpKind::Ne,
            CmpKind::Ne => CmpKind::Eq,
            CmpKind::Lt => CmpKind::Ge,
            CmpKind::Ge => CmpKind::Lt,
            CmpKind::Gt => CmpKind::Le,
            CmpKind::Le => CmpKind::Gt,
        }
    }
}

/// IR operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Op {
    // -- Constants --
    /// Load a compile-time constant.
    Const(Constant),

    // -- Comparison --
    Cmp(CmpKind, ValueId, ValueId),
    /// Conditional select: `cond ? on_true : on_false`
    Select {
        cond: ValueId,
        on_true: ValueId,
        on_false: ValueId,
    },

    // -- Memory / fields --
    /// Allocate a local variable.
    Alloc(Type),
    /// Load from a pointer/reference.
    Load(ValueId),
    /// Store to a pointer/reference.
    Store {
        ptr: ValueId,
        value: ValueId,
    },
    /// Read a field from a struct or object instance.
    /// For reference types (`Array`, `Map`, `Instance`), returns a live reference —
    /// mutations through the result via `SetIndex`/`SetField` propagate back to the source.
    /// Each backend must preserve this contract.
    GetField {
        object: ValueId,
        field: String,
    },
    /// Write a field on a struct or object instance.
    /// For reference types, see `GetField` note on reference semantics.
    SetField {
        object: ValueId,
        field: String,
        value: ValueId,
    },
    /// Read an element from an array or map by index.
    /// For reference types, returns a live reference — see `GetField` for the contract.
    GetIndex {
        collection: ValueId,
        index: ValueId,
    },
    /// Write an element to an array or map by index.
    /// Mutates in place for reference types — no copy.
    SetIndex {
        collection: ValueId,
        index: ValueId,
        value: ValueId,
    },

    // -- Calls --
    /// Direct function call.
    Call {
        func: FuncId,
        args: Vec<ValueId>,
    },
    /// Create a closure: packages a function with captured outer-scope values.
    /// `captures` are bound to the function's capture params (in declaration order).
    MakeClosure {
        func: String,
        captures: Vec<ValueId>,
    },
    /// Indirect call through a value (function pointer / closure).
    CallIndirect {
        callee: ValueId,
        args: Vec<ValueId>,
    },
    /// System trait method call — string-based, resolved at codegen.
    SystemCall {
        system: String,
        method: String,
        args: Vec<ValueId>,
    },
    /// Method call on a receiver: `receiver.method(args...)`.
    MethodCall {
        receiver: ValueId,
        method: String,
        args: Vec<ValueId>,
    },

    // -- Type operations --
    /// Cast a value to a type.
    Cast(ValueId, Type, CastKind),
    /// Runtime type check (returns bool).
    TypeCheck(ValueId, Type),

    // -- Aggregate construction --
    /// Construct a struct.
    StructInit {
        name: String,
        fields: Vec<(String, ValueId)>,
    },
    /// Construct an array.
    ArrayInit(Vec<ValueId>),
    /// Construct a tuple.
    TupleInit(Vec<ValueId>),

    // -- Coroutines --
    /// Yield a value from a coroutine.
    Yield(Option<ValueId>),
    /// Create a coroutine from a function reference.
    CoroutineCreate {
        func: String,
        args: Vec<ValueId>,
    },
    /// Resume a coroutine, returning the yielded value.
    CoroutineResume(ValueId),

    // -- Misc --
    /// Reference to a global variable.
    GlobalRef(String),
    /// Spread operator: marks a value for spreading in arrays/objects/calls.
    Spread(ValueId),
}

impl Op {
    /// Returns a short, stable name for the `Op` variant (e.g. `"Call"`, `"GetField"`).
    /// Used for diagnostic grouping — must not include operand data.
    pub fn variant_name(&self) -> &'static str {
        match self {
            Op::Const(_) => "Const",
            Op::Cmp(..) => "Cmp",
            Op::Select { .. } => "Select",
            Op::Alloc(_) => "Alloc",
            Op::Load(_) => "Load",
            Op::Store { .. } => "Store",
            Op::GetField { .. } => "GetField",
            Op::SetField { .. } => "SetField",
            Op::GetIndex { .. } => "GetIndex",
            Op::SetIndex { .. } => "SetIndex",
            Op::Call { .. } => "Call",
            Op::MakeClosure { .. } => "MakeClosure",
            Op::CallIndirect { .. } => "CallIndirect",
            Op::SystemCall { .. } => "SystemCall",
            Op::MethodCall { .. } => "MethodCall",
            Op::Cast(..) => "Cast",
            Op::TypeCheck(..) => "TypeCheck",
            Op::StructInit { .. } => "StructInit",
            Op::ArrayInit(_) => "ArrayInit",
            Op::TupleInit(_) => "TupleInit",
            Op::Yield(_) => "Yield",
            Op::CoroutineCreate { .. } => "CoroutineCreate",
            Op::CoroutineResume(_) => "CoroutineResume",
            Op::GlobalRef(_) => "GlobalRef",
            Op::Spread(_) => "Spread",
        }
    }
}

/// Block terminator — explicit control-flow edge at the end of each block.
///
/// Every complete block has exactly one terminator. Terminators carry their
/// successor block IDs and block-argument lists, replacing the old
/// `Op::Br`/`Op::BrIf`/`Op::Switch`/`Op::Return` variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Terminator {
    /// Unconditional branch.
    Br { target: BlockId, args: Vec<ValueId> },
    /// Conditional branch.
    BrIf {
        cond: ValueId,
        then_target: BlockId,
        then_args: Vec<ValueId>,
        else_target: BlockId,
        else_args: Vec<ValueId>,
    },
    /// Multi-way switch.
    Switch {
        value: ValueId,
        cases: Vec<(Constant, BlockId, Vec<ValueId>)>,
        default: (BlockId, Vec<ValueId>),
    },
    /// Return from function.
    Return(Option<ValueId>),
}

impl Default for Terminator {
    fn default() -> Self {
        Terminator::Return(None)
    }
}
