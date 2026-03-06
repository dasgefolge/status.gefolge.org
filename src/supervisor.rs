use {
    std::{
        cmp::Ordering::*,
        path::Path,
        pin::Pin,
        process::Stdio,
        sync::Arc,
        time::Duration,
    },
    futures::future::{
        self,
        Either,
        FutureExt as _,
    },
    itertools::{
        Itertools as _,
        Position,
    },
    log_lock::*,
    serde::{
        Deserialize,
        Serialize,
    },
    tokio::{
        process::Command,
        select,
        sync::{
            mpsc,
            watch,
        },
        time::sleep,
    },
    wheel::traits::{
        AsyncCommandOutputExt as _,
        IoResultExt as _,
        SendResultExt as _,
    },
    crate::GIT_COMMIT_HASH,
};

// uses global gitdir to allow caddy to access static files
const GEFOLGE_WEB_BUILD_REPO_PATH: &str = "/opt/git/github.com/dasgefolge/gefolge.org/build";
const STATUS_REPO_PATH: &str = "/opt/git/github.com/dasgefolge/status.gefolge.org/main";

pub(crate) struct Status {
    pub(crate) watch: watch::Sender<()>,
    pub(crate) gefolge_web_running: gix::ObjectId,
    pub(crate) gefolge_web_future: Vec<(gix::ObjectId, String, GefolgeWebCommitStatus)>,
    pub(crate) status_future: Vec<(gix::ObjectId, String, StatusCommitStatus)>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum GefolgeWebCommitStatus {
    Pending,
    Bundled,
    Deploy,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum StatusCommitStatus {
    Pending,
    Bundled,
    Build,
}

#[derive(Clone, Copy, Deserialize)]
pub(crate) enum RepoName {
    #[serde(rename = "gefolge.org")]
    GefolgeWeb,
    #[serde(rename = "status.gefolge.org")]
    Status,
}

#[derive(Clone)]
pub(crate) struct Supervisor {
    gefolge_web_build_repo_lock: Arc<Mutex<()>>,
    status_repo_lock: Arc<Mutex<()>>,
    status: Arc<RwLock<Status>>,
    webhook: mpsc::Sender<RepoName>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)] GitDecode(#[from] gix::diff::object::decode::Error),
    #[error(transparent)] GitDecodeHash(#[from] gix::hash::decode::Error),
    #[error(transparent)] GitFind(#[from] gix::object::find::existing::Error),
    #[error(transparent)] GitFindReference(#[from] gix::reference::find::existing::Error),
    #[error(transparent)] GitFindWithConversion(#[from] gix::object::find::existing::with_conversion::Error),
    #[error(transparent)] GitOpen(#[from] gix::open::Error),
    #[error(transparent)] GitPeel(#[from] gix::object::peel::to_kind::Error),
    #[error(transparent)] GitPeelReference(#[from] gix::reference::peel::to_kind::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
}

impl Supervisor {
    pub(crate) async fn new() -> Result<(Self, impl FnOnce(rocket::Shutdown) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>), Error> {
        let gefolge_web_running = gix::ObjectId::from_hex(Command::new("ssh").arg("gefolge.org").arg("env -C /opt/git/github.com/dasgefolge/gefolge.org/main git rev-parse HEAD").stdout(Stdio::piped()).check("ssh").await?.stdout.trim_ascii())?;
        let gefolge_web_built_commit = gefolge_web_running; //TODO check version of gefolge-web-next?
        let (webhook_tx, mut webhook_rx) = mpsc::channel(256);
        let this = Self {
            gefolge_web_build_repo_lock: Arc::default(),
            status_repo_lock: Arc::default(),
            status: Arc::new(RwLock::new(Status {
                watch: watch::Sender::default(),
                gefolge_web_running,
                gefolge_web_future: {
                    let mut future = Vec::default();
                    Command::new("git").arg("fetch").current_dir(GEFOLGE_WEB_BUILD_REPO_PATH).check("git fetch").await?; //TODO use GitHub API or gix (how?)
                    let repo = gix::open(GEFOLGE_WEB_BUILD_REPO_PATH)?;
                    let new_head = repo.find_reference("origin/main")?.peel_to_commit()?.id;
                    if new_head != gefolge_web_running {
                        let mut iter_commit = repo.find_commit(new_head)?;
                        future = vec![(new_head, iter_commit.message()?.summary().to_string())];
                        loop {
                            let Ok(parent) = iter_commit.parent_ids().exactly_one() else {
                                // initial commit or merge commit; skip parents for simplicity's sake
                                break
                            };
                            if parent == gefolge_web_running { break }
                            iter_commit = parent.object()?.peel_to_commit()?;
                            future.push((parent.detach(), iter_commit.message()?.summary().to_string()));
                        }
                    }
                    let built_idx = future.binary_search_by_key(&gefolge_web_built_commit, |(commit_hash, _)| *commit_hash).ok();
                    future.into_iter().enumerate().rev().map(|(idx, (commit_hash, commit_msg))| (commit_hash, commit_msg, if let Some(built_idx) = built_idx {
                        match idx.cmp(&built_idx) {
                            Less => GefolgeWebCommitStatus::Pending,
                            Equal => GefolgeWebCommitStatus::Deploy,
                            Greater => GefolgeWebCommitStatus::Bundled,
                        }
                    } else {
                        GefolgeWebCommitStatus::Pending
                    })).collect()
                },
                status_future: {
                    let mut future = Vec::default();
                    Command::new("git").arg("fetch").current_dir(STATUS_REPO_PATH).check("git fetch").await?; //TODO use GitHub API or gix (how?)
                    let repo = gix::open(STATUS_REPO_PATH)?;
                    let new_head = repo.find_reference("origin/main")?.peel_to_commit()?.id;
                    if new_head != GIT_COMMIT_HASH {
                        let mut iter_commit = repo.find_commit(new_head)?;
                        future = vec![(new_head, iter_commit.message()?.summary().to_string())];
                        loop {
                            let Ok(parent) = iter_commit.parent_ids().exactly_one() else {
                                // initial commit or merge commit; skip parents for simplicity's sake
                                break
                            };
                            if parent == GIT_COMMIT_HASH { break }
                            iter_commit = parent.object()?.peel_to_commit()?;
                            future.push((parent.detach(), iter_commit.message()?.summary().to_string()));
                        }
                    }
                    future.into_iter().rev().map(|(commit_hash, commit_msg)| (commit_hash, commit_msg, StatusCommitStatus::Pending)).collect()
                },
            })),
            webhook: webhook_tx,
        };
        let mut gefolge_web_build_task = None;
        let mut needs_gefolge_web_rebuild = false;
        let mut needs_status_restart = false;
        Ok((this.clone(), move |mut shutdown: rocket::Shutdown| async move {
            loop {
                let gefolge_web_build_task_or_pending = if let Some(build_task) = &mut gefolge_web_build_task {
                    Either::Left(build_task)
                } else {
                    Either::Right(future::pending())
                };
                select! {
                    () = &mut shutdown => break,
                    () = sleep(Duration::from_secs(24 * 60 * 60)) => {
                        if this.fetch_status().await? {
                            if gefolge_web_build_task.is_some() {
                                needs_status_restart = true;
                            } else {
                                lock!(@write status = this.status; for (pos, (_, _, status)) in status.status_future.iter_mut().with_position() {
                                    if let StatusCommitStatus::Pending = status {
                                        *status = match pos {
                                            Position::First | Position::Middle => StatusCommitStatus::Bundled,
                                            Position::Last | Position::Only => StatusCommitStatus::Build,
                                        };
                                    }
                                });
                                println!("supervisor: starting NixOS rebuild");
                                Command::new("/run/wrappers/bin/sudo")
                                    .arg("/run/current-system/sw/bin/nixos-rebuild")
                                    .arg("switch")
                                    .arg("--recreate-lock-file")
                                    .arg("--refresh")
                                    .arg("--no-write-lock-file")
                                    .arg("--flake=git+ssh://fenhl@fenhl.net/opt/git/localhost/dev/dev.git")
                                    .spawn().at_command("nixos-rebuild")?;
                                break
                            }
                        }
                    }
                    Some(repo_name) = webhook_rx.recv() => match repo_name {
                        RepoName::GefolgeWeb => if this.fetch_gefolge_web().await? {
                            if gefolge_web_build_task.is_some() {
                                needs_gefolge_web_rebuild = true;
                            } else {
                                gefolge_web_build_task = this.gefolge_web_build_task().await;
                            }
                        },
                        RepoName::Status => if this.fetch_status().await? {
                            if gefolge_web_build_task.is_some() {
                                needs_status_restart = true;
                            } else {
                                lock!(@write status = this.status; for (pos, (_, _, status)) in status.status_future.iter_mut().with_position() {
                                    if let StatusCommitStatus::Pending = status {
                                        *status = match pos {
                                            Position::First | Position::Middle => StatusCommitStatus::Bundled,
                                            Position::Last | Position::Only => StatusCommitStatus::Build,
                                        };
                                    }
                                });
                                println!("supervisor: starting NixOS rebuild");
                                Command::new("/run/wrappers/bin/sudo")
                                    .arg("/run/current-system/sw/bin/nixos-rebuild")
                                    .arg("switch")
                                    .arg("--recreate-lock-file")
                                    .arg("--refresh")
                                    .arg("--no-write-lock-file")
                                    .arg("--flake=git+ssh://fenhl@fenhl.net/opt/git/localhost/dev/dev.git")
                                    .spawn().at_command("nixos-rebuild")?;
                                break
                            }
                        },
                    },
                    res = gefolge_web_build_task_or_pending => {
                        let gefolge_web_built_commit = res??;
                        println!("supervisor: gefolge.org build finished");
                        lock!(@write status = this.status; {
                            let _ = status.watch.send(());
                            if let Ok(built_idx) = status.gefolge_web_future.binary_search_by_key(&gefolge_web_built_commit, |(commit_hash, _, _)| *commit_hash) {
                                for (idx, (_, _, status)) in status.gefolge_web_future.iter_mut().enumerate() {
                                    *status = match idx.cmp(&built_idx) {
                                        Less => GefolgeWebCommitStatus::Bundled,
                                        Equal => GefolgeWebCommitStatus::Deploy,
                                        Greater => GefolgeWebCommitStatus::Pending,
                                    };
                                }
                            }
                        });
                        gefolge_web_build_task = if needs_gefolge_web_rebuild {
                            needs_gefolge_web_rebuild = false;
                            this.gefolge_web_build_task().await
                        } else {
                            if needs_status_restart {
                                lock!(@write status = this.status; for (pos, (_, _, status)) in status.status_future.iter_mut().with_position() {
                                    if let StatusCommitStatus::Pending = status {
                                        *status = match pos {
                                            Position::First | Position::Middle => StatusCommitStatus::Bundled,
                                            Position::Last | Position::Only => StatusCommitStatus::Build,
                                        };
                                    }
                                });
                                println!("supervisor: starting NixOS rebuild");
                                Command::new("/run/wrappers/bin/sudo")
                                    .arg("/run/current-system/sw/bin/nixos-rebuild")
                                    .arg("switch")
                                    .arg("--recreate-lock-file")
                                    .arg("--refresh")
                                    .arg("--no-write-lock-file")
                                    .arg("--flake=git+ssh://fenhl@fenhl.net/opt/git/localhost/dev/dev.git")
                                    .spawn().at_command("nixos-rebuild")?;
                                break
                            }
                            None
                        };
                        lock!(@write status = this.status; {
                            let _ = status.watch.send(());
                            status.gefolge_web_running = gefolge_web_built_commit;
                            if let Some(idx) = status.gefolge_web_future.iter().position(|(iter_commit, _, _)| *iter_commit == gefolge_web_built_commit) {
                                status.gefolge_web_future.drain(..=idx);
                            }
                        });
                    }
                }
            }
            Ok(())
        }.boxed()))
    }

