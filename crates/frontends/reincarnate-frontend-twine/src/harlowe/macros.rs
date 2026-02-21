//! Harlowe macro classification.
//!
//! Classifies macros by kind to guide parsing: changers attach hooks,
//! commands are standalone, control flow creates branches, and value
//! macros return data.

/// The kind of a Harlowe macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroKind {
    /// Changers modify the presentation of their attached hook:
    /// `(color:)`, `(text-style:)`, `(font:)`, `(transition:)`, etc.
    Changer,
    /// Commands perform actions: `(set:)`, `(goto:)`, `(display:)`, `(print:)`.
    Command,
    /// Control flow macros create branches: `(if:)`, `(else-if:)`, `(else:)`, `(unless:)`.
    ControlFlow,
    /// Value macros return data: `(str:)`, `(num:)`, `(random:)`, `(a:)`, `(dm:)`.
    Value,
}

/// Normalize a Harlowe macro name.
///
/// Harlowe macro names are case-, dash-, and underscore-insensitive:
/// `(Go-To:)`, `(goto:)`, `(GOTO:)`, `(Go_To:)` are all equivalent.
/// Normalize by lowercasing and stripping all non-alphanumeric characters.
pub fn normalize_macro_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Classify a macro name into its kind.
///
/// The name must already be normalized via `normalize_macro_name`.
pub fn macro_kind(name: &str) -> MacroKind {
    match name {
        // Control flow
        "if" | "elseif" | "else" | "unless" | "for" | "loop" => MacroKind::ControlFlow,

        // Commands
        "set" | "put" | "move" | "goto" | "display" | "print" | "savegame"
        | "loadgame" | "alert" | "prompt" | "confirm" | "stop"
        | "replace" | "append" | "prepend" | "show" | "hide" | "rerun" | "redo" | "redirect"
        | "linkgoto" | "linkundo" | "linkreveal" | "linkrepeat"
        | "linkrevealgoto" | "linkrerun" | "linkreplace" | "linkfullscreen"
        | "click" | "clickreplace" | "clickappend"
        | "clickprepend" | "clickrerun" | "cyclinglink" | "seqlink" | "animate"
        | "gotourl" | "openurl" | "undo" | "restart" | "reload" | "scroll"
        | "after" => MacroKind::Command,

        // Third-party Border for Harlowe (b4r) library changers
        "b4r" | "b4rcolour" | "b4rcolor" => MacroKind::Changer,

        // Changers — includes t8n* aliases and transition-delay/text-rotate aliases
        "color" | "colour" | "textcolour" | "textcolor" | "textstyle" | "font" | "align"
        | "transition" | "t8n" | "transitiontime" | "t8ntime" | "transitionarrive" | "t8narrive"
        | "transitiondepart" | "t8ndepart" | "transitiondelay" | "t8ndelay"
        | "transitionskip" | "t8nskip"
        | "textrotatez" | "textrotatex" | "textrotatey" | "textrotate"
        | "hoverstyle" | "css" | "background" | "bg" | "box" | "floatbox" | "charstyle"
        | "linestyle" | "pagestyle" | "opacity" | "textindent" | "textsize" | "size"
        | "collapse" | "nobr" | "verbatim" | "hidden"
        | "action" => MacroKind::Changer,

        // Value macros
        "str" | "string" | "text" | "num" | "number" | "a" | "array" | "dm" | "datamap" | "ds"
        | "dataset" | "random" | "either" | "round" | "floor" | "ceil" | "abs" | "min"
        | "max" | "pow" | "sqrt" | "sin" | "cos" | "tan" | "exp" | "log" | "log10" | "log2"
        | "sign" | "clamp" | "lerp" | "sorted" | "sortedby" | "reversed" | "rotated"
        | "rotatedto" | "shuffled"
        | "interlaced" | "folded" | "altered" | "count" | "range" | "repeated" | "joined"
        | "somepass" | "allpass" | "nonepass" | "find" | "lowercase" | "uppercase"
        | "upperfirst" | "lowerfirst" | "split" | "splitted" | "unique"
        | "strfind" | "stringfind" | "strnth" | "stringnth" | "strrepeated" | "stringrepeated"
        | "strreplaced" | "stringreplaced" | "replaced" | "strreversed" | "stringreversed"
        | "trimmed" | "words" | "digitformat" | "plural" | "trunc"
        | "cond" | "nth" | "substring" | "subarray" | "bit" | "rgb" | "rgba" | "hsl"
        | "hsla" | "gradient" | "lch" | "lcha" | "complement" | "mix"
        | "weekday" | "monthday" | "monthname" | "yearday" | "currentdate" | "currenttime"
        | "savedgames" | "passage" | "passages" | "visited" | "visits" | "turns" | "history"
        | "hook" | "hooksnamed" | "source" | "datanames" | "datavalues" | "dataentries"
        | "dmnames" | "dmvalues" | "dmentries" | "dmaltered" | "datamapaltered"
        | "pass" | "permutations"
        | "v6" | "v8" | "metadata"
        | "macro" | "partial" | "bind" | "bind2bind" | "2bind"
        | "openstorylets" | "storyletsof"
        // Custom macro return values — `(output:)` and `(output-data:)` appear inside
        // `(macro:)` bodies and act as return statements.
        | "output" | "outputdata" => MacroKind::Value,

        // HAL (Harlowe Audio Library) — third-party audio macros
        "track" | "masteraudio" | "newtrack" | "newplaylist" | "newgroup"
        | "playlist" | "group" => MacroKind::Command,

        // Layout / interactive / state commands
        "columns" | "column" | "enchant" | "enchantin" | "forgetundos"
        | "forgetvisits" | "ignore" => MacroKind::Command,

        // Storylet system (Harlowe 3.3+)
        "storylet" | "exclusivity" | "iconundo" | "iconredo" | "iconrestart" => MacroKind::Command,

        // Live is command-like (attaches hook for timed behavior)
        "live" | "event" | "meter" | "dialog" | "dropdown" | "input" | "inputbox"
        | "checkbox" | "radiobutton" | "forcecheckbox" | "forcedropdown"
        | "forceinput" => MacroKind::Command,

        // Unknown macros default to Command (standalone, no hook)
        _ => MacroKind::Command,
    }
}

/// Check if a macro is a clause of an if-chain (`else-if`, `else`).
///
/// The name must already be normalized via `normalize_macro_name`.
pub fn is_if_clause(name: &str) -> bool {
    matches!(name, "elseif" | "else")
}

/// Check if a macro typically attaches a hook.
///
/// The name must already be normalized via `normalize_macro_name`.
pub fn expects_hook(name: &str) -> bool {
    let kind = macro_kind(name);
    matches!(kind, MacroKind::Changer | MacroKind::ControlFlow)
        || matches!(
            name,
            "link"
                | "linkgoto"
                | "linkreveal"
                | "linkrevealgoto"
                | "linkrepeat"
                | "linkrerun"
                | "linkreplace"
                | "click"
                | "clickreplace"
                | "clickappend"
                | "clickprepend"
                | "clickrerun"
                | "live"
                | "event"
                | "after"
                | "for"
                | "dialog"
                | "columns"
        )
}
