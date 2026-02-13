/// Data type for instruction operands (4-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DataType {
    Double = 0x0,
    Float = 0x1,
    Int32 = 0x2,
    Int64 = 0x3,
    Bool = 0x4,
    Variable = 0x5,
    String = 0x6,
    // 0x7..0xE unused
    Int16 = 0xF,
}

impl DataType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x0 => Some(Self::Double),
            0x1 => Some(Self::Float),
            0x2 => Some(Self::Int32),
            0x3 => Some(Self::Int64),
            0x4 => Some(Self::Bool),
            0x5 => Some(Self::Variable),
            0x6 => Some(Self::String),
            0xF => Some(Self::Int16),
            _ => None,
        }
    }
}

/// Comparison kind for Cmp instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ComparisonKind {
    Less = 1,
    LessEqual = 2,
    Equal = 3,
    NotEqual = 4,
    GreaterEqual = 5,
    Greater = 6,
}

impl ComparisonKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Less),
            2 => Some(Self::LessEqual),
            3 => Some(Self::Equal),
            4 => Some(Self::NotEqual),
            5 => Some(Self::GreaterEqual),
            6 => Some(Self::Greater),
            _ => None,
        }
    }
}

/// Instance type for variable access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum InstanceType {
    /// Current instance (`self`).
    Own = -1,
    /// Other instance in collision event.
    Other = -2,
    /// All instances.
    All = -3,
    /// No instance (object reference follows).
    Noone = -4,
    /// Global scope.
    Global = -5,
    /// Built-in variable.
    Builtin = -6,
    /// Local scope.
    Local = -7,
    /// Stack-top instance (GMS2).
    Stacktop = -9,
    /// Static variable (GMS2.3+).
    Static = -15,
    /// Argument variable.
    Arg = -16,
}

impl InstanceType {
    pub fn from_i16(v: i16) -> Option<Self> {
        match v {
            -1 => Some(Self::Own),
            -2 => Some(Self::Other),
            -3 => Some(Self::All),
            -4 => Some(Self::Noone),
            -5 => Some(Self::Global),
            -6 => Some(Self::Builtin),
            -7 => Some(Self::Local),
            -9 => Some(Self::Stacktop),
            -15 => Some(Self::Static),
            -16 => Some(Self::Arg),
            _ => None,
        }
    }
}

/// Variable reference in bytecode.
///
/// The second word of a variable instruction encodes both the variable ID
/// and a linked-list pointer for patching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariableRef {
    /// Variable index (into VARI chunk).
    pub variable_id: u32,
    /// Reference type bits.
    pub ref_type: u8,
}

impl VariableRef {
    pub fn from_raw(raw: u32) -> Self {
        Self {
            variable_id: raw & 0x00FF_FFFF,
            ref_type: ((raw >> 24) & 0xF8) as u8,
        }
    }
}
