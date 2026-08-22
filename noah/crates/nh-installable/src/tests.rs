use std::fs;

use super::*;

fn specified(installable: Installable) -> InstallableArgs {
    InstallableArgs::Specified(installable)
}

#[test]
fn resolve_non_unspecified_returns_unchanged() {
    let config = FlakeConfig::default();
    let flake = Installable::Flake {
        reference: String::from("/path/to/flake"),
        attribute: vec![String::from("host")],
    };
    let resolved =
        specified(flake.clone()).resolve(&config).unwrap().unwrap();
    assert_eq!(flake.to_args(), resolved.to_args());

    let file = Installable::File {
        path: PathBuf::from("/path/to/file.nix"),
        attribute: vec![String::from("config")],
    };
    let resolved =
        specified(file.clone()).resolve(&config).unwrap().unwrap();
    assert_eq!(file.to_args(), resolved.to_args());

    let store = Installable::Store {
        path: PathBuf::from("/nix/store/abc"),
    };
    let resolved =
        specified(store.clone()).resolve(&config).unwrap().unwrap();
    assert_eq!(store.to_args(), resolved.to_args());

    let expr = Installable::Expression {
        expression: String::from("{ pkgs }: pkgs.hello"),
        attribute: vec![],
    };
    let resolved =
        specified(expr.clone()).resolve(&config).unwrap().unwrap();
    assert_eq!(expr.to_args(), resolved.to_args());
}

#[test]
fn resolve_or_default_non_unspecified_returns_unchanged() {
    let config = FlakeConfig::default();
    let flake = Installable::Flake {
        reference: String::from("github:user/repo"),
        attribute: vec![String::from("host")],
    };

    let resolved = specified(flake.clone())
        .resolve_or_default(&config)
        .unwrap();

    assert_eq!(flake.to_args(), resolved.to_args());
}

#[test]
fn resolve_or_default_uses_env_before_default() {
    let flake_dir = tempfile::tempdir().unwrap();
    fs::write(flake_dir.path().join("flake.nix"), "{}").unwrap();
    let config = FlakeConfig {
        os_flake: Some(format!("{}#myhost", flake_dir.path().display())),
        ..Default::default()
    };

    let resolved = InstallableArgs::Unspecified
        .resolve_or_default(&config)
        .unwrap();

    match resolved {
        Installable::Flake {
            reference,
            attribute,
        } => {
            assert_eq!(reference, flake_dir.path().to_string_lossy());
            assert_eq!(attribute, vec!["myhost"]);
        }
        _ => panic!("Expected Flake, got {resolved:?}"),
    }
}

#[test]
fn resolve_or_default_accepts_existing_local_flake_path() {
    let config = FlakeConfig::default();
    let flake_dir = tempfile::tempdir().unwrap();
    fs::write(flake_dir.path().join("flake.nix"), "{}").unwrap();

    let installable = Installable::Flake {
        reference: flake_dir.path().to_string_lossy().into_owned(),
        attribute: vec![],
    };

    let resolved =
        specified(installable).resolve_or_default(&config).unwrap();

    assert_eq!(
        resolved.to_args(),
        vec![format!("{}#", flake_dir.path().display())]
    );
}

#[test]
fn resolve_or_default_rejects_missing_absolute_path() {
    let config = FlakeConfig::default();
    let parent = tempfile::tempdir().unwrap();
    let missing_path = parent.path().join("missing-flake");
    assert!(!missing_path.exists());

    let installable = Installable::Flake {
        reference: missing_path.to_string_lossy().into_owned(),
        attribute: vec![],
    };

    let err = specified(installable)
        .resolve_or_default(&config)
        .unwrap_err()
        .to_string();

    assert!(err.contains("Flake reference"));
    assert!(
        err.contains("does not exist or does not contain a flake.nix")
    );
    assert!(err.contains("NH_FLAKE/NH_OS_FLAKE"));
}

#[test]
fn resolve_or_default_rejects_existing_dir_without_flake_nix() {
    let config = FlakeConfig::default();
    let dir = tempfile::tempdir().unwrap();

    let installable = Installable::Flake {
        reference: dir.path().to_string_lossy().into_owned(),
        attribute: vec![],
    };

    let err = specified(installable)
        .resolve_or_default(&config)
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("does not exist or does not contain a flake.nix")
    );
}

#[test]
fn resolve_or_default_rejects_subdir_inside_flake() {
    let config = FlakeConfig::default();
    let flake_dir = tempfile::tempdir().unwrap();
    fs::write(flake_dir.path().join("flake.nix"), "{}").unwrap();
    let subdir = flake_dir.path().join("modules");
    fs::create_dir_all(&subdir).unwrap();

    let installable = Installable::Flake {
        reference: subdir.to_string_lossy().into_owned(),
        attribute: vec![],
    };

    let err = specified(installable)
        .resolve_or_default(&config)
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("does not exist or does not contain a flake.nix")
    );
}

