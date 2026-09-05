//! The `info` command: list the generations of a NixOS profile.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::str::FromStr;

use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use rootcause::Result;
use rootcause::report;
use tracing::debug;
use tracing::warn;

use super::CURRENT_PROFILE;
use super::from_dir;
use super::is_current;
use crate::nix::command::Kind;
use crate::nix::command::NixCommand;

#[derive(Clone, Debug)]
pub struct CliOpts {
    pub profile: PathBuf,
    pub fields: Option<Vec<Field>>,
}

/// Parse the `info` command.
#[must_use]
pub fn cli() -> impl Parser<CliOpts> {
    let profile = long("profile")
        .short('P')
        .argument::<String>("PROFILE")
        .help("Path to Nix' profiles directory")
        .fallback(String::from("/nix/var/nix/profiles/system"))
        .display_fallback()
        .map(PathBuf::from);
    let fields = long("fields")
        .argument::<String>("FIELDS")
        .help("Comma-delimited list of field(s) to display")
        .many()
        .parse(|values: Vec<String>| {
            if values.is_empty() {
                return Ok(None);
            }
            parse_field_selection(&values).map(Some)
        });

    construct!(CliOpts { profile, fields })
}

fn parse_field_selection(
    values: &[String],
) -> std::result::Result<Vec<Field>, String> {
    let mut fields = Vec::new();
    for value in values {
        for part in value.split(',') {
            fields.push(part.parse::<Field>()?);
        }
    }
    Ok(fields)
}

/// Run the `info` command.
///
/// # Errors
///
/// Returns an error if the profile cannot be read or output fails.
pub fn run(opts: &CliOpts) -> Result<()> {
    let profile = &opts.profile;

    if !profile.is_symlink() {
        return Err(report!(
            "No profile `{:?}` found",
            profile.file_name().unwrap_or_default()
        ));
    }

    let profile_dir = profile.parent().unwrap_or_else(|| Path::new("."));

    let generations: Vec<_> = fs::read_dir(profile_dir)?
        .filter_map(|entry| {
            let dir_entry = entry.ok()?;
            let path = dir_entry.path();
            path.file_name()?
                .to_str()?
                .starts_with(profile.file_name()?.to_str()?)
                .then_some(path)
        })
        .collect();

    let gen_dir_refs: Vec<&Path> =
        generations.iter().map(PathBuf::as_path).collect();
    let closure_sizes = get_closure_sizes_batch(&gen_dir_refs);

    let descriptions: Vec<GenerationInfo> = generations
        .iter()
        .filter_map(|gen_dir| {
            let size = closure_sizes
                .get(gen_dir)
                .cloned()
                .unwrap_or_else(|| String::from("Unknown"));
            describe(gen_dir, size)
        })
        .collect();
    print_info(descriptions, opts.fields.as_deref());

    Ok(())
}

#[derive(Debug, Clone)]
#[expect(
    clippy::module_name_repetitions,
    reason = "GenerationInfo is the domain term for a described generation"
)]
pub struct GenerationInfo {
    /// Number of a generation.
    pub number: u64,

    /// Date on switch a generation was built.
    pub date: String,

    /// `NixOS` version derived from `nixos-version`.
    pub nixos_version: String,

    /// Version of the bootable kernel for a given generation.
    pub kernel_version: String,

    /// Revision for a configuration. This will be the value
    /// set in `config.system.configurationRevision`.
    pub configuration_revision: Option<String>,

    /// Specialisations, if any.
    pub specialisations: Option<Vec<String>>,

    /// Whether a given generation is the current one.
    pub current: bool,

    /// Closure size of the generation.
    pub closure_size: String,
}

#[derive(Clone, Debug)]
pub enum Field {
    /// Generation Id.
    Id,

    /// Build Date.
    Date,

    /// Nixos Version.
    Nver,

    /// Kernel Version.
    Kernel,

    /// Configuration Revision.
    Confrev,

    /// Specialisations.
    Spec,

    /// Closure Size.
    Size,
}

