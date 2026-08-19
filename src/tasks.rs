use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use anyhow::Result;

use crate::config::{Config, ACCOUNT_TYPE_ELY_PASSWORD};
use crate::launch::pump_game_output;
use crate::worker::{LaunchDecision, LaunchRequest, TaskCtx, TaskEvent};

fn spawn_tx(
    tx: Sender<TaskEvent>,
    decisions: Receiver<LaunchDecision>,
    name: &'static str,
    f: impl FnOnce(TaskCtx) -> Result<()> + Send + 'static,
) {
    thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let ctx = match TaskCtx::new(tx.clone(), decisions) {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(TaskEvent::Error(
                        crate::lang::current().init_failed.replace("{}", &e.to_string()),
                    ));
                    let _ = tx.send(TaskEvent::Done(String::new()));
                    return;
                }
            };
            match f(ctx) {
                Ok(()) => {
                    let _ = tx.send(TaskEvent::Done(String::new()));
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    if msg.contains("__NEED_2FA__") {
                        let _ = tx.send(TaskEvent::NeedsTwoFactor);
                    } else {
                        let _ = tx.send(TaskEvent::Error(msg));
                    }
                    let _ = tx.send(TaskEvent::Done(String::new()));
                }
            }
        })
        .expect("gagal membuat thread");
}

pub fn refresh_versions(tx: Sender<TaskEvent>, decisions: Receiver<LaunchDecision>) {
    spawn_tx(tx, decisions, "refresh-versions", |ctx| {
        ctx.reporter.log(crate::lang::current().fetching_versions);
        let manifest = crate::minecraft::VersionManifest::from_remote(&ctx.client)?;
        let _ = ctx.tx.send(TaskEvent::VersionList {
            latest: Some(manifest.latest.release.clone()),
            versions: manifest.versions.clone(),
        });
        ctx.reporter.log(
            crate::lang::current()
                .found_versions
                .replace("{}", &manifest.versions.len().to_string()),
        );
        Ok(())
    });
}

pub fn install_version(tx: Sender<TaskEvent>, decisions: Receiver<LaunchDecision>, version_id: String, root: PathBuf) {
    spawn_tx(tx, decisions, "install", move |ctx| {
        crate::install::install_version(&ctx.client, &root, &version_id, &ctx.reporter)?;
        Ok(())
    });
}

pub fn login_oauth(tx: Sender<TaskEvent>, decisions: Receiver<LaunchDecision>) {
    spawn_tx(tx, decisions, "login-oauth", move |ctx| {
        let account = crate::auth::login_oauth_device(&ctx)?;
        if let Ok(Some((w, h, rgba))) = crate::auth::fetch_avatar(&ctx, &account.uuid) {
            let _ = ctx.tx.send(TaskEvent::AvatarReady {
                uuid: account.uuid.clone(),
                width: w,
                height: h,
                rgba,
            });
        }
        let _ = ctx.tx.send(TaskEvent::AccountAdded(Box::new(account.clone())));
        ctx.reporter.log(
            crate::lang::current()
                .login_success
                .replace("{}", &account.username),
        );
        Ok(())
    });
}

pub fn login_password(tx: Sender<TaskEvent>, decisions: Receiver<LaunchDecision>, username: String, password: String, twofa: Option<String>) {
    spawn_tx(tx, decisions, "login-password", move |ctx| {
        let client_token = uuid::Uuid::new_v4().to_string();
        let account = crate::auth::login_password(
            &ctx.client,
            &username,
            &password,
            &client_token,
            twofa.as_deref(),
        )?;
        if let Ok(Some((w, h, rgba))) = crate::auth::fetch_avatar(&ctx, &account.uuid) {
            let _ = ctx.tx.send(TaskEvent::AvatarReady {
                uuid: account.uuid.clone(),
                width: w,
                height: h,
                rgba,
            });
        }
        let _ = ctx.tx.send(TaskEvent::AccountAdded(Box::new(account.clone())));
        ctx.reporter.log(
            crate::lang::current()
                .login_success
                .replace("{}", &account.username),
        );
        Ok(())
    });
}

