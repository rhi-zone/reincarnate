use std::fmt;

use crate::entity::EntityRef;

use super::func::{Function, MethodKind, Visibility};
use super::inst::{CastKind, CmpKind, Op, Terminator};
use super::module::Module;
use super::ty::Type;
use super::value::Constant;
use super::ValueId;

fn fmt_type(ty: &Type, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match ty {
        Type::Void => write!(f, "void"),
        Type::Bool => write!(f, "bool"),
        Type::Int(bits) => write!(f, "i{bits}"),
        Type::UInt(bits) => write!(f, "u{bits}"),
        Type::Float(bits) => write!(f, "f{bits}"),
        Type::String => write!(f, "string"),
        Type::Array(elem) => {
            write!(f, "[")?;
            fmt_type(elem, f)?;
            write!(f, "]")
        }
        Type::Map(k, v) => {
            write!(f, "{{")?;
            fmt_type(k, f)?;
            write!(f, " -> ")?;
            fmt_type(v, f)?;
            write!(f, "}}")
        }
        Type::Option(inner) => {
            write!(f, "?")?;
            fmt_type(inner, f)
        }
        Type::Tuple(elems) => {
            write!(f, "(")?;
            for (i, elem) in elems.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                fmt_type(elem, f)?;
            }
            write!(f, ")")
        }
        Type::Instance(id) => write!(f, "type{}", id.index()),
        Type::ClassRef(id) => write!(f, "classref(type{})", id.index()),
        Type::Function(sig) => {
            write!(f, "fn(")?;
            for (i, p) in sig.params.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                fmt_type(p, f)?;
            }
            write!(f, ") -> ")?;
            fmt_type(&sig.return_ty, f)
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => {
            write!(f, "coroutine<")?;
            fmt_type(yield_ty, f)?;
            write!(f, ", ")?;
            fmt_type(return_ty, f)?;
            write!(f, ">")
        }
        Type::Var(id) => write!(f, "tvar{}", id.index()),
        Type::Union(types) => {
            for (i, t) in types.iter().enumerate() {
                if i > 0 {
                    write!(f, " | ")?;
                }
                fmt_type(t, f)?;
            }
            Ok(())
        }
        Type::Unknown => write!(f, "unknown"),
    }
}

fn fmt_value(v: ValueId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "v{}", v.index())
}

fn fmt_value_list(values: &[ValueId], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        fmt_value(*v, f)?;
    }
    Ok(())
}

fn fmt_constant(c: &Constant, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match c {
        Constant::Null => write!(f, "null"),
        Constant::Bool(b) => write!(f, "{b}"),
        Constant::Int(n) => write!(f, "{n}"),
        Constant::UInt(n) => write!(f, "{n}"),
        Constant::Float(v) => {
            if v.fract() == 0.0 && v.is_finite() {
                write!(f, "{v:.1}")
            } else {
                write!(f, "{v}")
            }
        }
        Constant::String(s) => write!(f, "{s:?}"),
    }
}

fn fmt_block_target(
    block: super::block::BlockId,
    args: &[ValueId],
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    write!(f, "block{}", block.index())?;
    write!(f, "(")?;
    fmt_value_list(args, f)?;
    write!(f, ")")
}

fn fmt_cmp_kind(kind: CmpKind, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match kind {
        CmpKind::Eq => write!(f, "eq"),
        CmpKind::Ne => write!(f, "ne"),
        CmpKind::Lt => write!(f, "lt"),
        CmpKind::Le => write!(f, "le"),
        CmpKind::Gt => write!(f, "gt"),
        CmpKind::Ge => write!(f, "ge"),
    }
}

fn fmt_method_kind(kind: MethodKind, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match kind {
        MethodKind::Free => Ok(()),
        MethodKind::Constructor => write!(f, " [constructor]"),
        MethodKind::Instance => write!(f, " [instance]"),
        MethodKind::Static => write!(f, " [static]"),
        MethodKind::StaticInit => write!(f, " [static_init]"),
        MethodKind::Getter => write!(f, " [getter]"),
        MethodKind::Setter => write!(f, " [setter]"),
        MethodKind::Closure => write!(f, " [closure]"),
    }
}