impl FromStr for Field {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "id" => Ok(Self::Id),
            "date" => Ok(Self::Date),
            "nver" => Ok(Self::Nver),
            "kernel" => Ok(Self::Kernel),
            "confRev" => Ok(Self::Confrev),
            "spec" => Ok(Self::Spec),
            "size" => Ok(Self::Size),
            other => Err(format!(
                "expected one of `id`, `date`, `nver`, `kernel`, `confRev`, \
                 `spec`, `size`, got `{other}`"
            )),
        }
    }
}

#[derive(Clone, Copy)]
struct ColumnWidths {
    id: usize,
    date: usize,
    nver: usize,
    kernel: usize,
    confrev: usize,
    spec: usize,
    size: usize,
}

impl Field {
    const fn column_info(
        &self,
        width: ColumnWidths,
    ) -> (&'static str, usize) {
        match self {
            Self::Id => ("Generation No", width.id),
            Self::Date => ("Build Date", width.date),
            Self::Nver => ("NixOS Version", width.nver),
            Self::Kernel => ("Kernel", width.kernel),
            Self::Confrev => ("Configuration Revision", width.confrev),
            Self::Spec => ("Specialisations", width.spec),
            Self::Size => ("Closure Size", width.size),
        }
    }
}

fn closure_size_from_json(
    json: &serde_json::Value,
    store_path_str: &str,
) -> Option<u64> {
    json.as_array().map_or_else(
        || {
            let obj = json.as_object()?;
            obj.get(store_path_str)
                .and_then(|value| value.get("closureSize"))
                .and_then(serde_json::Value::as_u64)
        },
        |arr| {
            arr.iter().find_map(|entry| {
                let path = entry.get("path")?.as_str()?;
                let size = entry.get("closureSize")?.as_u64()?;
                (path == store_path_str).then_some(size)
            })
        },
    )
}

