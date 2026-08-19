//! `quraa` — generate a QR code from stdin or the clipboard.

use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use bpaf::Args;
use bpaf::OptionParser;
use bpaf::Parser;
use bpaf::construct;
use bpaf::short;
use rootcause::prelude::ResultExt;
use tracing::debug;

use crate::applet::AppletResult;
use crate::applet::Outcome;
use crate::applet::RunFailure;

/// Applet selector name.
pub const NAME: &str = "quraa";

/// QR code input source; exactly one is required.
#[derive(Debug, Clone, Copy)]
pub enum Source {
    /// Take the text content of the clipboard.
    Clipboard,
    /// Read standard input until EOF.
    Stdin,
}

/// CLI parser for `quraa`.
#[must_use]
pub fn cli() -> OptionParser<Source> {
    let clipboard = short('c')
        .long("clipboard")
        .help("Use the content of the clipboard as the QR code")
        .req_flag(Source::Clipboard);
    let stdin = short('s')
        .long("stdin")
        .help("Read standard input as the QR code")
        .req_flag(Source::Stdin);
    // Exactly one source; both/neither rejected by bpaf itself.
    construct!([clipboard, stdin])
        .to_options()
        .descr(
            "Generate a QR code from stdin or the clipboard and open it",
        )
        .version(env!("CARGO_PKG_VERSION"))
}

/// Multicall entry: parse `args` (applet `argv[1..]`) and run.
///
/// # Errors
///
/// Returns a [`RunFailure::Cli`] for parse/help/version exits and a
/// [`RunFailure::Applet`] for runtime failures.
pub fn applet_main(args: &[OsString]) -> Result<Outcome, RunFailure> {
    let source = cli()
        .run_inner(Args::from(args).set_name(NAME))
        .map_err(RunFailure::Cli)?;
    run(source).map_err(RunFailure::Applet)
}

fn run(source: Source) -> AppletResult {
    debug!("read data for QR code");
    let data = match source {
        Source::Clipboard => read_clipboard()?,
        Source::Stdin => read_stdin()?,
    };
    debug!(?data);

    debug!("generate QR code image");
    let svg = render_svg(&data)?;

    debug!("saving QR code to tempfile");
    let svg_path = write_tempfile(&svg)?;
    debug!(?svg_path);

    debug!("showing generated QR code");
    open_viewer(&svg_path)?;

    Ok(Outcome::Notice(format!(
        "QR code opened: {}",
        svg_path.display()
    )))
}

fn read_clipboard() -> rootcause::Result<String> {
    debug!("data source is clipboard");
    let mut cb =
        arboard::Clipboard::new().context("Unable to handle clipboard")?;
    Ok(cb.get_text().context("Unable to read from clipboard")?)
}

fn read_stdin() -> rootcause::Result<String> {
    use std::io::read_to_string;
    use std::io::stdin;
    debug!("data source is stdin");
    // Blocks until EOF, so the user can type a message and press Ctrl-D.
    Ok(read_to_string(stdin().lock())
        .context("Unable to read from stdin")?)
}

fn render_svg(data: &str) -> rootcause::Result<String> {
    use qrcode::QrCode;
    use qrcode::render::svg;
    let code =
        QrCode::new(data).context("Unable to encode the QR code")?;
    Ok(code
        .render()
        .min_dimensions(128, 128)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// Temp files are intentionally not cleaned up.
/// Files use `UUIDv7` names, so collisions are not a concern
/// (and they sort nicely).
fn write_tempfile(svg: &str) -> rootcause::Result<PathBuf> {
    let filename = format!("{NAME}:{}.svg", uuid::Uuid::now_v7());
    let path = std::env::temp_dir().join(filename);
    std::fs::write(&path, svg)
        .context("Unable to write the QR code SVG")?;
    Ok(path)
}

fn open_viewer(svg_path: &Path) -> rootcause::Result<()> {
    let status = std::process::Command::new("xdg-open")
        .arg(svg_path)
        .status()
        .context("Unable to execute xdg-open")?;
    if !status.success() {
        rootcause::bail!("xdg-open exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    fn parse(args: &[&str]) -> Result<Source, bpaf::ParseFailure> {
        cli().run_inner(Args::from(&args[..]).set_name(NAME))
    }

    #[test]
    fn each_source_parses() {
        assert!(matches!(parse(&["--clipboard"]), Ok(Source::Clipboard)));
        assert!(matches!(parse(&["-s"]), Ok(Source::Stdin)));
    }

    #[test]
    fn neither_source_rejected() {
        assert!(matches!(parse(&[]), Err(bpaf::ParseFailure::Stderr(_))));
    }

    #[test]
    fn both_sources_rejected() {
        assert!(matches!(
            parse(&["-c", "-s"]),
            Err(bpaf::ParseFailure::Stderr(_))
        ));
    }

    #[test]
    fn help_goes_to_stdout() {
        assert!(matches!(
            parse(&["--help"]),
            Err(bpaf::ParseFailure::Stdout(..))
        ));
    }
}
