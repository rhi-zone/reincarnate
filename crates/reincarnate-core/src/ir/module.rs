use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::entity::PrimaryMap;
use crate::pipeline::Diagnostic;
use crate::project::{ExternalMethodSig, ExternalTypeDef};

use super::func::{FuncId, Function, MethodKind, Visibility};
use super::name_table::NameTable;
use super::ty::{FunctionSig, Type, TypeId};
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

/// An instance field in a struct or class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub ty: Type,
    pub default: Option<Constant>,
}

/// A struct definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructDef {
    pub name: String,
    #[serde(default)]
    pub namespace: Vec<String>,
    pub fields: Vec<FieldDef>,
    pub visibility: Visibility,
}

/// A lightweight helper for interning named types without borrowing the full [`Module`].
///
/// Obtained via [`Module::type_interner_mut`].  Useful when another field of
/// `Module` is already mutably borrowed (e.g. iterating `module.functions` while
/// also needing to intern new type names).
pub struct TypeInterner<'a> {
    types: &'a mut PrimaryMap<TypeId, TypeDecl>,
    type_names: &'a mut HashMap<String, TypeId>,
}

impl<'a> TypeInterner<'a> {
    /// Construct an interner from raw mutable references to the two type-index fields.
    ///
    /// Useful when the caller has already split the `Module` borrow (e.g. via
    /// `std::mem::take`) and cannot call `Module::type_interner_mut`.
    pub fn from_parts(
        types: &'a mut PrimaryMap<TypeId, TypeDecl>,
        type_names: &'a mut HashMap<String, TypeId>,
    ) -> Self {
        Self { types, type_names }
    }

    /// Get or create a [`TypeId`] for the given named Object type.
    pub fn intern(&mut self, name: &str) -> TypeId {
        if let Some(&id) = self.type_names.get(name) {
            return id;
        }
        let id = self.types.push(TypeDecl::Object {
            name: Some(name.to_string()),
            parent: None,
            fields: Vec::new(),
            methods: Vec::new(),
            class_ref: None,
            inferred: false,
        });
        self.type_names.insert(name.to_string(), id);
        id
    }

    /// Get or create and return `Type::Instance(id)`.
    pub fn instance(&mut self, name: &str) -> Type {
        Type::Instance(self.intern(name))
    }

    /// Get or create a `TypeDecl::Object` for the static side of a class, and
    /// return `Type::ClassRef(id)`.
    ///
    /// The static-side TypeDecl has the same name as the class but is stored
    /// under a mangled key `"classref::NAME"` to avoid colliding with the
    /// instance-side entry.
    pub fn classref(&mut self, name: &str) -> Type {
        let key = format!("classref::{name}");
        if let Some(&id) = self.type_names.get(&key) {
            return Type::ClassRef(id);
        }
        let id = self.types.push(TypeDecl::Object {
            name: Some(name.to_string()),
            parent: None,
            fields: Vec::new(),
            methods: Vec::new(),
            class_ref: None,
            inferred: false,
        });
        self.type_names.insert(key, id);
        Type::ClassRef(id)
    }

    /// Look up a TypeId by name without creating.
    pub fn find(&self, name: &str) -> Option<TypeId> {
        self.type_names.get(name).copied()
    }

    /// Look up the name of an already-interned TypeId.
    ///
    /// # Panics
    /// Panics if the TypeId is not in the arena, or if the TypeDecl has no name.
    pub fn name_of(&self, id: TypeId) -> &str {
        self.types[id].name_expect()
    }

    /// Check if a TypeId exists (is a valid interned id).
    pub fn contains_id(&self, id: TypeId) -> bool {
        self.types.get(id).is_some()
    }
}

/// A type declaration stored in the module's type arena.
///
/// Referenced by [`TypeId`] in [`Type::Instance`] and [`Type::ClassRef`] variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypeDecl {
    /// A struct or class type (instance side).
    Object {
        /// Short or qualified name of the type (e.g. `"MyClass"`, `"objects::Obj1"`).
        name: Option<String>,
        /// Superclass TypeId, if any (instance-side inheritance).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<TypeId>,
        /// Instance fields.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<FieldDef>,
        /// Instance method FuncIds (into module.funcs).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        methods: Vec<FuncId>,
        /// TypeId of the static-side TypeDecl::Object for this class, if any.
        /// None for pure structs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        class_ref: Option<TypeId>,
        /// Whether this type was inferred from constructor body vs. declared by frontend.
        #[serde(default)]
        inferred: bool,
    },
    /// An enum type.
    Enum {
        /// Name of the enum (e.g. `"CockTypesEnum"`).
        name: Option<String>,
        /// Enum variants.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        variants: Vec<EnumVariant>,
    },
}