    pub(crate) async fn handle_webhook(&self, repo_name: RepoName) {
        self.webhook.send(repo_name).await.allow_unreceived();
    }

    pub(crate) async fn status(&self) -> tokio::sync::RwLockReadGuard<'_, Status> {
        self.status.0.read().await
    }

    async fn fetch_gefolge_web(&self) -> Result<bool, Error> {
        Ok(lock!(repo_lock = self.gefolge_web_build_repo_lock; {
            Command::new("git").arg("fetch").current_dir(GEFOLGE_WEB_BUILD_REPO_PATH).check("git fetch").await?; //TODO use GitHub API or gix (how?)
            let repo = gix::open(GEFOLGE_WEB_BUILD_REPO_PATH)?;
            let new_head = repo.find_reference("origin/main")?.peel_to_commit()?.id;
            lock!(@write status = self.status; {
                let _ = status.watch.send(());
                let status_latest = status.gefolge_web_future.last().map_or(status.gefolge_web_running, |(latest, _, _)| *latest);
                if new_head != status_latest {
                    let mut iter_commit = repo.find_commit(new_head)?;
                    let mut to_add = vec![(new_head, iter_commit.message()?.summary().to_string())];
                    loop {
                        let Ok(parent) = iter_commit.parent_ids().exactly_one() else {
                            // initial commit or merge commit; skip parents for simplicity's sake
                            break
                        };
                        if parent == status_latest { break }
                        iter_commit = parent.object()?.peel_to_commit()?;
                        to_add.push((parent.detach(), iter_commit.message()?.summary().to_string()));
                    }
                    status.gefolge_web_future.extend(to_add.into_iter().rev().map(|(commit_hash, commit_msg)| (commit_hash, commit_msg, GefolgeWebCommitStatus::Pending)));
                    true
                } else {
                    false
                }
            })
        }))
    }