#[test]
fn resolve_or_default_rejects_missing_path_scheme() {
    let config = FlakeConfig::default();
    let parent = tempfile::tempdir().unwrap();
    let missing_path = parent.path().join("missing-flake");
    assert!(!missing_path.exists());

    let installable = Installable::Flake {
        reference: format!("path:{}", missing_path.display()),
        attribute: vec![],
    };

    let err = specified(installable)
        .resolve_or_default(&config)
        .unwrap_err()
        .to_string();

    assert!(err.contains("NH_FLAKE/NH_OS_FLAKE"));
}

#[test]
fn resolve_or_default_defers_parameterized_local_flake_refs_to_nix() {
    let config = FlakeConfig::default();
    let source_dir = tempfile::tempdir().unwrap();

    for reference in [
        format!("path:{}?lastModified=1", source_dir.path().display()),
        format!("path:{}?dir=nix/flakes", source_dir.path().display()),
        format!("{}?submodules=1", source_dir.path().display()),
    ] {
        let installable = Installable::Flake {
            reference: reference.clone(),
            attribute: vec![],
        };

        let resolved =
            specified(installable).resolve_or_default(&config).unwrap();

        assert_eq!(resolved.to_args(), vec![format!("{reference}#")]);
    }
}

#[test]
fn resolve_or_default_ignores_registry_and_url_refs() {
    let config = FlakeConfig::default();
    for reference in ["nixpkgs", "github:NixOS/nixpkgs"] {
        let installable = Installable::Flake {
            reference: reference.to_owned(),
            attribute: vec![],
        };

        specified(installable).resolve_or_default(&config).unwrap();
    }
}

#[test]
fn resolve_rejects_empty_nh_flake() {
    let config = FlakeConfig {
        flake: Some(String::new()),
        ..Default::default()
    };

    let err = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap_err()
        .to_string();

    assert!(err.contains("NH_FLAKE is empty"));
}

#[test]
fn resolve_rejects_empty_command_specific_flake() {
    let config = FlakeConfig {
        os_flake: Some(String::new()),
        flake: Some(String::from("github:user/repo")),
        ..Default::default()
    };

    let err = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap_err()
        .to_string();

    assert!(err.contains("NH_OS_FLAKE is empty"));
}

#[test]
fn resolve_rejects_env_flake_without_reference_before_attribute() {
    let config = FlakeConfig {
        flake: Some(String::from("#fallback")),
        ..Default::default()
    };

    let err = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap_err()
        .to_string();

    assert!(err.contains("NH_FLAKE missing reference part before `#`"));
}

#[test]
fn resolve_rejects_malformed_nh_attrp() {
    let config = FlakeConfig {
        file: Some(String::from("/path/to/file.nix")),
        attrp: String::from(r#"foo."bar"#),
        ..Default::default()
    };

    let err = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("NH_ATTRP contains an unclosed quoted attribute")
    );
}