pub fn download_java(tx: Sender<TaskEvent>, decisions: Receiver<LaunchDecision>, root: PathBuf, required_major: u32) {
    spawn_tx(tx, decisions, "download-java", move |ctx| {
        let dir = crate::java::download_runtime(&ctx.client, &root, required_major, &ctx.reporter)?;
        let _ = ctx.tx.send(TaskEvent::JavaReady(dir));
        Ok(())
    });
}

pub fn refresh_account(tx: Sender<TaskEvent>, decisions: Receiver<LaunchDecision>, account: crate::config::Account) {
    spawn_tx(tx, decisions, "refresh-account", move |ctx| {
        let updated = if account.account_type == crate::config::ACCOUNT_TYPE_ELY_OAUTH {
            crate::auth::refresh_oauth(&ctx.client, &account)?
        } else {
            crate::auth::refresh_password(&ctx.client, &account)?
        };
        let _ = ctx.tx.send(TaskEvent::AccountAdded(Box::new(updated.clone())));
        ctx.reporter.log(
            crate::lang::current()
                .token_updated
                .replace("{}", &updated.username),
        );
        Ok(())
    });
}

pub fn fetch_avatar(tx: Sender<TaskEvent>, decisions: Receiver<LaunchDecision>, uuid: String) {
    spawn_tx(tx, decisions, "avatar", move |ctx| {
        if let Ok(Some((w, h, rgba))) = crate::auth::fetch_avatar(&ctx, &uuid) {
            let _ = ctx.tx.send(TaskEvent::AvatarReady {
                uuid,
                width: w,
                height: h,
                rgba,
            });
        }
        Ok(())
    });
}

pub fn launch_game(tx: Sender<TaskEvent>, decisions: Receiver<LaunchDecision>, req: LaunchRequest, root: PathBuf) {
    spawn_tx(tx, decisions, "launch", move |ctx| {
        launch_inner(&ctx, &req, &root)?;
        Ok(())
    });
}

