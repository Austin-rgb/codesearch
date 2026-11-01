use anyhow::Result;
use clap::Parser;
use codesearch::cli::{Action, Cli, FailOn, OutputFormat};
use codesearch::index::Index;
use codesearch::lint::{self, report};
use codesearch::parser::parse_query;
use codesearch::search::{self, SearchOptions};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
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
        Action::Lint {
            path,
            format,
            rules,
            fail_on,
        } => {
            let outcome = lint::lint_project(&path, &rules)?;
            if outcome.update.changed() {
                eprintln!(
                    "Index updated: {} added, {} modified, {} removed.",
                    outcome.update.added, outcome.update.modified, outcome.update.removed
                );
            }
            match format {
                OutputFormat::Text => print!("{}", report::render_text(&outcome.report)),
                OutputFormat::Json => println!("{}", report::render_json(&outcome.report)?),
            }
            if fail_on == FailOn::Warn && outcome.report.has_findings() {
                return Ok(ExitCode::from(1));
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
    Ok(ExitCode::SUCCESS)
}
