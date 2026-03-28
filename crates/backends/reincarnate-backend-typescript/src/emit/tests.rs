use super::*;
use reincarnate_core::ir::builder::{FunctionBuilder, ModuleBuilder};
use reincarnate_core::ir::{
    ClassDef, CmpKind, EnumDef, EnumVariant, FieldDef, FunctionSig, Global, Import, MethodKind,
    StaticField, StructDef, Type, Visibility,
};
use reincarnate_core::pipeline::{DebugConfig, LoweringConfig};
use std::fs;

use super::imports::relative_import_path;

fn build_and_emit(build: impl FnOnce(&mut ModuleBuilder)) -> String {
    let mut mb = ModuleBuilder::new("test");
    build(&mut mb);
    let mut diagnostics = Vec::new();
    emit_module_to_string(
        &mut mb.build(),
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap()
}

#[test]
fn simple_add_function() {
    let out = build_and_emit(|mb| {
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
    });

    assert!(out.contains("export function add(v0: number, v1: number): number {"));
    // Single-use sum is inlined into return.
    assert!(
        out.contains("return v0 + v1;"),
        "Should inline sum into return:\n{out}"
    );
    assert!(
        !out.contains("let v2"),
        "Single-use v2 should be inlined:\n{out}"
    );
    // Single block → no dispatch loop.
    assert!(!out.contains("$block"));
}

#[test]
fn branching_with_block_args() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Bool, Type::Int(64), Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("choose", sig, Visibility::Public);

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

        mb.add_function(fb.build());
    });

    // Structured output: if/else instead of dispatch loop.
    assert!(
        !out.contains("$block"),
        "Should not use dispatch loop:\n{out}"
    );
    assert!(out.contains("if (v0)"), "Should have if (v0):\n{out}");
    // Block-param vars folded away: return v1/v2 directly.
    assert!(
        out.contains("return v1;"),
        "Should return v1 directly:\n{out}"
    );
    assert!(
        out.contains("return v2;"),
        "Should return v2 directly:\n{out}"
    );
}

#[test]
fn struct_emission() {
    let out = build_and_emit(|mb| {
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
    });

    assert!(out.contains("export interface Point {"));
    assert!(out.contains("  x: number;"));
    assert!(out.contains("  y: number;"));
}

#[test]
fn enum_emission() {
    let out = build_and_emit(|mb| {
        mb.add_enum(EnumDef {
            name: "Shape".into(),
            variants: vec![
                EnumVariant {
                    name: "Circle".into(),
                    fields: vec![Type::Float(64)],
                },
                EnumVariant {
                    name: "Rect".into(),
                    fields: vec![Type::Float(64), Type::Float(64)],
                },
            ],
            visibility: Visibility::Public,
        });
    });

    assert!(out.contains("export type Shape ="));
    assert!(out.contains("tag: \"Circle\""));
    assert!(out.contains("tag: \"Rect\""));
}

#[test]
fn global_variables() {
    let out = build_and_emit(|mb| {
        mb.add_global(Global {
            name: "counter".into(),
            ty: Type::Int(64),
            visibility: Visibility::Public,
            mutable: true,
            init: None,
        });
        mb.add_global(Global {
            name: "MAX_SIZE".into(),
            ty: Type::Int(64),
            visibility: Visibility::Private,
            mutable: false,
            init: None,
        });
    });

    assert!(out.contains("export let counter: number;"));
    // const without init demoted to let
    assert!(out.contains("let MAX_SIZE: number;"));
}

#[test]
fn imports() {
    let out = build_and_emit(|mb| {
        mb.add_import(Import {
            module: "utils".into(),
            name: "helper".into(),
            alias: None,
        });
        mb.add_import(Import {
            module: "math".into(),
            name: "add".into(),
            alias: Some("mathAdd".into()),
        });
    });

    assert!(out.contains("import { helper } from \"./utils\";"));
    assert!(out.contains("import { add as mathAdd } from \"./math\";"));
}

#[test]
fn system_call() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("init", sig, Visibility::Public);
        let x = fb.const_int(100, 64);
        let y = fb.const_int(200, 64);
        fb.system_call("renderer", "clear", &[x, y], Type::Void);
        fb.ret(None);
        mb.add_function(fb.build());
    });

    // Auto-injected runtime import.
    assert!(out.contains("import { renderer } from \"./runtime\";"));
    // Constants are inlined into the system call.
    assert!(
        out.contains("renderer.clear(100, 200);"),
        "Should inline consts into system call:\n{out}"
    );
}

#[test]
fn multiple_system_imports() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("tick", sig, Visibility::Public);
        fb.system_call("audio", "play", &[], Type::Void);
        fb.system_call("input", "update", &[], Type::Void);
        fb.system_call("renderer", "present", &[], Type::Void);
        fb.ret(None);
        mb.add_function(fb.build());
    });

    assert!(out.contains("import { audio, input, renderer } from \"./runtime\";"));
}

#[test]
fn no_runtime_import_without_system_calls() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("noop", sig, Visibility::Public);
        fb.ret(None);
        mb.add_function(fb.build());
    });

    assert!(!out.contains("import"));
}

#[test]
fn constants_all_types() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("constants", sig, Visibility::Public);
        // Each constant is passed to a call so it actually appears in output.
        let a = fb.const_null();
        fb.call("use_val", &[a], Type::Void);
        let b = fb.const_bool(true);
        fb.call("use_val", &[b], Type::Void);
        let c = fb.const_bool(false);
        fb.call("use_val", &[c], Type::Void);
        let d = fb.const_int(42, 64);
        fb.call("use_val", &[d], Type::Void);
        let e = fb.const_float(3.125);
        fb.call("use_val", &[e], Type::Void);
        let f = fb.const_string("hello \"world\"\nnewline");
        fb.call("use_val", &[f], Type::Void);
        fb.ret(None);
        mb.add_function(fb.build());
    });

    assert!(out.contains("null"), "Should contain null:\n{out}");
    assert!(out.contains("true"), "Should contain true:\n{out}");
    assert!(out.contains("false"), "Should contain false:\n{out}");
    assert!(out.contains("42"), "Should contain 42:\n{out}");
    assert!(out.contains("3.125"), "Should contain 3.125:\n{out}");
    // Multiline strings are emitted as template literals
    assert!(
        out.contains("`hello \"world\"\nnewline`"),
        "Should contain template literal string:\n{out}"
    );
}

#[test]
fn array_and_struct_init() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("init", sig, Visibility::Public);

        let a = fb.const_int(1, 64);
        let b = fb.const_int(2, 64);
        let arr = fb.array_init(&[a, b], Type::Int(64));
        fb.call("use_val", &[arr], Type::Void);

        let x = fb.const_float(10.0);
        let y = fb.const_float(20.0);
        let obj = fb.struct_init("Point", vec![("x".into(), x), ("y".into(), y)]);
        fb.call("use_val", &[obj], Type::Void);

        fb.ret(None);
        mb.add_function(fb.build());
    });

    // Constants are inlined into the aggregate expressions.
    assert!(
        out.contains("[1, 2]"),
        "Should inline consts into array:\n{out}"
    );
    assert!(
        out.contains("{ x: 10.0, y: 20.0 }"),
        "Should inline consts into struct:\n{out}"
    );
}

