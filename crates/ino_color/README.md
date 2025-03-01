# ino_color

A output coloring crate with limited functionality, limit platform support
and limited allocations. It's perfect for scratching tiny spot of itch.

## Basic Usage

```rust
// This's the trait that adds coloring methods.
use ino_color::InoColor;

// These two modules contain predefined colors and styles.
// As a personal preferrence, wildcard import is avoided,
// even though doing so makes the function call looks funnier.
use ino_color::fg;
use ino_color::style;

// The most basic usage
println!(
    "{}", "Hello Fancy".fg::<fg::Yellow>()
);

// It's also chainable!
println!(
    "{}", "Savoy blue".fg::<fg::Blue>().style::<style::Italic>()
);

// In fact, anything implements `std::fmt` traits can be colored.
println!( "{:?}", vec![123].fg::<fg::Green>() );
println!( "{:X}", 123.fg::<fg::Green>() );
```

## Pros & Cons

### Pros

- Good at scratching itch.
- Low amount of allocations.
- Follows the [_Standard for ANSI Colors in Terminals_](https://bixense.com/clicolors/) by default.
- Doesn't pollute the LSP completion with dozens of methods named after colors.

### Cons

- Linux only.
  - However it doesn't use platfor specific API, so it might also works on Darwin and modern Windows
    as long as the terminal emulator speaks ANSI SGR.

- Can't set background color (yet?).
  - Reason: After years of experience of using Linux, no legit usage of background colors has been encountered
    other than TUI frameworks. Remove it simplifies the implementation.

- Only supports 16 named (4-bit) colors.
  - Support for 8-bit color and true color isn't on the roadmap.

- All color and style selections are done in **type level**, meaning coloring can't be changed at runtime.
  - Such APIs will not be added in the near future.
  - Blame `owo-colors` for inventing this API, explained next section.

## About `owo-colors`

This implementation has similar interfaces with [owo-colors](https://github.com/jam1garner/owo-colors),
namely the using of generic to select color and styles.

`owo-color` is good and slime, however its interface is bloated with "convenient" color methods,
90% out of which will never be called anyway. More over, the caller needs to jump some hoops and be explicit
about whether to enable colors (the `if_supports_color` method), which is both good and bad.

`ino_color` removes the runtime color selection support, in gain it has a even slimer API. And as stated in *Pros*,
it follows the ANSI color standard by default, although the various checks do introduce amounts of costs, so it's
recommended to cache the colored result, or use `*_always` APIs to skip the check.
