//! Private implementation of the `cmd!` macro.
//!
//! The macro turns a shell-ish command string into a chain of `Cmd` builder
//! calls. The pipeline has three stages:
//!
//! 1. [`tokenize`] — split the raw string into [`Token`]s (bare words, quoted
//!    strings, interpolations, splats), recording adjacency between them.
//! 2. [`emit`] — turn each token into a [`Fragment`]: a snippet of token
//!    stream to splice into the expansion.
//! 3. [`try_cmd`] — assemble the fragments into the final program, rejecting
//!    the few invalid combinations (splat program, splat concatenation).

#![deny(missing_debug_implementations)]
#![expect(
    clippy::expect_used,
    reason = "proc-macro invariants guaranteed by the cmd! macro"
)]

use std::iter;

use proc_macro::{
    Delimiter, Group, Literal, Span, TokenStream, TokenTree,
};

#[doc(hidden)]
#[proc_macro]
pub fn __cmd(macro_arg: TokenStream) -> TokenStream {
    try_cmd(macro_arg).unwrap_or_else(|msg| {
        parse_ts(&format!("compile_error!({msg:?})"))
    })
}

type Result<T, E = String> = std::result::Result<T, E>;
/// Assembles the expansion: `<closure> ("prog").arg(...)…`.
fn try_cmd(macro_arg: TokenStream) -> Result<TokenStream> {
    let (cmd, literal) = {
        let mut iter = macro_arg.into_iter();
        let cmd = iter.next().ok_or("expected command expression")?;
        let literal = iter.next().ok_or("expected string literal")?;
        if iter.next().is_some() {
            return Err("expected exactly two arguments".to_owned());
        }
        (cmd, literal)
    };

    let literal = into_literal(&literal)
        .ok_or_else(|| "expected a plain string literal".to_owned())?;
    let literal_text = literal.to_string();
    if !literal_text.starts_with('"') {
        return Err("expected a plain string literal".to_owned());
    }

    let call_site = literal.span();
    let mut words = shell_words(&literal_text, call_site);

    let mut res = TokenStream::new();

    // The first word is the program. The `cmd!` macro passes a closure which
    // turns it into a `Cmd`, so the expansion opens with `<closure> ("prog")`.
    // The remaining words are appended as builder calls.
    {
        let program = words
            .next()
            .transpose()?
            .ok_or_else(|| "command can't be empty".to_owned())?;
        if program.splat_name.is_some() {
            return Err("can't splat program name".to_owned());
        }
        res.extend(Some(cmd));
        res.extend(program.ts);
    }

    let mut prev_splat: Option<String> = None;
    for frag in words {
        let frag = frag?;

        // A splat must be separated from its neighbors by whitespace: it can
        // neither concatenate onto the previous word, nor have the next word
        // concatenated onto it.
        if frag.joined_to_prev {
            if let Some(name) = &frag.splat_name {
                return Err(splat_concat_error(name));
            }
            if let Some(name) = &prev_splat {
                return Err(splat_concat_error(name));
            }
        }

        res.extend(parse_ts(frag.append_method()));
        res.extend(frag.ts);
        prev_splat = frag.splat_name;
    }

    Ok(res)
}

fn splat_concat_error(name: &str) -> String {
    format!(
        "can't combine splat with concatenation, add spaces around `{{{name}...}}`"
    )
}

/// Extracts the string literal from the second argument of `__cmd!`.
///
/// A `None`-delimited group wrapping a single literal is also accepted: the
/// declarative `cmd!` macro passes the literal through as such a group.
fn into_literal(ts: &TokenTree) -> Option<Literal> {
    match ts {
        TokenTree::Literal(literal) => Some(literal.clone()),
        TokenTree::Group(group) if group.delimiter() == Delimiter::None => {
            let mut it = group.stream().into_iter();
            match (it.next(), it.next()) {
                (Some(TokenTree::Literal(literal)), None) => Some(literal),
                _ => None,
            }
        }
        TokenTree::Group(_) | TokenTree::Ident(_) | TokenTree::Punct(_) => None,
    }
}

/// Lexes `cmd` into words and emits a [`Fragment`] for each.
fn shell_words(
    cmd: &str,
    call_site: Span,
) -> impl Iterator<Item = Result<Fragment>> + '_ {
    tokenize(cmd).map(move |token| {
        let token = token?;
        emit(&token, call_site)
    })
}

/// Turns a [`Token`] into the [`Fragment`] spliced into the expansion.
fn emit(token: &Token, call_site: Span) -> Result<Fragment> {
    let (ts, splat_name) = match token.kind {
        TokenKind::Word | TokenKind::Quoted => {
            (parse_ts(&format!("(\"{}\")", token.text)), None)
        }
        TokenKind::Interpolation => {
            let name = &token.text;
            validate_ident(name)?;
            let ts = respan(parse_ts(&format!("(&({name}))")), call_site);
            (ts, None)
        }
        TokenKind::Splat => {
            let name = token
                .text
                .strip_suffix("...")
                .unwrap_or(token.text.as_str());
            validate_ident(name)?;
            let ts = respan(parse_ts(&format!("({name})")), call_site);
            (ts, Some(name.to_owned()))
        }
    };

    Ok(Fragment {
        joined_to_prev: token.joined_to_prev,
        splat_name,
        ts,
    })
}

