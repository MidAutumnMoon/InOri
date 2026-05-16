use tracing::debug;

use anyhow::Context;
use anyhow::bail;

/// Generate QR Code from stdin or clipboard.
#[derive(clap::Parser, Debug)]
struct CliOpts {
    /// Use the content of clipboard as QR Code
    #[arg(short, exclusive = true)]
    clipboard: bool,

    /// Read standard input as Qr Code
    #[arg(short, exclusive = true)]
    stdin: bool,
}

fn run(cliopts: &CliOpts) -> anyhow::Result<()> {
    debug!("read data for Qr Code");

    let data = if cliopts.clipboard {
        debug!("data source is clipboard");
        let mut cb = arboard::Clipboard::new()
            .context("Unable to handle clipboard")?;
        cb.get_text().context("Unable to read from clipboard")?
    } else if cliopts.stdin {
        use std::io::{read_to_string, stdin};
        debug!("data source is stdin");
        // Blocks until EOF, so the user can type a message and press Ctrl-D.
        read_to_string(stdin().lock())
            .context("Unable to read from stdin")?
    } else {
        bail!(
            "Wrong command line options. \
                Run with --help to see usage."
        )
    };

    debug!(?data);

    debug!("generate Qr Code image");

    let qrcode = {
        use qrcode::QrCode;
        use qrcode::render::svg;
        let code = QrCode::new(&data)?;
        code.render()
            .min_dimensions(128, 128)
            .dark_color(svg::Color("#000000"))
            .light_color(svg::Color("#ffffff"))
            .build()
    };

    // Display Qr Code

    debug!("saving Qr Code to tempfile");

    // Temp files are intentionally not cleaned up.
    // Files use UUIDv7 names, so collisions are not a concern.
    let svg_path = {
        // Using UUIDv7 to make files nicely sorted
        let filename = format!("quraa:{}.svg", uuid::Uuid::now_v7());
        let path = std::env::temp_dir().join(filename);
        std::fs::write(&path, &qrcode)?;
        path
    };

    debug!(?svg_path);

    debug!("showing generated Qr Code");

    let status = std::process::Command::new("xdg-open")
        .arg(&svg_path)
        .status()
        .context("Unable to execute xdg-open")?;

    if !status.success() {
        bail!("xdg-open exited with {status}");
    }

    eprintln!("QR code opened: {}", svg_path.display());

    Ok(())
}

fn main() {
    ino_tracing::init_tracing_subscriber();

    let cliopts = <CliOpts as clap::Parser>::parse();

    debug!(?cliopts);

    let _ = run(&cliopts).inspect_err(|err| {
        eprintln!("{err:?}");
        std::process::exit(1);
    });
}