#[test]
fn mem2reg_plus_emit_eliminates_alloc_store_load() {
    use reincarnate_core::pipeline::Transform;
    use reincarnate_core::transforms::Mem2Reg;

    let sig = FunctionSig {
        params: vec![Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("identity", sig, Visibility::Public);
    let param = fb.param(0);
    // Alloc → Store → Load chain (typical local variable pattern).
    let ptr = fb.alloc(Type::Int(64));
    fb.store(ptr, param);
    let loaded = fb.load(ptr, Type::Int(64));
    fb.ret(Some(loaded));

    let mut mb = ModuleBuilder::new("test");
    mb.add_function(fb.build());
    let module = mb.build();

    // Run mem2reg IR pass, then emit.
    let mut result = Mem2Reg.apply(module).unwrap();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut result.module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    // The alloc/store/load should be eliminated; return refers to the
    // original parameter directly.
    assert!(out.contains("return v0;"));
    assert!(!out.contains("undefined"));
}

#[test]
fn sanitize_ident_handles_avm2_names() {
    assert_eq!(sanitize_ident("Flash.Object"), "Flash_Object");
    assert_eq!(
        sanitize_ident("flash.display::Loader"),
        "flash_display__Loader"
    );
    assert_eq!(sanitize_ident("4l9JT7u2nN1ZFk+5"), "_4l9JT7u2nN1ZFk_5");
    assert_eq!(sanitize_ident("l/YEs377IakicDh/"), "l_YEs377IakicDh_");
    assert_eq!(sanitize_ident("normal_name"), "normal_name");
}

#[test]
fn sanitize_ident_escapes_reserved_words() {
    assert_eq!(sanitize_ident("function"), "_function");
    assert_eq!(sanitize_ident("class"), "_class");
    assert_eq!(sanitize_ident("this"), "_this");
    assert_eq!(sanitize_ident("let"), "_let");
    // Non-reserved words pass through unchanged.
    assert_eq!(sanitize_ident("foo"), "foo");
}

#[test]
fn bracket_notation_for_non_ident_fields() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("get_prop", sig, Visibility::Public);
        let obj = fb.param(0);
        // Qualified field → short name extraction.
        let result = fb.get_field(obj, "flash.display::Loader", Type::Unknown);
        fb.ret(Some(result));
        mb.add_function(fb.build());
    });

    // Qualified field should extract short name and use dot notation.
    assert!(out.contains(".Loader"), "Should use short name:\n{out}");
    assert!(
        !out.contains("flash.display::Loader"),
        "Should not have full qualified name:\n{out}"
    );
}

#[test]
fn emit_structured_if_else() {
    //   entry: br_if cond, then, else
    //   then:  br merge
    //   else:  br merge
    //   merge: return
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("diamond", sig, Visibility::Public);
        let cond = fb.param(0);

        let then_block = fb.create_block();
        let else_block = fb.create_block();
        let merge_block = fb.create_block();

        fb.br_if(cond, then_block, &[], else_block, &[]);

        fb.switch_to_block(then_block);
        fb.br(merge_block, &[]);

        fb.switch_to_block(else_block);
        fb.br(merge_block, &[]);

        fb.switch_to_block(merge_block);
        fb.ret(None);

        mb.add_function(fb.build());
    });

    // Both branches are empty — entire if should be omitted.
    assert!(
        !out.contains("$block"),
        "Should not use dispatch loop:\n{out}"
    );
    assert!(
        !out.contains("if ("),
        "Empty diamond should omit entire if:\n{out}"
    );
    // Trailing void return should be stripped.
    assert!(
        !out.contains("return;"),
        "Should not have trailing return:\n{out}"
    );
}

#[test]
fn emit_while_loop() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("while_loop", sig, Visibility::Public);
        let cond = fb.param(0);

        let header = fb.create_block();
        let body = fb.create_block();
        let exit = fb.create_block();

        fb.br(header, &[]);

        fb.switch_to_block(header);
        fb.br_if(cond, body, &[], exit, &[]);

        fb.switch_to_block(body);
        fb.br(header, &[]);

        fb.switch_to_block(exit);
        fb.ret(None);

        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("$block"),
        "Should not use dispatch loop:\n{out}"
    );
    assert!(out.contains("while ("), "Should have while loop:\n{out}");
}

#[test]
fn emit_for_loop() {
    use reincarnate_core::ir::CmpKind;

    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("for_loop", sig, Visibility::Public);

        let (header, header_vals) = fb.create_block_with_params(&[Type::Int(64)]);
        let body = fb.create_block();
        let exit = fb.create_block();

        let v_init = fb.const_int(0, 64);
        fb.br(header, &[v_init]);

        fb.switch_to_block(header);
        let v_i = header_vals[0];
        let v_n = fb.const_int(10, 64);
        let v_cond = fb.cmp(CmpKind::Lt, v_i, v_n);
        fb.br_if(v_cond, body, &[], exit, &[]);

        fb.switch_to_block(body);
        let v_one = fb.const_int(1, 64);
        let v_next = fb.add(v_i, v_one);
        fb.br(header, &[v_next]);

        fb.switch_to_block(exit);
        fb.ret(None);

        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("$block"),
        "Should not use dispatch loop:\n{out}"
    );
    // For-loop emits as while(cond) with init assigns before and
    // update assigns inside.
    assert!(out.contains("while ("), "Should have loop:\n{out}");
    // Init assigns header param v0 from inlined const 0 (merged into decl).
    assert!(
        out.contains("let v0: number = 0;"),
        "Should have init assign:\n{out}"
    );
    // Update assigns header param v0 from inlined v0 + 1 (compound assignment).
    assert!(
        out.contains("v0 += 1;"),
        "Should have update assign:\n{out}"
    );
}