/// Splits `cmd` into [`Token`]s.
///
/// Yields a single `Err` (and then nothing) if the string is malformed.
fn tokenize(cmd: &str) -> impl Iterator<Item = Result<Token>> + '_ {
    let cmd = strip_outer_quotes(cmd);
    let mut rest = cmd;

    iter::from_fn(move || {
        // A token is "joined" to the previous one when there is no whitespace
        // between them; adjacent tokens are concatenated during assembly.
        let joined_to_prev =
            !rest.chars().next().is_some_and(char::is_whitespace);
        rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }

        match next_token(rest) {
            Ok((remaining, text, kind)) => {
                rest = remaining;
                Some(Ok(Token {
                    joined_to_prev,
                    text,
                    kind,
                }))
            }
            Err(err) => {
                rest = "";
                Some(Err(err))
            }
        }
    })
}

/// Classifies the token at the start of `s`, returning the unconsumed suffix,
/// its normalized text (quotes and braces already stripped), and its kind.
fn next_token(input: &str) -> Result<(&str, String, TokenKind)> {
    // `{name}` or `{name...}` — whitespace inside the braces is tolerated.
    if let Some(after_open) = input.strip_prefix('{') {
        let (inner, remaining) = after_open
            .split_once('}')
            .ok_or_else(|| "unclosed `{` in command".to_owned())?;
        let inner = inner.trim();
        let kind = if inner.ends_with("...") {
            TokenKind::Splat
        } else {
            TokenKind::Interpolation
        };
        return Ok((remaining, inner.to_owned(), kind));
    }

    // `'...'` — a quoted word; interpolation is disabled inside.
    if let Some(after_open) = input.strip_prefix('\'') {
        let (text, remaining) = after_open
            .split_once('\'')
            .ok_or_else(|| "unclosed `'` in command".to_owned())?;
        return Ok((remaining, text.to_owned(), TokenKind::Quoted));
    }

    // A bare word, running up to the next whitespace, quote or interpolation.
    let split_index = input
        .find(|character: char| {
            character.is_ascii_whitespace()
                || character == '\''
                || character == '{'
        })
        .unwrap_or(input.len());
    let (word, remaining) = input.split_at(split_index);
    Ok((remaining, word.to_owned(), TokenKind::Word))
}

/// A single shell word.
#[derive(Debug)]
struct Token {
    /// Whether this word directly follows the previous one, with no whitespace
    /// between them.
    joined_to_prev: bool,
    /// The normalized word text.
    text: String,
    kind: TokenKind,
}

#[derive(Debug, Clone, Copy)]
enum TokenKind {
    /// A bare word: `clone`.
    Word,
    /// A single-quoted word: `'hello world'` (interpolation disabled).
    Quoted,
    /// An interpolation: `{var}`.
    Interpolation,
    /// A splat interpolation: `{args...}`.
    Splat,
}

/// A code fragment ready to be spliced into the expansion.
#[derive(Debug)]
struct Fragment {
    /// Whether this fragment is adjacent to the previous one (no whitespace).
    joined_to_prev: bool,
    /// The variable name for splats; `None` otherwise.
    splat_name: Option<String>,
    /// The emitted expression.
    ts: TokenStream,
}

impl Fragment {
    /// The `Cmd` builder method this fragment appends with. A splat maps to
    /// `.args` regardless of adjacency — adjacency is validated separately,
    /// as a splat must be whitespace-separated.
    fn append_method(&self) -> &'static str {
        match (&self.splat_name, self.joined_to_prev) {
            (Some(_), _) => ".args",
            (None, true) => ".__extend_arg",
            (None, false) => ".arg",
        }
    }
}

/// Strips one pair of surrounding double quotes from `s`.
///
/// Unlike [`str::trim_matches`], at most one quote is removed from each side,
/// so `""` becomes an empty string and `"""` becomes `"`.
fn strip_outer_quotes(input: &str) -> &str {
    input.strip_circumfix('"', '"').unwrap_or(input)
}

/// Validates that `name` is a plain identifier (ASCII alphanumerics and
/// underscores). Interpolation targets must be simple variables.
fn validate_ident(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err("empty interpolation in command".to_owned());
    }
    if !name.chars().all(|char| char.is_ascii_alphanumeric() || char == '_') {
        return Err(format!(
            "can only interpolate simple variables, got this expression instead: `{name}`"
        ));
    }
    Ok(())
}

fn respan(ts: TokenStream, span: Span) -> TokenStream {
    let mut res = TokenStream::new();
    for tt in ts {
        let tt = match tt {
            TokenTree::Ident(mut ident) => {
                ident.set_span(
                    ident.span().resolved_at(span).located_at(span),
                );
                TokenTree::Ident(ident)
            }
            TokenTree::Group(group) => TokenTree::Group(Group::new(
                group.delimiter(),
                respan(group.stream(), span),
            )),
            TokenTree::Punct(_) | TokenTree::Literal(_) => tt,
        };
        res.extend(Some(tt));
    }
    res
}

fn parse_ts(input: &str) -> TokenStream {
    input.parse().expect("internally generated token stream")
}
