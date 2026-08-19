#![allow(dead_code)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use anyhow::{anyhow, Context, Result};

use crate::config::Account;
use crate::install::{client_jar_path, library_file_path, natives_dir};
use crate::minecraft::{RuleContext, Version};
use crate::util::{LAUNCHER_NAME, LAUNCHER_VERSION};

pub struct LaunchSpec {
    pub command: Command,
    pub java_path: PathBuf,
    pub classpath: String,
    pub display_args: Vec<String>,
}

fn classpath_sep() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

fn build_classpath(root: &Path, version: &Version) -> Result<String> {
    let ctx = RuleContext::current();
    let mut paths: Vec<PathBuf> = Vec::new();
    for lib in &version.libraries {
        if !lib.is_allowed(&ctx) {
            continue;
        }
        if let Some((artifact, is_native)) = lib.artifact() {
            if is_native {
                continue;
            }
            match &artifact.path {
                Some(p) => paths.push(root.join("libraries").join(p)),
                None => paths.push(library_file_path(root, lib)?),
            }
        } else {
            // Library without downloads info (legacy) - derive path from name.
            paths.push(library_file_path(root, lib)?);
        }
    }
    paths.push(client_jar_path(root, &version.id));
    Ok(paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(classpath_sep()))
}

fn replace_placeholders(arg: &str, map: &HashMap<String, String>) -> String {
    let mut out = arg.to_string();
    for (k, v) in map {
        out = out.replace(k, v);
    }
    out
}

