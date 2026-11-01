//! Command-line interface.
//!
//! ```text
//! codesearch [OPTIONS] "<QUERY>"        search
//! codesearch [OPTIONS] index [PATH]     build / refresh the index
//! codesearch [OPTIONS] stats            show index statistics
//! ```
//!
//! The query is a positional argument rather than a subcommand, so the primary
//! form stays `codesearch "(Username, Password) -> Session"`. The first
//! positional is therefore inspected by hand: `index` and `stats` select the
//! commands, anything else is the query.

use clap::{Parser, ValueEnum};
use std::path::PathBuf;

/// `lint` output format.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
}

/// `lint` exit-status policy.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailOn {
    /// Exit 1 when any finding exists.
    Warn,
    /// Always exit 0 unless an error occurred.
    Never,
}

const AFTER_HELP: &str = r#"Query syntax:
  (ParamType, ...) -> ReturnType

  Parameter order and count matter. Parentheses are required around multiple
  parameters, optional around a single one. `*` matches any one type.

Examples:
  codesearch "UserId -> UserDetails"
  codesearch "(Username, Password) -> Session"
  codesearch "(UserId, TenantId) -> (User, Metadata)"
  codesearch "() -> Config"
  codesearch "UserId -> ()"
  codesearch "UserId -> Option<UserDetails>"
  codesearch "(Username, Password) -> Result<Session, AuthError>"
  codesearch "(Username, *) -> Session"
  codesearch "(Username, Password) -> *"
  codesearch --ignore-self "UserId -> User"     # also finds fn get(&self, id: UserId) -> User
  codesearch index ./my-project
  codesearch --project ./my-project stats

Lint (flags public functions that are hard to find or easy to misuse):
  codesearch lint                                  # lint the current project
  codesearch lint ./my-project --format json
  codesearch lint --rule R001 --fail-on warn       # exit 1 if anything is flagged
"#;

#[derive(Parser, Debug)]
#[command(
    name = "codesearch",
    version,
    about = "Find Rust functions by their parameter and return types",
    override_usage = "codesearch [OPTIONS] <QUERY>\n       codesearch [OPTIONS] index [PATH]\n       codesearch [OPTIONS] stats\n       codesearch [OPTIONS] lint [PATH]",
    after_help = AFTER_HELP
)]
pub struct Cli {
    /// Project directory to index and search (the index is stored in <DIR>/.codesearch)
    #[arg(short, long, value_name = "DIR", default_value = ".")]
    pub project: PathBuf,

    /// `index` only: re-hash every file even if its size and modification time are unchanged
    #[arg(long)]
    pub force: bool,

    /// Search only: skip a `self` receiver when comparing parameters
    #[arg(long)]
    pub ignore_self: bool,

    /// Search only: use the index as it is, without checking for changed files first
    #[arg(long)]
    pub no_update: bool,

    /// `lint` only: output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// `lint` only: run just these rules (comma-separated ids, e.g. R001,R002)
    #[arg(long, value_delimiter = ',', value_name = "IDS")]
    pub rule: Vec<String>,

    /// `lint` only: `warn` exits 1 when anything is flagged; `never` always exits 0
    #[arg(long, value_enum, default_value_t = FailOn::Never)]
    pub fail_on: FailOn,

    /// A signature query, or one of the commands `index [PATH]` / `stats` / `lint [PATH]`
    #[arg(value_name = "QUERY | COMMAND")]
    pub args: Vec<String>,
}

/// What the user asked for, after resolving the positional arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Index {
        path: PathBuf,
        force: bool,
    },
    Stats {
        project: PathBuf,
    },
    Lint {
        path: PathBuf,
        format: OutputFormat,
        rules: Vec<String>,
        fail_on: FailOn,
    },
    Search {
        project: PathBuf,
        query: String,
        ignore_self: bool,
        update_index: bool,
    },
}

impl Cli {
    /// Resolves the parsed arguments into an [`Action`], or a user-facing error message.
    pub fn action(self) -> Result<Action, String> {
        let Cli {
            project,
            force,
            ignore_self,
            no_update,
            format,
            rule,
            fail_on,
            args,
        } = self;

        match args.first().map(String::as_str) {
            None => Err(
                "missing query. Try: codesearch \"(Username, Password) -> Session\" (see --help)"
                    .to_string(),
            ),
            Some("index") => {
                if args.len() > 2 {
                    return Err("`index` takes at most one directory".to_string());
                }
                let path = match args.get(1) {
                    Some(p) => PathBuf::from(p),
                    None => project,
                };
                Ok(Action::Index { path, force })
            }
            Some("stats") => {
                if args.len() > 1 {
                    return Err(
                        "`stats` takes no arguments; use --project to choose the directory"
                            .to_string(),
                    );
                }
                Ok(Action::Stats { project })
            }
            Some("lint") => {
                if args.len() > 2 {
                    return Err("`lint` takes at most one directory".to_string());
                }
                let path = match args.get(1) {
                    Some(p) => PathBuf::from(p),
                    None => project,
                };
                Ok(Action::Lint {
                    path,
                    format,
                    rules: rule,
                    fail_on,
                })
            }
            Some(_) => {
                if args.len() > 1 {
                    return Err(
                        "expected a single query; wrap it in quotes, e.g. \"(A, B) -> C\""
                            .to_string(),
                    );
                }
                Ok(Action::Search {
                    project,
                    query: args[0].clone(),
                    ignore_self,
                    update_index: !no_update,
                })
            }
        }
    }
}