fn fmt_op(op: &Op, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match op {
        Op::Const(c) => {
            write!(f, "const ")?;
            fmt_constant(c, f)
        }
        Op::Cmp(kind, a, b) => {
            write!(f, "cmp.")?;
            fmt_cmp_kind(*kind, f)?;
            write!(f, " ")?;
            fmt_value(*a, f)?;
            write!(f, ", ")?;
            fmt_value(*b, f)
        }
        Op::Alloc(ty) => {
            write!(f, "alloc ")?;
            fmt_type(ty, f)
        }
        Op::Load(ptr) => {
            write!(f, "load ")?;
            fmt_value(*ptr, f)
        }
        Op::Store { ptr, value } => {
            write!(f, "store ")?;
            fmt_value(*ptr, f)?;
            write!(f, ", ")?;
            fmt_value(*value, f)
        }
        Op::GetField { object, field } => {
            write!(f, "get_field ")?;
            fmt_value(*object, f)?;
            write!(f, ", {field:?}")
        }
        Op::SetField {
            object,
            field,
            value,
        } => {
            write!(f, "set_field ")?;
            fmt_value(*object, f)?;
            write!(f, ", {field:?}, ")?;
            fmt_value(*value, f)
        }
        Op::GetIndex { collection, index } => {
            write!(f, "get_index ")?;
            fmt_value(*collection, f)?;
            write!(f, ", ")?;
            fmt_value(*index, f)
        }
        Op::SetIndex {
            collection,
            index,
            value,
        } => {
            write!(f, "set_index ")?;
            fmt_value(*collection, f)?;
            write!(f, ", ")?;
            fmt_value(*index, f)?;
            write!(f, ", ")?;
            fmt_value(*value, f)
        }
        Op::Call { func, args } => {
            // Pretty-print builtin arithmetic/logic ops with their mnemonic
            // (e.g. `builtin.add_i64` → `add`), stripping the type suffix.
            if let Some(rest) = func.strip_prefix("builtin.") {
                let mnemonic = rest.rfind('_').map(|i| &rest[..i]).unwrap_or(rest);
                write!(f, "{mnemonic} ")?;
                fmt_value_list(args, f)
            } else {
                write!(f, "call {func:?}(")?;
                fmt_value_list(args, f)?;
                write!(f, ")")
            }
        }
        Op::CallIndirect { callee, args } => {
            write!(f, "call_indirect ")?;
            fmt_value(*callee, f)?;
            write!(f, "(")?;
            fmt_value_list(args, f)?;
            write!(f, ")")
        }
        Op::SystemCall {
            system,
            method,
            args,
        } => {
            write!(f, "syscall {system:?}.{method:?}(")?;
            fmt_value_list(args, f)?;
            write!(f, ")")
        }
        Op::MethodCall {
            receiver,
            method,
            args,
        } => {
            write!(f, "call_method ")?;
            fmt_value(*receiver, f)?;
            write!(f, ".{method:?}(")?;
            fmt_value_list(args, f)?;
            write!(f, ")")
        }
        Op::Cast(val, ty, kind) => {
            match kind {
                CastKind::NullableCoerce => write!(f, "nullable_coerce ")?,
                CastKind::Coerce => write!(f, "coerce ")?,
            }
            fmt_value(*val, f)?;
            write!(f, ", ")?;
            fmt_type(ty, f)
        }
        Op::TypeCheck(val, ty) => {
            write!(f, "type_check ")?;
            fmt_value(*val, f)?;
            write!(f, ", ")?;
            fmt_type(ty, f)
        }
        Op::StructInit { name, fields } => {
            write!(f, "struct_init {name:?} {{ ")?;
            for (i, (field_name, val)) in fields.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{field_name}: ")?;
                fmt_value(*val, f)?;
            }
            write!(f, " }}")
        }
        Op::ArrayInit(elems) => {
            write!(f, "array_init [")?;
            fmt_value_list(elems, f)?;
            write!(f, "]")
        }
        Op::TupleInit(elems) => {
            write!(f, "tuple_init (")?;
            fmt_value_list(elems, f)?;
            write!(f, ")")
        }
        Op::Yield(val) => {
            write!(f, "yield")?;
            if let Some(v) = val {
                write!(f, " ")?;
                fmt_value(*v, f)?;
            }
            Ok(())
        }
        Op::CoroutineCreate { func, args } => {
            write!(f, "coroutine_create {func:?}(")?;
            fmt_value_list(args, f)?;
            write!(f, ")")
        }
        Op::CoroutineResume(val) => {
            write!(f, "coroutine_resume ")?;
            fmt_value(*val, f)
        }
        Op::MakeClosure { func, captures } => {
            write!(f, "make_closure {func:?}")?;
            for cap in captures {
                write!(f, ", ")?;
                fmt_value(*cap, f)?;
            }
            Ok(())
        }
        Op::GlobalRef(name) => write!(f, "global_ref {name:?}"),
        Op::Spread(val) => {
            write!(f, "spread ")?;
            fmt_value(*val, f)
        }
        Op::Select {
            cond,
            on_true,
            on_false,
        } => {
            write!(f, "select ")?;
            fmt_value(*cond, f)?;
            write!(f, ", ")?;
            fmt_value(*on_true, f)?;
            write!(f, ", ")?;
            fmt_value(*on_false, f)
        }
    }
}

