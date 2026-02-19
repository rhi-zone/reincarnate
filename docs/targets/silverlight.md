# Silverlight (.NET IL)

**Status: Planned** — No implementation started.

## Format

Silverlight applications are distributed as `.xap` files — ZIP archives containing:
- One or more `.dll` assemblies (PE files containing .NET IL bytecode)
- `AppManifest.xaml` — entry point class and assembly references
- XAML files — UI description in XML
- Resource files — images, fonts, audio

.NET IL (Intermediate Language / CIL) is thoroughly documented (ECMA-335 standard). Decompilers (ILSpy, dnSpy, Mono.Cecil) can recover C# or VB.NET source code from IL with high fidelity.

Silverlight 4/5 (the last versions) targeted .NET 4 with a reduced API surface — mostly the XAML UI framework, WCF networking, and media playback.

## Lifting Strategy

Full recompilation (Tier 2).

.NET IL is arguably the easiest bytecode to lift:
- Fully typed (generics, interfaces, value types, delegates)
- Rich metadata (attribute system, reflection)
- Clean class hierarchy
- Well-documented at the instruction level

1. Unpack the `.xap`
2. Parse `.dll` assemblies using the PE format + CLI metadata tables
3. Decode IL bytecode per method
4. Emit IR with full type information from metadata
5. Identify Silverlight framework API boundaries

## What Needs Building

### Format Parser (new crate: `reincarnate-frontend-silverlight`)

- [ ] ZIP/XAP extractor
- [ ] PE/CLI assembly parser:
  - [ ] PE header + CLI header
  - [ ] Metadata tables (Module, TypeRef, TypeDef, Field, Method, Param, MemberRef, etc.)
  - [ ] Blob heap, String heap, GUID heap, UserString heap
  - [ ] IL bytecode decoder — ~200 opcodes
  - [ ] Generics (TypeSpec, MethodSpec)
  - [ ] Custom attributes
  - [ ] Resources (embedded + linked)
- [ ] XAML parser — XML-based UI description
  - [ ] Control tree extraction
  - [ ] Binding expressions (`{Binding Path=...}`)
  - [ ] Resource dictionaries
  - [ ] Styles and templates

### IL Instruction Set

The IL opcode set is clean and well-organized:
- Stack ops: `ldarg`, `ldloc`, `stloc`, `ldc.*`, `ldnull`, `dup`, `pop`
- Arithmetic: `add`, `sub`, `mul`, `div`, `rem`, all with `.ovf` and `.un` variants
- Comparison: `ceq`, `cgt`, `clt` with unsigned variants
- Branch: `br`, `brfalse`, `brtrue`, `beq`, `bge`, `bgt`, `ble`, `blt`, `bne.un`, `switch`
- Object: `newobj`, `ldobj`, `stobj`, `cpobj`, `initobj`, `box`, `unbox`, `unbox.any`, `castclass`, `isinst`
- Field access: `ldfld`, `ldflda`, `stfld`, `ldsfld`, `ldsflda`, `stsfld`
- Method calls: `call`, `callvirt`, `calli`, `tail.`
- Delegates: `ldftn`, `ldvirtftn`, `newobj` (delegate constructor)
- Arrays: `newarr`, `ldelem`, `stelem`, `ldelema`, `ldlen`
- Exception: `throw`, `rethrow`, `leave`, `.try` / `.catch` / `.finally` / `.fault`

### IR Mapping

- IL method → IR function
- `callvirt` → virtual dispatch via `Call` on self
- `call` → static/non-virtual `Call`
- `ldftn` / `ldvirtftn` + `newobj (delegate ctor)` → closure / function reference
- `box` / `unbox` → IR cast operations
- `throw` / `try` / `catch` → exception handlers (blocked on IR try/catch support)
- `initobj` → zero-initialize struct fields
- LINQ (`System.Linq`) → map/filter/reduce over collections

### Replacement Runtime (`runtime/silverlight/ts/`)

Silverlight's primary value proposition was its XAML UI framework and media playback. Key APIs:

**XAML / UI Framework:**
- [ ] `Application` — entry point, resource dictionaries, lifecycle
- [ ] `UserControl` / `Page` — root containers
- [ ] Layout: `Grid`, `StackPanel`, `Canvas`, `DockPanel`, `WrapPanel`
- [ ] Controls: `Button`, `TextBlock`, `TextBox`, `CheckBox`, `RadioButton`, `ComboBox`, `ListBox`, `Slider`, `ProgressBar`, `ScrollViewer`
- [ ] `Image` control — load from URL, stretch modes
- [ ] `MediaElement` — audio/video playback
- [ ] Data binding: `{Binding}`, `INotifyPropertyChanged`, `INotifyCollectionChanged`, `ObservableCollection<T>`
- [ ] XAML resources and styles: `ResourceDictionary`, `Style`, `ControlTemplate`
- [ ] Animations: `Storyboard`, `DoubleAnimation`, `ColorAnimation`, `Timeline`
- [ ] `Canvas` drawing (direct DrawingContext API, rarely used compared to XAML)

**Networking:**
- [ ] `WebClient.DownloadStringAsync`, `DownloadStringCompleted`
- [ ] `HttpWebRequest` / `HttpWebResponse`
- [ ] WCF client proxies (require server-side WCF endpoint to be present or mocked)

**Other:**
- [ ] `IsolatedStorage` — client-side file storage → IndexedDB
- [ ] `Clipboard` — text copy/paste
- [ ] `System.Windows.Browser.HtmlPage` — JS interop

## Known Challenges

- **XAML templating** — Silverlight's control template system is rich; faithfully reimplementing it means building a mini XAML runtime or mapping templates to React/Vue/DOM components
- **Data binding** — The `{Binding}` system with two-way bindings, value converters, and change notification is a significant sub-system
- **WCF data services** — Many Silverlight apps call server-side WCF endpoints; these services are not part of the client code
- **Generics** — .NET generics are reified (retained at runtime); TypeScript generics are erased. For collections this is fine; for reflection-heavy generic code it may require `Dynamic` fallback
- **P/Invoke / Native interop** — Silverlight allowed limited unmanaged code access on trust-elevated apps; these cannot be lifted

## References

- [ECMA-335 CLI Standard](https://www.ecma-international.org/publications-and-standards/standards/ecma-335/)
- [ILSpy decompiler](https://github.com/icsharpcode/ILSpy)
- [dnSpy decompiler/debugger](https://github.com/dnSpy/dnSpy)
- [Mono.Cecil IL manipulation library](https://github.com/jbevain/cecil)
- [Silverlight 5 API Reference (archived)](https://learn.microsoft.com/en-us/previous-versions/windows/silverlight/dotnet-windows-silverlight/)
