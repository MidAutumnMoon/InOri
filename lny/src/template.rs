use std::ops::Deref;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;

use ino_path::PathExt as _;
use minijinja::Environment;
use minijinja::UndefinedBehavior;
use minijinja::value::Value;
use rootcause::Result;
use rootcause::bail;
use rootcause::option_ext::OptionExt as _;
use rootcause::prelude::ResultExt as _;
use serde::Deserialize;
use tap::Tap as _;

use tracing::debug;
use tracing::trace;

#[expect(
    clippy::unwrap_used,
    reason = "single global init; failure is a programmer error"
)]
static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    debug!("Initialize global template engine");

    let context = ContextOfTemplate::new()
        .context("Failed to initialize context for template")
        .unwrap()
        .into_value();

    let mut environ = Environment::empty();
    environ.set_undefined_behavior(UndefinedBehavior::Strict);
    environ.set_recursion_limit(0);

    Engine { environ, context }.tap(|engine| trace!(?engine))
});

#[derive(Debug)]
pub struct Engine {
    environ: Environment<'static>,
    context: Value,
}

impl Engine {
    #[tracing::instrument(skip_all)]
    pub fn render(&self, tmpl: &str) -> Result<String> {
        debug!(?tmpl, "Render template");
        Ok(self
            .environ
            .render_str(tmpl, &self.context)
            .context_with(|| {
                format!(r#"Failed to render template "{tmpl}""#)
            })?
            .tap(|rendered| trace!(?rendered)))
    }
}

#[derive(Debug)]
pub struct ContextOfTemplate {
    home: String,
    config: String,
    data: String,
    cache: String,
    state: String,
}

impl ContextOfTemplate {
    // N.B. May fail where XDG variables can't be autodetected, e.g.
    // nix sandboxes. On Linux `etcetera` provides sensible defaults,
    // so failures are rare in practice.
    #[tracing::instrument(name = "template_context_new")]
    pub fn new() -> Result<Self> {
        use etcetera::BaseStrategy as _;
        use etcetera::choose_base_strategy;

        debug!("Initialize context for template");

        let xdg =
            choose_base_strategy().context("Failed to find XDG dirs")?;

        let home = dir_as_string(xdg.home_dir(), "home")?;
        let config = dir_as_string(&xdg.config_dir(), "config")?;
        let data = dir_as_string(&xdg.data_dir(), "data")?;
        let cache = dir_as_string(&xdg.cache_dir(), "cache")?;

        let state_dir = xdg
            .state_dir()
            .context("Failed to determine XDG state directory")?;
        let state = dir_as_string(&state_dir, "state")?;

        let me = Self {
            home,
            config,
            data,
            cache,
            state,
        }
        .tap(|context| trace!(?context));
        Ok(me)
    }

    fn into_value(self) -> Value {
        let Self {
            home,
            config,
            data,
            cache,
            state,
        } = self;

        Value::from_pairs([
            ("home", home),
            ("config", config),
            ("data", data),
            ("cache", cache),
            ("state", state),
        ])
    }
}

// A template value is a string, so a non-UTF-8 dir is an error
// rather than a lossy rewrite.
fn dir_as_string(dir: &Path, name: &str) -> Result<String> {
    let dir = dir.must_absolute()?;
    Ok(dir.to_str().map(str::to_owned).context_with(|| {
        format!(
            r#"XDG {name} directory is not valid UTF-8: "{}""#,
            dir.display()
        )
    })?)
}

/// A [`Path`] wrapper that guaranteed to not contains unrendered
/// templates and be absolute.
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
#[derive(serde::Serialize)]
#[serde(transparent)]
pub struct RenderedPath {
    inner: PathBuf,
}

impl RenderedPath {
    #[tracing::instrument(skip_all)]
    #[cfg(test)]
    pub fn from_unrendered(input: &str) -> Result<Self> {
        use serde::de::IntoDeserializer as _;
        use serde::de::value::Error as DeError;
        use serde::de::value::StrDeserializer;
        let der: StrDeserializer<DeError> = input.into_deserializer();
        Ok(Self::deserialize(der)?)
    }
}

impl Deref for RenderedPath {
    type Target = Path;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AsRef<Path> for RenderedPath {
    fn as_ref(&self) -> &Path {
        self
    }
}

impl<'de> Deserialize<'de> for RenderedPath {
    #[tracing::instrument(skip_all)]
    fn deserialize<D>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as DeError;

        debug!("Deserialize into RenderedPath");

        #[inline]
        fn ren(tmpl: &str) -> Result<PathBuf> {
            let path = PathBuf::from(ENGINE.render(tmpl)?);
            if !path.is_absolute() {
                bail!(
                    r#"Path must be absolute. Raw: "{}" Rendered: "{}""#,
                    tmpl,
                    path.display(),
                );
            }
            if path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
            {
                bail!(
                    r#"Path must not contain ".." components. \
                        Raw: "{}" Rendered: "{}""#,
                    tmpl,
                    path.display(),
                );
            }
            Ok(path)
        }
        let raw = String::deserialize(deserializer)?;
        let inner = ren(&raw).map_err(DeError::custom)?;
        trace!(?inner);
        Ok(Self { inner })
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Tests")]
mod test {

    use tracing::trace;

    use super::*;

    #[test]
    fn rendered_path() {
        let tmpls_to_ok = [
            // absolute path
            "/home",
            // valid template
            "{{ home }}",
            "{{ config }}",
            "{{ data }}",
            "{{ cache }}",
            "{{ state }}",
        ];

        let tmpls_to_err = [
            // not absolute
            "wow",
            // topology must remain lexical
            "/home/../etc",
            // invalid template
            "{{ home",
            "{{ what-no-kidding }}",
        ];

        for template in tmpls_to_ok {
            let path = RenderedPath::from_unrendered(template);
            trace!(?path);
            path.unwrap();
        }
        for template in tmpls_to_err {
            let path = RenderedPath::from_unrendered(template);
            trace!(?path);
            path.unwrap_err();
        }
    }
}
