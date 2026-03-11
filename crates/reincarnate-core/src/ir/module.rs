use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::entity::PrimaryMap;

use crate::project::{ExternalMethodSig, ExternalTypeDef};

use super::func::{FuncId, Function, MethodKind, Visibility};
use super::ty::Type;
use super::value::Constant;

/// Describes how the application is started.
///
/// Engine-agnostic: each frontend maps its own entry mechanism to the
/// appropriate variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntryPoint {
    /// Construct this class to start the application.
    /// Flash document class, Java Applet, RPG Maker Scene_Boot, etc.
    ConstructClass(String),
    /// Call this function to start the application.
    /// VB6 Sub Main, Director startMovie, Ren'Py label start, etc.
    CallFunction(FuncId),
}

/// A struct definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructDef {
    pub name: String,
    #[serde(default)]
    pub namespace: Vec<String>,
    pub fields: Vec<(String, Type, Option<Constant>)>,
    pub visibility: Visibility,
}

/// An enum variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Type>,
}

/// An enum definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub visibility: Visibility,
}

/// A global variable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Global {
    pub name: String,
    pub ty: Type,
    pub visibility: Visibility,
    pub mutable: bool,
    /// Optional compile-time default value (from script trait Slot/Const).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<Constant>,
}

/// An import from another module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    pub module: String,
    pub name: String,
    pub alias: Option<String>,
}

/// An import of an external runtime type (e.g. a Flash stdlib class).
///
/// Populated by frontends so the backend can emit import statements without
/// engine-specific namespace parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalImport {
    /// Short name used in generated code (e.g. `"MovieClip"`).
    pub short_name: String,
    /// Module path relative to the runtime directory root
    /// (e.g. `"flash/display"`, `"flash/runtime"`).
    pub module_path: String,
}

/// Groups a struct (fields) with its methods into a class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassDef {
    /// Short class name (e.g. `"Phouka"`).
    pub name: String,
    /// Namespace segments (e.g. `["classes", "Scenes", "Areas", "Bog"]`).
    pub namespace: Vec<String>,
    /// Index into `Module::structs`.
    pub struct_index: usize,
    /// Method `FuncId`s belonging to this class.
    pub methods: Vec<FuncId>,
    /// Superclass qualified name, if any.
    pub super_class: Option<String>,
    pub visibility: Visibility,
    /// Static (class-level) fields from Slot/Const traits on the Class object.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub static_fields: Vec<(String, Type, Option<Constant>, bool)>,
    /// Whether this class is an interface (emitted as `abstract class`).
    #[serde(default)]
    pub is_interface: bool,
    /// Interfaces implemented by this class (short names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<String>,
    /// Abstract member declarations for interface classes.
    /// Each entry is `(name, return_type, params, kind)` where `params` is
    /// the setter parameter type(s) or method parameter types, and `kind`
    /// is `MethodKind::Getter`, `Setter`, or `Instance`.
    /// Emitted as `abstract get/set name(): Type;` in TypeScript.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub abstract_members: Vec<(String, Type, Vec<Type>, MethodKind)>,
    /// AS3 `dynamic` class — allows arbitrary property access via `[]`.
    /// When true the TypeScript backend emits index signatures.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_dynamic: bool,
    /// All instance fields are zero-initialized before the constructor runs
    /// (true for AS3, false for GML/Twine).  When true the TypeScript backend
    /// emits `!` (definite-assignment assertion) on un-initialized fields.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub zero_initialized: bool,
    /// Emit TypeScript index signatures (`[key: string]: any`).
    ///
    /// True for AS3 `dynamic` classes and Proxy subclasses — these allow
    /// arbitrary property access by string or number key.  Set by the Flash
    /// frontend; read by the TypeScript backend.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub needs_index_signature: bool,
}

/// How the type inference pass should infer the result type of a `SystemCall`.
///
/// Frontends populate `Module::system_call_type_rules` with entries keyed by
/// `(system, method)`.  The type inference pass reads these rules instead of
/// hardcoding engine-specific logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemCallTypeRule {
    /// First arg is a const string → resolve as struct/class name → `Struct(name)`.
    ResolveClassName,
    /// First arg's type is `Struct(name)` → result is `Struct(name)`.
    ConstructFromFirstArgType,
    /// First arg is a const string → look up in `Module::globals` → that type.
    ResolveGlobalType,
    /// This system call stores a value into a global variable.
    /// `name_arg` is the index of the argument containing the global name
    /// (a const string), `value_arg` is the index of the argument containing
    /// the value being stored.  Used by `build_global_types` to collect
    /// global variable types without hardcoding engine-specific system names.
    GlobalStore { name_arg: usize, value_arg: usize },
}