    async fn fetch_status(&self) -> Result<bool, Error> {
        Ok(lock!(repo_lock = self.status_repo_lock; {
            Command::new("git").arg("fetch").current_dir(STATUS_REPO_PATH).check("git fetch").await?; //TODO use GitHub API or gix (how?)
            let repo = gix::open(STATUS_REPO_PATH)?;
            let new_head = repo.find_reference("origin/main")?.peel_to_commit()?.id;
            lock!(@write status = self.status; {
                let _ = status.watch.send(());
                let status_latest = status.status_future.last().map_or(GIT_COMMIT_HASH, |(latest, _, _)| *latest);
                if new_head != status_latest {
                    let mut iter_commit = repo.find_commit(new_head)?;
                    let mut to_add = vec![(new_head, iter_commit.message()?.summary().to_string())];
                    loop {
                        let Ok(parent) = iter_commit.parent_ids().exactly_one() else {
                            // initial commit or merge commit; skip parents for simplicity's sake
                            break
                        };
                        if parent == status_latest { break }
                        iter_commit = parent.object()?.peel_to_commit()?;
                        to_add.push((parent.detach(), iter_commit.message()?.summary().to_string()));
                    }
                    status.status_future.extend(to_add.into_iter().rev().map(|(commit_hash, commit_msg)| (commit_hash, commit_msg, StatusCommitStatus::Pending)));
                    true
                } else {
                    false
                }
            })
        }))
    }