#[test]
fn emit_class_with_methods() {
    let mut mb = ModuleBuilder::new("test");

    // Struct for class fields.
    mb.add_struct(StructDef {
        name: "Fighter".into(),
        namespace: vec!["classes".into(), "Scenes".into()],
        fields: vec![FieldDef {
            name: "hp".into(),
            ty: Type::Int(32),
            default: None,
        }],
        visibility: Visibility::Public,
    });

    // Constructor: (this: unknown) -> void
    let ctor_sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Fighter::new", ctor_sig, Visibility::Public);
    fb.set_class(
        vec!["classes".into(), "Scenes".into()],
        "Fighter".into(),
        MethodKind::Constructor,
    );
    fb.ret(None);
    let ctor_id = mb.add_function(fb.build());

    // Instance method: (this: unknown, amount: i32) -> void
    let method_sig = FunctionSig {
        params: vec![Type::Unknown, Type::Int(32)],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Fighter::attack", method_sig, Visibility::Public);
    fb.set_class(
        vec!["classes".into(), "Scenes".into()],
        "Fighter".into(),
        MethodKind::Instance,
    );
    let _this = fb.param(0);
    let _amount = fb.param(1);
    fb.ret(None);
    let method_id = mb.add_function(fb.build());

    // Static method: (self: unknown, amount: i32) -> i32
    // AVM2 register 0 is always reserved, so static methods include
    // a self/scope param that the emitter skips.
    let static_sig = FunctionSig {
        params: vec![Type::Unknown, Type::Int(32)],
        return_ty: Type::Int(32),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Fighter::create", static_sig, Visibility::Public);
    fb.set_class(
        vec!["classes".into(), "Scenes".into()],
        "Fighter".into(),
        MethodKind::Static,
    );
    let _self = fb.param(0);
    let p = fb.param(1);
    fb.ret(Some(p));
    let static_id = mb.add_function(fb.build());

    // Getter: (this: unknown) -> i32
    let getter_sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Int(32),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Fighter::get_health", getter_sig, Visibility::Public);
    fb.set_class(
        vec!["classes".into(), "Scenes".into()],
        "Fighter".into(),
        MethodKind::Getter,
    );
    let this = fb.param(0);
    let hp = fb.get_field(this, "hp", Type::Int(32));
    fb.ret(Some(hp));
    let getter_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Fighter".into(),
        namespace: vec!["classes".into(), "Scenes".into()],
        struct_index: 0,
        methods: vec![ctor_id, method_id, static_id, getter_id],
        super_class: Some("Object".into()),
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    // Class declaration — `extends Object` is suppressed (redundant in JS).
    assert!(
        out.contains("export class Fighter {"),
        "Should have class decl without extends Object:\n{out}"
    );
    // Field.
    assert!(out.contains("  hp: number;"), "Should have field:\n{out}");
    // Constructor — no `this` param; Flash injects `readonly _shims: FlashShims`.
    // super_class = Some("Object") → suppress_super = true → base class param.
    assert!(
        out.contains("  constructor(readonly _shims: FlashShims) {"),
        "Should have constructor with FlashShims param:\n{out}"
    );
    // Instance method — skips `this`.
    assert!(
        out.contains("  attack(v1: number): void {"),
        "Should have instance method:\n{out}"
    );
    // Static method — skips self param (AVM2 register 0).
    assert!(
        out.contains("  static create(v1: number): number {"),
        "Should have static method:\n{out}"
    );
    // Getter — strips `get_` prefix, body uses `this`.
    assert!(
        out.contains("  get health(): number {"),
        "Should have getter:\n{out}"
    );
    assert!(
        out.contains("this.hp"),
        "Getter body should use `this.hp`:\n{out}"
    );
}

#[test]
fn emit_class_and_free_functions() {
    let mut mb = ModuleBuilder::new("test");

    mb.add_struct(StructDef {
        name: "Foo".into(),
        namespace: Vec::new(),
        fields: vec![],
        visibility: Visibility::Public,
    });

    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Foo::new", sig, Visibility::Public);
    fb.set_class(Vec::new(), "Foo".into(), MethodKind::Constructor);
    fb.ret(None);
    let ctor_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Foo".into(),
        namespace: Vec::new(),
        struct_index: 0,
        methods: vec![ctor_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    // Free function.
    let sig = FunctionSig {
        params: vec![],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("init", sig, Visibility::Public);
    fb.ret(None);
    mb.add_function(fb.build());

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        out.contains("export class Foo {"),
        "Should have class:\n{out}"
    );
    assert!(
        out.contains("export function init(): void {"),
        "Should have free function:\n{out}"
    );
}

#[test]
fn relative_import_path_same_dir() {
    let from = vec![
        "classes".into(),
        "Scenes".into(),
        "Swamp".into(),
        "Swamp".into(),
    ];
    let to = vec![
        "classes".into(),
        "Scenes".into(),
        "Swamp".into(),
        "CorruptedDriderScene".into(),
    ];
    assert_eq!(relative_import_path(&from, &to), "./CorruptedDriderScene");
}

#[test]
fn relative_import_path_different_dir() {
    let from = vec![
        "classes".into(),
        "Scenes".into(),
        "Swamp".into(),
        "Swamp".into(),
    ];
    let to = vec!["classes".into(), "CoC".into()];
    assert_eq!(relative_import_path(&from, &to), "../../CoC");
}

#[test]
fn relative_import_path_no_common() {
    let from = vec!["a".into(), "b".into()];
    let to = vec!["c".into(), "d".into()];
    assert_eq!(relative_import_path(&from, &to), "../c/d");
}

#[test]
fn emit_nested_class_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut mb = ModuleBuilder::new("frame1");

    // Class with namespace → nested directory.
    mb.add_struct(StructDef {
        name: "Swamp".into(),
        namespace: vec!["classes".into(), "Scenes".into()],
        fields: vec![FieldDef {
            name: "hp".into(),
            ty: Type::Int(32),
            default: None,
        }],
        visibility: Visibility::Public,
    });

    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Swamp::new", sig, Visibility::Public);
    fb.set_class(
        vec!["classes".into(), "Scenes".into()],
        "Swamp".into(),
        MethodKind::Constructor,
    );
    fb.system_call("renderer", "clear", &[], Type::Void);
    fb.ret(None);
    let ctor_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Swamp".into(),
        namespace: vec!["classes".into(), "Scenes".into()],
        struct_index: 0,
        methods: vec![ctor_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    emit_module_to_dir(
        &mut module,
        dir.path(),
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    // Check nested file exists.
    let class_file = dir.path().join("frame1/classes/Scenes/Swamp.ts");
    assert!(class_file.exists(), "Nested class file should exist");

    let content = fs::read_to_string(&class_file).unwrap();

    // Runtime import should go up 3 levels (2 namespace segments + 1 module dir).
    assert!(
        content.contains("from \"../../../runtime\""),
        "Runtime import should use depth-relative path:\n{content}"
    );

    // Barrel file should have nested re-export.
    let barrel = fs::read_to_string(dir.path().join("frame1/index.ts")).unwrap();
    assert!(
        barrel.contains("export * from \"./classes/Scenes/Swamp\";"),
        "Barrel should have nested export path:\n{barrel}"
    );
}

#[test]
fn emit_intra_module_imports() {
    let dir = tempfile::tempdir().unwrap();
    let mut mb = ModuleBuilder::new("frame1");

    // Two classes: Monster (root) and Swamp (nested), where Swamp references Monster.
    // Intern the qualified name so the field type lookup works correctly.
    let monster_type_id = mb.intern_type("classes::Monster");
    mb.add_struct(StructDef {
        name: "Monster".into(),
        namespace: vec!["classes".into()],
        fields: vec![],
        visibility: Visibility::Public,
    });
    mb.add_struct(StructDef {
        name: "Swamp".into(),
        namespace: vec!["classes".into(), "Scenes".into()],
        fields: vec![FieldDef {
            name: "boss".into(),
            ty: Type::Instance(monster_type_id),
            default: None,
        }],
        visibility: Visibility::Public,
    });

    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Monster::new", sig.clone(), Visibility::Public);
    fb.set_class(
        vec!["classes".into()],
        "Monster".into(),
        MethodKind::Constructor,
    );
    fb.ret(None);
    let monster_ctor = mb.add_function(fb.build());

    let mut fb = FunctionBuilder::new("Swamp::new", sig, Visibility::Public);
    fb.set_class(
        vec!["classes".into(), "Scenes".into()],
        "Swamp".into(),
        MethodKind::Constructor,
    );
    fb.ret(None);
    let swamp_ctor = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Monster".into(),
        namespace: vec!["classes".into()],
        struct_index: 0,
        methods: vec![monster_ctor],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });
    mb.add_class(ClassDef {
        name: "Swamp".into(),
        namespace: vec!["classes".into(), "Scenes".into()],
        struct_index: 1,
        methods: vec![swamp_ctor],
        super_class: Some("classes::Monster".into()),
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    emit_module_to_dir(
        &mut module,
        dir.path(),
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    let swamp_file = dir.path().join("frame1/classes/Scenes/Swamp.ts");
    let content = fs::read_to_string(&swamp_file).unwrap();

    // Swamp extends Monster → should have import for Monster.
    assert!(
        content.contains("import { Monster } from \"../Monster\";"),
        "Should import Monster from parent dir:\n{content}"
    );
}

#[test]
fn construct_super_emits_super_call() {
    let mut mb = ModuleBuilder::new("test");

    // Add Parent as a user-defined class so Child's super call gets _shims injected.
    mb.add_struct(StructDef {
        name: "Parent".into(),
        namespace: Vec::new(),
        fields: vec![],
        visibility: Visibility::Public,
    });
    mb.add_class(ClassDef {
        name: "Parent".into(),
        namespace: Vec::new(),
        struct_index: 0,
        methods: vec![],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    mb.add_struct(StructDef {
        name: "Child".into(),
        namespace: Vec::new(),
        fields: vec![],
        visibility: Visibility::Public,
    });

    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Child::new", sig, Visibility::Public);
    fb.set_class(Vec::new(), "Child".into(), MethodKind::Constructor);
    let this = fb.param(0);
    // Place a field init before constructSuper — mimics AVM2 constructor order
    let val = fb.const_int(0, 64);
    fb.set_field(this, "x", val);
    fb.system_call("Flash.Class", "constructSuper", &[this], Type::Void);
    fb.ret(None);
    let ctor_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Child".into(),
        namespace: Vec::new(),
        struct_index: 1, // Parent is struct 0, Child is struct 1
        methods: vec![ctor_id],
        super_class: Some("Parent".into()),
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        out.contains("super(_shims);"),
        "constructSuper should emit super(_shims):\n{out}"
    );
    assert!(
        !out.contains("constructSuper"),
        "Should not have raw constructSuper call:\n{out}"
    );
    // super() must come before any this.field access
    let super_pos = out.find("super(_shims);").expect("super(_shims) not found");
    let field_pos = out.find("this.x").expect("this.x not found");
    assert!(
        super_pos < field_pos,
        "super(_shims) must precede this.x access:\n{out}"
    );
}

#[test]
fn find_prop_strict_get_field_construct_emits_new() {
    let mut mb = ModuleBuilder::new("test");

    // Two classes: Container and Widget. Container constructs a Widget.
    mb.add_struct(StructDef {
        name: "Container".into(),
        namespace: Vec::new(),
        fields: vec![],
        visibility: Visibility::Public,
    });
    mb.add_struct(StructDef {
        name: "Widget".into(),
        namespace: Vec::new(),
        fields: vec![],
        visibility: Visibility::Public,
    });

    // Container constructor does findPropStrict + getField + construct.
    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Container::new", sig, Visibility::Public);
    fb.set_class(Vec::new(), "Container".into(), MethodKind::Constructor);
    let _this = fb.param(0);

    // findPropStrict("Widget")
    let name = fb.const_string("Widget");
    let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
    // getField(scope, "Widget")
    let ctor = fb.get_field(scope, "Widget", Type::Unknown);
    // construct(ctor)
    let obj = fb.system_call("Flash.Object", "construct", &[ctor], Type::Unknown);
    // Use the result so it's not dead code.
    fb.set_field(_this, "child", obj);
    fb.ret(None);
    let ctor_id = mb.add_function(fb.build());

    // Widget constructor (empty).
    let sig2 = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb2 = FunctionBuilder::new("Widget::new", sig2, Visibility::Public);
    fb2.set_class(Vec::new(), "Widget".into(), MethodKind::Constructor);
    fb2.ret(None);
    let widget_ctor_id = mb.add_function(fb2.build());

    mb.add_class(ClassDef {
        name: "Container".into(),
        namespace: Vec::new(),
        struct_index: 0,
        methods: vec![ctor_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });
    mb.add_class(ClassDef {
        name: "Widget".into(),
        namespace: Vec::new(),
        struct_index: 1,
        methods: vec![widget_ctor_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        out.contains("new Widget(this._shims)"),
        "Should emit new Widget(this._shims):\n{out}"
    );
    assert!(
        !out.contains("Flash_Object.construct"),
        "Should not have Flash_Object.construct call:\n{out}"
    );
    assert!(
        !out.contains("Flash_Scope.findPropStrict"),
        "findPropStrict should be resolved away:\n{out}"
    );
}

#[test]
fn qualified_call_emits_method_dispatch() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let receiver = fb.param(0);
        let arg1 = fb.const_string("text");
        let arg2 = fb.const_bool(true);
        // Method call: receiver.outputText("text", true)
        let result = fb.call_method(receiver, "outputText", &[arg1, arg2], Type::Unknown);
        fb.ret(Some(result));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains(".outputText("),
        "Should emit method dispatch:\n{out}"
    );
    assert!(
        !out.contains("classes_BaseContent__outputText"),
        "Should not emit sanitized function call:\n{out}"
    );
}

#[test]
fn qualified_get_field_emits_short_name() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let obj = fb.param(0);
        let result = fb.get_field(obj, "classes:BaseContent::flags", Type::Unknown);
        fb.ret(Some(result));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains(".flags"),
        "Should use short field name:\n{out}"
    );
    assert!(
        !out.contains("BaseContent"),
        "Should not have qualified name:\n{out}"
    );
}

#[test]
fn qualified_set_field_emits_short_name() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown, Type::Int(32)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let obj = fb.param(0);
        let val = fb.param(1);
        fb.set_field(obj, "classes:BaseContent::flags", val);
        fb.ret(None);
        mb.add_function(fb.build());
    });

    assert!(
        out.contains(".flags = "),
        "Should use short field name for set:\n{out}"
    );
    assert!(
        !out.contains("BaseContent"),
        "Should not have qualified name:\n{out}"
    );
}

#[test]
fn find_prop_strict_resolves_to_this_for_own_class() {
    let mut mb = ModuleBuilder::new("test");

    mb.add_struct(StructDef {
        name: "Hero".into(),
        namespace: vec!["classes".into()],
        fields: vec![FieldDef {
            name: "hp".into(),
            ty: Type::Int(32),
            default: None,
        }],
        visibility: Visibility::Public,
    });

    // Instance method that does findPropStrict("classes:Hero::hp") + getField.
    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Unknown,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Hero::getHp", sig, Visibility::Public);
    fb.set_class(vec!["classes".into()], "Hero".into(), MethodKind::Instance);
    let _this = fb.param(0);
    let name = fb.const_string("classes:Hero::hp");
    let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
    let val = fb.get_field(scope, "classes:Hero::hp", Type::Int(32));
    fb.ret(Some(val));
    let method_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Hero".into(),
        namespace: vec!["classes".into()],
        struct_index: 0,
        methods: vec![method_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        out.contains("this.hp"),
        "findPropStrict for own class should resolve to this.hp:\n{out}"
    );
    assert!(
        !out.contains("findPropStrict"),
        "findPropStrict should be resolved away:\n{out}"
    );
}

#[test]
fn find_prop_strict_resolves_to_this_for_ancestor() {
    let mut mb = ModuleBuilder::new("test");

    mb.add_struct(StructDef {
        name: "Base".into(),
        namespace: vec!["classes".into()],
        fields: vec![FieldDef {
            name: "player".into(),
            ty: Type::Unknown,
            default: None,
        }],
        visibility: Visibility::Public,
    });
    mb.add_struct(StructDef {
        name: "Child".into(),
        namespace: vec!["classes".into()],
        fields: vec![],
        visibility: Visibility::Public,
    });

    // Child instance method does findPropStrict("classes:Base::player") + getField.
    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Unknown,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Child::getPlayer", sig, Visibility::Public);
    fb.set_class(vec!["classes".into()], "Child".into(), MethodKind::Instance);
    let _this = fb.param(0);
    let name = fb.const_string("classes:Base::player");
    let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
    let val = fb.get_field(scope, "classes:Base::player", Type::Unknown);
    fb.ret(Some(val));
    let method_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Base".into(),
        namespace: vec!["classes".into()],
        struct_index: 0,
        methods: vec![],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });
    mb.add_class(ClassDef {
        name: "Child".into(),
        namespace: vec!["classes".into()],
        struct_index: 1,
        methods: vec![method_id],
        super_class: Some("classes::Base".into()),
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        out.contains("this.player"),
        "findPropStrict for ancestor class should resolve to this.player:\n{out}"
    );
    assert!(
        !out.contains("findPropStrict"),
        "findPropStrict should be resolved away:\n{out}"
    );
}

#[test]
fn find_prop_strict_in_free_function_resolves_to_bare_name() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("init", sig, Visibility::Public);
        let name = fb.const_string("classes:Hero::hp");
        let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
        let val = fb.get_field(scope, "classes:Hero::hp", Type::Unknown);
        fb.ret(Some(val));
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("findPropStrict"),
        "findPropStrict should be resolved away:\n{out}"
    );
    assert!(
        out.contains("hp"),
        "Should resolve to bare field name:\n{out}"
    );
    assert!(
        !out.contains("this.hp"),
        "Free function should not resolve to this:\n{out}"
    );
}