#[expect(
    clippy::cast_precision_loss,
    reason = "display-only; precision loss is irrelevant for human-readable sizes"
)]
fn bytes_to_gb_string(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

/// Get closure sizes for all given generation directories in a single
/// `nix path-info` invocation.
///
/// # Returns
///
/// A map from generation directory path to formatted closure size
/// string.
#[must_use]
fn get_closure_sizes_batch(
    generation_dirs: &[&Path],
) -> HashMap<PathBuf, String> {
    if generation_dirs.is_empty() {
        return HashMap::new();
    }

    let store_paths: Vec<PathBuf> = generation_dirs
        .iter()
        .map(|path| {
            path.read_link().unwrap_or_else(|_| path.to_path_buf())
        })
        .collect();

    let output = match NixCommand::new(Kind::PathInfo)
        .args(["-Sh", "--json"])
        .args(generation_dirs)
        .output()
    {
        Ok(out) => out,
        Err(err) => {
            debug!(
                "get_closure_sizes_batch: failed to run nix path-info: {err:?}"
            );
            return HashMap::new();
        }
    };

    let output_str = String::from_utf8_lossy(&output.stdout);

    let json: serde_json::Value = match serde_json::from_str::<
        serde_json::Value,
    >(&output_str)
    {
        Ok(j) => j,
        Err(err) => {
            debug!(
                "get_closure_sizes_batch: failed to parse JSON: {err} output: \
           {output_str}"
            );
            return HashMap::new();
        }
    };

    generation_dirs
        .iter()
        .zip(store_paths.iter())
        .map(|(gen_dir, store_path)| {
            let store_path_str = store_path.to_string_lossy();
            let size_str = closure_size_from_json(&json, &store_path_str)
                .map_or_else(|| "Unknown".to_owned(), bytes_to_gb_string);
            (gen_dir.to_path_buf(), size_str)
        })
        .collect()
}

/// Describe a generation entry in full, for display.
///
/// `closure_size` is supplied by the caller (see
/// [`get_closure_sizes_batch`]); describing many generations with
/// per-entry queries is expensive.
#[must_use]
fn describe(
    generation_dir: &Path,
    closure_size: String,
) -> Option<GenerationInfo> {
    let generation_number = from_dir(generation_dir)?;
    // Get metadata once and reuse for both date and existence checks
    let metadata = fs::metadata(generation_dir).ok()?;
    let build_date = metadata
        .created()
        .or_else(|_| metadata.modified())
        .ok()
        .and_then(|system_time| Timestamp::try_from(system_time).ok())
        .map_or_else(
            || "Unknown".into(),
            |timestamp| timestamp.to_string(),
        );

    let nixos_version =
        fs::read_to_string(generation_dir.join("nixos-version"))
            .unwrap_or_else(|_| "Unknown".to_owned());

    // XXX: Nixpkgs appears to have changed where kernel modules are stored in a
    // recent change. I do not care to track which, but we should try the new path
    // and fall back to the old one IF and ONLY IF the new one fails. This is to
    // avoid breakage for outdated channels.
    let kernel_modules_dir_new =
        generation_dir.join("kernel-modules/lib/modules");
    let kernel_modules_dir_old = generation_dir
        .join("kernel")
        .canonicalize()
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| generation_dir.to_path_buf())
        .join("lib/modules");

    let kernel_version = if kernel_modules_dir_new.exists() {
        fs::read_dir(&kernel_modules_dir_new).map_or_else(
            |_| "Unknown".to_owned(),
            |entries| {
                let mut versions = Vec::with_capacity(4);
                for entry in entries.filter_map(std::result::Result::ok) {
                    if let Some(name) = entry.file_name().to_str() {
                        versions.push(name.to_owned());
                    }
                }
                versions.join(", ")
            },
        )
    } else if kernel_modules_dir_old.exists() {
        fs::read_dir(&kernel_modules_dir_old).map_or_else(
            |_| "Unknown".to_owned(),
            |entries| {
                let mut versions = Vec::with_capacity(4);
                for entry in entries.filter_map(std::result::Result::ok) {
                    if let Some(name) = entry.file_name().to_str() {
                        versions.push(name.to_owned());
                    }
                }
                versions.join(", ")
            },
        )
    } else {
        "Unknown".to_owned()
    };

    let configuration_revision = {
        let nixos_version_path =
            generation_dir.join("sw/bin/nixos-version");
        if nixos_version_path.exists() {
            process::Command::new(&nixos_version_path)
                .arg("--configuration-revision")
                .output()
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|revision| revision.trim().to_owned())
                .filter(|revision| !revision.is_empty())
        } else {
            None
        }
    };

    let specialisations = {
        let specialisation_path = generation_dir.join("specialisation");
        if specialisation_path.exists() {
            let specs = fs::read_dir(specialisation_path)
                .map(|entries| {
                    entries
                        .filter_map(std::result::Result::ok)
                        .filter_map(|entry| {
                            entry.file_name().into_string().ok()
                        })
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default();
            if specs.is_empty() { None } else { Some(specs) }
        } else {
            None
        }
    };

    // Check if this generation is the current one
    Some(GenerationInfo {
        number: generation_number,
        date: build_date,
        nixos_version,
        kernel_version,
        configuration_revision,
        specialisations,
        current: is_current(generation_dir),
        closure_size,
    })
}

/// Print information about the given generations.
#[expect(
    clippy::too_many_lines,
    reason = "field formatters stay together; splitting scatters the table layout"
)]
fn print_info(
    mut generations: Vec<GenerationInfo>,
    fields: Option<&[Field]>,
) {
    // Parse all dates at once and cache them
    let mut parsed_dates = HashMap::with_capacity(generations.len());
    for generation in &generations {
        let date = generation
            .date
            .parse()
            .unwrap_or(Timestamp::UNIX_EPOCH)
            .to_zoned(TimeZone::system());
        parsed_dates.insert(
            generation.date.clone(),
            date.strftime("%Y-%m-%d %H:%M:%S").to_string(),
        );
    }

    // Sort generations by numeric value of the generation number
    generations.sort_by_key(|generation| generation.number);

    let current_generation =
        generations.iter().find(|generation| generation.current);
    debug!(?current_generation);

    if let Some(current) = current_generation {
        println!("NixOS {}", current.nixos_version);
    } else {
        // Profile out of sync with /run/current-system.
        // This can happen if a previous switch failed during activation
        let fallback_version =
            fs::read_to_string(format!("{CURRENT_PROFILE}/nixos-version"))
                .unwrap_or_else(|_| "unknown".to_owned());
        warn!(
            "Profile is out of sync with /run/current-system. This may happen if a \
       previous switch failed during activation."
        );
        println!("NixOS {fallback_version} (profile may need sync)");
    }

    // Conditionally hide columns if they are empty for all generations
    let has_confrev = generations
        .iter()
        .any(|generation| generation.configuration_revision.is_some());
    let has_spec = generations
        .iter()
        .any(|generation| generation.specialisations.is_some());

    let visible_fields: Vec<Field> = fields.map_or_else(
        || {
            use Field::{Confrev, Date, Id, Kernel, Nver, Size, Spec};
            let all_fields = [Id, Date, Nver, Kernel, Confrev, Spec, Size];

            all_fields
                .into_iter()
                .filter(|field| match field {
                    Confrev => has_confrev,
                    Spec => has_spec,
                    Id | Date | Nver | Kernel | Size => true,
                })
                .collect()
        },
        <[Field]>::to_vec,
    );

    // Determine column widths for pretty printing
    let max_nixos_version_len = generations
        .iter()
        .map(|generation| generation.nixos_version.len())
        .max()
        .unwrap_or(22); // length of version + date + rev, assumes no tags

    let max_kernel_len = generations
        .iter()
        .map(|generation| generation.kernel_version.len())
        .max()
        .unwrap_or(12); // arbitrary value

    let max_generation_no_len = generations
        .iter()
        .map(|generation| generation.number.to_string().len())
        .max()
        .unwrap_or(5);

    let widths = ColumnWidths {
        id: max_generation_no_len + 10, // "Generation No"
        date: 20,                       // "Build Date"
        nver: max_nixos_version_len,
        kernel: max_kernel_len,
        confrev: 22, // "Configuration Revision"
        spec: 15,    // "Specialisations"
        size: 12,    // "Closure Size"
    };

    let header = visible_fields
        .iter()
        .map(|field| {
            let (name, width) = field.column_info(widths);
            format!("{name:<width$}")
        })
        .collect::<Vec<String>>()
        .join(" ");
    println!("{header}");

    // Print generations in descending order
    for generation in generations.iter().rev() {
        let formatted_date = parsed_dates
            .get(&generation.date)
            .cloned()
            .unwrap_or_else(|| "Unknown".to_owned());

        let specialisations =
            generation.specialisations.as_ref().map(|specs| {
                specs
                    .iter()
                    .map(|name| format!("*{name}"))
                    .collect::<Vec<String>>()
                    .join(" ")
            });

        let row: String = visible_fields
            .iter()
            .map(|field| {
                let (_, width) = field.column_info(widths);
                let cell_content = match field {
                    Field::Id => {
                        format!(
                            "{}{}",
                            generation.number,
                            if generation.current {
                                " (current)"
                            } else {
                                ""
                            }
                        )
                    }
                    Field::Date => formatted_date.clone(),
                    Field::Nver => generation.nixos_version.clone(),
                    Field::Kernel => generation.kernel_version.clone(),
                    Field::Confrev => generation
                        .configuration_revision
                        .clone()
                        .unwrap_or_default(),
                    Field::Spec => {
                        specialisations.clone().unwrap_or_default()
                    }
                    Field::Size => generation.closure_size.clone(),
                };
                format!("{cell_content:width$}")
            })
            .collect::<Vec<String>>()
            .join(" ");
        println!("{row}");
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, clippy::panic, reason = "Test assertions")]
mod tests {
    use std::path::PathBuf;

    use bpaf::{Args, Parser as _};

    use super::CliOpts;
    use super::Field;
    use super::cli;

    #[test]
    fn fields_split_on_commas() {
        let options = cli().to_options();
        options.check_invariants(false);
        let opts = options
            .run_inner(
                Args::from(&["--fields", "id,confRev,date"][..])
                    .set_name("test"),
            )
            .unwrap();

        let Some(fields) = opts.fields else {
            panic!("fields must be present");
        };
        assert!(matches!(
            fields.as_slice(),
            [Field::Id, Field::Confrev, Field::Date]
        ));
    }

    #[test]
    fn fields_have_canonical_default_profile() {
        let options = cli().to_options();
        let CliOpts { profile, fields } = options
            .run_inner(Args::from(&[] as &[&str]).set_name("test"))
            .unwrap();

        assert!(fields.is_none());
        assert_eq!(profile, PathBuf::from("/nix/var/nix/profiles/system"));
    }
}
