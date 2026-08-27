use std::path::Path;
use std::str::FromStr;

use anyhow::Context as _;
use anyhow::Result as AnyResult;
use anyhow::bail;
use anyhow::ensure;
use itertools::Itertools as _;
use serdev::Deserialize;
use tap::Tap as _;
use tracing::debug;
use tracing::trace;

use crate::template::RenderedPath;

const CURRENT_BLUEPRINT_VERSION: usize = 1;

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[serde(validate = "Self::validate")]
pub struct Blueprint {
    version: usize,
    symlinks: Vec<Symlink>,
}

impl Blueprint {
    #[tracing::instrument]
    pub fn from_file(path: &Path) -> AnyResult<Self> {
        debug!("read the blueprint file");
        ensure! { path.is_file(),
            r#"The given path "{}" is not file"#, path.display()
        };
        let raw = std::fs::read_to_string(path)
            .context("Failed to read blueprint file")?;
        Self::from_str(&raw)
            .context("Failed to parse the blueprint's content")
    }

    /// Consume the blueprint, yielding its symlink declarations.
    pub fn into_symlinks(self) -> Vec<Symlink> {
        self.symlinks
    }

    #[tracing::instrument(skip_all)]
    fn validate(&self) -> AnyResult<()> {
        debug!("validate the blueprint");
        ensure! {
            self.version == CURRENT_BLUEPRINT_VERSION,
            r#"Blueprint version mismatch, expect "{}", got "{}""#,
            CURRENT_BLUEPRINT_VERSION,
            self.version
        };
        if let Some([left, right]) = self
            .symlinks
            .iter()
            .array_combinations::<2>()
            .find(|[left, right]| left.dst_overlaps(right))
        {
            bail!(
                r#"Conflicting symlink destinations "{}" and "{}" overlap"#,
                left.dst.display(),
                right.dst.display()
            );
        }
        ensure! {
            self.symlinks
                .iter()
                .all(|it| it.src != it.dst),
            "Some symlinks have identical src and dst, \
                which would produce a self-referential symlink"
        };
        Ok(())
    }
}

impl FromStr for Blueprint {
    type Err = anyhow::Error;

    #[tracing::instrument(skip_all)]
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        debug!("try parse the input as json");
        Ok(serde_json::from_str::<Self>(raw)
            .context("Blueprint contains invalid JSON")?
            .tap(|blueprint| trace!(?blueprint)))
    }
}

#[derive(Deserialize, Debug, Clone)]
#[derive(PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Symlink {
    pub src: RenderedPath,
    pub dst: RenderedPath,
}

impl Symlink {
    pub fn same_dst(&self, other: &Self) -> bool {
        self.dst == other.dst
    }

    pub fn dst_is_ancestor_of(&self, other: &Self) -> bool {
        self.dst != other.dst && other.dst.starts_with(&self.dst)
    }

    pub fn dst_overlaps(&self, other: &Self) -> bool {
        self.same_dst(other)
            || self.dst_is_ancestor_of(other)
            || other.dst_is_ancestor_of(self)
    }

    pub fn same_src(&self, other: &Self) -> bool {
        self.src == other.src
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Tests")]
#[expect(clippy::expect_used, reason = "Tests")]
mod test {

    use super::*;
    use serde::de::IntoDeserializer as _;

    #[test]
    fn symlinks_are_unique() {
        let json = serde_json::json! { {
            "version": CURRENT_BLUEPRINT_VERSION,
            "symlinks": [
                { "src": "/a", "dst": "/tar" },
                { "src": "/b", "dst": "/tar" },
            ]
        } };
        let error = Blueprint::deserialize(json.into_deserializer())
            .expect_err("duplicate destinations should be rejected");
        let message = error.to_string();

        assert!(message.contains("Conflicting"));
        assert!(message.contains("/tar"));
    }

    #[test]
    fn reject_nested_destinations() {
        let json = serde_json::json! { {
            "version": CURRENT_BLUEPRINT_VERSION,
            "symlinks": [
                { "src": "/a", "dst": "/target" },
                { "src": "/b", "dst": "/target/child" },
            ]
        } };
        let error = Blueprint::deserialize(json.into_deserializer())
            .expect_err("nested destinations should be rejected");
        let message = error.to_string();

        assert!(message.contains("/target"));
        assert!(message.contains("/target/child"));
    }

    #[test]
    fn be_strict_when_parsing() {
        let json = serde_json::json!( {
            "version": CURRENT_BLUEPRINT_VERSION,
            "yolo": "once",
            "symlinks": [ { "src": "/", "dst": "/", "aa": "bb" } ]
        } );
        let der = json.into_deserializer();
        let res = Blueprint::deserialize(der);
        res.unwrap_err();
    }

    #[test]
    fn reject_self_referential_symlink() {
        let json = serde_json::json! {
            {
                "version": CURRENT_BLUEPRINT_VERSION,
                "symlinks": [
                    { "src": "/foo", "dst": "/foo" }
                ]
            }
        };
        let der = json.into_deserializer();
        let res = Blueprint::deserialize(der);
        assert!(res.is_err());
        assert!(
            res.expect_err("it should error")
                .to_string()
                .contains("self-referential")
        );
    }
}