#[test]
fn find_prop_strict_non_ancestor_resolves_to_bare_name() {
    let mut mb = ModuleBuilder::new("test");

    mb.add_struct(StructDef {
        name: "Hero".into(),
        namespace: vec!["classes".into()],
        fields: vec![],
        visibility: Visibility::Public,
    });
    mb.add_struct(StructDef {
        name: "Villain".into(),
        namespace: vec!["classes".into()],
        fields: vec![FieldDef {
            name: "power".into(),
            ty: Type::Int(32),
            default: None,
        }],
        visibility: Visibility::Public,
    });

    // Hero method does findPropStrict("classes:Villain::power") — unrelated class.
    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Unknown,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Hero::spy", sig, Visibility::Public);
    fb.set_class(vec!["classes".into()], "Hero".into(), MethodKind::Instance);
    let _this = fb.param(0);
    let name = fb.const_string("classes:Villain::power");
    let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
    let val = fb.get_field(scope, "classes:Villain::power", Type::Int(32));
    fb.ret(Some(val));
    let method_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Hero".into(),
        namespace: vec!["classes".into()],
        struct_index: 0,
        methods: vec![method_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });
    mb.add_class(ClassDef {
        name: "Villain".into(),
        namespace: vec!["classes".into()],
        struct_index: 1,
        methods: vec![],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        !out.contains("this.power"),
        "Non-ancestor should not resolve to this:\n{out}"
    );
    assert!(
        !out.contains("findPropStrict"),
        "findPropStrict should be resolved to bare name:\n{out}"
    );
    assert!(
        out.contains("power"),
        "Should resolve to bare field name:\n{out}"
    );
}

