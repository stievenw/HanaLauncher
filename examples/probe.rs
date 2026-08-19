//! Diagnostic probe: launch the real game like the launcher does and print the
//! game's stdout/stderr so we can see why the window never appears.
use std::io::{BufRead, BufReader};
use std::sync::mpsc::channel;
use std::time::Duration;

use hana_launcher::config::Account;
use hana_launcher::worker::TaskEvent;

fn redact(line: &str, account: &Account) -> String {
    let mut out = line.to_string();
    if !account.access_token.is_empty() {
        out = out.replace(&account.access_token, "****");
    }
    if let Some(rt) = &account.refresh_token {
        out = out.replace(rt, "****");
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = hana_launcher::config::load_config();
    let account = cfg
        .active_account()
        .expect("tidak ada akun aktif")
        .clone();
    let inst = cfg
        .active_instance()
        .expect("tidak ada instans aktif")
        .clone();
    let root = hana_launcher::config::minecraft_root()?;

    let (tx, _rx) = channel::<TaskEvent>();
    let reporter = hana_launcher::worker::Reporter::new(tx);
    let client = reqwest::blocking::Client::new();

    let version_id = if inst.is_latest {
        let m = hana_launcher::minecraft::VersionManifest::from_remote(&client)?;
        m.latest.release.clone()
    } else {
        inst.version_id.clone().expect("tidak ada versi dipilih")
    };
    let version = hana_launcher::install::load_or_fetch_version(&client, &root, &version_id)?;

    if !hana_launcher::install::verify_version_installed(&root, &version) {
        println!("=== VERIFY: TIDAK LENGKAP, memperbaiki...");
        hana_launcher::install::install_version(&client, &root, &version_id, &reporter)?;
        println!("=== VERIFY: PERBAIKAN SELESAI");
    } else {
        println!("=== VERIFY: LENGKAP");
    }

    let java =
        hana_launcher::java::detect_java(inst.java_path.as_deref(), &root).expect("java tidak ditemukan");
    let authlib = hana_launcher::install::ensure_authlib_injector(&client, &root, &reporter)?;

    let mut spec = hana_launcher::launch::build_launch_command(
        &java,
        &root,
        &root,
        &version,
        &account,
        inst.memory_mb,
        inst.width,
        inst.height,
        &inst.extra_jvm_args,
        &inst.authlib_url,
        &authlib,
        &cfg.brand,
        &cfg.channel,
    )?;

    println!("=== JAVA: {}", java.display());
    println!("=== VERSION: {version_id}");
    println!("=== MAIN: {}", version.main_class);
    println!("=== CLASSPATH JARS: {}", spec.classpath.split(';').count());

    let mut child = spec.command.spawn()?;
    println!("=== PID: {}", child.id());

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let acc = account.clone();
    let h = std::thread::spawn(move || {
        let mut read = |tag: &str, r: Box<dyn BufRead + Send>| {
            for line in r.lines().map_while(Result::ok) {
                println!("[{tag}] {}", redact(&line, &acc));
            }
        };
        if let Some(o) = stdout {
            read("stdout", Box::new(BufReader::new(o)));
        }
        if let Some(e) = stderr {
            read("stderr", Box::new(BufReader::new(e)));
        }
    });

    std::thread::sleep(Duration::from_secs(45));

    let mut k = std::process::Command::new("taskkill");
    k.args(["/F", "/T", "/PID", &child.id().to_string()]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        k.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = k.output();
    let _ = child.wait();
    let _ = h.join();
    println!("=== DONE");
    Ok(())
}
