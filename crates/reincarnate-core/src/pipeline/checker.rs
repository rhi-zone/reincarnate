use std::path::PathBuf;

use crate::error::CoreError;

/// A single diagnostic from a language-level type checker.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Diagnostic {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub code: DiagnosticCode,
    pub severity: Severity,
    pub message: String,
}

/// Diagnostic code — either a pipeline-internal code or an external checker code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DiagnosticCode {
    /// Reincarnate pipeline diagnostic.
    Rc(RcDiagnostic),
    /// External checker code (e.g. "TS2304" from tsc).
    External(String),
}

impl std::fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiagnosticCode::Rc(rc) => write!(f, "{rc}"),
            DiagnosticCode::External(s) => write!(f, "{s}"),
        }
    }
}

/// Pipeline-internal diagnostic codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RcDiagnostic {
    /// Duplicate case value in switch or sequential if-chain.
    DuplicateCase,
    /// Duplicate key in object literal.
    DuplicateObjectKey,
    /// Function in `function_modules` without a `function_signatures` entry.
    MissingFunctionSignature,
    /// Global variable has conflicting concrete write-site types.
    ///
    /// Fired when write-site type inference observes two or more distinct
    /// concrete types being stored into the same variable (e.g. both
    /// `Array(Dynamic)` and `String`).  This is a game-author bug: the
    /// same variable is used to hold values of incompatible types.
    WriteConflict,
}

impl std::fmt::Display for RcDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            RcDiagnostic::DuplicateCase => "RC0001",
            RcDiagnostic::DuplicateObjectKey => "RC0002",
            RcDiagnostic::MissingFunctionSignature => "RC0003",
            RcDiagnostic::WriteConflict => "RC0004",
        };
        write!(f, "{code}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Severity {
    Error,
    Warning,
}

/// Input to a checker — the output directory to typecheck.
pub struct CheckerInput {
    pub output_dir: PathBuf,
}

/// Output from a checker.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckerOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub summary: CheckSummary,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckSummary {
    pub output_dir: String,
    pub total_errors: usize,
    pub total_warnings: usize,
    /// Error codes sorted by count descending.
    pub by_code: Vec<(DiagnosticCode, usize)>,
    /// Unique messages sorted by count descending: (message, code, count).
    #[serde(default)]
    pub by_message: Vec<(String, DiagnosticCode, usize)>,
}

/// Checker trait — validates emitted code using an external type checker.
pub trait Checker {
    fn name(&self) -> &str;
    fn check(&self, input: CheckerInput) -> Result<CheckerOutput, CoreError>;
}