#[test]
fn unqualified_find_prop_strict_resolves_to_bare_name() {
    // findPropStrict("rand") + getField("rand") → rand
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let name = fb.const_string("rand");
        let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
        let rand_fn = fb.get_field(scope, "rand", Type::Unknown);
        let result = fb.call_indirect(rand_fn, &[x], Type::Unknown);
        fb.ret(Some(result));
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("Flash_Scope.findPropStrict"),
        "findPropStrict call should be resolved away:\n{out}"
    );
    assert!(
        out.contains("rand("),
        "Should resolve to bare function call:\n{out}"
    );
}

#[test]
fn find_property_set_field_resolves_to_bare_assignment() {
    // findProperty("X") + setField("X", 5) → X = 5
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let name = fb.const_string("X");
        let scope = fb.system_call("Flash.Scope", "findProperty", &[name], Type::Unknown);
        let val = fb.const_int(5, 64);
        fb.set_field(scope, "X", val);
        fb.ret(None);
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("Flash_Scope.findProperty"),
        "findProperty call should be resolved away:\n{out}"
    );
    assert!(
        out.contains("X = 5"),
        "Should resolve to bare assignment:\n{out}"
    );
}

#[test]
fn find_property_resolves_to_this_for_ancestor() {
    // findProperty("classes:Base::temp") in instance method → this
    let mut mb = ModuleBuilder::new("test");

    mb.add_struct(StructDef {
        name: "Base".into(),
        namespace: vec!["classes".into()],
        fields: vec![FieldDef {
            name: "temp".into(),
            ty: Type::Unknown,
            default: None,
        }],
        visibility: Visibility::Public,
    });

    let sig = FunctionSig {
        params: vec![Type::Unknown, Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Base::setTemp", sig, Visibility::Public);
    fb.set_class(vec!["classes".into()], "Base".into(), MethodKind::Instance);
    let _this = fb.param(0);
    let v = fb.param(1);
    let name = fb.const_string("classes:Base::temp");
    let scope = fb.system_call("Flash.Scope", "findProperty", &[name], Type::Unknown);
    fb.set_field(scope, "classes:Base::temp", v);
    fb.ret(None);
    let method_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Base".into(),
        namespace: vec!["classes".into()],
        struct_index: 0,
        methods: vec![method_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        out.contains("this.temp = "),
        "findProperty for own class should resolve to this.temp:\n{out}"
    );
    assert!(
        !out.contains("findProperty"),
        "findProperty should be resolved away:\n{out}"
    );
}

#[test]
fn qualified_find_prop_strict_non_class_resolves_to_bare_name() {
    // findPropStrict("flash.events::Event") + getField("Event") → Event
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let name = fb.const_string("flash.events::Event");
        let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
        let event_cls = fb.get_field(scope, "flash.events::Event", Type::Unknown);
        let change = fb.get_field(event_cls, "CHANGE", Type::Unknown);
        fb.ret(Some(change));
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("Flash_Scope.findPropStrict"),
        "findPropStrict call should be resolved away:\n{out}"
    );
    assert!(out.contains("Event"), "Should resolve to Event:\n{out}");
}

#[test]
fn call_unqualified_strips_scope_receiver() {
    // findPropStrict("rand") + call_method(scope, "rand", [x]) → rand(x)
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let name = fb.const_string("rand");
        let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
        let result = fb.call_method(scope, "rand", &[x], Type::Int(64));
        fb.ret(Some(result));
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("Flash_Scope.findPropStrict"),
        "findPropStrict call should be resolved away:\n{out}"
    );
    assert!(
        out.contains("rand(v0)"),
        "Should emit rand(v0) without scope arg:\n{out}"
    );
}

