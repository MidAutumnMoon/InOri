use std::sync::LazyLock;

use regex::Regex;
use tracing::trace;
use yansi::{Color, Paint};

use crate::search::{
    CHANNEL,
    types::{OptionSearchResult, PackageSearchResult},
};

static HYPERLINKS_SUPPORTED: LazyLock<bool> =
    LazyLock::new(supports_hyperlinks::supports_hyperlinks);

static HTML_TAG: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(clippy::expect_used)]
    Regex::new(r"<[^>]*>").expect("HTML tag regex should always be valid")
});

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

pub fn print_packages(platforms: bool, documents: &[PackageSearchResult]) {
    for elem in documents.iter().rev() {
        println!();
        trace!("{elem:#?}");

        print!("{}", Paint::new(&elem.package_attr_name).fg(Color::Blue));
        let version = &elem.package_pversion;
        if !version.is_empty() {
            print!(" ({})", Paint::new(version).fg(Color::Green));
        }
        println!();

        if let Some(description) = &elem.package_description {
            print_wrapped(&description.replace('\n', " "));
        }

        if let Some(main_program) = &elem.package_mainProgram {
            print_wrapped_field("Main program", main_program);
        }

        for url in &elem.package_homepage {
            print_field_link("Homepage", url);
        }

        if platforms && !elem.package_platforms.is_empty() {
            println!("  Platforms: {}", elem.package_platforms.join(", "));
        }

        if let Some(position) = &elem.package_position {
            let filepath = position.split(':').next().unwrap_or(position);
            let url = format!(
                "https://github.com/NixOS/nixpkgs/blob/{CHANNEL}/{filepath}"
            );
            print_field_link("GitHub link", &url);
        }
    }
}

pub fn print_options(documents: &[OptionSearchResult]) {
    for elem in documents.iter().rev() {
        println!();
        trace!("{elem:#?}");

        print!("{}", Paint::new(&elem.option_name).fg(Color::Blue));

        if let Some(option_type) = &elem.option_type {
            print!(" :: {}", Paint::new(option_type).fg(Color::Green));
        }

        if let Some(example) = &elem.option_example {
            print!(
                " (example: {})",
                Paint::new(example).fg(Color::Yellow)
            );
        }

        println!();
        println!("  Scope: {}", elem.r#type);

        if let Some(description) = &elem.option_description {
            let description = strip_html(description);
            print_wrapped(&description.replace('\n', " "));
        }

        if let Some(default) = &elem.option_default {
            print_wrapped_field("Default", default);
        }

        if let Some(source) = &elem.option_source {
            let filepath = source.split(':').next().unwrap_or(source);
            let url = format!(
                "https://github.com/NixOS/nixpkgs/blob/{CHANNEL}/{filepath}"
            );
            print_field_link("Source", &url);
        }
    }
}

/// Prints `  label: url` with an OSC-8 hyperlink when the terminal supports
/// them, dimmed plain text otherwise.
fn print_field_link(label: &str, url: &str) {
    let text = format!("{DIM}{url}{RESET}");
    let field = if *HYPERLINKS_SUPPORTED {
        format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
    } else {
        text
    };
    println!("  {label}: {field}");
}

fn print_wrapped(text: &str) {
    for line in textwrap::wrap(text, textwrap::Options::with_termwidth()) {
        println!("  {line}");
    }
}

fn print_wrapped_field(label: &str, value: &str) {
    let prefix = format!("  {label}: ");
    let indent = " ".repeat(prefix.chars().count());

    for (index, line) in
        textwrap::wrap(value, textwrap::Options::with_termwidth())
            .iter()
            .enumerate()
    {
        if index == 0 {
            println!("{prefix}{line}");
        } else {
            println!("{indent}{line}");
        }
    }
}

fn strip_html(html: &str) -> String {
    HTML_TAG.replace_all(html, "").trim().to_string()
}
