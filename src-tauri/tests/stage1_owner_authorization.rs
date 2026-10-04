#![cfg(feature = "runtime-helper")]

use jarvis_dashboard::owner_authorization::binding::{
    bootstrap_record_bytes, parse_bootstrap_record, parse_frozen_binding,
};
use jarvis_dashboard::owner_authorization::issuer::{
    issue_and_run_authorization, IssueContext, AUTHORIZATION_PROMPT,
};
use jarvis_dashboard::owner_authorization::paths::{
    open_stage1_directories, AccountHomeResolver, PathError,
};
use jarvis_dashboard::owner_authorization::process::{
    NativeStage1ProcessRunner, Stage1ChildRequest, Stage1ProcessError, Stage1ProcessRunner,
};
use jarvis_dashboard::owner_authorization::SIGNING_DOMAIN;
use jarvis_dashboard::platform::owner_authorization::{KeyProof, OwnerKeyBackend, OwnerKeyError};
use std::fs::{self, File};
use std::os::fd::AsFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const RECEIPT_DIGEST: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "jarvis-stage1-process-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _root: TestRoot,
    script: PathBuf,
    ledger: File,
    workspace: File,
    receipts: File,
}

impl Fixture {
    fn new(body: impl FnOnce(&Path, [&fs::Metadata; 3]) -> String) -> Self {
        let root = TestRoot::new("fixture");
        let bundle_root = root
            .path()
            .join("J.A.R.V.I.S.app/Contents/Resources/jarvis/stage1");
        fs::create_dir_all(&bundle_root).unwrap();
        for path in [&bundle_root, root.path()] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let ledger_path = root.path().join("ledger");
        let workspace_path = root.path().join("workspace");
        let receipts_path = root.path().join("receipts");
        for path in [&ledger_path, &workspace_path, &receipts_path] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let ledger = File::open(&ledger_path).unwrap();
        let workspace = File::open(&workspace_path).unwrap();
        let receipts = File::open(&receipts_path).unwrap();
        let ledger_metadata = ledger.metadata().unwrap();
        let workspace_metadata = workspace.metadata().unwrap();
        let receipts_metadata = receipts.metadata().unwrap();
        let script = bundle_root.join("prebuilt_runtime_stage1.py");
        let source = body(
            &script,
            [&ledger_metadata, &workspace_metadata, &receipts_metadata],
        );
        fs::write(&script, source).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o500)).unwrap();
        Self {
            _root: root,
            script,
            ledger,
            workspace,
            receipts,
        }
    }

    fn request<'a>(&'a self, envelope: &'a [u8], nonce: &'a [u8; 32]) -> Stage1ChildRequest<'a> {
        Stage1ChildRequest {
            script: &self.script,
            envelope,
            nonce,
            ledger_fd: self.ledger.as_fd(),
            workspace_parent_fd: self.workspace.as_fd(),
            receipt_parent_fd: self.receipts.as_fd(),
        }
    }
}

fn python_literal(value: &Path) -> String {
    serde_json::to_string(value.to_str().unwrap()).unwrap()
}

fn identity(metadata: &fs::Metadata) -> String {
    format!(
        "({}, {}, {})",
        metadata.dev(),
        metadata.ino(),
        metadata.uid()
    )
}

fn success_script(script: &Path, directories: [&fs::Metadata; 3]) -> String {
    format!(
        r#"import os
import stat
import sys

def read_all(fd):
    chunks = []
    while True:
        chunk = os.read(fd, 4096)
        if not chunk:
            return b''.join(chunks)
        chunks.append(chunk)

def identity(fd):
    value = os.fstat(fd)
    assert stat.S_ISDIR(value.st_mode)
    return (value.st_dev, value.st_ino, value.st_uid)

envelope = read_all(3)
nonce = read_all(4)
assert envelope == b'owner-envelope'
assert nonce == bytes(range(32))
assert identity(5) == {ledger}
assert identity(6) == {workspace}
assert identity(7) == {receipts}
assert sys.argv == [{script}]
assert sys.flags.dont_write_bytecode == 1
assert sys.flags.ignore_environment == 1
assert sys.flags.no_user_site == 1
assert os.getcwd() == '/'
environment = dict(os.environ)
for name, value in {{'LANG': 'C', 'LC_ALL': 'C', 'PATH': '/usr/bin:/bin'}}.items():
    assert environment.pop(name) == value
assert set(environment).issubset({{'__CF_USER_TEXT_ENCODING', 'SDKROOT', 'CPATH', 'LIBRARY_PATH', 'MANPATH'}}), repr(environment)
assert envelope.hex() not in repr(sys.argv) + repr(dict(os.environ))
assert nonce.hex() not in repr(sys.argv) + repr(dict(os.environ))
for fd in range(8, 64):
    try:
        os.fstat(fd)
    except OSError:
        continue
    raise AssertionError('unexpected inherited descriptor')
sys.stdout.write('{digest}\n')
sys.stdout.flush()
"#,
        ledger = identity(directories[0]),
        workspace = identity(directories[1]),
        receipts = identity(directories[2]),
        script = python_literal(script),
        digest = RECEIPT_DIGEST,
    )
}