/// A module — the top-level compilation unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
    pub name: String,
    pub functions: PrimaryMap<FuncId, Function>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub globals: Vec<Global>,
    pub imports: Vec<Import>,
    #[serde(default)]
    pub classes: Vec<ClassDef>,
    /// How to start the application (set by frontends that know the answer).
    #[serde(default)]
    pub entry_point: Option<EntryPoint>,
    /// External runtime imports, keyed by qualified name (e.g.
    /// `"flash.display::MovieClip"`).  Populated by frontends so the backend
    /// can emit import statements without engine-specific parsing.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub external_imports: BTreeMap<String, ExternalImport>,
    /// External type definitions from the runtime package.
    /// Populated by the CLI before running transforms so that type inference
    /// and constraint solving can resolve fields/methods on external types.
    /// Skipped during serialization to avoid bloating IR JSON output.
    #[serde(default, skip_serializing)]
    pub external_type_defs: BTreeMap<String, ExternalTypeDef>,
    /// External function signatures from the runtime package.
    /// Maps function name → signature for free functions (not methods on types).
    /// Used by type inference and constraint solving to infer return types.
    #[serde(default, skip_serializing)]
    pub external_function_sigs: BTreeMap<String, ExternalMethodSig>,
    /// Room creation code: maps room index → function name.
    /// Populated by frontends so the scaffold can wire up per-room init functions.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub room_creation_code: BTreeMap<usize, String>,
    /// PascalCase name of the initial/first room (e.g. "Preload", "Init").
    /// Populated by frontends so the scaffold can emit `initialRoom: Rooms.<name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_room_name: Option<String>,
    /// Sprite names indexed by sprite ID. Contains PascalCase names matching
    /// the `Sprites` enum keys in data output. Used to resolve `sprite_index`
    /// field defaults to named constants instead of raw integers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sprite_names: Vec<String>,
    /// Object names indexed by object ID. Contains PascalCase names matching
    /// the emitted class names. Used by backend rewrites to resolve integer
    /// object indices to class-based `ObjName.instances[0]` accesses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub object_names: Vec<String>,
    /// Passage names: original display name → sanitized function name.
    /// Populated by the Twine frontend so the scaffold can build a passage
    /// registry mapping names to callable functions.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub passage_names: BTreeMap<String, String>,
    /// Passage tags: display name → list of tags.
    /// Populated by the Twine frontend so the scaffold can emit a tag
    /// registry for runtime use (nobr, widget, special passages, etc.).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub passage_tags: BTreeMap<String, Vec<String>>,
    /// Passage source texts: display name → raw source string.
    /// Populated by the Twine frontend when source string emission is enabled.
    /// Used by the scaffold to build a `sourceMap` for `(source:)` at runtime.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub passage_sources: BTreeMap<String, String>,
    /// Storylet conditions: passage display name → condition function name.
    /// Populated by the Harlowe frontend when `(storylet: when expr)` is
    /// encountered. The scaffold uses this to register each passage's
    /// availability condition with the runtime's storylet system.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub passage_storylets: BTreeMap<String, String>,
    /// Type inference rules for `SystemCall` results, keyed by `(system, method)`.
    ///
    /// Populated by frontends so the shared type inference pass can infer
    /// result types without hardcoding engine-specific dispatch.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub system_call_type_rules: BTreeMap<(String, String), SystemCallTypeRule>,
    /// System calls whose callbacks hide the real return path.
    ///
    /// Functions that call any `(system, method)` in this set may have their
    /// real return value propagated through a callback side channel (e.g.
    /// `live_result`), so `infer_bool_return` must not falsely promote the
    /// outer function's return type to Bool based on visible `Op::Return`
    /// paths.
    ///
    /// GML sets `[("GameMaker.Instance", "withInstances")]`; other engines
    /// leave this empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub callback_return_calls: BTreeMap<(String, String), ()>,
}

impl Module {
    pub fn new(name: String) -> Self {
        Self {
            name,
            functions: PrimaryMap::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            globals: Vec::new(),
            imports: Vec::new(),
            classes: Vec::new(),
            entry_point: None,
            external_imports: BTreeMap::new(),
            external_type_defs: BTreeMap::new(),
            external_function_sigs: BTreeMap::new(),
            room_creation_code: BTreeMap::new(),
            initial_room_name: None,
            sprite_names: Vec::new(),
            object_names: Vec::new(),
            passage_names: BTreeMap::new(),
            passage_tags: BTreeMap::new(),
            passage_sources: BTreeMap::new(),
            passage_storylets: BTreeMap::new(),
            system_call_type_rules: BTreeMap::new(),
            callback_return_calls: BTreeMap::new(),
        }
    }
}
