use anyhow::Result;
use clap::Parser;
use codesearch::cli::{Action, Cli};
use codesearch::index::Index;
use codesearch::parser::parse_query;
use codesearch::search::{self, SearchOptions};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let action = cli.action().map_err(anyhow::Error::msg)?;

    match action {
        Action::Index { path, force } => {
            let mut index = Index::open(&path)?;
            let report = index.update(force)?;
            let stats = index.stats()?;
            print!(
                "{}",
                search::format_index_summary(index.root(), &report, &stats)
            );
        }
        Action::Stats { project } => {
            let index = Index::open(&project)?;
            let stats = index.stats()?;
            print!("{}", search::format_stats(&stats));
            if stats.files == 0 {
                println!("\nThe index is empty. Run `codesearch index` to build it.");
            }
        }
        Action::Search {
            project,
            query,
            ignore_self,
            update_index,
        } => {
            // Parse first: a malformed query should fail fast and never touch the index.
            let query = parse_query(&query)?;
            let opts = SearchOptions {
                ignore_self,
                update_index,
            };
            let outcome = search::search_project(&project, &query, &opts)?;
            if let Some(report) = outcome.update.as_ref().filter(|r| r.changed()) {
                eprintln!(
                    "Index updated: {} added, {} modified, {} removed.",
                    report.added, report.modified, report.removed
                );
            }
            print!("{}", search::format_results(&outcome.results));
        }
    }
    Ok(())
}