impl TypeDecl {
    /// Return the name of this type declaration, if any.
    pub fn name(&self) -> Option<&str> {
        match self {
            TypeDecl::Object { name, .. } => name.as_deref(),
            TypeDecl::Enum { name, .. } => name.as_deref(),
        }
    }

    /// Return the name of this type declaration, panicking if unnamed.
    ///
    /// # Panics
    /// Panics if this TypeDecl has no name.
    pub fn name_expect(&self) -> &str {
        self.name().expect("TypeDecl has no name")
    }

    /// Return the fields of an Object TypeDecl, or an empty slice for enums.
    pub fn fields(&self) -> &[FieldDef] {
        match self {
            TypeDecl::Object { fields, .. } => fields,
            TypeDecl::Enum { .. } => &[],
        }
    }

    /// Return a mutable reference to the fields of an Object TypeDecl.
    ///
    /// # Panics
    /// Panics if this TypeDecl is not an Object.
    pub fn fields_mut(&mut self) -> &mut Vec<FieldDef> {
        match self {
            TypeDecl::Object { fields, .. } => fields,
            TypeDecl::Enum { .. } => panic!("TypeDecl::Enum has no fields"),
        }
    }

    /// Return whether this is an inferred (not declared) type.
    pub fn inferred(&self) -> bool {
        match self {
            TypeDecl::Object { inferred, .. } => *inferred,
            TypeDecl::Enum { .. } => false,
        }
    }

    /// Set the inferred flag on an Object TypeDecl.
    ///
    /// # Panics
    /// Panics if this TypeDecl is not an Object.
    pub fn set_inferred(&mut self, value: bool) {
        match self {
            TypeDecl::Object { inferred, .. } => *inferred = value,
            TypeDecl::Enum { .. } => panic!("TypeDecl::Enum has no inferred flag"),
        }
    }
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

/// A static (class-level) field from a Slot/Const trait.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticField {
    pub name: String,
    pub ty: Type,
    pub default: Option<Constant>,
    pub is_const: bool,
}

/// An abstract member declaration on an interface class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbstractMember {
    pub name: String,
    pub return_ty: Type,
    pub params: Vec<Type>,
    pub kind: MethodKind,
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
    pub static_fields: Vec<StaticField>,
    /// Whether this class is an interface (emitted as `abstract class`).
    #[serde(default)]
    pub is_interface: bool,
    /// Interfaces implemented by this class (short names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<String>,
    /// Abstract member declarations for interface classes.
    /// Emitted as `abstract get/set name(): Type;` in TypeScript.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub abstract_members: Vec<AbstractMember>,
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
    /// Like `ResolveGlobalType` but only participates in Phase 3 struct
    /// inference.  Phase 2 (Array/Function use-site inference) is skipped so
    /// that JS built-in lookups (e.g. `Engine.resolve("Date")`) are not
    /// incorrectly typed as function values.  Struct casts are still injected
    /// when the inferred type is `Struct(_)`.
    ///
    /// `skip_names` lists names that are known JS globals (from the runtime's
    /// typed overloads) and must never receive struct type inference regardless
    /// of how their resolve results are used in the IR.
    ResolveGlobalTypeStructOnly { skip_names: Vec<String> },
    /// This system call stores a value into a global variable.
    /// `name_arg` is the index of the argument containing the global name
    /// (a const string), `value_arg` is the index of the argument containing
    /// the value being stored.  Used by `build_global_types` to collect
    /// global variable types without hardcoding engine-specific system names.
    GlobalStore { name_arg: usize, value_arg: usize },
    /// First arg is a receiver `Instance(id)`, second arg is a const string
    /// field name.  Result type is the field's static type from the module's
    /// struct definitions.
    ResolveInstanceField,
}

