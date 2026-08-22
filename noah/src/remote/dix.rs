use std::path::Path;
use std::path::PathBuf;

use rootcause::Result;
use rootcause::bail;
use rootcause::report;

use super::Host;
use super::SshConfig;
use super::get_nix_sshopts_env;
use super::run_remote_command;
use crate::external_report;

/// A remote store path after resolving symlinks such as
/// `/run/current-system`.
#[derive(Debug, Clone)]
pub struct ResolvedRemoteStorePath {
    host: Host,
    path: PathBuf,
}

impl ResolvedRemoteStorePath {
    /// Resolve a remote path to the store path that should be queried.
    ///
    /// Direct store entries are returned unchanged. Other paths are resolved on
    /// the remote host with `readlink -f` and validated as Nix store paths.
    ///
    /// # Errors
    ///
    /// Returns an error if the remote path cannot be resolved or resolves outside
    /// `/nix/store`.
    pub fn resolve(
        host: &Host,
        path: &Path,
        ssh_config: &SshConfig,
    ) -> Result<Self> {
        if path.parent() == Some(Path::new("/nix/store")) {
            return Ok(Self {
                host: host.clone(),
                path: path.to_path_buf(),
            });
        }

        let path = path.to_str().ok_or_else(|| {
            report!("remote path contains invalid UTF-8")
        })?;
        let output = run_remote_command(
            host,
            &["readlink", "-f", "--", path],
            true,
            ssh_config,
        )?
        .ok_or_else(|| {
            report!("readlink did not return a resolved path")
        })?;
        let mut paths = output.lines();
        let resolved_path = paths.next().ok_or_else(|| {
            report!("readlink did not return a resolved path")
        })?;

        if paths.next().is_some() {
            bail!(
                "readlink returned multiple paths for one requested path"
            );
        }

        Self::new(host, PathBuf::from(resolved_path), path)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Query this resolved remote Nix store path and convert it to dix's snapshot
    /// model.
    ///
    /// # Errors
    ///
    /// Returns an error if Nix cannot query the remote store or returns invalid
    /// JSON/path data.
    pub fn query_snapshot(
        &self,
        ssh_config: &SshConfig,
    ) -> Result<dix::StoreSnapshot> {
        let backend = dix::CommandBackend::default()
            .store_url(self.host.nix_store_uri())
            .env("NIX_SSHOPTS", get_nix_sshopts_env(ssh_config));
        dix::query_store_snapshot_with_backend(&backend, self.path())
            .map_err(external_report)
    }

    fn new(host: &Host, path: PathBuf, original: &str) -> Result<Self> {
        if !path.starts_with("/nix/store") {
            bail!(
                "resolved remote path '{}' for '{}' is not in /nix/store",
                path.display(),
                original
            );
        }

        Ok(Self {
            host: host.clone(),
            path,
        })
    }
}

#[cfg(test)]
#[expect(clippy::panic_in_result_fn, reason = "tests")]
mod tests {
    use super::*;

    const BASH: &str =
        "/nix/store/2123456789abcdefghijklmnopqrstuv-bash-5.3";

    #[test]
    fn resolved_remote_store_path_preserves_direct_store_entry()
    -> Result<()> {
        let host = Host::parse("target.example")?;

        let root = ResolvedRemoteStorePath::resolve(
            &host,
            Path::new(BASH),
            &SshConfig::default(),
        )?;

        assert_eq!(root.path(), Path::new(BASH));

        Ok(())
    }
}
