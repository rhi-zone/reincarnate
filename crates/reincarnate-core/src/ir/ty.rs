use serde::{Deserialize, Serialize};

use crate::define_entity;

define_entity!(TypeVarId);
define_entity!(TypeId);

/// A resolved type in the IR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Type {
    /// Void / unit.
    Void,
    /// Boolean.
    Bool,
    /// Signed integer with bit width.
    Int(u8),
    /// Unsigned integer with bit width.
    UInt(u8),
    /// Floating point with bit width (32 or 64).
    Float(u8),
    /// UTF-8 string.
    String,
    /// Array of a uniform element type.
    Array(Box<Type>),
    /// Associative map.
    Map(Box<Type>, Box<Type>),
    /// Optional / nullable.
    Option(Box<Type>),
    /// Tuple of types.
    Tuple(Vec<Type>),
    /// Instance of a named type (struct or class), referenced by stable ID.
    ///
    /// This is the canonical interned form used throughout the IR and transforms.
    /// Frontends may emit `Struct(String)` before interning; `ModuleBuilder::build()`
    /// normalizes all `Struct` to `Instance` by interning into `module.types`.
    Instance(TypeId),
    /// Instance of a named type, referenced by string name.
    ///
    /// Used at frontend/backend boundaries and for runtime-provided type names
    /// (e.g. `GameRuntime`, `SugarCubeRuntime`) that don't have an IR struct.
    /// Core transforms should match `Instance(_)` rather than `Struct(_)`.
    /// `ModuleBuilder::build()` converts these to `Instance(TypeId)` automatically.
    Struct(String),
    /// Named enum reference.
    Enum(String),
    /// Class constructor reference — the class itself, not an instance.
    /// TypeScript: `typeof ClassName`. String-keyed because ClassRef types don't
    /// need interning benefits (no field lookups on constructors).
    ClassRef(String),
    /// Function type.
    Function(Box<FunctionSig>),
    /// Coroutine that yields a type and returns a type.
    Coroutine {
        yield_ty: Box<Type>,
        return_ty: Box<Type>,
    },
    /// Unresolved type variable (pre-inference).
    Var(TypeVarId),
    /// Union of distinct concrete types.
    Union(Vec<Type>),
    /// Unknown — type-safe top type representing an inference gap.
    /// TypeScript: `unknown`; Rust: `Value` enum with runtime dispatch.
    Unknown,
}

/// Function signature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionSig {
    pub params: Vec<Type>,
    pub return_ty: Type,
    /// Default values for parameters (parallel vec, `None` = no default).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defaults: Vec<Option<super::value::Constant>>,
    /// Whether the last parameter is a rest/variadic parameter (`...args`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_rest_param: bool,
}

impl Default for FunctionSig {
    fn default() -> Self {
        Self {
            params: Vec::new(),
            return_ty: Type::Void,
            defaults: Vec::new(),
            has_rest_param: false,
        }
    }
}

/// Parse a type notation string (from `runtime.json` type_definitions) into an IR `Type`.
///
/// | Notation      | IR Type               |
/// |---------------|-----------------------|
/// | `"number"`    | `Type::Float(64)`     |
/// | `"int"`       | `Type::Int(32)`       |
/// | `"uint"`      | `Type::UInt(32)`      |
/// | `"boolean"`   | `Type::Bool`          |
/// | `"string"`    | `Type::String`        |
/// | `"void"`      | `Type::Void`          |
/// | `"*"`         | `Type::Unknown`       |
/// | `"Function"`  | `Type::Unknown`       |
/// | `"Array"`     | `Type::Array(Unknown)`|
/// | `"classref"`  | `Type::Unknown`       |
/// | `"ClassName"` | `Type::Unknown`       |
///
/// Note: named struct/class types return `Type::Unknown` because `TypeId`s are
/// not available at parse time — callers that need real `TypeId`s must use the
/// module's `intern_type` directly.
pub fn parse_type_notation(s: &str) -> Type {
    match s {
        "number" => Type::Float(64),
        "int" => Type::Int(32),
        "uint" => Type::UInt(32),
        "boolean" => Type::Bool,
        "string" => Type::String,
        "void" => Type::Void,
        // "classref" marks a GML integer object-index parameter in runtime.json.
        // It has no IR equivalent — the backend rewrite resolves integer literals
        // to class-name Var references; in the IR the parameter type is Unknown.
        "*" | "any" | "dynamic" | "Function" | "Object" | "Class" | "classref" => Type::Unknown,
        "Array" => Type::Array(Box::new(Type::Unknown)),
        name if name.ends_with("[]") => {
            let elem = &name[..name.len() - 2];
            Type::Array(Box::new(parse_type_notation(elem)))
        }
        // Named struct/class types: callers must use module.intern_type(name) directly.
        _name => Type::Unknown,
    }
}

/// Constraint generated during type inference.
#[derive(Debug, Clone)]
pub enum TypeConstraint {
    /// Two types must be equal.
    Equal(Type, Type),
    /// A type variable must be a subtype of a concrete type.
    Subtype { sub: Type, sup: Type },
    /// A type must have a specific field.
    HasField {
        ty: Type,
        field: String,
        field_ty: Type,
    },
    /// A type must be callable with given args and return type.
    Callable {
        ty: Type,
        args: Vec<Type>,
        ret: Type,
    },
}