/// A module — the top-level compilation unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
    pub name: String,
    /// Centralized name storage for IR symbols.
    #[serde(default)]
    pub name_table: NameTable,
    pub functions: PrimaryMap<FuncId, Function>,
    /// Type declaration arena: maps [`TypeId`] → [`TypeDecl`].
    /// Serializes as a plain Vec (via `PrimaryMap`'s `serde(transparent)` impl).
    #[serde(default, skip_serializing_if = "PrimaryMap::is_empty")]
    pub types: PrimaryMap<TypeId, TypeDecl>,
    /// Name → TypeId reverse index. Not serialized — rebuilt from `types` on load.
    #[serde(skip)]
    pub type_names: HashMap<String, TypeId>,
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
    /// Engine-declared function signatures (params + return type) for external/builtin functions.
    /// Not serialized — populated by frontends at translate time.
    /// TypeInference merges these into both `func_return_types` and `func_sigs`.
    #[serde(default, skip_serializing)]
    pub extern_sigs: HashMap<String, FunctionSig>,
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
    /// Diagnostics accumulated during compilation (transforms, backend, etc.).
    ///
    /// These are pipeline-generated warnings/info about the source program
    /// (e.g. game-author bugs like duplicate switch cases). They are merged
    /// with external checker diagnostics (e.g. TypeScript errors) in the CLI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    /// Whether the source language implicitly returns a value from every
    /// function (e.g. GML returns `0.0` by default).
    ///
    /// When `true`, type inference keeps `Unknown` return types for functions
    /// that have no value-bearing `Return` instructions, because callers may
    /// still use the implicit return value.  When `false` (the default, e.g.
    /// Flash/AS3), such functions are narrowed to `Void`.
    #[serde(default)]
    pub implicit_return_value: bool,
    /// Struct names that are accessed with dynamic string keys (e.g. `obj[strVar]`).
    ///
    /// Populated by type inference when it detects `Op::GetIndex`/`Op::SetIndex` on
    /// values with struct provenance.  The TypeScript backend emits a
    /// `[key: string]: any` index signature for these structs so that dynamic
    /// key access does not produce TS7053 errors.
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub string_indexed_structs: HashSet<String>,
    /// Function names that should be treated as array-length-like by core passes.
    ///
    /// When a value is passed as the first argument to any function in this set,
    /// that value is treated as a collection (suppressing narrowing to scalar
    /// types in `CallSiteTypeFlow` and `ConstraintSolve2`).
    ///
    /// Populated by frontends. GML sets `["array_length"]` because
    /// `array_length(arr)` is emitted as `arr.length` and the IR uses a `Call`
    /// op rather than `GetField`. Core passes read this set instead of
    /// hardcoding engine-specific function names.
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub array_like_fns: HashSet<String>,
}

impl Module {
    /// Look up the name of a function by its `FuncId`.
    ///
    /// # Panics
    /// Panics if the `FuncId` is not in the name table.
    pub fn func_name(&self, id: FuncId) -> &str {
        self.name_table.func_name(id)
    }

    /// Rebuild the `NameTable` from the `name` fields on `Function` structs.
    ///
    /// Called after deserialization (which populates `Function::name` but not
    /// the `NameTable`) and after any direct mutation of `functions` that
    /// bypasses `ModuleBuilder`.
    pub fn rebuild_name_table(&mut self) {
        self.name_table.func_names = PrimaryMap::new();
        for (_id, func) in self.functions.iter() {
            self.name_table.func_names.push(func.name.clone());
        }
    }

    /// Rebuild the `type_names` reverse index from `types`.
    ///
    /// Called after deserialization (which populates `types` but skips
    /// `type_names`) and after any direct mutation of `types`.
    pub fn rebuild_type_index(&mut self) {
        self.type_names.clear();
        for (id, td) in self.types.iter() {
            if let Some(name) = td.name() {
                self.type_names.insert(name.to_string(), id);
            }
        }
    }

    /// Get or create a [`TypeId`] for a named Object type.
    ///
    /// If a type with this name already exists, returns its `TypeId`.
    /// Otherwise, allocates a new `TypeDecl::Object` with empty fields and returns the new id.
    pub fn intern_type(&mut self, name: &str) -> TypeId {
        if let Some(&id) = self.type_names.get(name) {
            return id;
        }
        let id = self.types.push(TypeDecl::Object {
            name: Some(name.to_string()),
            parent: None,
            fields: Vec::new(),
            methods: Vec::new(),
            class_ref: None,
            inferred: false,
        });
        self.type_names.insert(name.to_string(), id);
        id
    }

