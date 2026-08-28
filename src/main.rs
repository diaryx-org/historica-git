//! The command line: one conversion, stated plainly.

use std::path::PathBuf;
use std::process::ExitCode;

use historica_git::{export, import};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments.iter().map(String::as_str).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("historica-git: {message}");
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
            println!("historica-git {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ["import", repository, folder] => convert(repository, folder),
        ["import", ..] => Err(format!(
            "`import` takes a repository and a folder to make\n\n{USAGE}"
        )),
        ["write", folder, repository] => write(folder, repository),
        ["write", ..] => Err(format!(
            "`write` takes a store and a repository to make\n\n{USAGE}"
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
    if report.held > 0 {
        println!(
            "{} of them the store already held, and were recognised rather than \
             converted again",
            report.held
        );
    }
    if !report.bookmarks.is_empty() {
        println!("\nbookmarks:");
        for bookmark in &report.bookmarks {
            println!("  - {bookmark}");
        }
    }
    if !report.moved.is_empty() {
        println!("\nbookmarks moved to where git has the branch:");
        for bookmark in &report.moved {
            println!("  - {bookmark}");
        }
    }
    if report.onto {
        println!(
            "\nthe folder was left as it was; `historica update` is what brings it \
             forward"
        );
    }
    say(&report.uncarried);
    Ok(())
}

/// Decision 0004: `write`, not `export`. Historica's decision 0042 gives
/// `export` to a copy of a store to take away, and one binary cannot use the
/// word for two things without the collision landing in the reader's head.
fn write(folder: &str, repository: &str) -> Result<(), String> {
    let folder = PathBuf::from(folder);
    let repository = PathBuf::from(repository);
    let report = export::to_repository(&folder, &repository).map_err(|error| error.to_string())?;

    println!(
        "read {} revisions, wrote {} commits in {}",
        report.revisions,
        report.commits,
        repository.display()
    );
    if report.reused > 0 {
        println!(
            "{} the repository already held, and were named rather than written again",
            report.reused
        );
    }
    if !report.references.is_empty() {
        println!("\nrefs:");
        for reference in &report.references {
            println!("  - {reference}");
        }
    }
    if !report.deleted.is_empty() {
        println!("\nrefs deleted, because the store no longer names them:");
        for reference in &report.deleted {
            println!("  - {reference}");
        }
    }
    if let Some(branch) = &report.branch {
        match report.onto {
            true => println!("\nthe working tree was brought up to {branch}"),
            false => println!("\nchecked out {branch}"),
        }
    }
    say(&report.uncarried);
    Ok(())
}

/// Decision 0001: a conversion states what it could not carry, because the
/// person holding the result otherwise believes it is the thing it came from.
fn say(uncarried: &[String]) {
    if uncarried.is_empty() {
        return;
    }
    println!("\nwhat did not cross:");
    for line in uncarried {
        println!("  - {line}");
    }
}

const USAGE: &str = "\
historica-git converts between git repositories and Historica stores.

usage: historica-git <command>

commands:

  import <repository> <folder>   convert <repository> into the store at <folder>
  write  <folder> <repository>   write the store at <folder> out as a git repository

A target that is empty or absent gets a whole conversion. A folder already
holding a store, or a directory already holding a repository, gets what the
other side has gained since — decision 0007 — and nothing of a person's is
touched: a second import writes into `history/` and leaves the folder for
`historica update`, and a second write moves only refs this tool made or git
left where it found them. Anything else in the way is refused. `git` must be
on PATH, which decision 0002 makes this tool's one dependency.
";
