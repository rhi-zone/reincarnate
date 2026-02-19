# Java Applets

**Status: Planned** — No implementation started.

## Format

Java Applets are distributed as `.jar` files (ZIP archives containing `.class` files) or as individual `.class` files. The format is:
- **JVM bytecode** — standardized, exhaustively documented in the JVM specification
- **Class files** — magic `0xCAFEBABE`, constant pool, access flags, field/method descriptors
- **Jar manifest** — `META-INF/MANIFEST.MF` specifying the main class and applet class

Java bytecode is one of the best-documented and most-reversible formats in existence. Mature open-source decompilers (CFR, Fernflower, Procyon) can recover Java source that closely matches the original.

## Lifting Strategy

Full recompilation (Tier 2).

Java bytecode → IR is highly practical:
- JVM bytecode has a clean type system (all types annotated in the constant pool and descriptors)
- The class hierarchy maps directly to TypeScript/Rust class hierarchies
- Generics information is available in class signatures (Java 5+)
- Method signatures are fully typed

1. Unpack the `.jar` and parse `.class` files
2. Decode JVM bytecode per method using the JVM specification
3. Emit IR with full type information from class/method descriptors
4. Identify `java.applet.*`, `java.awt.*`, and `javax.swing.*` API boundaries

## What Needs Building

### Format Parser (new crate: `reincarnate-frontend-java`)

- [ ] JAR/ZIP extractor
- [ ] Class file parser:
  - [ ] Constant pool (UTF8, Class, FieldRef, MethodRef, InterfaceMethodRef, String, Integer, Float, Long, Double, NameAndType, MethodHandle, MethodType, Dynamic, InvokeDynamic, Module, Package)
  - [ ] Field descriptors (type signatures)
  - [ ] Method descriptors (parameter + return types)
  - [ ] JVM bytecode decoder — ~200 opcodes, well-specified
  - [ ] Stack map frames (Java 6+ for type verification)
  - [ ] Exception table
  - [ ] Annotations (`RuntimeVisibleAnnotations`, `RuntimeInvisibleAnnotations`)
  - [ ] Generics signature (`Signature` attribute)

### JVM Instruction Set Categories

- Load/store: `iload`, `aload`, `istore`, `astore`, array variants, field access (`getfield`, `putfield`, `getstatic`, `putstatic`)
- Arithmetic: `iadd`, `isub`, `imul`, `idiv`, `irem`, etc. for int/long/float/double
- Conversions: `i2l`, `l2i`, `i2f`, `f2i`, `i2b`, `i2c`, `i2s`, etc.
- Comparison: `if_icmpeq`, `if_acmpeq`, `ifnull`, `ifnonnull`, `lcmp`, `fcmpl`, `dcmpg`, etc.
- Control: `goto`, `jsr`/`ret` (subroutines, Java <6), `tableswitch`, `lookupswitch`, `return` variants
- Invocation: `invokevirtual`, `invokeinterface`, `invokestatic`, `invokespecial`, `invokedynamic` (Java 7+)
- Object: `new`, `newarray`, `anewarray`, `multianewarray`, `instanceof`, `checkcast`
- Exception: `athrow`, `monitorenter`, `monitorexit`

### IR Mapping

- Java method → IR function
- Java `instanceof` / `checkcast` → IR type check / cast operations
- `try`/`catch`/`finally` → IR exception handlers (blocked on IR try/catch support)
- `invokevirtual` → IR `Call` on `self` with virtual dispatch
- `invokestatic` → IR direct `Call`
- `invokedynamic` → IR indirect call (lambda desugaring — Java 8+)
- `synchronized` → no-op in single-threaded web environment

### Replacement Runtime (`runtime/java-applets/ts/`)

Java Applets target `java.applet.Applet` which is a subclass of `java.awt.Panel`. The AWT/Swing API surface needed:

**Core AWT (required for applets):**
- [ ] `Applet` — lifecycle: `init()`, `start()`, `stop()`, `destroy()`, `paint(Graphics)`
- [ ] `Graphics` / `Graphics2D` — drawing primitives
- [ ] `Component` — paint, repaint, event handling, preferred size
- [ ] `Container` — add/remove components, layout managers
- [ ] `Panel`, `Frame`, `Canvas`
- [ ] Layout managers: `FlowLayout`, `BorderLayout`, `GridLayout`
- [ ] Event model: `ActionListener`, `MouseListener`, `KeyListener`, `FocusListener`
- [ ] `Font`, `FontMetrics`
- [ ] `Color`, `Image`, `MediaTracker`
- [ ] `AudioClip`

**Common Swing (for Swing applets):**
- [ ] `JPanel`, `JLabel`, `JButton`, `JTextField`, `JTextArea`, `JComboBox`, `JList`
- [ ] `JScrollPane`, `JTabbedPane`, `JSplitPane`
- [ ] `JDialog`, `JOptionPane`
- [ ] `SwingUtilities.invokeLater` → `setTimeout(fn, 0)`
- [ ] Swing look-and-feel (Metal L&F is default; approximate with CSS)

**Java standard library stubs:**
- [ ] `java.lang` — String, StringBuilder, Math, Integer, Double, etc.
- [ ] `java.util` — ArrayList, HashMap, Collections, Arrays, Random, Date, Calendar
- [ ] `java.io` — streams, readers, writers (mapped to fetch/Blob/ArrayBuffer)
- [ ] `java.net.URL` — `openStream()` → fetch

## Known Challenges

- **Threading** — Java applets often use threads (`Thread`, `Runnable`, `synchronized`). JavaScript is single-threaded. Lightweight threading (cooperative coroutines via state machines) can handle simple cases; heavy multi-threading requires Web Workers.
- **AWT/Swing size** — The full AWT+Swing API is enormous. Most applets use a small subset; coverage can grow organically from test cases.
- **`invokedynamic` (Java 8+)** — Lambda expressions and method references use invokedynamic with BootstrapMethods. These need special handling to recover the original lambda.
- **Reflection** — `Class.forName()`, `getDeclaredMethod()`, etc. break static analysis.
- **Serialization** — `java.io.Serializable` applets that persist state require a binary serialization format.

## References

- [JVM Specification](https://docs.oracle.com/javase/specs/jvms/se21/html/)
- [CFR Java Decompiler](https://github.com/leibnitz27/cfr)
- [ASM Bytecode Library](https://asm.ow2.io/)
- [Java Applet API (archived)](https://docs.oracle.com/javase/8/docs/api/java/applet/package-summary.html)