#[test]
fn call_qualified_strips_scope_receiver() {
    // findPropStrict("flash.net::registerClassAlias") + call_method with qualified name
    // → registerClassAlias("Foo", Foo)
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let name = fb.const_string("flash.net::registerClassAlias");
        let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
        let alias = fb.const_string("Foo");
        let cls = fb.const_string("FooCls");
        fb.call_method(scope, "registerClassAlias", &[alias, cls], Type::Void);
        fb.ret(None);
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("Flash_Scope.findPropStrict"),
        "findPropStrict call should be resolved away:\n{out}"
    );
    assert!(
        out.contains("registerClassAlias("),
        "Should emit bare registerClassAlias call:\n{out}"
    );
    assert!(
        !out.contains(".registerClassAlias("),
        "Should not emit method dispatch on scope:\n{out}"
    );
}

#[test]
fn call_method_emits_receiver_dot_method() {
    // MethodCall with explicit receiver → receiver.method(args).
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Int(64), Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let a = fb.param(0);
        let b = fb.param(1);
        let result = fb.call_method(a, "add", &[b], Type::Int(64));
        fb.ret(Some(result));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("v0.add(v1)"),
        "Should emit receiver.method(args) for MethodCall:\n{out}"
    );
}

#[test]
fn binop_add_strips_scope_operand() {
    // findPropStrict("int") + int(x) → int(x)
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let name = fb.const_string("int");
        let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
        let int_fn = fb.get_field(scope, "int", Type::Unknown);
        let casted = fb.call_indirect(int_fn, &[x], Type::Int(64));
        let sum = fb.add(scope, casted);
        fb.ret(Some(sum));
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("findPropStrict"),
        "findPropStrict should not appear in function body:\n{out}"
    );
    // The add should collapse to just the non-scope operand.
    assert!(
        out.contains("return int(v0);"),
        "Should resolve to int(x) without scope operand:\n{out}"
    );
}

#[test]
fn standalone_scope_lookup_not_emitted() {
    // findPropStrict("rand") with no GetField/Call use → no output
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let name = fb.const_string("rand");
        let _scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
        fb.ret(None);
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("findPropStrict"),
        "Standalone scope lookup should not emit findPropStrict:\n{out}"
    );
}

#[test]
fn scope_lookup_call_resolves_to_this_for_inherited_method() {
    // In a method context, an unqualified call with scope-lookup receiver
    // whose name matches a method in the class hierarchy should emit
    // this.method() instead of method().
    let mut mb = ModuleBuilder::new("test");

    mb.add_struct(StructDef {
        name: "Base".into(),
        namespace: vec![],
        fields: vec![],
        visibility: Visibility::Public,
    });
    mb.add_struct(StructDef {
        name: "Child".into(),
        namespace: vec![],
        fields: vec![],
        visibility: Visibility::Public,
    });

    // Base class with isNaga method.
    let base_sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Bool,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Base::isNaga", base_sig, Visibility::Public);
    fb.set_class(vec![], "Base".into(), MethodKind::Instance);
    let _this = fb.param(0);
    let c = fb.const_bool(false);
    fb.ret(Some(c));
    let base_method_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Base".into(),
        namespace: vec![],
        struct_index: 0,
        methods: vec![base_method_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    // Child class with a method that calls isNaga via scope lookup.
    let child_sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Bool,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Child::check", child_sig, Visibility::Public);
    fb.set_class(vec![], "Child".into(), MethodKind::Instance);
    let _this = fb.param(0);
    let name = fb.const_string("isNaga");
    let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
    let result = fb.call_method(scope, "isNaga", &[], Type::Bool);
    fb.ret(Some(result));
    let child_method_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Child".into(),
        namespace: vec![],
        struct_index: 1,
        methods: vec![child_method_id],
        super_class: Some("Base".into()),
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        out.contains("this.isNaga()"),
        "Should emit this.isNaga() for inherited method call:\n{out}"
    );
    assert!(
        !out.contains("findPropStrict"),
        "findPropStrict should be resolved away:\n{out}"
    );
}

#[test]
fn unqualified_callproperty_emits_receiver_dot_method() {
    // MethodCall pattern: call_method(player, "isNaga", []) → player.isNaga()
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let player = fb.param(0);
        let result = fb.call_method(player, "isNaga", &[], Type::Bool);
        fb.ret(Some(result));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("v0.isNaga()"),
        "Should emit receiver.method() for MethodCall:\n{out}"
    );
}

#[test]
fn cast_elided_when_source_type_matches() {
    // Cast(v, Bool) where v is already Bool → no "as boolean".
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let casted = fb.cast(x, Type::Bool);
        fb.ret(Some(casted));
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("as boolean"),
        "Redundant cast should be elided:\n{out}"
    );
}

