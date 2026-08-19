use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crate::config::{Account, Config};
use crate::minecraft::ManifestVersion;

/// Answer sent back from the UI thread when a background task asks the user
/// to choose something (e.g. update the "latest" instance or keep playing).
#[derive(Debug, Clone)]
pub enum LaunchDecision {
    /// Play the given version id.
    PlayVersion(String),
    /// Abort the current task.
    Cancel,
}

#[derive(Debug, Clone)]
pub enum TaskEvent {
    /// Generic progress for downloads etc.
    Progress { stage: String, current: u64, total: u64 },
    /// Informational log line shown in the console / status area.
    Log(String),
    /// A serious error occurred during the current task.
    Error(String),
    /// Task finished successfully with an optional human readable summary.
    Done(String),
    /// A new account was logged in through a worker thread.
    AccountAdded(Box<Account>),
    /// Avatar (head skin) bytes decoded as RGBA.
    AvatarReady { uuid: String, width: u32, height: u32, rgba: Vec<u8> },
    /// The remote version list was fetched (newest stable release + full list).
    VersionList { latest: Option<String>, versions: Vec<ManifestVersion> },
    /// Game process produced a line of output.
    GameOutput(String),
    /// Game process was spawned successfully.
    GameStarted(u32),
    /// Game process exited with the given code.
    GameExited(i32),
    /// Java runtime was downloaded/extracted to this directory.
    JavaReady(PathBuf),
    /// The "latest" instance found a newer stable release than the one
    /// installed; the UI must ask the user to continue or update.
    NeedVersionChoice { newest: String, current: String },
    /// Password login requires a TOTP code (2FA).
    NeedsTwoFactor,
    /// OAuth device flow: the user must enter this code at the verification page.
    DeviceCodeRequired { code: String, verification_uri: String },
}

/// Small helper to push events from worker threads back to the UI thread.
#[derive(Clone)]
pub struct Reporter {
    tx: Sender<TaskEvent>,
}

impl Reporter {
    pub fn new(tx: Sender<TaskEvent>) -> Self {
        Self { tx }
    }

    pub fn progress(&self, stage: &str, current: u64, total: u64) {
        let _ = self
            .tx
            .send(TaskEvent::Progress {
                stage: stage.to_string(),
                current,
                total,
            });
    }

    pub fn log(&self, msg: impl Into<String>) {
        let _ = self.tx.send(TaskEvent::Log(msg.into()));
    }

    #[allow(dead_code)]
    pub fn error(&self, msg: impl Into<String>) {
        let _ = self.tx.send(TaskEvent::Error(msg.into()));
    }
}

/// Shared context handed to worker threads.
pub struct TaskCtx {
    pub client: reqwest::blocking::Client,
    pub reporter: Reporter,
    pub tx: Sender<TaskEvent>,
    /// Channel on which the UI thread answers questions the worker asks.
    pub decisions: Receiver<LaunchDecision>,
}

impl TaskCtx {
    pub fn new(
        tx: Sender<TaskEvent>,
        decisions: Receiver<LaunchDecision>,
    ) -> anyhow::Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .timeout(std::time::Duration::from_secs(300))
            .user_agent(format!(
                "{}/{}",
                crate::util::LAUNCHER_NAME,
                crate::util::LAUNCHER_VERSION
            ))
            .build()?;
        Ok(Self {
            client,
            reporter: Reporter::new(tx.clone()),
            tx,
            decisions,
        })
    }
}

/// Configuration snapshot passed to background tasks.
#[derive(Clone)]
pub struct LaunchRequest {
    pub config: Config,
    pub account: Account,
    /// Resolved version id (already computed for the "latest" instance).
    pub version: Option<String>,
}