pub fn build_launch_command(
    java_path: &Path,
    root: &Path,
    game_dir: &Path,
    version: &Version,
    account: &Account,
    memory_mb: u32,
    width: u32,
    height: u32,
    extra_jvm_args: &str,
    authlib_url: &str,
    authlib_jar_path: &Path,
    brand: &str,
    channel: &str,
) -> Result<LaunchSpec> {
    let natives = natives_dir(root, &version.id);
    let classpath = build_classpath(root, version)?;

    let mut map = HashMap::new();
    map.insert("${auth_player_name}".to_string(), account.username.clone());
    map.insert(
        "${auth_session}".to_string(),
        format!("token:{}:{}", account.access_token, account.uuid_no_dashes()),
    );
    map.insert("${auth_access_token}".to_string(), account.access_token.clone());
    map.insert("${auth_uuid}".to_string(), account.uuid.clone());
    map.insert("${user_type}".to_string(), "mojang".to_string());
    map.insert("${version_name}".to_string(), version.id.clone());
    map.insert("${version_type}".to_string(), version.type_label().to_string());
    map.insert("${assets_index_name}".to_string(), version.asset_index_name().to_string());
    map.insert("${assets_root}".to_string(), root.join("assets").to_string_lossy().into_owned());
    map.insert("${game_directory}".to_string(), game_dir.to_string_lossy().into_owned());
    map.insert("${game_dir}".to_string(), game_dir.to_string_lossy().into_owned());
    // Legacy versions use `--assetsDir ${game_assets}` and read the unpacked
    // asset folder directly (see install::unpack_dir_for_index).
    map.insert(
        "${game_assets}".to_string(),
        crate::install::unpack_dir_for_index(root, &version.asset_index.id)
            .filter(|d| d.join(".complete").exists())
            .unwrap_or_else(|| root.join("assets"))
            .to_string_lossy()
            .into_owned(),
    );
    map.insert("${natives_directory}".to_string(), natives.to_string_lossy().into_owned());
    map.insert("${launcher_name}".to_string(), LAUNCHER_NAME.to_string());
    map.insert("${launcher_version}".to_string(), LAUNCHER_VERSION.to_string());
    map.insert("${classpath}".to_string(), classpath.clone());
    map.insert("${resolution_width}".to_string(), width.to_string());
    map.insert("${resolution_height}".to_string(), height.to_string());
    map.insert("${user_properties}".to_string(), "{}".to_string());
    map.insert("${username}".to_string(), account.username.clone());
    // Offline/ELY sessions have no Microsoft client id or xuid.
    map.insert("${clientid}".to_string(), String::new());
    map.insert("${auth_xuid}".to_string(), String::new());

    let mut ctx = RuleContext::current();
    // We always pass a custom resolution (the instance's width/height).
    ctx.has_custom_resolution = true;

    // ---- JVM arguments ----
    let mut jvm: Vec<String> = match &version.arguments {
        Some(args) => args.jvm.iter().flat_map(|a| a.to_strings(&ctx)).collect(),
        None => Vec::new(),
    };

    // Ensure natives + -cp are present.
    let natives_arg = format!("-Djava.library.path={}", natives.display());
    if !jvm.iter().any(|a| a.contains("${natives_directory}") || a.contains("-Djava.library.path")) {
        jvm.push(natives_arg.clone());
    }
    let has_cp = jvm.iter().any(|a| a.contains("${classpath}") || a == "-cp");
    if !has_cp {
        jvm.push("-cp".to_string());
        jvm.push("${classpath}".to_string());
    }

    // Memory + branding + authlib-injector + user extra args.
    jvm.insert(0, format!("-Xmx{memory_mb}M"));
    jvm.insert(1, format!("-Xms{}M", memory_mb.min(512).max(256)));
    jvm.push(format!("-Dminecraft.launcher.brand={LAUNCHER_NAME}"));
    jvm.push(format!("-Dminecraft.launcher.version={LAUNCHER_VERSION}"));
    // Legacy Launcher style brand/channel used for bootstrap + statistics profile.
    jvm.push(format!("-Dtlauncher.bootstrap.brand={brand}"));
    jvm.push(format!("-Dtlauncher.bootstrap.channel={channel}"));
    // Older Java runtimes (8u for legacy versions) need TLS 1.2 forced or the
    // authlib-injector / auth endpoints fail with a TLS handshake error.
    jvm.push("-Djdk.tls.client.protocols=TLSv1.2".to_string());
    jvm.push("-Dhttps.protocols=TLSv1.2".to_string());
    jvm.push("-Dcom.sun.net.ssl.checkRevocation=false".to_string());
    // authlib-injector only makes sense for authlib-based versions (1.6+);
    // pre-1.6 games take the username/token straight from the launch args.
    if account.is_ely() && version.uses_authlib() {
        jvm.push(format!(
            "-javaagent:{}={}",
            authlib_jar_path.display(),
            authlib_url
        ));
    }
    jvm.extend(extra_jvm_args.split_whitespace().map(|s| s.to_string()));
    let jvm: Vec<String> = jvm.into_iter().map(|a| replace_placeholders(&a, &map)).collect();

    // ---- Game arguments ----
    let game: Vec<String> = match &version.arguments {
        Some(args) => {
            // Add the flags the launcher always appends (modern protocol).
            let mut g: Vec<String> = args.game.iter().flat_map(|a| a.to_strings(&ctx)).collect();
            let needed = [
                ("--username", "${auth_player_name}"),
                ("--version", "${version_name}"),
                ("--gameDir", "${game_directory}"),
                ("--assetsDir", "${assets_root}"),
                ("--assetIndex", "${assets_index_name}"),
                ("--uuid", "${auth_uuid}"),
                ("--accessToken", "${auth_access_token}"),
                ("--userType", "${user_type}"),
                ("--versionType", "${version_type}"),
            ];
            for (flag, placeholder) in needed {
                if !g.iter().any(|a| a == flag) {
                    g.push(flag.to_string());
                    g.push(placeholder.to_string());
                }
            }
            g
        }
        None => {
            let template = version
                .minecraft_arguments
                .as_deref()
                .ok_or_else(|| anyhow!(crate::lang::current().no_launch_args))?;
            template.split_whitespace().map(|s| s.to_string()).collect()
        }
    };
    let game: Vec<String> = game.into_iter().map(|a| replace_placeholders(&a, &map)).collect();

    let mut command = Command::new(java_path);
    command
        .current_dir(game_dir)
        .args(&jvm)
        .arg(&version.main_class)
        .args(&game)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Never pop up a console window for the game process (or its children).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    // Logged copy of the arguments must never leak the access token.
    let token = &account.access_token;
    let redact = |arg: String| -> String {
        if !token.is_empty() {
            arg.replace(token, "****")
        } else {
            arg
        }
    };
    let display_args: Vec<String> = [jvm, vec![version.main_class.clone()], game]
        .concat()
        .into_iter()
        .map(redact)
        .collect();

    Ok(LaunchSpec {
        command,
        java_path: java_path.to_path_buf(),
        classpath,
        display_args,
    })
}

pub fn spawn_game(spec: &mut LaunchSpec) -> Result<Child> {
    spec.command
        .spawn()
        .context(crate::lang::current().failed_launch_game)
}

/// Read game stdout/stderr line by line and forward them through the reporter.
pub fn pump_game_output(
    child: &mut Child,
    on_line: impl Fn(&str) + Send + 'static,
) -> Result<i32> {
    use std::io::{BufRead, BufReader};
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let handle = std::thread::spawn(move || {
        if let Some(out) = stdout {
            let reader = BufReader::new(out);
            for line in reader.lines().map_while(Result::ok) {
                on_line(&line);
            }
        }
        if let Some(err) = stderr {
            let reader = BufReader::new(err);
            for line in reader.lines().map_while(Result::ok) {
                on_line(&line);
            }
        }
    });

    let status = child
        .wait()
        .context(crate::lang::current().failed_wait_game)?;
    let _ = handle.join();
    Ok(status.code().unwrap_or(-1))
}