#[test]
fn cast_inlined_uses_as_t() {
    // Single-use cast (inlined into return) → needs "as T" wrapper.
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let casted = fb.cast(x, Type::Bool);
        fb.ret(Some(casted));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("return v0 as boolean"),
        "Inlined cast should use 'as T':\n{out}"
    );
}

#[test]
fn cast_binding_uses_type_annotation() {
    // Multi-use cast (assigned to variable) → type annotation instead of "as T".
    // Both uses must survive DCE to prevent single-use const folding.
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let casted = fb.cast(x, Type::Bool);
        // Use the cast result in a non-dead side-effecting call to keep both uses alive.
        fb.system_call("console", "log", &[casted], Type::Void);
        fb.ret(Some(casted));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains(": boolean = v0"),
        "Multi-use cast should use type annotation:\n{out}"
    );
    assert!(
        !out.contains("as boolean"),
        "Multi-use cast should not use 'as T':\n{out}"
    );
}

#[test]
fn if_else_empty_else_suppressed() {
    // if (cond) { body } else {} → if (cond) { body }
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let cond = fb.param(0);

        let then_block = fb.create_block();
        let else_block = fb.create_block();
        let merge = fb.create_block();

        fb.br_if(cond, then_block, &[], else_block, &[]);

        // then: has a call
        fb.switch_to_block(then_block);
        fb.system_call("renderer", "clear", &[], Type::Void);
        fb.br(merge, &[]);

        // else: empty — just branches to merge
        fb.switch_to_block(else_block);
        fb.br(merge, &[]);

        fb.switch_to_block(merge);
        fb.ret(None);

        mb.add_function(fb.build());
    });

    assert!(out.contains("if (v0) {"), "Should have if block:\n{out}");
    assert!(
        !out.contains("} else {"),
        "Empty else should be suppressed:\n{out}"
    );
}

#[test]
fn if_else_empty_then_flips_condition() {
    // if (cond) {} else { body } → if (!cond) { body }
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let cond = fb.param(0);

        let then_block = fb.create_block();
        let else_block = fb.create_block();
        let merge = fb.create_block();

        fb.br_if(cond, then_block, &[], else_block, &[]);

        // then: empty
        fb.switch_to_block(then_block);
        fb.br(merge, &[]);

        // else: has a call
        fb.switch_to_block(else_block);
        fb.system_call("renderer", "clear", &[], Type::Void);
        fb.br(merge, &[]);

        fb.switch_to_block(merge);
        fb.ret(None);

        mb.add_function(fb.build());
    });

    assert!(
        out.contains("if (!v0) {"),
        "Should flip condition when then is empty:\n{out}"
    );
    assert!(
        !out.contains("} else {"),
        "Should not have else branch:\n{out}"
    );
}

#[test]
fn if_else_both_empty_omits_entire_if() {
    // if (cond) {} else {} → nothing
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let cond = fb.param(0);

        let then_block = fb.create_block();
        let else_block = fb.create_block();
        let merge = fb.create_block();

        fb.br_if(cond, then_block, &[], else_block, &[]);

        // both empty
        fb.switch_to_block(then_block);
        fb.br(merge, &[]);

        fb.switch_to_block(else_block);
        fb.br(merge, &[]);

        fb.switch_to_block(merge);
        fb.ret(None);

        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("if ("),
        "Both branches empty — entire if should be omitted:\n{out}"
    );
}

#[test]
fn if_else_flip_unwraps_not_instead_of_double_negating() {
    // BrIf on Not(cond) with empty then → should emit if (cond), not if (!(!cond))
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let cond = fb.param(0);
        let not_cond = fb.not(cond);

        let then_block = fb.create_block();
        let else_block = fb.create_block();
        let merge = fb.create_block();

        fb.br_if(not_cond, then_block, &[], else_block, &[]);

        // then: empty
        fb.switch_to_block(then_block);
        fb.br(merge, &[]);

        // else: has a call
        fb.switch_to_block(else_block);
        fb.system_call("renderer", "clear", &[], Type::Void);
        fb.br(merge, &[]);

        fb.switch_to_block(merge);
        fb.ret(None);

        mb.add_function(fb.build());
    });

    assert!(
        out.contains("if (v0) {"),
        "Should unwrap Not and use original condition:\n{out}"
    );
    assert!(!out.contains("!(!"), "Should not double-negate:\n{out}");
}

#[test]
fn if_else_empty_then_flips_cmp_operator() {
    // BrIf(Cmp(Ge, a, b), then=empty, else=body) → if (a < b) { body }
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Int(64), Type::Int(64)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let a = fb.param(0);
        let b = fb.param(1);
        let cmp = fb.cmp(CmpKind::Ge, a, b);

        let then_block = fb.create_block();
        let else_block = fb.create_block();
        let merge = fb.create_block();

        fb.br_if(cmp, then_block, &[], else_block, &[]);

        // then: empty
        fb.switch_to_block(then_block);
        fb.br(merge, &[]);

        // else: has a call
        fb.switch_to_block(else_block);
        fb.system_call("renderer", "clear", &[], Type::Void);
        fb.br(merge, &[]);

        fb.switch_to_block(merge);
        fb.ret(None);

        mb.add_function(fb.build());
    });

    assert!(
        out.contains("if (v0 < v1) {"),
        "Should flip Cmp(Ge) to < when then is empty:\n{out}"
    );
    assert!(!out.contains("!("), "Should not wrap with !():\n{out}");
}