    /// Get or create a [`TypeId`] for a named Enum type.
    ///
    /// If an entry with this name already exists, returns its `TypeId` (reuses it
    /// even if the existing entry is an Object — callers should be consistent).
    /// Otherwise, allocates a new `TypeDecl::Enum` and returns the new id.
    pub fn intern_enum(&mut self, name: &str, variants: Vec<EnumVariant>) -> TypeId {
        if let Some(&id) = self.type_names.get(name) {
            return id;
        }
        let id = self.types.push(TypeDecl::Enum {
            name: Some(name.to_string()),
            variants,
        });
        self.type_names.insert(name.to_string(), id);
        id
    }

    /// Get or create a [`TypeId`] and return `Type::Instance(id)`.
    ///
    /// Convenience wrapper around `intern_type`.
    pub fn intern_type_instance(&mut self, name: &str) -> Type {
        Type::Instance(self.intern_type(name))
    }

    /// Get or create a static-side `TypeDecl::Object` for a class and return
    /// `Type::ClassRef(id)`.
    ///
    /// The static-side TypeDecl is stored under the key `"classref::NAME"` so
    /// it does not collide with the instance-side entry.
    pub fn intern_type_classref(&mut self, name: &str) -> Type {
        let key = format!("classref::{name}");
        if let Some(&id) = self.type_names.get(&key) {
            return Type::ClassRef(id);
        }
        let id = self.types.push(TypeDecl::Object {
            name: Some(name.to_string()),
            parent: None,
            fields: Vec::new(),
            methods: Vec::new(),
            class_ref: None,
            inferred: false,
        });
        self.type_names.insert(key, id);
        Type::ClassRef(id)
    }

    /// Look up the name of a type by its `TypeId`.
    ///
    /// # Panics
    /// Panics if the `TypeId` is not in the type arena or the TypeDecl has no name.
    pub fn type_name(&self, id: TypeId) -> &str {
        self.types[id].name_expect()
    }

    /// Find a `TypeId` by name without creating a new entry.
    pub fn find_type(&self, name: &str) -> Option<TypeId> {
        self.type_names.get(name).copied()
    }

