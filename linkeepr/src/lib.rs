mod plan_schema;
mod template;

use tracing::debug;
use tracing::trace;

use anyhow::Context;
use std::path::PathBuf;

/// See documentation on [`CliOpts`] to have an outline of this program.
#[ derive( Debug ) ]
pub struct App;

impl App {
    #[ tracing::instrument ]
    pub fn run_with( cliopts: CliOpts, env: Envvars ) -> anyhow::Result<()> {
        use tap::Pipe;

        debug!( ?cliopts, ?env, "App launch" );

        let CliOpts { plan, prev_plans } = cliopts;

        // Step 1. Parse all input plans

        let plan = plan_schema::Plan::from_file( &plan )
            .with_context(
                || format!( r#"Failed to parse plan "{}""#, plan.display() )
            )? ;

        let prev_plans = if let Some( plans ) = prev_plans {
            // not use .map() because it complicates error reporting
            let mut parsed =
                Vec::with_capacity( plans.len() );
            for plan in plans {
                let errmsg =
                    || format! { "While parsing plan from file {}", plan.display() };
                plan_schema::Plan::from_file( &plan )
                    .with_context( errmsg )?
                    .pipe( |it| parsed.push( it ) )
            }
            Some( parsed )
        } else {
            None
        };

        todo!()
    }
}

/// Manage symlinks according to plan.
///
#[ derive( clap::Parser, Debug ) ]
pub struct CliOpts {
    /// Create new symlinks following this plan.
    #[ arg( long, short='n' ) ]
    pub plan: PathBuf,
    /// Remove old symlinks in these plans.
    /// Must not overlap with path given to --plan
    #[ arg( long, short='o' ) ]
    pub prev_plans: Option< Vec<PathBuf> >,
}

impl CliOpts {
    /// Parse the cli options and validate it.
    #[ tracing::instrument( name = "cliopts" ) ]
    pub fn new() -> anyhow::Result<Self> {
        use anyhow::ensure;

        debug!( "Parse and validate cli options" );

        let cliopts = <Self as clap::Parser>::parse();
        let CliOpts { plan, prev_plans } = &cliopts;

        if let Some( pps ) = prev_plans {
            for ps in pps {
                use same_file::is_same_file;
                let check_no_overlap = ps != plan;
                let check_not_same_file = !is_same_file( &ps, &plan )
                    .context( "Can't check same file" )?
                ;
                ensure! { check_no_overlap && check_not_same_file,
                    r#""{}" and "{}" overlap or they are the same file"#,
                    ps.display(), plan.display(),
                }
            }
        }

        Ok( cliopts )
    }
}

#[ derive( Debug, serde::Serialize ) ]
pub struct Envvars {
    pub home: PathBuf,
    pub xdg_config_home: PathBuf,
    pub xdg_data_home: PathBuf,
}

impl Envvars {
    pub fn new() -> anyhow::Result<Self> {
        // TODO: impl fallback paths
        macro_rules! var {
            ( $name:literal ) => { {
                let content = std::env::var( $name )
                    .context( concat!( "Can't read envvar ", $name ) )?
                ;
                PathBuf::from( content )
            } }
        }
        Ok( Self {
            home: var!( "HOME" ),
            xdg_config_home: var!( "XDG_CONFIG_HOME" ),
            xdg_data_home: var!( "XDG_DATA_HOME" ),
        } )
    }

    /// Create new set of Envvars with everything empty.
    /// Used in itegration tests.
    pub unsafe fn new_unchecked() -> Self {
        macro_rules! e { () => {{ PathBuf::new() }} }
        Self {
            home: e!(),
            xdg_data_home: e!(),
            xdg_config_home: e!(),
        }
    }
}


#[ cfg( test ) ]
mod tests {

    use super::*;

}

