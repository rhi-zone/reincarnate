pub mod ast;
pub(crate) mod ast_passes;
pub mod block;
pub mod builder;
pub mod coroutine;
pub mod func;
pub mod inst;
pub mod linear;
pub mod module;
pub mod name_interner;
pub mod name_table;
pub mod printer;
pub mod structurize;
pub mod ty;
pub mod value;

pub use ast::{AstFunction, Expr, Stmt};
pub use block::{Block, BlockId, BlockParam};
pub use builder::{FunctionBuilder, ModuleBuilder};
pub use coroutine::CoroutineInfo;
pub use func::{CaptureMode, FuncId, Function, IntrinsicKind, MethodKind, Visibility};
pub use inst::{CastKind, CmpKind, Inst, InstId, Op, Span, Terminator};
pub use linear::lower_function_linear;
pub use module::{
    AbstractMember, ClassDef, EntryPoint, EnumDef, EnumVariant, ExternalImport, FieldDef, Global,
    Import, Module, StaticField, StructDef, SystemCallTypeRule, TypeDecl, TypeInterner,
};
pub use name_interner::NameInterner;
pub use name_table::NameTable;
pub use structurize::structurize;
pub use ty::{parse_type_notation, FunctionSig, Type, TypeConstraint, TypeId, TypeVarId};
pub use value::{Constant, ValueId};
