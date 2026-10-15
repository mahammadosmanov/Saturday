use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::os::unix::fs::MetadataExt;
use std::io::Read;
fn digest(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

fn file_digest(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| format!("{}: {error}", path.display()))?;
        if count == 0 { break; }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn hash_roster(entries: &[(String, String)]) -> String {
    let mut hash = Sha256::new();
    for (path, sha) in entries {
        hash.update(path.as_bytes());
        hash.update([0]);
        hash.update(sha.as_bytes());
        hash.update([b'\n']);
    }
    format!("{:x}", hash.finalize())
}

fn sha_field(value: &serde_json::Value) -> Result<&str, String> {
    let value = value.as_str().ok_or_else(|| "digest is not a string".to_owned())?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err("digest is not lowercase SHA-256".into());
    }
    Ok(value)
}

fn raw_digest(value: &str) -> Result<Vec<u8>, String> {
    if value.len() != 64 { return Err("invalid SHA-256 digest width".into()); }
    (0..64).step_by(2).map(|index| u8::from_str_radix(&value[index..index + 2], 16).map_err(|error| error.to_string())).collect()
}
const OWNER_APPLICATION_TAG: &[u8] = b"dev.rohtolos.saturday.voice.owner-key.v1";
const OWNER_PUBLIC_KEY_SOURCE_PATH: &str = "src-tauri/generated/trusted_owner_public_key.x963";
const OWNER_KEY_PIN_SOURCE_PATH: &str = "src-tauri/generated/voice-owner-key-pin.json";

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct OwnerKeyPin {
    application_tag_sha256: String,
    bootstrap_record: String,
    bootstrap_record_sha256: String,
    bootstrap_witness: String,
    bootstrap_witness_sha256: String,
    owner_public_key_sha256: String,
    owner_public_key_x963_hex: String,
    schema: String,
}

fn decode_base64url_unpadded(encoded: &str) -> Result<Vec<u8>, String> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let input = encoded.as_bytes();
    if input.is_empty() || input.len() % 4 == 1 {
        return Err("invalid unpadded base64url bootstrap record".into());
    }
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut index = 0;
    while index + 4 <= input.len() {
        let a = value(input[index]).ok_or("invalid base64url character")? as u32;
        let b = value(input[index + 1]).ok_or("invalid base64url character")? as u32;
        let c = value(input[index + 2]).ok_or("invalid base64url character")? as u32;
        let d = value(input[index + 3]).ok_or("invalid base64url character")? as u32;
        let bits = (a << 18) | (b << 12) | (c << 6) | d;
        output.extend_from_slice(&[(bits >> 16) as u8, (bits >> 8) as u8, bits as u8]);
        index += 4;
    }
    let remaining = input.len() - index;
    if remaining >= 2 {
        let a = value(input[index]).ok_or("invalid base64url character")? as u32;
        let b = value(input[index + 1]).ok_or("invalid base64url character")? as u32;
        output.push(((a << 2) | (b >> 4)) as u8);
        if remaining == 2 && b & 0x0f != 0 {
            return Err("noncanonical base64url trailing bits".into());
        }
        if remaining == 3 {
            let c = value(input[index + 2]).ok_or("invalid base64url character")? as u32;
            output.push(((b << 4) | (c >> 2)) as u8);
            if c & 0x03 != 0 {
                return Err("noncanonical base64url trailing bits".into());
            }
        }
    }
    Ok(output)
}

