//! Rendering a compact forecast from `nix build --dry-run` output.

use std::collections::HashSet;
use std::io;
use std::io::Write as _;
use std::str;

use rootcause::Result;
use rootcause::bail;
use serde_json::Map;
use serde_json::Value;
use subprocess::ExitStatus;
use tracing::warn;

use super::command::Kind;
use super::command::NixCommand;

const DERIVATION_BATCH_SIZE: usize = 256;
const BIG_PARALLEL: &str = "big-parallel";
const DERIVATION_JSON_VERSION: u64 = 4;
const UNKNOWN_PATHS_HEADER: &str = "don't know how to build these paths:";

/// Presentation options for a build forecast.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForecastOptions {
    show_fetches: bool,
}

impl ForecastOptions {
    #[must_use]
    pub const fn new(show_fetches: bool) -> Self {
        Self { show_fetches }
    }
}

/// Run a dry build, preserving diagnostics and replacing Nix's plan stanzas
/// with a compact forecast.
///
/// Parsing and derivation inspection are advisory. If Nix changes its
/// human-readable plan format, the original output is shown instead.
///
/// # Errors
///
/// Returns an error if the command cannot be run or terminal output cannot be
/// written.
pub(super) fn run(
    command: NixCommand,
    options: ForecastOptions,
) -> Result<ExitStatus> {
    let capture = command.output()?;
    io::stdout().lock().write_all(&capture.stdout)?;

    if !capture.success() {
        io::stderr().lock().write_all(&capture.stderr)?;
        return Ok(capture.exit_status);
    }

    let stderr_text = match str::from_utf8(&capture.stderr) {
        Ok(stderr_text) => stderr_text,
        Err(error) => {
            warn!(%error, "Nix dry-run output was not UTF-8; showing it unchanged");
            io::stderr().lock().write_all(&capture.stderr)?;
            return Ok(capture.exit_status);
        }
    };
    let parsed = match parse_plan(stderr_text) {
        Ok(parsed) => parsed,
        Err(error) => {
            warn!(%error, "Could not parse Nix dry-run plan; showing it unchanged");
            io::stderr().lock().write_all(&capture.stderr)?;
            return Ok(capture.exit_status);
        }
    };

    let big_parallel = match inspect_big_parallel(&parsed.plan.derivations)
    {
        Ok(derivations) => derivations,
        Err(error) => {
            warn!(?error, "Could not inspect planned derivations");
            HashSet::new()
        }
    };

    let mut stderr_writer = io::stderr().lock();
    write_diagnostics(&mut stderr_writer, &parsed.diagnostics)?;
    render_forecast(
        &mut stderr_writer,
        &parsed.plan,
        &big_parallel,
        options,
    )?;

    Ok(capture.exit_status)
}

#[derive(Debug, Default, Eq, PartialEq)]
struct BuildPlan<'input> {
    derivations: Vec<&'input str>,
    fetch: Option<FetchPlan<'input>>,
}

#[derive(Debug, Eq, PartialEq)]
struct FetchPlan<'input> {
    paths: Vec<&'input str>,
    download_size: &'input str,
    unpacked_size: &'input str,
}

#[derive(Debug, Eq, PartialEq)]
struct ParsedOutput<'input> {
    plan: BuildPlan<'input>,
    diagnostics: Vec<&'input str>,
}

enum Header<'input> {
    Builds(usize),
    Fetches {
        count: usize,
        download_size: &'input str,
        unpacked_size: &'input str,
    },
}

fn parse_plan(
    output: &str,
) -> std::result::Result<ParsedOutput<'_>, String> {
    let mut lines = output.lines();
    let mut plan = BuildPlan::default();
    let mut diagnostics = Vec::new();
    let mut saw_builds = false;

    while let Some(line) = lines.next() {
        match parse_header(line)? {
            Some(Header::Builds(count)) => {
                if saw_builds {
                    return Err(String::from(
                        "Nix printed more than one derivation plan",
                    ));
                }
                saw_builds = true;
                plan.derivations =
                    take_paths(&mut lines, count, PathKind::Derivation)?;
            }
            Some(Header::Fetches {
                count,
                download_size,
                unpacked_size,
            }) => {
                if plan.fetch.is_some() {
                    return Err(String::from(
                        "Nix printed more than one fetch plan",
                    ));
                }
                plan.fetch = Some(FetchPlan {
                    paths: take_paths(
                        &mut lines,
                        count,
                        PathKind::Output,
                    )?,
                    download_size,
                    unpacked_size,
                });
            }
            None => diagnostics.push(line),
        }
    }

    Ok(ParsedOutput { plan, diagnostics })
}

