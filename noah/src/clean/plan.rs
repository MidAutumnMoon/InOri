use std::path::PathBuf;
use std::time::SystemTime;

use inquire::Confirm;
use nix::errno::Errno;
use nix::fcntl::{self, AtFlags};
use nix::sys::stat;
use nix::unistd::{AccessFlags, Uid, User, faccessat};
use regex::Regex;
use rootcause::{Result, bail, prelude::ResultExt as _, report};
use tracing::{Level, debug, span, warn};
use walkdir::WalkDir;
use yansi::{Color, Paint};

use super::{
    CliOpts, DIRENV_REGEX, GcRootTagged, Options, ProfilesTagged, Scope,
    cleanable_generations, filter_existing_dirs, gcroot_matches_filter,
    gcroot_path_to_remove, profiles_in_dir, remove_path_nofail,
};
use crate::command::Command;
use crate::runtime::Config;

struct CleanPlan {
    profiles: ProfilesTagged,
    gcroots: Vec<GcRootTagged>,
    orphan_gcroots: Vec<PathBuf>,
}

pub(super) fn run(opts: &CliOpts, config: &Config) -> Result<()> {
    let plan = CleanPlan::build(opts, config)?;
    plan.render(&opts.options);

    if opts.options.ask
        && !Confirm::new("Confirm the cleanup plan?")
            .with_default(false)
            .prompt()?
    {
        bail!("User rejected the cleanup plan");
    }

    plan.apply(&opts.options, config)
}