fn owner_public_key_pin(tauri: &Path) -> Result<(Vec<u8>, String, String), String> {
    #[link(name = "c")]
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    let directory = tauri.join("target/voice-owner-build");
    let pin_path = directory.join("voice-owner-key-pin.json");
    let directory_metadata = fs::symlink_metadata(&directory)
        .map_err(|error| format!("private owner-key directory: {error}"))?;
    if directory_metadata.file_type().is_symlink()
        || !directory_metadata.is_dir()
        || directory_metadata.uid() != unsafe { geteuid() }
        || directory_metadata.mode() & 0o777 != 0o700
    {
        return Err("private owner-key directory is not current-user-owned mode 0700".into());
    }
    let metadata = fs::symlink_metadata(&pin_path)
        .map_err(|error| format!("final owner-key pin: {error}"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.mode() & 0o777 != 0o600
        || metadata.len() == 0
        || metadata.len() > 16_384
    {
        return Err("final owner-key pin is not a private bounded regular file".into());
    }
    use std::os::unix::fs::OpenOptionsExt;
    let mut pin_file = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&pin_path).map_err(|error| error.to_string())?;
    let opened_metadata = pin_file.metadata().map_err(|error| error.to_string())?;
    if opened_metadata.dev() != metadata.dev() || opened_metadata.ino() != metadata.ino() {
        return Err("owner-key pin changed during the bounded open".into());
    }
    let mut pin_bytes = Vec::with_capacity(metadata.len() as usize);
    pin_file.take(16_385).read_to_end(&mut pin_bytes).map_err(|error| error.to_string())?;
    if pin_bytes.len() as u64 != metadata.len() || pin_bytes.len() > 16_384 {
        return Err("owner-key pin size changed during the bounded read".into());
    }
    let pin: OwnerKeyPin = serde_json::from_slice(&pin_bytes)
        .map_err(|error| format!("invalid final owner-key pin: {error}"))?;
    let mut canonical_pin = serde_json::to_vec(&pin).map_err(|error| error.to_string())?;
    canonical_pin.push(b'\n');
    if canonical_pin != pin_bytes || pin.schema != "saturday.voice.owner-key-pin.v1" {
        return Err("final owner-key pin is noncanonical or has an unexpected schema".into());
    }
    let bootstrap_record = decode_base64url_unpadded(&pin.bootstrap_record)?;
    if bootstrap_record.len() > 8192 || digest(&bootstrap_record) != pin.bootstrap_record_sha256 {
        return Err("signed owner-key bootstrap record digest is invalid".into());
    }
    let authenticated_key = saturday_voice_parent_auth::verify_owner_key_bootstrap_record(&bootstrap_record)
        .map_err(|error| format!("signed owner-key bootstrap verification failed: {error:?}"))?;
    let bootstrap_witness = decode_base64url_unpadded(&pin.bootstrap_witness)?;
    if bootstrap_witness.len() > 8192 || digest(&bootstrap_witness) != pin.bootstrap_witness_sha256 {
        return Err("signed owner-key bootstrap witness digest is invalid".into());
    }
    saturday_voice_parent_auth::verify_owner_key_bootstrap_witness(&bootstrap_witness, &bootstrap_record)
        .map_err(|error| format!("live owner-key bootstrap witness verification failed: {error:?}"))?;
    let supplied_hex = env::var("SATURDAY_VOICE_TRUSTED_OWNER_PUBLIC_KEY_X963_HEX")
        .map_err(|_| "worker owner public-key pin was not supplied")?;
    if pin.owner_public_key_x963_hex != supplied_hex || supplied_hex.len() != 130
        || !supplied_hex.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !supplied_hex.starts_with("04")
    {
        return Err("worker owner public-key pin is not canonical uncompressed X9.63".into());
    }
    let key = (0..supplied_hex.len()).step_by(2)
        .map(|index| u8::from_str_radix(&supplied_hex[index..index + 2], 16).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    if key.len() != 65 || key[0] != 0x04 || key[1..].iter().all(|byte| *byte == 0)
        || key != authenticated_key
    {
        return Err("worker key differs from the authenticated owner-key bootstrap record".into());
    }
    let key_sha = digest(&key);
    let expected_sha = env::var("SATURDAY_VOICE_TRUSTED_OWNER_PUBLIC_KEY_SHA256")
        .map_err(|_| "worker owner public-key digest was not supplied")?;
    if expected_sha != key_sha
        || pin.owner_public_key_sha256 != key_sha
        || pin.application_tag_sha256 != digest(OWNER_APPLICATION_TAG)
    {
        return Err("worker owner public-key or application-tag digest mismatch".into());
    }
    println!("cargo:rerun-if-changed={}", pin_path.display());
    println!("cargo:rerun-if-env-changed=SATURDAY_VOICE_TRUSTED_OWNER_PUBLIC_KEY_X963_HEX");
    println!("cargo:rerun-if-env-changed=SATURDAY_VOICE_TRUSTED_OWNER_PUBLIC_KEY_SHA256");
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    let output_path = output_dir.join("trusted_owner_public_key.x963");
    use std::io::Write;
    let mut output = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600)
        .open(&output_path).map_err(|error| format!("write pinned owner public key: {error}"))?;
    output.write_all(&key).map_err(|error| error.to_string())?;
    output.sync_all().map_err(|error| error.to_string())?;
    Ok((key, key_sha, digest(&pin_bytes)))
}

fn string_list(value: &serde_json::Value) -> Result<Vec<&str>, String> {
    value.as_array().ok_or_else(|| "expected string array".to_owned())?.iter()
        .map(|item| item.as_str().ok_or_else(|| "expected string array item".to_owned())).collect()
}

fn files(root: &Path, relative: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let directory = root.join(relative);
    for entry in fs::read_dir(&directory).map_err(|error| format!("{}: {error}", directory.display()))? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let path = relative.join(entry.file_name());
        if file_type.is_symlink() { return Err(format!("symlink in sealed tree: {}", path.display())); }
        if file_type.is_dir() { files(root, &path, out)?; }
        else if file_type.is_file() { out.push(path); }
        else { return Err(format!("non-regular item in sealed tree: {}", path.display())); }
    }
    Ok(())
}
fn promoted_tree_digest(root: &Path) -> Result<String, String> {
    let mut paths = Vec::new();
    files(root, Path::new("."), &mut paths)?;
    let mut entries = Vec::new();
    for path in paths {
        let relative = path.strip_prefix(Path::new(".")).map_err(|error| error.to_string())?.to_string_lossy().replace('\\', "/");
        let full_path = root.join(&path);
        let metadata = fs::symlink_metadata(&full_path).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!("invalid promoted runtime item: {}", full_path.display()));
        }
        let file_sha = raw_digest(&file_digest(&full_path)?)?;
        entries.push((relative, metadata.mode() & 0o7777, metadata.size(), file_sha));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hash = Sha256::new();
    for (path, mode, size, file_sha) in entries {
        hash.update(path.as_bytes());
        hash.update([0]);
        hash.update(format!("{mode:04o}").as_bytes());
        hash.update([0]);
        hash.update(size.to_string().as_bytes());
        hash.update([0]);
        hash.update(file_sha);
        hash.update([b'\n']);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn build() -> Result<(), String> {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") || env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("aarch64") {
        return Err("voice workers are supported only for the reviewed arm64 macOS target".into());
    }
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("missing CARGO_MANIFEST_DIR")?);
    let tauri = manifest_dir.parent().ok_or("invalid worker package path")?;
    let repo = tauri.parent().ok_or("invalid Tauri package path")?;
    let pkg = env::var("CARGO_PKG_NAME").map_err(|error| error.to_string())?;
    let (role, archive_rel, provider, bridge, bridge_header, archive_name, source_commit, source_tree, source_archive_sha) = match pkg.as_str() {
        "saturday-voice-asr-worker" => ("asr", "asr/libSaturdayWhisperRuntime.a", "whisper-cpp", "whisper_bridge.cpp", "whisper_bridge.h", "libSaturdayWhisperRuntime.a",
            "d1be6fde11ac6e0407606b4e42fe72d34add8037", "6fe2755682242a5c01aba65a22b3c0305d747841", "645445da884df6dbd52b30f82d7be7c01d1767c938714e477e786b88d529e9f4"),
        "saturday-voice-llm-worker" => ("llm", "llm/libSaturdayLlamaRuntime.a", "llama-cpp", "llama_adapter.cpp", "llama_adapter.h", "libSaturdayLlamaRuntime.a",
            "8345f333951c661d166b00e6f9362e553768f292", "88538af4019aa9d18026ff43e446c34fb7456adf", "84050e783dbd0c4a4678f90f253b0881df04e90a98c20f73ae3709751c4a4850"),
        _ => return Err(format!("unexpected worker package: {pkg}")),
    };
    let runtime_root = tauri.join("resources/voice-runtime-build");
    let receipt_path = runtime_root.join("build-artifact-manifest.json");
    let receipt_bytes = fs::read(&receipt_path).map_err(|error| format!("{}: {error}", receipt_path.display()))?;
    let receipt: serde_json::Value = serde_json::from_slice(&receipt_bytes).map_err(|error| format!("invalid runtime receipt: {error}"))?;
    if receipt.get("schema").and_then(|value| value.as_str()) != Some("saturday.voice-runtime-artifacts.v1")
        || receipt.get("state").and_then(|value| value.as_str()) != Some("independently_reviewed_and_main_approved")
        || receipt.get("product_readiness").and_then(|value| value.as_str()) != Some("approved_for_product_use")
        || receipt.get("review_status").and_then(|value| value.as_str()) != Some("independent_artifact_review_passed")
        || receipt.get("main_approval").and_then(|value| value.as_bool()) != Some(true) {
        return Err("runtime artifacts lack independent review and Main approval".into());
    }
    let source_manifest_path = tauri.join("resources/voice-runtime-sources.json");
    let source_manifest_sha = file_digest(&source_manifest_path)?;
    if sha_field(receipt.get("source_manifest_sha256").ok_or("source manifest digest absent")?)? != source_manifest_sha {
        return Err("runtime source manifest does not match the build receipt".into());
    }
    let source_manifest: serde_json::Value = serde_json::from_slice(&fs::read(&source_manifest_path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("invalid runtime source manifest: {error}"))?;
    if source_manifest.get("schema").and_then(|value| value.as_str()) != Some("saturday.voice-runtime-sources.v1") {
        return Err("unexpected pinned runtime source manifest schema".into());
    }
    let runtime_sources = source_manifest.get("runtimes").and_then(|value| value.as_array()).ok_or("runtime source roster absent")?;
    if runtime_sources.len() != 2 { return Err("runtime source manifest must contain exactly two providers".into()); }
    let providers = receipt.get("providers").and_then(|value| value.as_array()).ok_or("runtime receipt has no provider roster")?;
    if providers.len() != 2 { return Err("runtime receipt must contain exactly two role-separated providers".into()); }
    let matches: Vec<_> = providers.iter().filter(|item| item.get("provider").and_then(|value| value.as_str()) == Some(provider)).collect();
    if matches.len() != 1 { return Err(format!("runtime receipt must contain exactly one {provider} provider")); }
    let provider_record = matches[0];
    let source_match: Vec<_> = runtime_sources.iter().filter(|item| item.get("id").and_then(|value| value.as_str()) == Some(provider)).collect();
    if source_match.len() != 1 { return Err(format!("pinned source manifest must contain exactly one {provider} source")); }
    if source_match[0].get("commit").and_then(|value| value.as_str()) != Some(source_commit)
        || source_match[0].get("tree").and_then(|value| value.as_str()) != Some(source_tree)
        || source_match[0].get("archive_sha256").and_then(|value| value.as_str()) != Some(source_archive_sha)
        || provider_record.get("source_tree").and_then(|value| value.as_str()) != Some(source_tree)
        || provider_record.get("source_archive_sha256").and_then(|value| value.as_str()) != Some(source_archive_sha) {
        return Err(format!("{provider} artifact receipt differs from its pinned source manifest"));
    }
    for (field, expected) in [
        ("framework_dependencies", &["Accelerate", "Foundation", "Metal", "MetalKit"][..]),
        ("framework_link_order", &["Accelerate", "Foundation", "Metal", "MetalKit", "c++"][..]),
        ("framework_link_flags", &["-framework", "Accelerate", "-framework", "Foundation", "-framework", "Metal", "-framework", "MetalKit", "-lc++"][..]),
    ] {
        if string_list(provider_record.get(field).ok_or(format!("provider {field} absent"))?)?.as_slice() != expected {
            return Err(format!("{provider} {field} differs from fixed link contract"));
        }
    }
    if provider_record.get("archive").and_then(|value| value.as_str()) != Some(archive_rel) {
        return Err(format!("{provider} archive path differs from fixed role path"));
    }
    let archive = runtime_root.join(archive_rel);
    let archive_sha = file_digest(&archive)?;
    if sha_field(provider_record.get("archive_sha256").ok_or("archive digest absent")?)? != archive_sha {
        return Err("runtime archive hash does not match reviewed receipt".into());
    }
    if provider_record.get("archive_bytes").and_then(|value| value.as_u64()) != Some(fs::metadata(&archive).map_err(|error| error.to_string())?.len()) {
        return Err("runtime archive size does not match reviewed receipt".into());
    }
    let headers = provider_record.get("headers").and_then(|value| value.as_array()).ok_or("provider header roster absent")?;
    if headers.is_empty() { return Err("provider header roster is empty".into()); }
    let include_root = runtime_root.join(role).join("include");
    let mut header_roster = Vec::new();
    let mut expected_paths = Vec::new();
    for header in headers {
        let relative = header.get("path").and_then(|value| value.as_str()).ok_or("header path absent")?;
        let relative_path = Path::new(relative);
        let mut components = relative_path.components();
        if components.next().and_then(|item| item.as_os_str().to_str()) != Some("include")
            || components.next().and_then(|item| item.as_os_str().to_str()).is_none()
            || components.next().is_some() {
            return Err("promoted provider header paths must be include/<basename>".into());
        }
        let basename = relative_path.file_name().and_then(|name| name.to_str()).ok_or("invalid header basename")?;
        let path = include_root.join(basename);
        let sha = file_digest(&path)?;
        if sha_field(header.get("sha256").ok_or("header digest absent")?)? != sha { return Err(format!("header hash mismatch: {basename}")); }
        expected_paths.push(basename.to_owned());
        header_roster.push((relative.to_owned(), sha));
        println!("cargo:rerun-if-changed={}", path.display());
    }
    expected_paths.sort();
    if expected_paths.windows(2).any(|pair| pair[0] == pair[1]) { return Err("duplicate provider header path".into()); }
    let mut actual = Vec::new();
    files(&include_root, Path::new("."), &mut actual)?;
    let mut actual_names: Vec<_> = actual.iter().map(|path| path.file_name().and_then(|name| name.to_str()).ok_or("invalid header filename").map(str::to_owned)).collect::<Result<_, _>>()?;
    actual_names.sort();
    if actual_names != expected_paths { return Err("promoted provider header tree differs from reviewed roster".into()); }
    header_roster.sort();
    let header_seal = hash_roster(&header_roster);
    let notices = provider_record.get("notices").and_then(|value| value.as_array()).ok_or("provider notice roster absent")?;
    if notices.is_empty() { return Err("provider notice roster is empty".into()); }
    for notice in notices {
        let relative = notice.get("path").and_then(|value| value.as_str()).ok_or("notice path absent")?;
        let relative_path = Path::new(relative);
        if relative_path.components().next().and_then(|item| item.as_os_str().to_str()) != Some("notices")
            || relative_path.components().any(|item| item.as_os_str() == "..") {
            return Err("notice path escapes the fixed notices root".into());
        }
        let path = runtime_root.join(role).join(relative_path);
        if sha_field(notice.get("sha256").ok_or("notice digest absent")?)? != file_digest(&path)? {
            return Err(format!("notice hash mismatch: {relative}"));
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let role_root = runtime_root.join(role);
    let promoted_tree = promoted_tree_digest(&role_root)?;
    if sha_field(provider_record.get("promoted_tree_sha256").ok_or("promoted tree digest absent")?)? != promoted_tree {
        return Err("promoted runtime tree does not match approved receipt".into());
    }
    println!("cargo:rerun-if-changed={}", role_root.display());

    let recipe_path = tauri.join("resources/voice-runtime-build-recipe.json");
    let recipe_sha = file_digest(&recipe_path)?;
    if sha_field(receipt.get("recipe_sha256").ok_or("recipe digest absent")?)? != recipe_sha {
        return Err("runtime build recipe does not match the artifact receipt".into());
    }
    let (_owner_public_key, owner_key_sha, owner_key_pin_sha) = owner_public_key_pin(tauri)?;
    let native = tauri.join("native");
    let source_files = [
        ("src-tauri/Cargo.lock", repo.join("src-tauri/Cargo.lock")),
        ("src-tauri/Cargo.toml", tauri.join("Cargo.toml")),
        ("src-tauri/tauri.conf.json", tauri.join("tauri.conf.json")),
        ("src-tauri/resources/voice-runtime-build/build-artifact-manifest.json", receipt_path.clone()),
        ("src-tauri/resources/voice-runtime-sources.json", source_manifest_path.clone()),
        ("src-tauri/resources/voice-runtime-build/archive", archive.clone()),
        ("src-tauri/voice-worker-protocol/Cargo.toml", tauri.join("voice-worker-protocol/Cargo.toml")),
        ("src-tauri/voice-worker-protocol/src/lib.rs", tauri.join("voice-worker-protocol/src/lib.rs")),
        ("src-tauri/voice-types/Cargo.toml", tauri.join("voice-types/Cargo.toml")),
        ("src-tauri/voice-types/src/lib.rs", tauri.join("voice-types/src/lib.rs")),
        ("src-tauri/voice-parent-auth", tauri.join("voice-parent-auth")),
        ("src-tauri/resources/voice-runtime-build/headers", include_root.clone()),
        ("src-tauri/resources/voice-runtime-build-recipe.json", recipe_path.clone()),
        ("scripts/build_voice_workers.py", repo.join("scripts/build_voice_workers.py")),
        ("scripts/voice_worker_build.rs", repo.join("scripts/voice_worker_build.rs")),
        ("src-tauri/native/worker_bridge.cpp", native.join(bridge)),
        ("src-tauri/native/worker_bridge.h", native.join(bridge_header)),
        ("src-tauri/voice-worker/Cargo.toml", manifest_dir.join("Cargo.toml")),
        ("src-tauri/voice-worker/build.rs", manifest_dir.join("build.rs")),
        ("src-tauri/voice-worker/src/main.rs", manifest_dir.join("src/main.rs")),
        ("src-tauri/voice-worker/src/delegate.rs", manifest_dir.join(if role == "asr" { "src/asr.rs" } else { "src/llm.rs" })),
    ];
    let mut source_roster = Vec::new();
    for (logical, path) in &source_files {
        let sha = if path.is_dir() {
            let mut tree_files = Vec::new();
            files(path, Path::new("."), &mut tree_files)?;
            let mut tree_roster = Vec::new();
            for item in tree_files {
                let relative = item.strip_prefix(Path::new(".")).map_err(|error| error.to_string())?.to_string_lossy().replace('\\', "/");
                tree_roster.push((relative, file_digest(&path.join(item))?));
            }
            tree_roster.sort();
            hash_roster(&tree_roster)
        } else { file_digest(path)? };
        source_roster.push((logical.to_string(), sha));
        println!("cargo:rerun-if-changed={}", path.display());
    }
    source_roster.push((OWNER_PUBLIC_KEY_SOURCE_PATH.to_owned(), owner_key_sha.clone()));
    source_roster.push((OWNER_KEY_PIN_SOURCE_PATH.to_owned(), owner_key_pin_sha.clone()));
    source_roster.sort();
    let source_seal = hash_roster(&source_roster);
    let mut resource_entries = vec![
        ("receipt".to_owned(), digest(&receipt_bytes)),
        ("archive".to_owned(), archive_sha),
        ("headers".to_owned(), header_seal),
        ("promoted-tree".to_owned(), promoted_tree),
        ("runtime-recipe".to_owned(), recipe_sha.to_owned()),
        ("runtime-source-generation".to_owned(), source_manifest_sha.to_owned()),
    ];
    resource_entries.push(("trusted-owner-public-key-x963".to_owned(), owner_key_sha));
    resource_entries.push(("owner-key-pin-record".to_owned(), owner_key_pin_sha));
    resource_entries.sort();
    let resource_seal = hash_roster(&resource_entries);
    println!("cargo:rustc-env=SATURDAY_VOICE_SOURCE_SEAL_SHA256={source_seal}");
    println!("cargo:rustc-env=SATURDAY_VOICE_RESOURCE_SEAL_SHA256={resource_seal}");

    let bridge_path = native.join(bridge);
    let mut bridge_build = cc::Build::new();
    bridge_build.cpp(true).std("c++17").file(&bridge_path).include(&include_root).include(&native).warnings(true).compile("saturday_voice_role_bridge");
    println!("cargo:rerun-if-changed={}", archive.display());
    println!("cargo:rustc-link-search=native={}", archive.parent().ok_or("archive has no parent")?.display());
    println!("cargo:rustc-link-lib=static={}", archive_name.trim_start_matches("lib").trim_end_matches(".a"));
    for framework in ["Security", "CoreFoundation", "Accelerate", "Foundation", "Metal", "MetalKit"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rustc-link-lib=c++");
    Ok(())
}

pub fn run() {
    if let Err(error) = build() {
        panic!("voice worker build denied: {error}");
    }
}