fn launch_inner(ctx: &TaskCtx, req: &LaunchRequest, root: &PathBuf) -> Result<()> {
    let lang = crate::lang::current();
    let inst = req
        .config
        .active_instance()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!(lang.no_active_instance_err))?;

    // Resolve which version to launch. The built-in "latest" instance always
    // re-checks the newest stable release from the manifest on every play.
    let mut version_id = match (req.version.clone(), inst.version_id.clone()) {
        (Some(v), _) => Some(v),
        (None, v) => v,
    };
    if inst.is_latest {
        ctx.reporter.log(lang.checking_latest);
        let manifest = crate::minecraft::VersionManifest::from_remote(&ctx.client)?;
        let newest = manifest.latest.release.clone();
        let current = crate::install::newest_installed_release(root, &manifest);
        // Newer release available than what is installed -> ask the user.
        if let Some(cur) = current {
            if cur != newest {
                ctx.reporter.log(
                    lang.newer_available
                        .replace("{}", &newest)
                        .replacen("{}", &cur, 1),
                );
                let _ = ctx.tx.send(TaskEvent::NeedVersionChoice {
                    newest: newest.clone(),
                    current: cur.clone(),
                });
                let decision = ctx.decisions.recv().map_err(|_| {
                    anyhow::anyhow!(lang.choice_dialog_closed)
                })?;
                match decision {
                    LaunchDecision::PlayVersion(v) => {
                        ctx.reporter
                            .log(lang.continuing_version.replace("{}", &v));
                        version_id = Some(v);
                    }
                    LaunchDecision::Cancel => anyhow::bail!(lang.update_cancelled),
                }
            } else {
                version_id = Some(newest);
            }
        } else {
            version_id = Some(newest);
        }
    }
    let version_id =
        version_id.ok_or_else(|| anyhow::anyhow!(lang.no_version_selected_err))?;

    let version = crate::install::load_or_fetch_version(&ctx.client, root, &version_id)?;

    // Resolve Java runtime. The selected Java must be *compatible* with the
    // version: legacy LaunchWrapper versions need Java 8 exactly (Java 9+ and
    // the new module system crash them), while 1.17+ pin 17/21/25.
    let java_path = {
        let required = version.required_java_major();
        let detected = crate::java::detect_java(inst.java_path.as_deref(), root);
        let compatible = |p: &std::path::Path| {
            crate::java::java_major(p)
                .map(|m| crate::java::java_compatible(m, required))
                .unwrap_or(false)
        };
        if let Some(p) = detected {
            if compatible(&p) {
                p
            } else {
                // Wrong major: prefer a cached runtime that fits, otherwise
                // download the correct one.
                ctx.reporter.log(
                    lang.java_too_old
                        .replace("{}", &crate::java::java_major(&p).unwrap_or(0).to_string())
                        .replacen("{}", &required.to_string(), 1),
                );
                if let Some(cached) = crate::java::find_cached_runtime_major(root, required) {
                    cached.join("bin").join(crate::java::java_binary_name())
                } else if inst.download_java {
                    let dir = crate::java::download_runtime(
                        &ctx.client,
                        root,
                        required,
                        &ctx.reporter,
                    )?;
                    dir.join("bin").join(crate::java::java_binary_name())
                } else {
                    anyhow::bail!(
                        lang.java_mismatch
                            .replace("{}", &crate::java::java_major(&p).unwrap_or(0).to_string())
                            .replacen("{}", &required.to_string(), 1)
                    );
                }
            }
        } else if inst.download_java {
            let dir = crate::java::download_runtime(
                &ctx.client,
                root,
                required,
                &ctx.reporter,
            )?;
            dir.join("bin").join(crate::java::java_binary_name())
        } else {
            anyhow::bail!(
                lang.java_missing
                    .replace(
                        "{}",
                        &version.required_java_major().to_string()
                    )
            );
        }
    };
    let java_major = crate::java::java_major(&java_path)?;
    ctx.reporter
        .log(lang.using_java.replace("{}", &java_major.to_string()).replacen("{}", &java_path.display().to_string(), 1));

    // Make sure the version is fully installed (libraries, client jar,
    // assets). A previous interrupted install is repaired here.
    if !crate::install::verify_version_installed(root, &version) {
        ctx.reporter.log(lang.repairing_install);
        crate::install::install_version(&ctx.client, root, &version_id, &ctx.reporter)?;
    }

    let authlib_jar = crate::install::ensure_authlib_injector(&ctx.client, root, &ctx.reporter)?;

    let game_dir = inst.game_dir_for(root);
    let _ = std::fs::create_dir_all(&game_dir);
    ctx.reporter
        .log(format!("gameDir: {}", game_dir.to_string_lossy()));

    let mut spec = crate::launch::build_launch_command(
        &java_path,
        root,
        &game_dir,
        &version,
        &req.account,
        inst.memory_mb,
        inst.width,
        inst.height,
        &inst.extra_jvm_args,
        &inst.authlib_url,
        &authlib_jar,
        &req.config.brand,
        &req.config.channel,
    )?;

    ctx.reporter.log(lang.launch_args);
    for arg in &spec.display_args {
        ctx.reporter.log(format!("  {arg}"));
    }

    let mut child = crate::launch::spawn_game(&mut spec)?;
    ctx.reporter
        .log(lang.game_launched_pid.replace("{}", &child.id().to_string()));
    let _ = ctx.tx.send(TaskEvent::GameStarted(child.id()));

    let tx = ctx.tx.clone();
    let code = pump_game_output(&mut child, move |line| {
        let _ = tx.send(TaskEvent::GameOutput(line.to_string()));
    })?;

    let _ = ctx.tx.send(TaskEvent::GameExited(code));
    ctx.reporter
        .log(lang.game_exited.replace("{}", &code.to_string()));
    Ok(())
}

/// Force-kill a game process by PID.
pub fn kill_game(pid: u32) {
    thread::spawn(move || {
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/F", "/T", "/PID", &pid.to_string()]);
        // Never pop a console window for the kill command.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let _ = cmd.output();
    });
}

#[allow(dead_code)]
pub fn account_label(account: &crate::config::Account) -> String {
    let lang = crate::lang::current();
    if account.account_type == ACCOUNT_TYPE_ELY_PASSWORD {
        lang.account_type_password
            .replace("{}", &account.username)
    } else {
        lang.account_type_oauth.replace("{}", &account.username)
    }
}

pub fn _type_hint(_c: &Config) {}