impl CleanPlan {
    fn build(opts: &CliOpts, config: &Config) -> Result<Self> {
        let options = &opts.options;
        let mut profiles = Vec::new();
        let mut gcroots = Vec::new();
        let mut orphan_gcroots = Vec::new();
        let now = SystemTime::now();
        let mut profile_only = false;

        let uid = Uid::effective();
        match &opts.scope {
            Scope::Profile(profile) => {
                profiles.push(profile.clone());
                profile_only = true;
            }
            Scope::All => {
                if !uid.is_root() {
                    Command::self_elevate(&config.elevation, &config.env);
                }

                let paths_to_check = [
                    PathBuf::from("/nix/var/nix/profiles"),
                    PathBuf::from("/nix/var/nix/profiles/per-user"),
                ];

                profiles.extend(
                    filter_existing_dirs(paths_to_check).flat_map(
                        |path| {
                            if path.ends_with("per-user") {
                                path.read_dir()
                                    .map(|read_dir| {
                                        read_dir
                                            .filter_map(
                                                std::result::Result::ok,
                                            )
                                            .map(|entry| entry.path())
                                            .filter(|entry_path| {
                                                entry_path.is_dir()
                                            })
                                            .flat_map(profiles_in_dir)
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default()
                            } else {
                                profiles_in_dir(path)
                            }
                        },
                    ),
                );

                let uid_min = 1000;
                let uid_max = uid_min + 100;
                debug!(
                    "Scanning XDG profiles for users 0, {uid_min}-{uid_max}"
                );

                if let Some(user) = User::from_uid(Uid::from_raw(0))? {
                    debug!(?user, "Adding XDG profiles for root user");
                    let user_profiles_path =
                        user.dir.join(".local/state/nix/profiles");
                    if user_profiles_path.is_dir() {
                        profiles
                            .extend(profiles_in_dir(user_profiles_path));
                    }
                }

                for candidate_uid in uid_min..uid_max {
                    if let Some(user) =
                        User::from_uid(Uid::from_raw(candidate_uid))?
                    {
                        debug!(?user, "Adding XDG profiles for user");
                        let user_profiles_path =
                            user.dir.join(".local/state/nix/profiles");
                        if user_profiles_path.is_dir() {
                            profiles.extend(profiles_in_dir(
                                user_profiles_path,
                            ));
                        }
                    }
                }
            }
            Scope::User => {
                if uid.is_root() {
                    bail!("nh clean user: don't run me as root!");
                }
                let user = User::from_uid(uid)?.ok_or_else(|| {
                    report!("User not found for uid {}", uid)
                })?;
                let home_dir = user.dir;
                let paths_to_check = [
                    home_dir.join(".local/state/nix/profiles"),
                    PathBuf::from("/nix/var/nix/profiles/per-user")
                        .join(&user.name),
                ];

                profiles.extend(
                    filter_existing_dirs(paths_to_check)
                        .flat_map(profiles_in_dir),
                );

                if profiles.is_empty() {
                    warn!(
                        "No active profile directories found for the current user. \
                         Nothing to clean."
                    );
                }
            }
        }

        let mut tagged_profiles = ProfilesTagged::new();
        for path in profiles {
            tagged_profiles.insert(
                path.clone(),
                cleanable_generations(
                    &path,
                    options.keep,
                    options.keep_since,
                )?,
            );
        }

        let regexes: &[&Regex] = if options.no_direnv {
            &[]
        } else {
            &[&*DIRENV_REGEX]
        };

        if !profile_only && !options.no_gcroots {
            let dirfd = fcntl::open(
                ".",
                fcntl::OFlag::O_DIRECTORY,
                stat::Mode::empty(),
            )?;

            for entry in WalkDir::new("/nix/var/nix/gcroots")
                .follow_links(false)
                .same_file_system(!options.cross_filesystems)
                .into_iter()
                .filter_map(|entry| {
                    entry
                        .map_err(|error| {
                            warn!(?error, "gcroot walk error");
                        })
                        .ok()
                })
                .filter(|entry| entry.path().is_symlink())
            {
                let src = entry.path().to_path_buf();
                let dst = src
                    .read_link()
                    .context("Reading symlink destination")?;
                let span = span!(Level::TRACE, "gcroot detection", ?dst);
                let _entered = span.enter();
                debug!(?src);

                if !dst.is_symlink() && !dst.exists() {
                    debug!(
                        ?src,
                        "gcroot is orphaned (dst missing), tagging for removal"
                    );
                    orphan_gcroots.push(src);
                    continue;
                }

                match faccessat(
                    &dirfd,
                    &dst,
                    AccessFlags::F_OK | AccessFlags::W_OK,
                    AtFlags::AT_SYMLINK_NOFOLLOW,
                ) {
                    Ok(()) => {
                        if dst.metadata().is_err() {
                            debug!(
                                ?dst,
                                "gcroot target already GC'd, tagging for removal"
                            );
                            gcroots.push(GcRootTagged {
                                src,
                                dst,
                                tbr: true,
                            });
                            continue;
                        }

                        if !gcroot_matches_filter(&src, &dst, regexes) {
                            debug!(
                                "dst doesn't match any gcroot filter, skipping"
                            );
                            continue;
                        }

                        if options.keep_one
                            && DIRENV_REGEX
                                .is_match(&dst.to_string_lossy())
                        {
                            gcroots.push(GcRootTagged {
                                src,
                                dst,
                                tbr: false,
                            });
                        } else {
                            let duration = now.duration_since(
                                dst.symlink_metadata()
                                    .context("Reading gcroot metadata")?
                                    .modified()?,
                            );
                            debug!(?duration);
                            match duration {
                                Err(error) => {
                                    warn!(
                                        ?error,
                                        ?now,
                                        "Failed to compare time!"
                                    );
                                }
                                Ok(value)
                                    if value
                                        <= std::time::Duration::from(
                                            options.keep_since,
                                        ) =>
                                {
                                    gcroots.push(GcRootTagged {
                                        src,
                                        dst,
                                        tbr: false,
                                    });
                                }
                                Ok(_) => {
                                    gcroots.push(GcRootTagged {
                                        src,
                                        dst,
                                        tbr: true,
                                    });
                                }
                            }
                        }
                    }
                    Err(Errno::ENOENT) => {
                        debug!(
                            ?src,
                            "gcroot is orphaned (dst missing), tagging for removal"
                        );
                        orphan_gcroots.push(src);
                    }
                    Err(
                        error @ (Errno::EACCES
                        | Errno::EROFS
                        | Errno::EPERM),
                    ) => {
                        debug!(
                            ?error,
                            ?dst,
                            "gcroot target not writable, skipping"
                        );
                    }
                    Err(error) => {
                        bail!(
                            report!(
                                "Checking access for gcroot {:?}, unknown error",
                                dst
                            )
                            .context(error)
                        );
                    }
                }
            }
        }

        Ok(Self {
            profiles: tagged_profiles,
            gcroots,
            orphan_gcroots,
        })
    }

    fn render(&self, options: &Options) {
        let regexes: &[&Regex] = if options.no_direnv {
            &[]
        } else {
            &[&*DIRENV_REGEX]
        };

        println!();
        println!("{}", Paint::new("Welcome to nh clean").bold());
        println!(
            "Keeping {} generation(s)",
            Paint::new(options.keep).fg(Color::Green)
        );
        println!(
            "Keeping paths newer than {}",
            Paint::new(options.keep_since).fg(Color::Green)
        );
        if options.keep_one {
            println!("Keeping all active direnv gcroots");
        }
        if options.no_direnv {
            println!("Skipping all direnv gcroots");
        }
        println!();
        println!("legend:");
        println!(
            "{}: path regular expression to be matched",
            Paint::new("RE").fg(Color::Magenta)
        );
        println!("{}: path to be kept", Paint::new("OK").fg(Color::Green));
        println!(
            "{}: path to be removed",
            Paint::new("DEL").fg(Color::Red)
        );
        println!();

        if !self.orphan_gcroots.is_empty() {
            println!(
                "{}",
                Paint::new("orphaned gcroots").fg(Color::Blue).bold()
            );
            for path in &self.orphan_gcroots {
                println!(
                    "- {} {}",
                    Paint::new("DEL").fg(Color::Red),
                    path.to_string_lossy()
                );
            }
            println!();
        }

        if !self.gcroots.is_empty() {
            println!("{}", Paint::new("gcroots").fg(Color::Blue).bold());
            for regex in regexes {
                println!(
                    "- {}  {}",
                    Paint::new("RE").fg(Color::Magenta),
                    regex.as_str()
                );
            }
            println!(
                "- {}  /nix/store direct children",
                Paint::new("RE").fg(Color::Magenta)
            );
            for gcroot in &self.gcroots {
                if gcroot.tbr {
                    println!(
                        "- {} {}",
                        Paint::new("DEL").fg(Color::Red),
                        gcroot.dst.to_string_lossy()
                    );
                } else {
                    println!(
                        "- {} {}",
                        Paint::new("OK ").fg(Color::Green),
                        gcroot.dst.to_string_lossy()
                    );
                }
            }
            println!();
        }

        #[expect(
            clippy::iter_over_hash_type,
            reason = "Don't care about the ordering"
        )]
        for (profile, generations) in &self.profiles {
            println!(
                "{}",
                Paint::new(profile.to_string_lossy())
                    .fg(Color::Blue)
                    .bold()
            );
            for (generation, remove) in generations.iter().rev() {
                if *remove {
                    println!(
                        "- {} {}",
                        Paint::new("DEL").fg(Color::Red),
                        generation.path.to_string_lossy()
                    );
                } else {
                    println!(
                        "- {} {}",
                        Paint::new("OK ").fg(Color::Green),
                        generation.path.to_string_lossy()
                    );
                }
            }
            println!();
        }
    }

    fn apply(&self, options: &Options, config: &Config) -> Result<()> {
        if !options.dry {
            for gcroot in &self.gcroots {
                if gcroot.tbr {
                    remove_path_nofail(gcroot_path_to_remove(gcroot));
                }
            }

            for path in &self.orphan_gcroots {
                remove_path_nofail(path);
            }

            #[expect(
                clippy::iter_over_hash_type,
                reason = "Don't care about the ordering"
            )]
            for generations in self.profiles.values() {
                for (generation, remove) in generations.iter().rev() {
                    if *remove {
                        remove_path_nofail(&generation.path);
                    }
                }
            }
        }

        if !options.no_gc {
            let mut gc_args = vec!["store", "gc"];
            if let Some(max) = &options.max {
                gc_args.push("--max");
                gc_args.push(max.as_str());
            }
            Command::new("nix", &config.env, &config.elevation)
                .args(gc_args)
                .dry(options.dry)
                .message("Performing garbage collection on the nix store")
                .show_output(true)
                .run()?;
        }

        if options.optimise {
            Command::new("nix-store", &config.env, &config.elevation)
                .arg("--optimise")
                .dry(options.dry)
                .message("Optimising the nix store")
                .show_output(true)
                .run()?;
        }

        Ok(())
    }
}
