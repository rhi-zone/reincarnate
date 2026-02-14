//! SugarCube story format parser and IR lowering.
//!
//! SugarCube (v2.x) uses a macro DSL with `<<macro>>` syntax, TwineScript
//! expressions (a JS superset with `is`, `isnot`, `to`, `not` keywords),
//! and `[[link|passage]]` shorthand navigation.