fn parse_header(
    line: &str,
) -> std::result::Result<Option<Header<'_>>, String> {
    if line == UNKNOWN_PATHS_HEADER {
        return Err(String::from(
            "Nix reported unknown paths; the forecast would be incomplete",
        ));
    }

    if line == "this derivation will be built:" {
        return Ok(Some(Header::Builds(1)));
    }
    if let Some(count) = line
        .strip_prefix("these ")
        .and_then(|line| line.strip_suffix(" derivations will be built:"))
    {
        return parse_count(count)
            .map(|count| Some(Header::Builds(count)));
    }

    let fetch = if let Some(sizes) =
        line.strip_prefix("this path will be fetched (")
    {
        Some((1, sizes))
    } else if let Some(rest) = line.strip_prefix("these ") {
        rest.split_once(" paths will be fetched (")
            .map(|(count, sizes)| {
                parse_count(count).map(|count| (count, sizes))
            })
            .transpose()?
    } else {
        None
    };

    if let Some((count, sizes)) = fetch {
        let sizes =
            sizes.strip_suffix(" unpacked):").ok_or_else(|| {
                format!("unrecognized Nix fetch-plan header: {line}")
            })?;
        let (download_size, unpacked_size) =
            sizes.split_once(" download, ").ok_or_else(|| {
                format!("unrecognized Nix fetch-plan sizes: {line}")
            })?;
        if download_size.is_empty() || unpacked_size.is_empty() {
            return Err(format!("empty Nix fetch-plan size: {line}"));
        }
        return Ok(Some(Header::Fetches {
            count,
            download_size,
            unpacked_size,
        }));
    }

    if line.contains("will be built:") || line.contains("will be fetched")
    {
        return Err(format!("unrecognized Nix plan header: {line}"));
    }

    Ok(None)
}

fn parse_count(count: &str) -> std::result::Result<usize, String> {
    count
        .parse()
        .map_err(|error| format!("invalid path count `{count}`: {error}"))
}

#[derive(Clone, Copy)]
enum PathKind {
    Derivation,
    Output,
}

fn take_paths<'input>(
    lines: &mut impl Iterator<Item = &'input str>,
    count: usize,
    kind: PathKind,
) -> std::result::Result<Vec<&'input str>, String> {
    let mut paths = Vec::new();

    for printed in 0..count {
        let line = lines.next().ok_or_else(|| {
            format!(
                "Nix plan declared {count} paths but printed only {printed}"
            )
        })?;
        let Some(path) = line.strip_prefix("  ") else {
            return Err(format!("unrecognized Nix plan path: {line}"));
        };
        let Some(name) = path.strip_prefix("/nix/store/") else {
            return Err(format!("unrecognized Nix plan path: {line}"));
        };
        if name.is_empty() || name.contains('/') {
            return Err(format!("unrecognized Nix plan path: {line}"));
        }
        if matches!(kind, PathKind::Derivation)
            && name.strip_suffix(".drv").is_none()
        {
            return Err(format!(
                "build-plan path is not a derivation: {line}"
            ));
        }
        paths.push(path);
    }

    Ok(paths)
}

fn inspect_big_parallel<'path>(
    derivations: &[&'path str],
) -> Result<HashSet<&'path str>> {
    let mut marked = HashSet::new();

    for batch in derivations.chunks(DERIVATION_BATCH_SIZE) {
        let capture = NixCommand::new(Kind::Derivation)
            .arg("show")
            .args(batch.iter().copied())
            .output()?;
        if !capture.success() {
            bail!(
                "nix derivation show exited with {:?}: {}",
                capture.exit_status,
                String::from_utf8_lossy(&capture.stderr).trim()
            );
        }

        let output: Value = serde_json::from_slice(&capture.stdout)?;
        collect_big_parallel(&output, batch, &mut marked)?;
    }

    Ok(marked)
}

fn collect_big_parallel<'path>(
    output: &Value,
    derivations: &[&'path str],
    marked: &mut HashSet<&'path str>,
) -> Result<()> {
    let derivation_map = derivation_map(output)?;

    for &path in derivations {
        let derivation = derivation_map.get(path).ok_or_else(|| {
            rootcause::report!(
                "nix derivation show omitted planned derivation {path}"
            )
        })?;
        if derivation_has_feature(derivation, BIG_PARALLEL)? {
            marked.insert(path);
        }
    }

    Ok(())
}

fn derivation_map(output: &Value) -> Result<&Map<String, Value>> {
    let root = output.as_object().ok_or_else(|| {
        rootcause::report!("nix derivation show returned a non-object")
    })?;
    let Some(version) = root.get("version") else {
        return Ok(root);
    };
    let version = version.as_u64().ok_or_else(|| {
        rootcause::report!(
            "nix derivation show returned a non-numeric version"
        )
    })?;
    if version != DERIVATION_JSON_VERSION {
        bail!("unsupported nix derivation JSON version {version}");
    }

    root.get("derivations")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            rootcause::report!(
                "nix derivation show version {version} omitted derivations"
            )
        })
}

