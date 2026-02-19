# Visual Basic 6

**Status: Planned** — No implementation started.

## Format

VB6 applications are compiled into Windows `.exe` or `.dll` files in one of two modes:
- **P-Code** (pseudo-code) — compiled to VB6's proprietary bytecode VM; the default and most common
- **Native code** — compiled to x86 machine code via MASM; much harder to decompile

The P-Code VM is well-documented through reverse engineering. The executable contains:
- A PE header with the standard Windows executable structure
- A VB6-specific header (`VBHeader`) with project name, threading model, and control references
- P-Code bytecode in code segments, one per procedure
- String and type information tables
- COM object references (for controls: `VBForm`, `TextBox`, `CommandButton`, etc.)
- Resource data (icons, bitmaps, dialogs)

## Lifting Strategy

Full recompilation for P-Code (Tier 2). Native-code VB6 requires native decompilation (much harder, lower priority).

1. Parse the PE file and locate the VB6-specific header
2. Walk the project structure to find all forms, modules, and class modules
3. Decode P-Code bytecode per procedure
4. Identify VB6 runtime boundaries (`MSVBVM60.DLL` function imports)

VB6 is event-driven: forms have event handlers (`Form_Load`, `Button1_Click`, etc.) and properties. The object model maps cleanly to TypeScript classes.

## What Needs Building

### Format Parser (new crate: `reincarnate-frontend-vb6`)

- [ ] PE header parser (or reuse an existing crate)
- [ ] VB6 header (`VBHeader`, `EXEPROJECTINFO`) parser
- [ ] Project structure walker: forms, standard modules, class modules
- [ ] P-Code bytecode decoder — the P-Code instruction set is documented in community research (VB Decompiler, p-code research papers)
- [ ] Form description extractor (control layout, property values, event bindings)
- [ ] Type library references (for external COM control types)

### P-Code Instruction Set

VB6 P-Code is a stack-based VM with ~200 opcodes. Key categories:
- Load/store (local variables, module-level variables, properties)
- Arithmetic (integer, single, double, currency, date)
- String operations
- Comparison and logical
- Control flow (branch, call, return)
- Object operations (COM dispatch: `CreateObject`, `Set`, `Nothing`, `Is`)
- Collection operations (`For Each`, `With`)
- Error handling (`On Error GoTo`, `Resume`)
- Array operations (fixed-size and dynamic `ReDim`)
- I/O operations (file-based: `Open`, `Print #`, `Input #`, `Write #`, etc.)
- UI operations (form/control method calls via COM dispatch)

### IR Mapping

- VB6 procedure → IR function
- `Form_Load` / `Button1_Click` → event handler functions
- COM dispatch on controls → `SystemCall("VB6.Control", methodName, args)`
- `MsgBox` → `SystemCall("VB6.MsgBox", text, buttons, title)`
- `InputBox` → `SystemCall("VB6.InputBox", prompt, title, default)` + `Yield`
- File I/O → `SystemCall("VB6.File.Open", ...)` etc.
- `DoEvents` → `Yield` (yield to event loop)
- `Timer` function → `SystemCall("VB6.Timer")`
- `Err` object → error state in function context

### Replacement Runtime (`runtime/vb6/ts/`)

VB6's object model is COM-based. The replacement runtime needs to model:
- [ ] `Form` — window with title bar, resize, minimize, maximize, close
- [ ] `Label` — text display
- [ ] `TextBox` — single/multiline text input
- [ ] `CommandButton` — button with click event
- [ ] `CheckBox` / `OptionButton` — toggle/radio
- [ ] `ComboBox` / `ListBox` — dropdown/list selection
- [ ] `Frame` / `PictureBox` / `Image` — containers and image display
- [ ] `Timer` control — recurring interval events
- [ ] `CommonDialog` — file open/save dialog
- [ ] `MsgBox` / `InputBox` — modal dialog functions
- [ ] `Printer` object — print output (→ browser print API)
- [ ] File I/O (`Open`, `Close`, `Print #`, `Input #`, `Line Input #`, `Get`, `Put`)
- [ ] Registry access (`GetSetting`, `SaveSetting`) → localStorage
- [ ] Clipboard (`Clipboard.GetText`, `Clipboard.SetText`) → navigator.clipboard
- [ ] `Date` / `Time` / `Now` functions
- [ ] `Shell` function — launch external program (→ web Worker or stub)

## Known Challenges

- **Native code VB6** — If compiled with `/native`, the bytecode is x86. This is outside the Tier 2 scope; it would require a native decompiler (Ghidra, IDA). Much rarer in practice than P-Code.
- **COM controls** — VB6 apps heavily use ActiveX controls (MSCOMCTL.OCX, Sheridan, Infragistics, etc.). The commonly used controls can be shimmed; obscure third-party ActiveX controls may be impossible to replicate.
- **Database access** — Many VB6 apps use ADO/DAO for database access. These would map to IndexedDB or a backend API.
- **MDI forms** — Multiple Document Interface forms (parent + child windows) have no direct web equivalent; need a window manager shim.
- **VB6 string type** — VB6 `String` is a BSTR (COM string, length-prefixed, Unicode). Arithmetic on strings follows VB coercion rules, not JavaScript rules.

## References

- [VB Decompiler](https://www.vb-decompiler.org/) — commercial P-Code decompiler
- [p-code research (Andrea Cini)](https://web.archive.org/web/*/http://www.dotnetspider.com/resources/p-code.aspx)
- [Visual Basic 6.0 Language Reference](https://docs.microsoft.com/en-us/previous-versions/visualstudio/visual-basic-6/)