fn issue_success_script(script: &Path, directories: [&fs::Metadata; 3]) -> String {
    format!(
        r#"import json
import os
import stat
import sys

def read_all(fd):
    chunks = []
    while True:
        chunk = os.read(fd, 4096)
        if not chunk:
            return b''.join(chunks)
        chunks.append(chunk)

def identity(fd):
    value = os.fstat(fd)
    assert stat.S_ISDIR(value.st_mode)
    return (value.st_dev, value.st_ino, value.st_uid)

envelope = json.loads(read_all(3))
nonce = read_all(4)
assert envelope['schema'] == 'jarvis.local-provider.stage1-owner-authorization-envelope.v1'
assert nonce == bytes([0x55]) * 32
assert identity(5) == {ledger}
assert identity(6) == {workspace}
assert identity(7) == {receipts}
assert sys.argv == [{script}]
sys.stdout.write('{digest}\n')
sys.stdout.flush()
"#,
        ledger = identity(directories[0]),
        workspace = identity(directories[1]),
        receipts = identity(directories[2]),
        script = python_literal(script),
        digest = RECEIPT_DIGEST,
    )
}

fn run_script(source: &str) -> Result<String, Stage1ProcessError> {
    let fixture = Fixture::new(|_, _| source.to_owned());
    NativeStage1ProcessRunner
        .run(fixture.request(b"owner-envelope", &std::array::from_fn(|i| i as u8)))
}

#[test]
fn exact_python_command_transfers_only_fixed_descriptors_and_minimal_environment() {
    let fixture = Fixture::new(success_script);
    let nonce = std::array::from_fn(|index| index as u8);
    assert_eq!(
        NativeStage1ProcessRunner.run(fixture.request(b"owner-envelope", &nonce)),
        Ok(RECEIPT_DIGEST.to_owned())
    );
}

#[test]
fn stdout_must_be_one_bounded_newline_terminated_lowercase_digest() {
    for source in [
        "print('a' * 64)\nprint('extra')\n",
        "print('A' * 64)\n",
        "print('a' * 63)\n",
        "import sys\nsys.stdout.write('a' * 64)\n",
        "print('a' * 129)\n",
        "import sys\nsys.stderr.write('unexpected')\nprint('a' * 64)\n",
    ] {
        assert_eq!(run_script(source), Err(Stage1ProcessError::InvalidOutput));
    }
}

#[test]
fn nonzero_or_signal_exit_is_a_bounded_child_failure() {
    assert_eq!(
        run_script("import sys\nsys.stderr.write('fixed failure')\nraise SystemExit(7)\n"),
        Err(Stage1ProcessError::ChildFailed)
    );
    assert_eq!(
        run_script("import os, signal\nos.kill(os.getpid(), signal.SIGKILL)\n"),
        Err(Stage1ProcessError::ChildFailed)
    );
    assert_eq!(
        run_script(&format!(
            "import sys\nsys.stderr.write('x' * {})\nraise SystemExit(1)\n",
            4097
        )),
        Err(Stage1ProcessError::InvalidOutput)
    );
}

