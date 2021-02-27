#[macro_use]
extern crate log;

use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use git2::{Config, ConfigLevel, Repository, Status, StatusEntry};
use notify::{watcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;

static PROG_NAME: &str = "git-sync";

mod repository;
use repository::RepoInformation;

fn main() {
    let _ = env_logger::builder().is_test(true).try_init();
    let matches = App::new(PROG_NAME)
        .version("0.1.0")
        .author("Olivier Lischer <olivier.lischer@liolin.ch>")
        .about("Check your git repo periodicaly for changes and syncs it to your remote")
        .setting(AppSettings::SubcommandRequired)
        .subcommand(
            SubCommand::with_name("setup")
                .about("Initial setup routine")
                .arg(
                    Arg::with_name("directory")
                        .short("d")
                        .long("directory")
                        .value_name("DIR")
                        .help("Sets the git repository")
                        .takes_value(true)
                        .required(true),
                )
                .arg(
                    Arg::with_name("author")
                        .short("a")
                        .long("author")
                        .takes_value(true)
                        .value_name("AUTHOR")
                        .help("The name of the author in the commit message")
                        .required(true),
                )
                .arg(
                    Arg::with_name("email")
                        .short("e")
                        .long("email")
                        .takes_value(true)
                        .value_name("EMAIL")
                        .help("The email of the author in the commit message")
                        .required(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("timer")
                .about("Looks for changes in the repository all x minutes")
                .arg(
                    Arg::with_name("directory")
                        .short("d")
                        .long("directory")
                        .value_name("DIR")
                        .help("Sets the git repository")
                        .takes_value(true)
                        .required(true),
                )
                .arg(
                    Arg::with_name("time")
                        .short("t")
                        .long("time")
                        .help("How often in seconds the repo should be check for changes")
                        .takes_value(true)
                        .required(true)
                        .value_name("TIME"),
                )
                .arg(
                    Arg::with_name("branch")
                        .short("b")
                        .long("branch")
                        .help("The name of the branch, for example \"master\"")
                        .takes_value(true)
                        .required(true)
                        .value_name("BRANCH"),
                )
                .arg(
                    Arg::with_name("remote")
                        .short("r")
                        .long("remote")
                        .help("The name of the remote, for example \"origin\"")
                        .takes_value(true)
                        .required(true)
                        .value_name("REMOTE"),
                ),
        )
        .get_matches();

    match matches.subcommand() {
        ("setup", Some(matches)) => run_setup(matches).unwrap(),
        ("timer", Some(matches)) => run_timer(matches),
        _ => unreachable!("The cli parser should prevent reaching here"),
    }
}

fn run_setup(matches: &ArgMatches) -> Result<(), git2::Error> {
    let dir = matches
        .value_of("directory")
        .expect("The cli parser should prevent reaching here");

    let author = matches
        .value_of("author")
        .expect("The cli parser should prevent reaching here")
        .to_owned();
    let email = matches
        .value_of("email")
        .expect("The cli parser should prevent reaching here")
        .to_owned();
    let dir = Path::new(dir);

    // TODO: Improve opening Repository (new or init or something diffrent?)
    // TODO: Replace unwrap with proper error handling
    let repo_information = RepoInformation::new(dir.to_str().unwrap(), "", "");
    let mut git_config = Config::new().unwrap();
    let git_config_file = dir.join(".git").join("config");
    git_config.add_file(&git_config_file, ConfigLevel::Local, false)?;
    git_config.set_str("user.name", &author).unwrap();
    git_config.set_str("user.email", &email).unwrap();

    repo_information.commit("Initial commit")?;
    Ok(())
}

fn run_timer(matches: &ArgMatches) -> ! {
    let dir = matches
        .value_of("directory")
        .expect("The cli parser should prevent reaching here");
    let remote = matches
        .value_of("remote")
        .expect("The cli parser should prevent reaching here");
    let branch = matches
        .value_of("branch")
        .expect("The cli parser should prevent reaching here");
    // let seconds = matches
    //     .value_of("time")
    //     .expect("The cli parser should prevent reaching here")
    //     .parse()
    //     .unwrap();

    let repo_information = RepoInformation::new(dir, remote, branch);
    let commit = repo_information.fetch().unwrap();
    repo_information.merge(commit).unwrap();

    let (tx, rx) = channel();
    let mut watcher = watcher(tx, Duration::from_millis(10)).unwrap();
    watcher.watch(dir, RecursiveMode::Recursive).unwrap();

    loop {
        match rx.recv() {
            // TODO: Replace unwrap with proper error handeling
            Ok(_) => update(&repo_information).unwrap(),
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}

fn update(repo_information: &RepoInformation) -> Result<(), git2::Error> {
    let statuses = repo_information.git_repo().statuses(None)?;
    if statuses.is_empty() {
        return Ok(());
    }

    let mut msg = String::new();
    for s in repo_information.git_repo().statuses(None).unwrap().iter() {
        msg = match s.status() {
            Status::WT_NEW | Status::WT_MODIFIED => adding_file(repo_information.git_repo(), s)?,
            Status::WT_DELETED => remove_file(repo_information.git_repo(), s)?,
            _ => panic!("unhandled git state: {:?}", s.status()),
        }
    }

    repo_information.commit(msg.as_str())?;
    repo_information.push()?;
    Ok(())
}

fn adding_file(repo: &Repository, s: StatusEntry) -> Result<String, git2::Error> {
    // TODO: Replace unwrap
    let new_file = Path::new(s.path().unwrap());
    let mut index = repo.index()?;
    let msg = format!("Add changes from {} to the repository", new_file.display());
    info!("{}", msg);

    index.add_path(new_file)?;
    index.write()?;
    Ok(msg)
}

fn remove_file(repo: &Repository, s: StatusEntry) -> Result<String, git2::Error> {
    // TODO: Replace unwrap
    let new_file = Path::new(s.path().unwrap());
    let mut index = repo.index()?;
    let msg = format!("Remove {} from the repository", new_file.display());
    info!("{}", msg);

    index.remove_path(Path::new(s.path().unwrap()))?;
    index.write()?;
    Ok(msg)
}