    async fn gefolge_web_build_task(&self) -> Option<tokio::task::JoinHandle<Result<gix::ObjectId, Error>>> {
        lock!(@write status = self.status; {
            let _ = status.watch.send(());
            for (_, _, status) in &mut status.gefolge_web_future {
                if let GefolgeWebCommitStatus::Pending = status {
                    *status = GefolgeWebCommitStatus::Bundled;
                }
            }
            if let Some((new_head, _, status @ GefolgeWebCommitStatus::Bundled)) = status.gefolge_web_future.last_mut() {
                *status = GefolgeWebCommitStatus::Deploy;
                let new_head = *new_head;
                Some(tokio::spawn(async move {
                    Command::new("git").arg("reset").arg("--hard").arg(new_head.to_string()).current_dir(GEFOLGE_WEB_BUILD_REPO_PATH).check("git reset").await?;
                    //TODO cargo sweep (limit to once per Rust version)
                    println!("supervisor: building gefolge-web {new_head}");
                    Command::new("cargo").arg("build").arg("--release").arg("--target=x86_64-unknown-linux-musl").arg("--package=gefolge-web").arg("--package=gefolge-web-back").current_dir(GEFOLGE_WEB_BUILD_REPO_PATH).kill_on_drop(true).check("cargo build").await?;
                    Command::new("ssh").arg("gefolge.org").arg("env -C /opt/git/github.com/dasgefolge/gefolge.org/main git pull").check("ssh").await?;
                    Command::new("ssh").arg("gefolge.org").arg("sudo systemctl stop gefolge-web").check("ssh").await?;
                    Command::new("scp").arg(Path::new(GEFOLGE_WEB_BUILD_REPO_PATH).join("target").join("release").join("gefolge-web")).arg("gefolge.org:bin/gefolge-web").check("scp").await?;
                    Command::new("scp").arg(Path::new(GEFOLGE_WEB_BUILD_REPO_PATH).join("target").join("release").join("gefolge-web-back")).arg("gefolge.org:bin/gefolge-web-back").check("scp").await?;
                    Command::new("ssh").arg("gefolge.org").arg("/opt/git/github.com/dasgefolge/gefolge.org/main/assets/deploy.sh").check("ssh").await?;
                    Command::new("ssh").arg("gefolge.org").arg("sudo systemctl start gefolge-web").check("ssh").await?;
                    Ok(new_head)
                }))
            } else {
                None
            }
        })
    }
}
