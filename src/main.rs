//! The command line: one conversion, stated plainly.

use std::path::PathBuf;
use std::process::ExitCode;

use historigit::import;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments.iter().map(String::as_str).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("historigit: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[&str]) -> Result<(), String> {
    match arguments {
        [] | ["-h" | "--help" | "help"] => {
            print!("{USAGE}");
            Ok(())
        }
        ["-V" | "--version"] => {
            println!("historigit {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ["import", repository, folder] => convert(repository, folder),
        ["import", ..] => Err(format!(
            "`import` takes a repository and a folder to make\n\n{USAGE}"
        )),
        [command, ..] => Err(format!("unknown command `{command}`\n\n{USAGE}")),
    }
}

fn convert(repository: &str, folder: &str) -> Result<(), String> {
    let repository = PathBuf::from(repository);
    let folder = PathBuf::from(folder);
    let report =
        import::from_repository(&repository, &folder).map_err(|error| error.to_string())?;

    println!(
        "read {} commits, recorded {} revisions in {}",
        report.commits,
        report.revisions,
        folder.join("history").display()
    );
    // Decision 0001: a conversion states what it could not carry, because the
    // person holding the result otherwise believes it is the thing it came
    // from.
    if !report.uncarried.is_empty() {
        println!("\nwhat did not cross:");
        for line in &report.uncarried {
            println!("  - {line}");
        }
    }
    Ok(())
}

const USAGE: &str = "\
historigit converts a git repository into a Historica store.

usage: historigit <command>

commands:

  import <repository> <folder>   convert <repository> into a new store at <folder>

The folder must be empty or absent: a conversion only writes where it can be
sure it owns everything it removes. `git` must be on PATH, which decision 0002
makes this tool's one dependency.
";
