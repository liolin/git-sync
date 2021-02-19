#[macro_use]
extern crate log;

use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use git2::{Config, ConfigLevel, Repository, Signature, Status, StatusEntry};
use std::path::Path;
use std::{thread, time};
use thiserror::Error;

static PROG_NAME: &str = "git-sync";

#[derive(Error, Debug)]
enum GitSyncError {
    #[error("Can not open the repository")]
    OpenRepository {
        #[from]
        source: git2::Error,
    },
}

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
                ),
        )
        .get_matches();

    match matches.subcommand() {
        ("setup", Some(matches)) => run_setup(matches).unwrap(),
        ("timer", Some(matches)) => run_timer(matches),
        _ => unreachable!("The cli parser should prevent reaching here"),
    }
}

fn run_setup(matches: &ArgMatches) -> Result<(), GitSyncError> {
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

    let repo = Repository::open(dir).or_else(|_| Repository::init(dir))?;
    let mut git_config = Config::new().unwrap();
    let git_config_file = dir.join(".git").join("config");
    git_config.add_file(&git_config_file, ConfigLevel::Local, false)?;
    git_config.set_str("user.name", &author).unwrap();
    git_config.set_str("user.email", &email).unwrap();

    create_initial_commit(&repo);
    Ok(())
}

fn run_timer(matches: &ArgMatches) -> ! {
    let dir = matches
        .value_of("directory")
        .expect("The cli parser should prevent reaching here");
    let seconds = matches
        .value_of("time")
        .expect("The cli parser should prevent reaching here")
        .parse()
        .unwrap();

    let repo = Repository::open(dir).unwrap();
    loop {
        let statuses = repo.statuses(None).unwrap();

        if statuses.is_empty() {
            let time_to_wait = time::Duration::from_secs(seconds);
            thread::sleep(time_to_wait);
            continue;
        }

        for s in repo.statuses(None).unwrap().iter() {
            match s.status() {
                Status::WT_NEW | Status::WT_MODIFIED => adding_file(&repo, s),
                Status::WT_DELETED => remove_file(&repo, s),
                _ => panic!("unhandled git state: {:?}", s.status()),
            }
        }
        create_commit(&repo, "my commit msg", false);
        // TODO: Make a push to the remote
    }
}

fn adding_file(repo: &Repository, s: StatusEntry) {
    let new_file = Path::new(s.path().unwrap());
    let mut index = repo.index().expect("cannot get the Index file");

    info!("Add changes from {} to the repository", new_file.display());

    index.add_path(new_file).unwrap();
    index.write().unwrap();
}

fn remove_file(repo: &Repository, s: StatusEntry) {
    let new_file = Path::new(s.path().unwrap());
    let mut index = repo.index().expect("cannot get the Index file");

    info!("Remove {} from the repository", new_file.display());

    index.remove_path(Path::new(s.path().unwrap())).unwrap();
    index.write().unwrap();
}

fn create_initial_commit(repo: &Repository) {
    create_commit(repo, "Initial commit", true);
}

fn create_commit(repo: &Repository, msg: &str, initial: bool) {
    let config = repo.config().unwrap().snapshot().unwrap();
    let author = config.get_str("user.name").unwrap();
    let email = config.get_str("user.email").unwrap();

    let update_ref = "HEAD";
    let signature = Signature::now(author, email).unwrap();
    let mut index = repo.index().expect("cannot get the index file");
    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();

    info!("New commit: {}, {}, {}", update_ref, &signature, msg);

    // TODO: Does not exist a better way?
    // e.g:
    //     let commit = if initial { ... } else { ... };
    //     repo.commit(..., commit).unwrap();
    if initial {
        repo.commit(Some(update_ref), &signature, &signature, msg, &tree, &[])
            .unwrap();
    } else {
        let commit = &[&repo
            .find_commit(repo.head().unwrap().target().unwrap())
            .unwrap()];
        repo.commit(Some(update_ref), &signature, &signature, msg, &tree, commit)
            .unwrap();
    }
}