fn derivation_has_feature(
    derivation: &Value,
    feature: &str,
) -> Result<bool> {
    if value_has_feature(
        derivation.pointer("/env/requiredSystemFeatures"),
        feature,
    )? || value_has_feature(
        derivation.pointer("/structuredAttrs/requiredSystemFeatures"),
        feature,
    )? {
        return Ok(true);
    }

    let Some(encoded_attrs) = derivation.pointer("/env/__json") else {
        return Ok(false);
    };
    let encoded_attrs = encoded_attrs.as_str().ok_or_else(|| {
        rootcause::report!(
            "nix derivation show returned a non-string env.__json"
        )
    })?;
    let attrs: Value = serde_json::from_str(encoded_attrs)?;
    value_has_feature(attrs.get("requiredSystemFeatures"), feature)
}

fn value_has_feature(
    value: Option<&Value>,
    feature: &str,
) -> Result<bool> {
    match value {
        None => Ok(false),
        Some(Value::String(features)) => Ok(features
            .split_ascii_whitespace()
            .any(|item| item == feature)),
        Some(Value::Array(features)) => {
            let mut found = false;
            for item in features {
                let Some(item) = item.as_str() else {
                    bail!(
                        "nix derivation show returned a non-string system feature"
                    );
                };
                found |= item == feature;
            }
            Ok(found)
        }
        Some(_) => {
            bail!("nix derivation show returned invalid system features")
        }
    }
}

fn write_diagnostics(
    writer: &mut dyn io::Write,
    diagnostics: &[&str],
) -> io::Result<()> {
    for line in diagnostics {
        writeln!(writer, "{line}")?;
    }
    if diagnostics.iter().any(|line| !line.is_empty()) {
        writeln!(writer)?;
    }
    Ok(())
}

fn render_forecast(
    writer: &mut dyn io::Write,
    plan: &BuildPlan<'_>,
    big_parallel: &HashSet<&str>,
    options: ForecastOptions,
) -> io::Result<()> {
    writeln!(writer, "Build forecast")?;
    writeln!(writer)?;

    let build_count = plan.derivations.len();
    writeln!(
        writer,
        "Builds: {build_count} {}",
        plural(build_count, "derivation", "derivations")
    )?;
    for &derivation in &plan.derivations {
        let annotation = if big_parallel.contains(derivation) {
            "  [big-parallel]"
        } else {
            ""
        };
        writeln!(writer, "  {}{annotation}", derivation_name(derivation))?;
    }

    let heavy_count = plan
        .derivations
        .iter()
        .filter(|derivation| big_parallel.contains(**derivation))
        .count();
    if heavy_count > 0 {
        writeln!(writer)?;
        writeln!(
            writer,
            "Heavy hint: big-parallel required by {heavy_count} {}.",
            plural(heavy_count, "derivation", "derivations")
        )?;
    }

    writeln!(writer)?;
    let fetch_count =
        plan.fetch.as_ref().map_or(0, |fetch| fetch.paths.len());
    writeln!(
        writer,
        "Fetch: {fetch_count} {}",
        plural(fetch_count, "path", "paths")
    )?;
    if let Some(fetch) = &plan.fetch {
        writeln!(writer, "  {} download", fetch.download_size)?;
        writeln!(writer, "  {} unpacked", fetch.unpacked_size)?;
        if options.show_fetches {
            for &path in &fetch.paths {
                writeln!(writer, "  {path}")?;
            }
        } else {
            writeln!(
                writer,
                "  Paths hidden; pass --show-fetches to list them."
            )?;
        }
    }

    Ok(())
}

fn derivation_name(path: &str) -> &str {
    path.strip_prefix("/nix/store/")
        .and_then(|name| name.split_once('-'))
        .map(|(_, name)| name)
        .and_then(|name| name.strip_suffix(".drv"))
        .unwrap_or(path)
}

const fn plural(
    count: usize,
    singular: &'static str,
    plural: &'static str,
) -> &'static str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Test assertions")]
mod tests {
    use super::*;