#[test]
fn cli_installable_rejects_empty_flake_reference() {
    let cmd = InstallableArgs::augment_args(clap::Command::new("test"));
    let err = InstallableArgs::from_arg_matches(
        &cmd.try_get_matches_from(["test", ""]).unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert!(err.contains("installable argument is empty"));
}

#[test]
fn cli_installable_rejects_attribute_without_reference() {
    let cmd = InstallableArgs::augment_args(clap::Command::new("test"));
    let err = InstallableArgs::from_arg_matches(
        &cmd.try_get_matches_from(["test", "#fallback"]).unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert!(err.contains(
        "installable argument missing reference part before `#`"
    ));
}

#[test]
fn cli_file_rejects_malformed_attribute() {
    let cmd = InstallableArgs::augment_args(clap::Command::new("test"));
    let matches = cmd
        .try_get_matches_from([
            "test",
            "--file",
            "file.nix",
            r#"foo."bar"#,
        ])
        .unwrap();
    let err = InstallableArgs::from_arg_matches(&matches)
        .unwrap_err()
        .to_string();

    assert!(
        err.contains(
            "attribute path contains an unclosed quoted attribute"
        )
    );
}

#[test]
fn uses_flakes_checks_cli_and_env_inputs() {
    let config = FlakeConfig::default();

    assert!(!InstallableArgs::Unspecified.uses_flakes(&config));

    let file = specified(Installable::File {
        path: PathBuf::from("/path/to/file.nix"),
        attribute: vec![],
    });
    assert!(!file.uses_flakes(&config));

    let flake = specified(Installable::Flake {
        reference: String::from("github:user/repo"),
        attribute: vec![],
    });
    assert!(flake.uses_flakes(&config));

    let config = FlakeConfig {
        flake: Some(String::from("github:user/repo")),
        ..Default::default()
    };
    assert!(InstallableArgs::Unspecified.uses_flakes(&config));
}

#[test]
fn uses_flakes_respects_resolution_precedence() {
    let config = FlakeConfig {
        flake: Some(String::from("github:user/repo")),
        ..Default::default()
    };

    let file = specified(Installable::File {
        path: PathBuf::from("/path/to/file.nix"),
        attribute: vec![],
    });
    assert!(!file.uses_flakes(&config));

    let config = FlakeConfig {
        flake: Some(String::from("github:user/repo")),
        file: Some(String::from("/path/to/file.nix")),
        ..Default::default()
    };
    assert!(!InstallableArgs::Unspecified.uses_flakes(&config));

    let config = FlakeConfig {
        flake: Some(String::from("github:user/repo")),
        file: Some(String::from("/path/to/file.nix")),
        os_flake: Some(String::from("github:user/os")),
        ..Default::default()
    };
    assert!(InstallableArgs::Unspecified.uses_flakes(&config));
}

#[test]
fn uses_flakes_ignores_empty_env_values() {
    // Application startup filters empty values before constructing this model.
    // Constructing empty fields directly still must not select a flake source.
    let config = FlakeConfig {
        os_flake: Some(String::new()),
        flake: Some(String::new()),
        ..Default::default()
    };

    assert!(!InstallableArgs::Unspecified.uses_flakes(&config));
}

#[test]
fn resolve_os_context_uses_nh_os_flake() {
    let config = FlakeConfig {
        os_flake: Some(String::from("/etc/nixos#myhost")),
        ..Default::default()
    };

    let resolved = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap()
        .unwrap();
    match resolved {
        Installable::Flake {
            reference,
            attribute,
        } => {
            assert_eq!(reference, "/etc/nixos");
            assert_eq!(attribute, vec!["myhost"]);
        }
        _ => panic!("Expected Flake, got {resolved:?}"),
    }
}

#[test]
fn resolve_os_context_prefers_os_flake_over_generic() {
    let config = FlakeConfig {
        os_flake: Some(String::from("/etc/nixos#myhost")),
        flake: Some(String::from("/home/user/flake#other")),
        ..Default::default()
    };

    let resolved = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap()
        .unwrap();
    match resolved {
        Installable::Flake {
            reference,
            attribute,
        } => {
            assert_eq!(reference, "/etc/nixos");
            assert_eq!(attribute, vec!["myhost"]);
        }
        _ => panic!("Expected Flake, got {resolved:?}"),
    }
}

#[test]
fn resolve_os_context_falls_back_to_nh_flake() {
    let config = FlakeConfig {
        flake: Some(String::from("/home/user/flake#fallback")),
        ..Default::default()
    };

    let resolved = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap()
        .unwrap();
    match resolved {
        Installable::Flake {
            reference,
            attribute,
        } => {
            assert_eq!(reference, "/home/user/flake");
            assert_eq!(attribute, vec!["fallback"]);
        }
        _ => panic!("Expected Flake, got {resolved:?}"),
    }
}

#[test]
fn resolve_no_env_vars_returns_unspecified() {
    let config = FlakeConfig::default();

    let resolved = InstallableArgs::Unspecified.resolve(&config).unwrap();
    assert!(resolved.is_none());
}

#[test]
fn resolve_with_empty_attribute() {
    let config = FlakeConfig {
        os_flake: Some(String::from("/etc/nixos")),
        ..Default::default()
    };

    let resolved = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap()
        .unwrap();
    match resolved {
        Installable::Flake {
            reference,
            attribute,
        } => {
            assert_eq!(reference, "/etc/nixos");
            assert!(attribute.is_empty());
        }
        _ => panic!("Expected Flake, got {resolved:?}"),
    }
}

#[test]
fn resolve_with_nested_attribute() {
    let config = FlakeConfig {
        os_flake: Some(String::from(
            "/etc/nixos#nixosConfigurations.myhost",
        )),
        ..Default::default()
    };

    let resolved = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap()
        .unwrap();
    match resolved {
        Installable::Flake {
            reference,
            attribute,
        } => {
            assert_eq!(reference, "/etc/nixos");
            assert_eq!(attribute, vec!["nixosConfigurations", "myhost"]);
        }
        _ => panic!("Expected Flake, got {resolved:?}"),
    }
}

#[test]
fn resolve_command_specific_isolation() {
    let config = FlakeConfig {
        os_flake: Some(String::from("/etc/nixos#myhost")),
        ..Default::default()
    };

    // OS-specific flake should be used by Os context
    let resolved = InstallableArgs::Unspecified
        .resolve(&config)
        .unwrap()
        .unwrap();
    match resolved {
        Installable::Flake {
            reference,
            attribute,
        } => {
            assert_eq!(reference, "/etc/nixos");
            assert_eq!(attribute, vec!["myhost"]);
        }
        _ => panic!("Expected Flake, got {resolved:?}"),
    }
}
