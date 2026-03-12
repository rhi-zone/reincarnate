//! Flash-specific rewrite context.

use std::collections::{HashMap, HashSet};

use crate::js_ast::JsFunction;

/// Context needed for Flash/AVM2 scope resolution and rewrite decisions.
pub struct FlashRewriteCtx {
    /// Qualified class name → sanitized short name.
    pub class_names: HashMap<String, String>,
    /// Short names of the current class and all its ancestors.
    pub ancestors: HashSet<String>,
    /// Instance method short names visible in the class hierarchy.
    pub method_names: HashSet<String>,
    /// Instance field short names visible in the class hierarchy.
    pub instance_fields: HashSet<String>,
    /// Whether we are inside a method (have a `this`).
    pub has_self: bool,
    /// Suppress `super()` calls (class has no real superclass).
    pub suppress_super: bool,
    /// Parent class is a Flash runtime type (not a user-defined class).
    /// `constructSuper` should emit `super()` without `_shims` injection — the
    /// runtime class doesn't accept it — while the user class itself still gets
    /// `readonly _shims: FlashShims` in its constructor.
    pub parent_is_runtime: bool,
    /// Whether we are inside a cinit (class static initializer).
    pub is_cinit: bool,
    /// Whether we are inside a regular instance constructor (not cinit).
    pub is_constructor: bool,
    /// Whether we are inside a static method (not cinit).
    pub is_static: bool,
    /// Static field short names declared on the current class.
    pub static_fields: HashSet<String>,
    /// Static method short name → owning class short name (across hierarchy).
    pub static_method_owners: HashMap<String, String>,
    /// Const field short name → owning class short name (across hierarchy).
    pub static_field_owners: HashMap<String, String>,
    /// Instance Const fields promoted to static readonly — `this.FIELD` → `ClassName.FIELD`.
    pub const_instance_fields: HashSet<String>,
    /// Short name of the current class (for `this.CONST` → `ClassName.CONST` rewrites).
    pub class_short_name: Option<String>,
    /// Instance/Free method names that need `cachedBind` wrapping when used outside callee position.
    pub bindable_methods: HashSet<String>,
    /// Pre-compiled closure bodies (short name → JsFunction), for inlining as arrow functions.
    pub closure_bodies: HashMap<String, JsFunction>,
    /// All known class short names (module classes + runtime type_definitions).
    /// Used to detect class coercions: `ClassName(obj)` → `asType(obj, ClassName)`.
    pub known_classes: HashSet<String>,
    /// Static field names unique across the entire module → owning class short name.
    /// Enables `instance.UNIQUE_STATIC_FIELD` → `OwnerClass.UNIQUE_STATIC_FIELD` rewrites.
    pub unique_static_fields: HashMap<String, String>,
    /// Name of the activation-scope variable in the current context, if any.
    /// Set when rewriting a function with closures: scope-chain lookups that
    /// don't resolve to a class, instance field, or static fall back to
    /// `activation_var.fieldname` instead of a bare `fieldname` Var.
    pub activation_var: Option<String>,
    /// Field names assigned to the activation-scope object.
    pub activation_slots: HashSet<String>,
}

pub(super) enum ScopeResolution {
    /// The lookup matched an ancestor class.
    Ancestor(String),
    /// Generic scope lookup (class ref, global, etc.).
    ScopeLookup,
}
