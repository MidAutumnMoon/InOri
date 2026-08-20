use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[expect(non_snake_case, clippy::struct_field_names)]
pub struct PackageSearchResult {
    // r#type: String,
    pub package_attr_name: String,
    pub package_attr_set: String,
    pub package_pname: String,
    pub package_pversion: String,
    pub package_platforms: Vec<String>,
    pub package_outputs: Vec<String>,
    pub package_default_output: Option<String>,
    pub package_programs: Vec<String>,
    pub package_mainProgram: Option<String>,
    // package_license: Vec<License>,
    pub package_license_set: Vec<String>,
    // package_maintainers: Vec<HashMap<String, String>>,
    pub package_description: Option<String>,
    pub package_longDescription: Option<String>,
    pub package_hydra: (),
    pub package_system: String,
    pub package_homepage: Vec<String>,
    pub package_position: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OptionSearchResult {
    pub r#type: String,
    pub option_name: String,
    pub option_description: Option<String>,
    pub option_type: Option<String>,
    pub option_default: Option<String>,
    pub option_example: Option<String>,
    pub option_source: Option<String>,
    pub option_flake: Option<Vec<String>>,
    pub flake_name: Option<String>,
    pub flake_description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JsonOutput<'a, T> {
    pub query: &'a str,
    pub channel: &'a str,
    pub elapsed_ms: u128,
    pub results: &'a [T],
}
