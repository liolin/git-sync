#[macro_use]
extern crate log;

use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use git2::{AnnotatedCommit, Config};
use git2::{
    ConfigLevel, Cred, FetchOptions, PushOptions, RemoteCallbacks, Repository, Signature, Status,
    StatusEntry,
};
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
    let repository = Repository::open(repo_information.path()).unwrap();
    let commit = repo_information.fetch().unwrap();
    repo_information.merge(commit).unwrap();

    let (tx, rx) = channel();
    let mut watcher = watcher(tx, Duration::from_millis(10)).unwrap();
    watcher.watch(dir, RecursiveMode::Recursive).unwrap();

    loop {
        match rx.recv() {
            Ok(_) => update(&repository),
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}

fn update(repo: &Repository) {
    let statuses = repo.statuses(None).unwrap();
    if statuses.is_empty() {
        return;
    }

    let mut msg = String::from("Empty commit");

    for s in repo.statuses(None).unwrap().iter() {
        msg = match s.status() {
            Status::WT_NEW | Status::WT_MODIFIED => adding_file(&repo, s),
            Status::WT_DELETED => remove_file(&repo, s),
            _ => panic!("unhandled git state: {:?}", s.status()),
        }
    }
    create_commit(&repo, msg.as_str(), false);
    // TODO: Make a push to the remote
    perform_push(&repo);
}

fn adding_file(repo: &Repository, s: StatusEntry) -> String {
    let new_file = Path::new(s.path().unwrap());
    let mut index = repo.index().expect("cannot get the Index file");
    let msg = format!("Add changes from {} to the repository", new_file.display());
    info!("{}", msg);

    index.add_path(new_file).unwrap();
    index.write().unwrap();

    msg
}

fn remove_file(repo: &Repository, s: StatusEntry) -> String {
    let new_file = Path::new(s.path().unwrap());
    let mut index = repo.index().expect("cannot get the Index file");
    let msg = format!("Remove {} from the repository", new_file.display());
    info!("{}", msg);

    index.remove_path(Path::new(s.path().unwrap())).unwrap();
    index.write().unwrap();

    msg
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
        repo.commit(Some(update_ref), &signature, &signature, &msg, &tree, &[])
            .unwrap();
    } else {
        let commit = &[&repo
            .find_commit(repo.head().unwrap().target().unwrap())
            .unwrap()];
        repo.commit(
            Some(update_ref),
            &signature,
            &signature,
            &msg,
            &tree,
            commit,
        )
        .unwrap();
    }
}

fn perform_push(repo: &Repository) {
    info!("Perform push request");
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, username_from_url, _allowed_types| {
        Cred::ssh_key_from_agent(username_from_url.unwrap())
    });
    let mut push_options = PushOptions::new();
    push_options.remote_callbacks(callbacks);

    let remotes = repo.remotes().unwrap();
    let mut remote = repo.find_remote(remotes.get(0).unwrap()).unwrap();
    remote
        .push(
            &["refs/heads/master:refs/heads/master"],
            Some(&mut push_options),
        )
        .unwrap();
}

fn perform_fetch<'a>(
    repo: &'a Repository,
    remote: &mut git2::Remote,
    refs: &[&str],
) -> Result<AnnotatedCommit<'a>, git2::Error> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_url, username_from_url, _allowed_types| {
        info!("Ask agent for SSH key");
        Cred::ssh_key_from_agent(username_from_url.unwrap())
    });

    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);
    //fetch_options.download_tags(git2::AutotagOption::All);
    info!(
        "Fetching {}/{} for repo",
        remote.name().unwrap(),
        refs.get(0).unwrap()
    );
    remote.fetch(refs, Some(&mut fetch_options), None)?;

    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    let commit = repo.reference_to_annotated_commit(&fetch_head)?;
    Ok(commit)
}

fn perfrom_merge(
    repo: &Repository,
    branch: &str,
    commit: AnnotatedCommit,
) -> Result<(), git2::Error> {
    // to fast forward
    let refname = format!("refs/heads/{}", branch);
    let mut refe = repo.find_reference(&refname)?;

    refe.set_target(commit.id(), "Fast-Forward")?;
    repo.set_head(refe.name().unwrap())?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
    Ok(())
}