#[test]
fn timeout_and_late_output_kill_and_reap_the_owned_process_group() {
    for source in [
        "import time\ntime.sleep(60)\n",
        concat!(
            "import sys, time\n",
            "sys.stdout.write('",
            "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
            "\\n')\n",
            "sys.stdout.flush()\n",
            "time.sleep(60)\n"
        ),
    ] {
        let started = Instant::now();
        assert_eq!(run_script(source), Err(Stage1ProcessError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}

#[test]
fn surviving_descendant_fails_and_is_killed_before_return() {
    let pid_root = TestRoot::new("survivor-pid");
    let pid_path = pid_root.path().join("child.pid");
    let pid_literal = python_literal(&pid_path);
    let fixture = Fixture::new(|_, _| {
        format!(
            r#"import os
import time
pid = os.fork()
if pid == 0:
    time.sleep(60)
    raise SystemExit(0)
with open({pid_path}, 'w', encoding='ascii') as handle:
    handle.write(str(pid))
print('{digest}')
"#,
            pid_path = pid_literal,
            digest = RECEIPT_DIGEST,
        )
    });
    let nonce = std::array::from_fn(|index| index as u8);
    assert_eq!(
        NativeStage1ProcessRunner.run(fixture.request(b"owner-envelope", &nonce)),
        Err(Stage1ProcessError::SurvivingProcess)
    );
    let child_pid: i32 = fs::read_to_string(pid_path).unwrap().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let result = unsafe { libc::kill(child_pid, 0) };
        if result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            break;
        }
        assert!(Instant::now() < deadline, "descendant survived cleanup");
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct TestResolver(PathBuf);

impl AccountHomeResolver for TestResolver {
    fn effective_user_home(&self) -> Result<PathBuf, PathError> {
        Ok(self.0.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyCall {
    Sign { message: Vec<u8>, prompt: String },
}

struct FakeKeyBackend {
    public_key: Vec<u8>,
    calls: Mutex<Vec<KeyCall>>,
}

impl OwnerKeyBackend for FakeKeyBackend {
    fn provision(&self, _challenge: &[u8]) -> Result<KeyProof, OwnerKeyError> {
        Err(OwnerKeyError::System)
    }

    fn copy_public_key(&self) -> Result<Vec<u8>, OwnerKeyError> {
        Ok(self.public_key.clone())
    }

    fn sign(&self, message: &[u8], prompt: &str) -> Result<KeyProof, OwnerKeyError> {
        self.calls.lock().unwrap().push(KeyCall::Sign {
            message: message.to_vec(),
            prompt: prompt.to_owned(),
        });
        Ok(KeyProof {
            public_key_x963: self.public_key.clone(),
            signature_der: vec![0x30, 0x00],
        })
    }

    fn delete_after_proof(&self, _challenge: &[u8], _prompt: &str) -> Result<(), OwnerKeyError> {
        Err(OwnerKeyError::System)
    }
}

#[test]
fn fake_owner_authorization_signs_once_and_runs_the_exact_python_boundary() {
    const BINDING_BYTES: &[u8] =
        include_bytes!("../../scripts/tests/fixtures/stage1-owner-binding-frozen.json");
    let root = TestRoot::new("issue-and-run");
    let directories = open_stage1_directories(&TestResolver(root.path().to_path_buf())).unwrap();
    let canonical_root = root
        .path()
        .join("Library/Application Support/dev.rohtolos.jarvis/Stage1OwnerAuthorization");
    let script = root
        .path()
        .join("J.A.R.V.I.S.app/Contents/Resources/jarvis/stage1/prebuilt_runtime_stage1.py");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    let metadata = [
        fs::metadata(canonical_root.join("ledger")).unwrap(),
        fs::metadata(canonical_root.join("workspace")).unwrap(),
        fs::metadata(canonical_root.join("receipts")).unwrap(),
    ];
    fs::write(
        &script,
        issue_success_script(&script, [&metadata[0], &metadata[1], &metadata[2]]),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o500)).unwrap();

    let binding = parse_frozen_binding(BINDING_BYTES).unwrap();
    let mut public_key = vec![0x42; 65];
    public_key[0] = 0x04;
    let anchor = parse_bootstrap_record(&bootstrap_record_bytes(&public_key).unwrap()).unwrap();
    let backend = FakeKeyBackend {
        public_key,
        calls: Mutex::new(Vec::new()),
    };

    assert_eq!(
        issue_and_run_authorization(
            IssueContext {
                binding: &binding,
                directories: &directories,
                now_unix: 1_700_000_000,
                generation: [0x33; 16],
                nonce: [0x55; 32],
            },
            &script,
            &backend,
            &anchor,
            &NativeStage1ProcessRunner,
        ),
        Ok(RECEIPT_DIGEST.to_owned())
    );

    let calls = backend.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let KeyCall::Sign { message, prompt } = &calls[0];
    assert_eq!(prompt, AUTHORIZATION_PROMPT);
    assert!(message.starts_with(SIGNING_DOMAIN));
}
