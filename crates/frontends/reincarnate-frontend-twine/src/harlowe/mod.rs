//! Harlowe story format parser and IR lowering.
//!
//! Harlowe uses a hook-based macro syntax with `(macro:)` calls and
//! `[hook]` content blocks. Its expression language is distinct from
//! JavaScript — it uses `is`, `is not`, `contains`, `is in`, etc.