fn fmt_terminator(term: &Terminator, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match term {
        Terminator::Br { target, args } => {
            write!(f, "br ")?;
            fmt_block_target(*target, args, f)
        }
        Terminator::BrIf {
            cond,
            then_target,
            then_args,
            else_target,
            else_args,
        } => {
            write!(f, "br_if ")?;
            fmt_value(*cond, f)?;
            write!(f, ", ")?;
            fmt_block_target(*then_target, then_args, f)?;
            write!(f, ", ")?;
            fmt_block_target(*else_target, else_args, f)
        }
        Terminator::Switch {
            value,
            cases,
            default,
        } => {
            write!(f, "switch ")?;
            fmt_value(*value, f)?;
            write!(f, ", [")?;
            for (i, (constant, block_target, args)) in cases.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                fmt_constant(constant, f)?;
                write!(f, " -> ")?;
                fmt_block_target(*block_target, args, f)?;
            }
            write!(f, "], default -> ")?;
            fmt_block_target(default.0, &default.1, f)
        }
        Terminator::Return(val) => {
            write!(f, "return")?;
            if let Some(v) = val {
                write!(f, " ")?;
                fmt_value(*v, f)?;
            }
            Ok(())
        }
    }
}

/// Write a function IR with an explicit name to a formatter.
///
/// Used by `Module::Display` (which has access to the `NameTable`)
/// and by `Function::Display` (which prints `<unnamed>`).
pub fn write_function_with_name(
    f: &mut fmt::Formatter<'_>,
    func: &Function,
    name: &str,
) -> fmt::Result {
    // Function header
    match func.visibility {
        Visibility::Public => write!(f, "pub ")?,
        Visibility::Protected => write!(f, "protected ")?,
        Visibility::Private => {}
    }
    // Class/namespace annotation
    if let Some(class) = &func.class {
        if !func.namespace.is_empty() {
            write!(f, "{}.", func.namespace.join("."))?;
        }
        write!(f, "{class}::")?;
    }
    write!(f, "fn {name}")?;
    fmt_method_kind(func.method_kind, f)?;
    write!(f, "(")?;
    let entry = &func.blocks[func.entry];
    for (i, param) in entry.params.iter().enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        fmt_value(param.value, f)?;
        write!(f, ": ")?;
        fmt_type(&param.ty, f)?;
    }
    write!(f, ") -> ")?;
    fmt_type(&func.sig.return_ty, f)?;
    writeln!(f, " {{")?;

    // Blocks
    for (block_id, block) in func.blocks.iter() {
        // Block header
        write!(f, "  block{}", block_id.index())?;
        write!(f, "(")?;
        for (i, param) in block.params.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            fmt_value(param.value, f)?;
            write!(f, ": ")?;
            fmt_type(&param.ty, f)?;
        }
        writeln!(f, "):")?;

        // Instructions
        for &inst_id in &block.insts {
            let inst = &func.insts[inst_id];

            write!(f, "    ")?;

            // Result prefix
            if let Some(result) = inst.result {
                fmt_value(result, f)?;
                let ty = &func.value_types[result];
                write!(f, ": ")?;
                fmt_type(ty, f)?;
                write!(f, " = ")?;
            }

            // Operation
            fmt_op(&inst.op, f)?;

            writeln!(f)?;
        }

        // Terminator
        {
            let term = &block.terminator;
            write!(f, "    ")?;
            fmt_terminator(term, f)?;
            writeln!(f)?;
        }

        // Blank line between blocks (except after last)
        if block_id.index() + 1 < func.blocks.len() as u32 {
            writeln!(f)?;
        }
    }

    write!(f, "}}")
}

impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_function_with_name(f, self, &self.name)
    }
}

impl fmt::Display for Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "module {:?}", self.name)?;

        // Imports
        for import in &self.imports {
            write!(f, "\nimport {}::{}", import.module, import.name)?;
            if let Some(alias) = &import.alias {
                write!(f, " as {alias}")?;
            }
            writeln!(f)?;
        }

        // Struct definitions
        for s in &self.structs {
            writeln!(f)?;
            writeln!(f, "struct {} {{", s.name)?;
            for field in &s.fields {
                write!(f, "    {}: ", field.name)?;
                fmt_type(&field.ty, f)?;
                writeln!(f, ",")?;
            }
            writeln!(f, "}}")?;
        }

        // Enum definitions
        for e in &self.enums {
            writeln!(f)?;
            writeln!(f, "enum {} {{", e.name)?;
            for variant in &e.variants {
                if variant.fields.is_empty() {
                    writeln!(f, "    {},", variant.name)?;
                } else {
                    write!(f, "    {}(", variant.name)?;
                    for (i, ty) in variant.fields.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        fmt_type(ty, f)?;
                    }
                    writeln!(f, "),")?;
                }
            }
            writeln!(f, "}}")?;
        }

        // Classes
        for class in &self.classes {
            writeln!(f)?;
            if !class.namespace.is_empty() {
                write!(f, "class {}.{}", class.namespace.join("."), class.name)?;
            } else {
                write!(f, "class {}", class.name)?;
            }
            if let Some(super_class) = &class.super_class {
                write!(f, " extends {super_class}")?;
            }
            writeln!(f)?;
        }

        // Globals
        for g in &self.globals {
            writeln!(f)?;
            match g.visibility {
                Visibility::Public => write!(f, "pub ")?,
                Visibility::Protected => write!(f, "protected ")?,
                Visibility::Private => {}
            }
            write!(f, "global ")?;
            if g.mutable {
                write!(f, "mut ")?;
            }
            write!(f, "{}: ", g.name)?;
            fmt_type(&g.ty, f)?;
            writeln!(f)?;
        }

        // Functions
        for (func_id, func) in self.functions.iter() {
            writeln!(f)?;
            write_function_with_name(f, func, self.func_name(func_id))?;
            writeln!(f)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::builder::{FunctionBuilder, ModuleBuilder};
    use super::super::func::Visibility;
    use super::super::module::{EnumDef, EnumVariant, FieldDef, Global, Import, StructDef};
    use super::super::ty::{FunctionSig, Type};

    #[test]
    fn print_simple_add() {
        let sig = FunctionSig {
            params: vec![Type::Int(64), Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("add", sig, Visibility::Public);
        let a = fb.param(0);
        let b = fb.param(1);
        let sum = fb.add(a, b);
        fb.ret(Some(sum));
        let func = fb.build();

        let output = format!("{func}");
        assert_eq!(
            output,
            "\
pub fn add(v0: i64, v1: i64) -> i64 {
  block0(v0: i64, v1: i64):
    v2: i64 = add v0, v1
    return v2
}"
        );
    }

    #[test]
    fn print_branching() {
        let sig = FunctionSig {
            params: vec![Type::Bool, Type::Int(64), Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("choose", sig, Visibility::Private);

        let cond = fb.param(0);
        let x = fb.param(1);
        let y = fb.param(2);

        let (then_block, then_vals) = fb.create_block_with_params(&[Type::Int(64)]);
        let (else_block, else_vals) = fb.create_block_with_params(&[Type::Int(64)]);

        fb.br_if(cond, then_block, &[x], else_block, &[y]);

        fb.switch_to_block(then_block);
        fb.ret(Some(then_vals[0]));

        fb.switch_to_block(else_block);
        fb.ret(Some(else_vals[0]));

        let func = fb.build();
        let output = format!("{func}");
        assert_eq!(
            output,
            "\
fn choose(v0: bool, v1: i64, v2: i64) -> i64 {
  block0(v0: bool, v1: i64, v2: i64):
    br_if v0, block1(v1), block2(v2)

  block1(v3: i64):
    return v3

  block2(v4: i64):
    return v4
}"
        );
    }

    #[test]
    fn print_module() {
        let mut mb = ModuleBuilder::new("example");

        mb.add_import(Import {
            module: "render".into(),
            name: "draw_sprite".into(),
            alias: None,
        });
        mb.add_import(Import {
            module: "audio".into(),
            name: "play_sound".into(),
            alias: Some("play".into()),
        });

        mb.add_struct(StructDef {
            name: "Point".into(),
            namespace: Vec::new(),
            fields: vec![
                FieldDef {
                    name: "x".into(),
                    ty: Type::Float(64),
                    default: None,
                },
                FieldDef {
                    name: "y".into(),
                    ty: Type::Float(64),
                    default: None,
                },
            ],
            visibility: Visibility::Public,
        });

        mb.add_enum(EnumDef {
            name: "Shape".into(),
            variants: vec![
                EnumVariant {
                    name: "Circle".into(),
                    fields: vec![Type::Float(64)],
                },
                EnumVariant {
                    name: "Rect".into(),
                    fields: vec![
                        Type::Float(64),
                        Type::Float(64),
                        Type::Float(64),
                        Type::Float(64),
                    ],
                },
            ],
            visibility: Visibility::Public,
        });

        mb.add_global(Global {
            name: "counter".into(),
            ty: Type::Int(64),
            visibility: Visibility::Private,
            mutable: true,
            init: None,
        });
        mb.add_global(Global {
            name: "PI".into(),
            ty: Type::Float(64),
            visibility: Visibility::Private,
            mutable: false,
            init: None,
        });

        // Add a simple function
        let sig = FunctionSig {
            params: vec![Type::Int(64), Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("add", sig, Visibility::Public);
        let a = fb.param(0);
        let b = fb.param(1);
        let sum = fb.add(a, b);
        fb.ret(Some(sum));
        mb.add_function(fb.build());

        let module = mb.build();
        let output = format!("{module}");
        assert_eq!(
            output,
            r#"module "example"

import render::draw_sprite

import audio::play_sound as play

struct Point {
    x: f64,
    y: f64,
}

enum Shape {
    Circle(f64),
    Rect(f64, f64, f64, f64),
}

global mut counter: i64

global PI: f64

pub fn add(v0: i64, v1: i64) -> i64 {
  block0(v0: i64, v1: i64):
    v2: i64 = add v0, v1
    return v2
}
"#
        );
    }

    #[test]
    fn print_constants() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("consts", sig, Visibility::Private);
        fb.const_int(42);
        fb.const_string("hello");
        fb.const_bool(true);
        fb.const_null();
        fb.const_float(2.5);
        fb.ret(None);
        let func = fb.build();

        let output = format!("{func}");
        assert_eq!(
            output,
            "\
fn consts() -> void {
  block0():
    v0: i64 = const 42
    v1: string = const \"hello\"
    v2: bool = const true
    v3: ?unknown = const null
    v4: f64 = const 2.5
    return
}"
        );
    }

    #[test]
    fn print_types() {
        // Test various type formatting via a function that uses them
        let sig = FunctionSig {
            params: vec![
                Type::Array(Box::new(Type::Int(32))),
                Type::Map(Box::new(Type::String), Box::new(Type::Bool)),
                Type::Option(Box::new(Type::Float(64))),
                Type::Tuple(vec![Type::Int(64), Type::Bool]),
            ],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("types", sig, Visibility::Private);
        fb.ret(None);
        let func = fb.build();

        let output = format!("{func}");
        assert!(output.contains("v0: [i32]"));
        assert!(output.contains("v1: {string -> bool}"));
        assert!(output.contains("v2: ?f64"));
        assert!(output.contains("v3: (i64, bool)"));
    }

    #[test]
    fn print_syscall() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("draw", sig, Visibility::Private);
        let x = fb.const_int(10);
        let y = fb.const_int(20);
        fb.system_call("Graphics", "drawRect", &[x, y], Type::Void);
        fb.ret(None);
        let func = fb.build();

        let output = format!("{func}");
        assert!(output.contains(r#"v2: void = syscall "Graphics"."drawRect"(v0, v1)"#));
    }

    #[test]
    fn print_void_return_function() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("noop", sig, Visibility::Private);
        fb.ret(None);
        let func = fb.build();

        let output = format!("{func}");
        assert_eq!(
            output,
            "\
fn noop() -> void {
  block0():
    return
}"
        );
    }
}
