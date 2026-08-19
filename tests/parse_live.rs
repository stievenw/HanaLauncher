//! Live smoke test: parse the real Mojang version manifest and a real
//! version JSON to validate the serde structures. Needs network access.

#[test]
fn parse_live_manifest_and_version() {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    let manifest_text = client
        .get("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")
        .send()
        .expect("gagal fetch manifest")
        .text()
        .unwrap();
    let manifest: hana_launcher::minecraft::VersionManifest =
        serde_json::from_str(&manifest_text).expect("manifest tidak bisa diparse");
    assert!(!manifest.versions.is_empty());
    assert!(manifest.latest.release.len() > 2);

    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == manifest.latest.release)
        .expect("release tidak ada di daftar");

    let version_text = client.get(&entry.url).send().unwrap().text().unwrap();
    let version: hana_launcher::minecraft::Version =
        serde_json::from_str(&version_text).expect("version json tidak bisa diparse");
    assert!(version.main_class.contains("net.minecraft"));
    assert!(!version.libraries.is_empty());
    assert_eq!(version.asset_index_name(), &version.asset_index.id);
    assert!(version.required_java_major() >= 8);

    // Evaluate library rules for the current platform.
    let ctx = hana_launcher::minecraft::RuleContext::current();
    let allowed: Vec<_> = version
        .libraries
        .iter()
        .filter(|l| l.is_allowed(&ctx))
        .collect();
    assert!(!allowed.is_empty());

    // Older versions (<= 1.18.x) carry a `natives` map + classifier downloads;
    // validate that native classifier resolution works there.
    let old_entry = manifest
        .versions
        .iter()
        .find(|v| v.id == "1.18.2")
        .expect("1.18.2 tidak ada di daftar");
    let old_text = client.get(&old_entry.url).send().unwrap().text().unwrap();
    let old: hana_launcher::minecraft::Version =
        serde_json::from_str(&old_text).expect("version json 1.18.2 tidak bisa diparse");
    let natives: Vec<_> = old
        .libraries
        .iter()
        .filter(|l| l.natives.is_some() && l.is_allowed(&ctx))
        .collect();
    assert!(!natives.is_empty(), "1.18.2 seharusnya punya library native");
    let mut resolved_native = false;
    for lib in &natives {
        if let Some((_artifact, is_native)) = lib.artifact() {
            if is_native {
                resolved_native = true;
                break;
            }
        }
    }
    assert!(resolved_native, "classifier native tidak bisa di-resolve");
}

#[test]
fn build_launch_command_smoke() {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();
    let manifest_text = client
        .get("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")
        .send()
        .unwrap()
        .text()
        .unwrap();
    let manifest: hana_launcher::minecraft::VersionManifest =
        serde_json::from_str(&manifest_text).unwrap();
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == "1.18.2")
        .unwrap();
    let version_text = client.get(&entry.url).send().unwrap().text().unwrap();
    let version: hana_launcher::minecraft::Version = serde_json::from_str(&version_text).unwrap();

    let account = hana_launcher::config::Account {
        uuid: "12345678-1234-1234-1234-123456789abc".to_string(),
        username: "TestPlayer".to_string(),
        access_token: "tok-123".to_string(),
        refresh_token: None,
        client_token: Some("ct-123".to_string()),
        account_type: hana_launcher::config::ACCOUNT_TYPE_ELY_PASSWORD.to_string(),
        expires_at: Some(0),
    };
    let root = std::env::temp_dir().join("hana_test_launch_root");
    let java = root.join("java.exe");
    let authlib = root.join("authlib-injector.jar");

    let spec = hana_launcher::launch::build_launch_command(
        &java,
        &root,
        &root,
        &version,
        &account,
        1024,
        1280,
        720,
        "",
        "ely.by",
        &authlib,
        "hana",
        "hanakama",
    )
    .expect("build_launch_command gagal");

    let args: Vec<String> = spec
        .command
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let joined = args.join(" ");

    // Core protocol flags must be present and placeholders replaced.
    assert!(joined.contains("--username TestPlayer"), "user flag");
    assert!(
        joined.contains("--accessToken tok-123"),
        "access token flag: {joined}"
    );
    assert!(joined.contains("net.minecraft.client.main.Main"));
    assert!(joined.contains("-cp"));
    assert!(joined.contains("javaagent"));
    assert!(joined.contains("ely.by"));
    assert!(joined.contains("1.18.2.jar"), "client jar di classpath");
    // Classpath must not contain native jars (they are extracted instead).
    assert!(
        !joined.contains("natives-windows.jar"),
        "native jar tidak boleh di classpath"
    );

    // macOS-only JVM flag must never be passed on Windows, and demo/quick-play
    // feature args must not leak through as unresolved placeholders.
    assert!(
        !joined.contains("-XstartOnFirstThread"),
        "flag osx bocor ke windows: {joined}"
    );
    assert!(!joined.contains("--demo"), "demo arg tidak boleh ada: {joined}");
    assert!(
        !joined.contains("${quickPlay"),
        "placeholder quick play tidak boleh ada: {joined}"
    );
    assert!(
        !joined.contains("${resolution"),
        "placeholder resolusi tidak boleh ada: {joined}"
    );
    assert!(
        !joined.contains("${clientid}") && !joined.contains("${auth_xuid}"),
        "placeholder clientid/xuid tidak boleh ada: {joined}"
    );

    // The logged copy must be redacted while the real command keeps the token.
    let display = spec.display_args.join(" ");
    assert!(
        !display.contains("tok-123"),
        "token bocor di display_args: {display}"
    );
    assert!(!display.contains("-XstartOnFirstThread"), "display args bocor");
}

#[test]
fn parse_asset_index() {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();
    let manifest_text = client
        .get("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json")
        .send()
        .unwrap()
        .text()
        .unwrap();
    let manifest: hana_launcher::minecraft::VersionManifest =
        serde_json::from_str(&manifest_text).unwrap();
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == "1.18.2")
        .unwrap();
    let version_text = client.get(&entry.url).send().unwrap().text().unwrap();
    let version: hana_launcher::minecraft::Version = serde_json::from_str(&version_text).unwrap();

    // Download the real asset index (with sha1 verification against the
    // version metadata) and parse it.
    let bytes = client.get(&version.asset_index.url).send().unwrap().bytes().unwrap();
    let actual_sha1 = hana_launcher::util::hex(&<sha1::Sha1 as sha1::Digest>::digest(&bytes));
    assert_eq!(
        Some(actual_sha1.as_str()),
        version.asset_index.sha1.as_deref(),
        "sha1 index aset"
    );
    let index: hana_launcher::minecraft::AssetIndexJson =
        serde_json::from_slice(&bytes).expect("asset index tidak bisa diparse");
    assert!(!index.objects.is_empty());
    let some = index.objects.values().next().unwrap();
    assert_eq!(some.hash.len(), 40);
}