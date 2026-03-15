use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;

/// Argument/return type for an extension function parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtArgType {
    /// String argument (type tag 1).
    String,
    /// Real/double argument (type tag 2 or any other value).
    Real,
}

impl ExtArgType {
    fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::String,
            _ => Self::Real,
        }
    }
}

/// A single exported function from an extension include file.
#[derive(Debug)]
pub struct ExtFunction {
    /// GML-side name of the function (e.g. `FS_unique_fname`).
    pub name: StringRef,
    /// External (native) symbol name (e.g. `find_unique_fname`).
    pub external_name: StringRef,
    /// Return type.
    pub return_type: ExtArgType,
    /// Argument types (length = arity).
    pub args: Vec<ExtArgType>,
}

/// A single include file entry within an extension.
#[derive(Debug)]
pub struct ExtInclude {
    /// Filename (e.g. `GMResource.dll`).
    pub filename: StringRef,
    /// Init function name (called at extension load).
    pub init_fn: StringRef,
    /// Cleanup function name (called at extension unload).
    pub final_fn: StringRef,
    /// Include-file kind: 1=DLL, 2=GML placeholder, others.
    pub kind: u32,
    /// Exported functions from this include file.
    pub functions: Vec<ExtFunction>,
}

/// A single extension (one entry in the EXTN chunk pointer list).
#[derive(Debug)]
pub struct ExtEntry {
    /// Extension name (e.g. `GMFileSystem`).
    pub name: StringRef,
    /// Class name (typically empty).
    pub class_name: StringRef,
    /// Include files (DLLs or GML placeholder files).
    pub includes: Vec<ExtInclude>,
}

/// Parsed EXTN chunk.
#[derive(Debug)]
pub struct Extn {
    pub extensions: Vec<ExtEntry>,
}

impl Extn {
    /// Parse the EXTN chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte chunk header).
    /// `data` is the full file data for resolving string references.
    pub fn parse(chunk_data: &[u8], data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let ext_ptrs = c.read_pointer_list()?;

        let mut extensions = Vec::with_capacity(ext_ptrs.len());
        for ext_ptr in ext_ptrs {
            let ext = parse_extension(ext_ptr as usize, data)?;
            extensions.push(ext);
        }

        Ok(Self { extensions })
    }

    /// Iterate over all exported extension functions across all extensions and
    /// include files.
    pub fn all_functions(&self) -> impl Iterator<Item = &ExtFunction> {
        self.extensions
            .iter()
            .flat_map(|e| e.includes.iter())
            .flat_map(|i| i.functions.iter())
    }
}

fn parse_extension(p: usize, data: &[u8]) -> Result<ExtEntry> {
    let mut c = Cursor::new(data);
    c.seek(p);

    let _empty_str = StringRef(c.read_u32()?); // always "" — skip
    let name = StringRef(c.read_u32()?);
    let class_name = StringRef(c.read_u32()?);
    let include_count = c.read_u32()? as usize;

    // Include pointer list: N consecutive u32 pointers immediately following.
    let mut inc_ptrs = Vec::with_capacity(include_count);
    for _ in 0..include_count {
        inc_ptrs.push(c.read_u32()? as usize);
    }

    let mut includes = Vec::with_capacity(include_count);
    for inc_p in inc_ptrs {
        includes.push(parse_include(inc_p, data)?);
    }

    Ok(ExtEntry {
        name,
        class_name,
        includes,
    })
}

fn parse_include(p: usize, data: &[u8]) -> Result<ExtInclude> {
    let mut c = Cursor::new(data);
    c.seek(p);

    let filename = StringRef(c.read_u32()?);
    let final_fn = StringRef(c.read_u32()?);
    let init_fn = StringRef(c.read_u32()?);
    let kind = c.read_u32()?;
    let func_count = c.read_u32()? as usize;

    // Function pointer list: N consecutive u32 pointers immediately following.
    let mut func_ptrs = Vec::with_capacity(func_count);
    for _ in 0..func_count {
        func_ptrs.push(c.read_u32()? as usize);
    }

    let mut functions = Vec::with_capacity(func_count);
    for func_p in func_ptrs {
        functions.push(parse_function(func_p, data)?);
    }

    Ok(ExtInclude {
        filename,
        init_fn,
        final_fn,
        kind,
        functions,
    })
}

fn parse_function(p: usize, data: &[u8]) -> Result<ExtFunction> {
    let mut c = Cursor::new(data);
    c.seek(p);

    let name = StringRef(c.read_u32()?);
    let _function_id = c.read_i32()?;
    let _kind = c.read_i32()?;
    let return_type = ExtArgType::from_i32(c.read_i32()?);
    let external_name = StringRef(c.read_u32()?);
    let arg_count = c.read_u32()? as usize;

    let mut args = Vec::with_capacity(arg_count);
    for _ in 0..arg_count {
        args.push(ExtArgType::from_i32(c.read_i32()?));
    }

    Ok(ExtFunction {
        name,
        external_name,
        return_type,
        args,
    })
}