    /// Borrow the type arena fields for use as a split borrow.
    ///
    /// Returns a `TypeInterner` backed by the module's type arena and name index.
    /// This allows callers that already hold a mutable borrow on another field of
    /// `Module` (e.g. `functions`) to still intern types without conflicting borrows.
    pub fn type_interner_mut(&mut self) -> TypeInterner<'_> {
        TypeInterner {
            types: &mut self.types,
            type_names: &mut self.type_names,
        }
    }

    /// Normalize compound types in function signatures, value types, struct/class
    /// field types, and abstract member types, resolving any nested type references.
    ///
    /// This is called by `ModuleBuilder::build()` after all structs/classes are
    /// registered.
    pub fn normalize_struct_types(&mut self) {
        // Normalize struct field types.
        for i in 0..self.structs.len() {
            for j in 0..self.structs[i].fields.len() {
                let ty = self.structs[i].fields[j].ty.clone();
                self.structs[i].fields[j].ty = normalize_type(ty);
            }
        }
        // Normalize class static fields and abstract member types.
        for i in 0..self.classes.len() {
            for j in 0..self.classes[i].static_fields.len() {
                let ty = self.classes[i].static_fields[j].ty.clone();
                self.classes[i].static_fields[j].ty = normalize_type(ty);
            }
            for j in 0..self.classes[i].abstract_members.len() {
                let ret_ty = self.classes[i].abstract_members[j].return_ty.clone();
                self.classes[i].abstract_members[j].return_ty = normalize_type(ret_ty);
                for k in 0..self.classes[i].abstract_members[j].params.len() {
                    let ty = self.classes[i].abstract_members[j].params[k].clone();
                    self.classes[i].abstract_members[j].params[k] = normalize_type(ty);
                }
            }
        }
        // Normalize global types.
        for i in 0..self.globals.len() {
            let ty = self.globals[i].ty.clone();
            self.globals[i].ty = normalize_type(ty);
        }
        // Collect function IDs to avoid borrow conflicts.
        let func_ids: Vec<_> = self.functions.keys().collect();
        for func_id in func_ids {
            // Normalize parameter types in the signature.
            let param_count = self.functions[func_id].sig.params.len();
            for i in 0..param_count {
                let ty = self.functions[func_id].sig.params[i].clone();
                let normalized = normalize_type(ty);
                self.functions[func_id].sig.params[i] = normalized;
            }
            let ret_ty = self.functions[func_id].sig.return_ty.clone();
            self.functions[func_id].sig.return_ty = normalize_type(ret_ty);

            // Normalize value types.
            let value_ids: Vec<_> = self.functions[func_id].value_types.keys().collect();
            for vid in value_ids {
                let ty = self.functions[func_id].value_types[vid].clone();
                let normalized = normalize_type(ty);
                self.functions[func_id].value_types[vid] = normalized;
            }

            // Normalize block parameter types.
            let block_ids: Vec<_> = self.functions[func_id].blocks.keys().collect();
            for bid in block_ids {
                let param_count = self.functions[func_id].blocks[bid].params.len();
                for i in 0..param_count {
                    let ty = self.functions[func_id].blocks[bid].params[i].ty.clone();
                    let normalized = normalize_type(ty);
                    self.functions[func_id].blocks[bid].params[i].ty = normalized;
                }
            }

            // Normalize types embedded in instructions (Alloc, Cast, TypeCheck).
            let inst_ids: Vec<_> = self.functions[func_id].insts.keys().collect();
            for iid in inst_ids {
                use super::inst::Op;
                match &self.functions[func_id].insts[iid].op {
                    Op::Alloc(_) | Op::Cast(_, _, _) | Op::TypeCheck(_, _) => {
                        let op = self.functions[func_id].insts[iid].op.clone();
                        let normalized_op = match op {
                            Op::Alloc(ty) => Op::Alloc(normalize_type(ty)),
                            Op::Cast(v, ty, kind) => Op::Cast(v, normalize_type(ty), kind),
                            Op::TypeCheck(v, ty) => Op::TypeCheck(v, normalize_type(ty)),
                            other => other,
                        };
                        self.functions[func_id].insts[iid].op = normalized_op;
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn new(name: String) -> Self {
        Self {
            name,
            name_table: NameTable::new(),
            functions: PrimaryMap::new(),
            types: PrimaryMap::new(),
            type_names: HashMap::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            globals: Vec::new(),
            imports: Vec::new(),
            classes: Vec::new(),
            entry_point: None,
            external_imports: BTreeMap::new(),
            external_type_defs: BTreeMap::new(),
            external_function_sigs: BTreeMap::new(),
            extern_sigs: HashMap::new(),
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
            diagnostics: Vec::new(),
            implicit_return_value: false,
            string_indexed_structs: HashSet::new(),
            array_like_fns: HashSet::new(),
        }
    }
}

/// Recursively normalize compound types (Array, Map, Option, etc.) so that
/// any nested type references in the tree are passed through unchanged.
///
/// `Type::Struct` no longer exists; all named types are already `Type::Instance`
/// after the frontend builds the IR. This function handles compound wrappers only.
fn normalize_type(ty: Type) -> Type {
    match ty {
        Type::Array(elem) => Type::Array(Box::new(normalize_type(*elem))),
        Type::Map(k, v) => Type::Map(Box::new(normalize_type(*k)), Box::new(normalize_type(*v))),
        Type::Option(inner) => Type::Option(Box::new(normalize_type(*inner))),
        Type::Tuple(elems) => Type::Tuple(elems.into_iter().map(normalize_type).collect()),
        Type::Union(elems) => Type::Union(elems.into_iter().map(normalize_type).collect()),
        Type::Function(sig) => {
            let params = sig.params.into_iter().map(normalize_type).collect();
            let return_ty = normalize_type(sig.return_ty);
            Type::Function(Box::new(super::ty::FunctionSig {
                params,
                return_ty,
                defaults: sig.defaults,
                has_rest_param: sig.has_rest_param,
            }))
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => Type::Coroutine {
            yield_ty: Box::new(normalize_type(*yield_ty)),
            return_ty: Box::new(normalize_type(*return_ty)),
        },
        // All other types (Instance, ClassRef, primitives, etc.) are already normalized.
        other => other,
    }
}
