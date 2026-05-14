# ino_color

An output coloring crate with limited functionality, limit platform support
and zero allocations. It's perfect for scratching tiny spot of itch.

## Basic Usage

```rust
use ino_color::cprintln;
use ino_color::fg;
use ino_color::style;

// Foreground color only
cprintln!(fg::Yellow, "Hello Fancy");

// Foreground + style
cprintln!((fg::Blue, style::Italic), "Savoy blue");

// Foreground + background + style
cprintln!((fg::Green, bg::Black, style::Bold), "Green on black, bold");

// All format traits work as expected
cprintln!(fg::Green, "{:?}", vec![123]);
cprintln!(fg::Green, "{:X}", 123);
```

## Pros & Cons

### Pros

- Good at scratching itch.
- Zero allocations — writes directly to the stream.
- Follows [`FORCE_COLOR`](https://force-color.org/) and
  the [_Standard for ANSI Colors in Terminals_](https://bixense.com/clicolors/) by default.
- Doesn't pollute the LSP completion with dozens of methods named after colors.
- Stream-aware: `cprint!`/`cprintln!` check stdout,
  `ceprint!`/`ceprintln!` check stderr.

### Cons

- Linux only.
  - However, it doesn't use platform specific API, so it might also works on Darwin and modern Windows
    as long as the terminal emulator speaks ANSI SGR.

- Only supports 16 named (4-bit) colors.
  - Support for 8-bit color and true color isn't on the roadmap.

- All color and style selections are done in **type level**, meaning coloring can't be changed at runtime.
  - Such APIs will not be added in the near future.
  - Blame `owo-colors` for inventing this API, explained next section.

- No per-value coloring inside a format string.
  - The macros color the entire output; you can't color individual arguments differently
    within a single macro call. Use multiple macro calls instead.

## About `owo-colors`

This implementation has similar interfaces with [owo-colors](https://github.com/jam1garner/owo-colors),
namely the using of generic to select color and styles.

`owo-color` is good and slime, however its interface is bloated with "convenient" color methods,
90% out of which will never be called anyway. More over, the caller needs to jump some hoops and be explicit
about whether to enable colors (the `if_supports_color` method), which is both good and bad.

`ino_color` takes a different approach: color is applied at the print-site via macros,
which naturally knows which stream to check and can emit a single SGR sequence
(instead of nested open/close pairs that stomp on each other).