    const FIREFOX_DRV: &str = "/nix/store/wi7bg5haxa3mbr9977x57j7qr6cafbmq-firefox-unwrapped-155.0.drv";
    const FETCHED_PATH: &str =
        "/nix/store/603yaax3l2jmc0hfv6g3hgjr1qk5jfxk-clang-21.1.8";
    const FETCHED_DOC: &str = "/nix/store/8glg7jn1763pqjj5473zpgnh5phmq326-shellcheck-0.11.0-doc";
    const PLAN: &str = "warning: sample diagnostic\n\
these 2 derivations will be built:\n  \
/nix/store/rkalfnd132n0r10jm0jbn5y1g5n8yzcd-distribution.ini.drv\n  \
/nix/store/wi7bg5haxa3mbr9977x57j7qr6cafbmq-firefox-unwrapped-155.0.drv\n\
these 2 paths will be fetched (1050.61 MiB download, 3168.50 MiB unpacked):\n  \
/nix/store/603yaax3l2jmc0hfv6g3hgjr1qk5jfxk-clang-21.1.8\n  \
/nix/store/8glg7jn1763pqjj5473zpgnh5phmq326-shellcheck-0.11.0-doc\n";

    #[test]
    fn forecast_keeps_build_names_and_summarizes_fetches() {
        let parsed = parse_plan(PLAN).unwrap();
        let big_parallel = HashSet::from([FIREFOX_DRV]);
        let mut output = Vec::new();

        render_forecast(
            &mut output,
            &parsed.plan,
            &big_parallel,
            ForecastOptions::new(false),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Builds: 2 derivations"));
        assert!(output.contains("\n  distribution.ini\n"));
        assert!(
            output
                .contains("\n  firefox-unwrapped-155.0  [big-parallel]\n")
        );
        assert!(output.contains("1050.61 MiB download"));
        assert!(output.contains("3168.50 MiB unpacked"));
        assert!(!output.contains(FETCHED_PATH));
        assert!(!output.contains(FETCHED_DOC));
    }

    #[test]
    fn requested_fetch_details_preserve_store_paths() {
        let parsed = parse_plan(PLAN).unwrap();
        let mut output = Vec::new();

        render_forecast(
            &mut output,
            &parsed.plan,
            &HashSet::new(),
            ForecastOptions::new(true),
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(FETCHED_PATH));
        assert!(output.contains(FETCHED_DOC));
        assert!(!output.contains("Paths hidden"));
    }

    #[test]
    fn singular_plan_is_accepted() {
        let plan = format!(
            "this derivation will be built:\n  {FIREFOX_DRV}\n\
this path will be fetched (0.01 MiB download, 0.02 MiB unpacked):\n  {FETCHED_DOC}\n"
        );
        let parsed = parse_plan(&plan).unwrap();

        assert_eq!(parsed.plan.derivations, [FIREFOX_DRV]);
        assert_eq!(parsed.plan.fetch.unwrap().paths, [FETCHED_DOC]);
    }

    #[test]
    fn truncated_plan_is_rejected_instead_of_underreporting() {
        let error = parse_plan(
            "these 2 derivations will be built:\n  \
/nix/store/wi7bg5haxa3mbr9977x57j7qr6cafbmq-firefox-unwrapped-155.0.drv\n",
        )
        .unwrap_err();

        assert!(error.contains("declared 2 paths"));
    }

    #[test]
    fn big_parallel_is_read_from_plain_and_structured_attributes() {
        let plain = serde_json::json!({
            "env": { "requiredSystemFeatures": "kvm big-parallel" }
        });
        let structured = serde_json::json!({
            "env": {
                "__json": "{\"requiredSystemFeatures\":[\"big-parallel\"]}"
            }
        });

        assert!(derivation_has_feature(&plain, BIG_PARALLEL).unwrap());
        assert!(
            derivation_has_feature(&structured, BIG_PARALLEL).unwrap()
        );
    }

    #[test]
    fn absent_and_malformed_features_remain_distinct() {
        let absent = serde_json::json!({ "env": {} });
        let malformed = serde_json::json!({ "env": { "__json": [] } });

        assert!(!derivation_has_feature(&absent, BIG_PARALLEL).unwrap());
        derivation_has_feature(&malformed, BIG_PARALLEL).unwrap_err();
    }

    #[test]
    fn big_parallel_metadata_accepts_legacy_and_version_four() {
        let derivation = serde_json::json!({
            "env": { "requiredSystemFeatures": "big-parallel" }
        });
        let legacy = serde_json::json!({ (FIREFOX_DRV): derivation });
        let version_four = serde_json::json!({
            "version": 4,
            "derivations": { (FIREFOX_DRV): derivation }
        });

        for output in [&legacy, &version_four] {
            let mut marked = HashSet::new();
            collect_big_parallel(output, &[FIREFOX_DRV], &mut marked)
                .unwrap();
            assert_eq!(marked, HashSet::from([FIREFOX_DRV]));
        }
    }

    #[test]
    fn unknown_paths_make_the_forecast_incomplete() {
        let error = parse_plan(
            "don't know how to build these paths:\n  \
/nix/store/00000000000000000000000000000000-missing.drv\n",
        )
        .unwrap_err();

        assert!(error.contains("unknown"));
    }
}