#[test]
fn cinit_scope_lookup_emits_this_dot_field() {
    // In cinit, findPropStrict that doesn't resolve to an ancestor should
    // still emit `this.field = value` (not bare `field = value`).
    let mut mb = ModuleBuilder::new("test");

    mb.add_struct(StructDef {
        name: "Settings".into(),
        namespace: vec!["classes".into()],
        fields: vec![],
        visibility: Visibility::Public,
    });

    // cinit: static initializer that sets a static field via scope lookup
    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("Settings::cinit", sig, Visibility::Public);
    fb.set_class(
        vec!["classes".into()],
        "Settings".into(),
        MethodKind::StaticInit,
    );
    let _scope_param = fb.param(0);
    let name = fb.const_string("debugBuild");
    let scope = fb.system_call("Flash.Scope", "findPropStrict", &[name], Type::Unknown);
    let val = fb.const_bool(true);
    fb.set_field(scope, "debugBuild", val);
    fb.ret(None);
    let cinit_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "Settings".into(),
        namespace: vec!["classes".into()],
        struct_index: 0,
        methods: vec![cinit_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![StaticField {
            name: "debugBuild".into(),
            ty: Type::Bool,
            default: None,
            is_const: false,
        }],
        is_interface: false,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();
    assert!(
        out.contains("this.debugBuild = true"),
        "cinit should emit this.field, not bare field:\n{out}"
    );
    assert!(
        !out.contains("Flash_Scope.findPropStrict"),
        "findPropStrict call should be resolved away:\n{out}"
    );
}

#[test]
fn emit_interface_class() {
    let mut mb = ModuleBuilder::new("test");

    mb.add_struct(StructDef {
        name: "IEventListener".into(),
        namespace: Vec::new(),
        fields: vec![],
        visibility: Visibility::Public,
    });

    // Interface constructor (will be skipped).
    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("IEventListener::new", sig, Visibility::Public);
    fb.set_class(Vec::new(), "IEventListener".into(), MethodKind::Constructor);
    fb.ret(None);
    let ctor_id = mb.add_function(fb.build());

    mb.add_class(ClassDef {
        name: "IEventListener".into(),
        namespace: Vec::new(),
        struct_index: 0,
        methods: vec![ctor_id],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: true,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        out.contains("export abstract class IEventListener {"),
        "Interface should emit abstract class:\n{out}"
    );
    assert!(
        !out.contains("constructor("),
        "Interface should not have constructor:\n{out}"
    );
    assert!(
        !out.contains("registerClassTraits("),
        "Interface should not have registerClassTraits:\n{out}"
    );
    assert!(
        out.contains("registerClass(IEventListener)"),
        "Interface should still have registerClass:\n{out}"
    );
}

#[test]
fn emit_class_with_interfaces() {
    let mut mb = ModuleBuilder::new("test");

    // Interface.
    mb.add_struct(StructDef {
        name: "IClickable".into(),
        namespace: Vec::new(),
        fields: vec![],
        visibility: Visibility::Public,
    });
    let sig = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("IClickable::new", sig.clone(), Visibility::Public);
    fb.set_class(Vec::new(), "IClickable".into(), MethodKind::Constructor);
    fb.ret(None);
    let iface_ctor = mb.add_function(fb.build());
    mb.add_class(ClassDef {
        name: "IClickable".into(),
        namespace: Vec::new(),
        struct_index: 0,
        methods: vec![iface_ctor],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: true,
        interfaces: vec![],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    // Implementing class.
    mb.add_struct(StructDef {
        name: "Button".into(),
        namespace: Vec::new(),
        fields: vec![],
        visibility: Visibility::Public,
    });
    let mut fb = FunctionBuilder::new("Button::new", sig, Visibility::Public);
    fb.set_class(Vec::new(), "Button".into(), MethodKind::Constructor);
    fb.ret(None);
    let button_ctor = mb.add_function(fb.build());
    mb.add_class(ClassDef {
        name: "Button".into(),
        namespace: Vec::new(),
        struct_index: 1,
        methods: vec![button_ctor],
        super_class: None,
        visibility: Visibility::Public,
        static_fields: vec![],
        is_interface: false,
        interfaces: vec!["IClickable".into()],
        abstract_members: vec![],
        is_dynamic: false,
        zero_initialized: false,
        needs_index_signature: false,
    });

    let mut module = mb.build();
    let mut diagnostics = Vec::new();
    let out = emit_module_to_string(
        &mut module,
        &LoweringConfig::default(),
        None,
        &DebugConfig::none(),
        &mut diagnostics,
    )
    .unwrap();

    assert!(
        out.contains("registerInterface(Button, IClickable)"),
        "Implementing class should have registerInterface:\n{out}"
    );
}

#[test]
fn type_check_struct_uses_is_type() {
    let out = build_and_emit(|mb| {
        let monster_id = mb.intern_type("Monster");
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let check = fb.type_check(x, Type::Instance(monster_id));
        fb.ret(Some(check));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("isType(v0, Monster)"),
        "TypeCheck with Struct should use isType():\n{out}"
    );
    assert!(
        !out.contains("instanceof"),
        "Should not use instanceof:\n{out}"
    );
}

#[test]
fn cast_struct_uses_as_type() {
    let out = build_and_emit(|mb| {
        let monster_id = mb.intern_type("Monster");
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Instance(monster_id),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let casted = fb.cast(x, Type::Instance(monster_id));
        fb.ret(Some(casted));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("asType(v0, Monster)"),
        "Cast with Struct should use asType():\n{out}"
    );
    assert!(
        !out.contains("as Monster"),
        "Should not use 'as Monster':\n{out}"
    );
}

#[test]
fn coerce_int_emits_int_call() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Int(32),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let coerced = fb.coerce(x, Type::Int(32));
        fb.ret(Some(coerced));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("int(v0)"),
        "Coerce+Int(32) should emit int():\n{out}"
    );
}

#[test]
fn coerce_float_emits_number_call() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let coerced = fb.coerce(x, Type::Float(64));
        fb.ret(Some(coerced));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("Number(v0)"),
        "Coerce+Float(64) should emit Number():\n{out}"
    );
}

#[test]
fn coerce_uint_emits_uint_call() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::UInt(32),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let coerced = fb.coerce(x, Type::UInt(32));
        fb.ret(Some(coerced));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("uint(v0)"),
        "Coerce+UInt(32) should emit uint():\n{out}"
    );
}

#[test]
fn coerce_string_emits_string_call() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::String,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let coerced = fb.coerce(x, Type::String);
        fb.ret(Some(coerced));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("String(v0)"),
        "Coerce+String should emit String():\n{out}"
    );
}

#[test]
fn coerce_bool_emits_boolean_call() {
    let out = build_and_emit(|mb| {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let coerced = fb.coerce(x, Type::Bool);
        fb.ret(Some(coerced));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("Boolean(v0)"),
        "Coerce+Bool should emit Boolean():\n{out}"
    );
}

#[test]
fn coerce_struct_emits_ts_assertion() {
    let out = build_and_emit(|mb| {
        let monster_id = mb.intern_type("Monster");
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Instance(monster_id),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        let coerced = fb.coerce(x, Type::Instance(monster_id));
        fb.ret(Some(coerced));
        mb.add_function(fb.build());
    });

    assert!(
        out.contains("as Monster"),
        "Coerce+Struct should emit TS assertion:\n{out}"
    );
    assert!(
        !out.contains("asType"),
        "Coerce+Struct should NOT use asType():\n{out}"
    );
}

#[test]
fn redundant_astype_eliminated() {
    // When value is already typed as the target, Cast should be eliminated.
    let out = build_and_emit(|mb| {
        let monster_id = mb.intern_type("Monster");
        let sig = FunctionSig {
            params: vec![Type::Instance(monster_id)],
            return_ty: Type::Instance(monster_id),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Public);
        let x = fb.param(0);
        // Cast to same type — should be eliminated by linear lowering.
        let casted = fb.cast(x, Type::Instance(monster_id));
        fb.ret(Some(casted));
        mb.add_function(fb.build());
    });

    assert!(
        !out.contains("asType"),
        "Redundant asType should be eliminated:\n{out}"
    );
}
