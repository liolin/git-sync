type GitResult<T> = Result<T, git2::Error>;
static FETCH_HEAD: &str = "FETCH_HEAD";

pub struct RepoInformation<'a> {
    path: &'a str,
    remote: &'a str,
    branch: &'a str,
    git_repo: git2::Repository,
}

impl<'a> RepoInformation<'a> {
    pub fn init(path: &'a str, remote: &'a str, branch: &'a str) -> Self {
        let git_repo = git2::Repository::init(path).unwrap();
        Self {
            path,
            remote,
            branch,
            git_repo,
        }
    }

    pub fn new(path: &'a str, remote: &'a str, branch: &'a str) -> Self {
        let git_repo = git2::Repository::open(path).unwrap();
        Self {
            path,
            remote,
            branch,
            git_repo,
        }
    }

    pub fn is_repo(path: &str) -> bool {
        git2::Repository::open(path).is_ok()
    }

    pub fn path(&self) -> &'a str {
        self.path
    }

    pub fn remote(&self) -> &'a str {
        self.remote
    }

    pub fn branch(&self) -> &'a str {
        self.branch
    }

    pub fn git_repo(&self) -> &git2::Repository {
        &self.git_repo
    }

    pub fn commit(&self, commit_msg: &str) -> GitResult<()> {
        let config = self.git_repo.config()?.snapshot()?;
        let author = config.get_str("user.name")?;
        let email = config.get_str("user.email")?;

        let update_ref = "HEAD";
        let signature = git2::Signature::now(author, email)?;
        let mut index = self.git_repo.index()?;
        let tree_oid = index.write_tree()?;
        let tree = self.git_repo.find_tree(tree_oid)?;

        info!("New commit: {}, {}, {}", update_ref, &signature, commit_msg);

        let commits = match self.git_repo.head() {
            // TODO: Replace unwrap
            Ok(r) => {
                let oid = r.target().unwrap();
                vec![self.git_repo.find_commit(oid)?]
            }

            Err(_) => {
                // HEAD does not Exist; Return a vector without any commits
                Vec::new()
            }
        };

        self.git_repo.commit(
            Some(update_ref),
            &signature,
            &signature,
            &commit_msg,
            &tree,
            &commits.iter().collect::<Vec<_>>(),
        )?;
        Ok(())
    }

    pub fn fetch(&self) -> GitResult<git2::AnnotatedCommit> {
        let mut remote = self.git_repo.find_remote(self.remote()).unwrap();

        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, _allowed_types| {
            info!("Ask agent for SSH key");
            git2::Cred::ssh_key_from_agent(username_from_url.unwrap())
        });

        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);
        //fetch_options.download_tags(git2::AutotagOption::All);
        info!(
            "Fetching {}/{} for repo",
            remote.name().unwrap(),
            self.branch()
        );
        remote.fetch(&[self.branch()], Some(&mut fetch_options), None)?;

        let fetch_head = self.git_repo.find_reference(FETCH_HEAD)?;
        let commit = self.git_repo.reference_to_annotated_commit(&fetch_head)?;
        Ok(commit)
    }

    pub fn merge(&self, remote: git2::AnnotatedCommit) -> GitResult<()> {
        info!("Let's do a merge");
        let analysis = self.git_repo.merge_analysis(&[&remote])?;

        if analysis.0.is_fast_forward() {
            info!("Merging with Fastforward");
            self.do_fast_forward(remote)?;
        } else if analysis.0.is_normal() {
            info!("Do a normal merge");
            // search the local commit
            // TODO: Better git2 error
            let local_oid = self.git_repo().head()?.target().ok_or(git2::Error::new(
                git2::ErrorCode::NotFound,
                git2::ErrorClass::Object,
                "Some git2 error occured",
            ))?;
            let local = self.git_repo().find_annotated_commit(local_oid)?;
            self.do_normal_merge(remote, local)?;
        } else {
            info!("There is nothing to do");
        }
        Ok(())
    }

    pub fn push(&self) -> GitResult<()> {
        info!("Perform push request");
        // TODO: One place to retrieve callbacks
        let mut callbacks = git2::RemoteCallbacks::new();
        callbacks.credentials(|_url, username_from_url, _allowed_types| {
            info!("Ask agent for SSH key");
            git2::Cred::ssh_key_from_agent(username_from_url.unwrap())
        });
        let mut push_options = git2::PushOptions::new();
        push_options.remote_callbacks(callbacks);

        let mut remote = self.get_remote();
        // TODO: Not a static refspec
        remote.push(
            &["refs/heads/master:refs/heads/master"],
            Some(&mut push_options),
        )?;
        Ok(())
    }

    fn do_fast_forward(&self, commit: git2::AnnotatedCommit) -> GitResult<()> {
        let refname = format!("refs/heads/{}", self.branch());
        let mut refe = self.git_repo.find_reference(&refname)?;

        // TODO: Better reflog message
        refe.set_target(commit.id(), "Fast-Forward")?;
        self.git_repo.set_head(refe.name().unwrap())?;
        self.git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
        Ok(())
    }

    fn do_normal_merge(
        &self,
        local: git2::AnnotatedCommit,
        remote: git2::AnnotatedCommit,
    ) -> Result<(), git2::Error> {
        unimplemented!();
        let local_tree = self.git_repo().find_commit(local.id())?.tree()?;
        let remote_tree = self.git_repo().find_commit(remote.id())?.tree()?;
        let ancestor = self
            .git_repo()
            .find_commit(self.git_repo().merge_base(local.id(), remote.id())?)?
            .tree()?;
        let mut idx = self
            .git_repo()
            .merge_trees(&ancestor, &local_tree, &remote_tree, None)?;

        if idx.has_conflicts() {
            info!("Merge conficts detected...");
            self.git_repo().checkout_index(Some(&mut idx), None)?;
            return Ok(());
        }
        let result_tree = self
            .git_repo()
            .find_tree(idx.write_tree_to(self.git_repo())?)?;
        // now create the merge commit
        let msg = format!("Merge: {} into {}", remote.id(), local.id());
        let sig = self.git_repo().signature()?;
        let local_commit = self.git_repo().find_commit(local.id())?;
        let remote_commit = self.git_repo().find_commit(remote.id())?;
        // Do our merge commit and set current branch head to that commit.
        let _merge_commit = self.git_repo().commit(
            Some("HEAD"),
            &sig,
            &sig,
            &msg,
            &result_tree,
            &[&local_commit, &remote_commit],
        )?;
        // Set working tree to match head.
        self.git_repo().checkout_head(None)?;
        Ok(())
    }

    fn get_remote(&self) -> git2::Remote {
        // TODO: Proper error handeling
        self.git_repo.find_remote(self.remote()).unwrap()
    }
}
