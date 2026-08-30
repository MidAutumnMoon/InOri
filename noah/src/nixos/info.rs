use std::fs;
use std::path::{Path, PathBuf};

use rootcause::{Result, report};

use super::generations;
use super::request::GenerationsRequest;
pub(super) fn run(request: &GenerationsRequest) -> Result<()> {
    let profile = &request.profile;

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

    let gen_dir_refs: Vec<&std::path::Path> =
        generations.iter().map(PathBuf::as_path).collect();
    let closure_sizes =
        generations::get_closure_sizes_batch(&gen_dir_refs);

    let descriptions: Vec<generations::GenerationInfo> = generations
        .iter()
        .filter_map(|gen_dir| {
            let size = closure_sizes
                .get(gen_dir)
                .cloned()
                .unwrap_or_else(|| String::from("Unknown"));
            generations::describe(gen_dir, size)
        })
        .collect();
    generations::print_info(descriptions, request.fields.as_deref())?;

    Ok(())
}
