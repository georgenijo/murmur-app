//! Synthetic fixtures only: this test host has no Tauri or native UI runtime.
use murmur_core::{
    audio_level::{compute_peak, compute_rms},
    context::{
        AppContext, BackgroundFuture, BackgroundWork, ContextError, MainThread, Paths,
        PermissionStatus, Permissions, Task,
    },
    model_artifact::{vad_model_path, validate_binary_model, validate_binary_model_on_worker},
    model_digest::model_file_digest,
};
use std::{path::PathBuf, sync::Arc};

struct TestHost {
    root: PathBuf,
    scheduling_error: bool,
    drop_work: bool,
}
impl Paths for TestHost {
    fn app_data_dir(&self) -> Result<PathBuf, ContextError> {
        Ok(self.root.join("data"))
    }
    fn models_dir(&self) -> Option<PathBuf> {
        Some(self.root.join("models"))
    }
}
impl Permissions for TestHost {
    fn microphone_status(&self) -> PermissionStatus {
        PermissionStatus::Denied
    }
}
impl MainThread for TestHost {
    fn dispatch(&self, _: Task) -> Result<(), ContextError> {
        Err(ContextError::SchedulingFailed)
    }
}
impl BackgroundWork for TestHost {
    fn spawn(&self, future: BackgroundFuture) -> Result<(), ContextError> {
        std::thread::spawn(|| futures_executor::block_on(future));
        Ok(())
    }
    fn spawn_blocking(&self, task: Task) -> Result<(), ContextError> {
        if self.scheduling_error {
            return Err(ContextError::SchedulingFailed);
        }
        if !self.drop_work {
            std::thread::spawn(task);
        }
        Ok(())
    }
}
fn context(root: PathBuf, scheduling_error: bool, drop_work: bool) -> AppContext {
    let host = Arc::new(TestHost {
        root,
        scheduling_error,
        drop_work,
    });
    AppContext {
        paths: host.clone(),
        permissions: host.clone(),
        background: host.clone(),
        main_thread: host,
    }
}

#[test]
fn levels_preserve_empty_silence_and_signed_sample_behavior() {
    assert_eq!(compute_rms(&[]), 0.0);
    assert_eq!(compute_peak(&[]), 0.0);
    assert_eq!(compute_rms(&[0.0; 128]), 0.0);
    assert_eq!(compute_peak(&[0.25, -0.75, 0.5]), 0.75);
    assert_eq!(compute_rms(&[0.5, -0.5]), 0.5);
    assert!((compute_rms(&[0.0, 1.0]) - 0.5_f32.sqrt()).abs() < f32::EPSILON);
}

#[test]
fn digest_streams_synthetic_files_and_reports_missing_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fixture.bin");
    std::fs::write(&path, b"abc").unwrap();
    assert_eq!(
        model_file_digest(&path).unwrap(),
        (
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()
        )
    );
    // Cross the reader's 1 MiB buffer boundary. Known SHA-256 of these bytes.
    std::fs::write(&path, vec![b'a'; 1_048_577]).unwrap();
    let (size, digest) = model_file_digest(&path).unwrap();
    assert_eq!(size, 1_048_577);
    assert_eq!(
        digest,
        "4a3f0c0c213adea174f9a3d4c13177315b588bdb2e9c1012d3d0bf0453ca0f6a"
    );
    assert!(model_file_digest(&root.path().join("missing")).is_err());
}

#[test]
fn artifact_validation_preserves_size_type_and_error_document_checks() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fixture.bin");
    assert!(validate_binary_model(&path, 4, "Fixture").is_err());
    assert!(validate_binary_model(root.path(), 0, "Fixture")
        .unwrap_err()
        .contains("regular file"));
    std::fs::write(&path, b"abc").unwrap();
    assert!(validate_binary_model(&path, 4, "Fixture")
        .unwrap_err()
        .contains("incomplete"));
    for prefix in [
        "  <html>error</html>",
        "{\"error\":true}",
        "[1,2]",
        "AccessDenied",
        "Error: proxy",
    ] {
        std::fs::write(&path, prefix).unwrap();
        assert!(validate_binary_model(&path, 4, "Fixture")
            .unwrap_err()
            .contains("web error"));
    }
    std::fs::write(&path, b"lmgg\0\0\0\0synthetic model").unwrap();
    validate_binary_model(&path, 4, "Fixture").unwrap();
}

#[test]
fn context_drives_real_artifact_validation_without_a_host_runtime() {
    futures_executor::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let cx = context(root.path().into(), false, false);
        let path = vad_model_path(&cx, "synthetic.bin").unwrap();
        assert_eq!(path, root.path().join("models/synthetic.bin"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"lmgg\0\0\0\0synthetic model").unwrap();
        validate_binary_model_on_worker(&cx, path.clone(), 4, "Fixture")
            .await
            .unwrap();
        assert!(validate_binary_model_on_worker(&cx, path, 1_000, "Fixture")
            .await
            .is_err());
        let caller = std::thread::current().id();
        let worker = cx.blocking(|| std::thread::current().id()).await.unwrap();
        assert_ne!(caller, worker);
    });
}

#[test]
fn context_propagates_scheduling_and_worker_loss() {
    futures_executor::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let cx = context(root.path().into(), true, false);
        assert_eq!(
            cx.blocking(|| 42).await,
            Err(ContextError::SchedulingFailed)
        );
        let cx = context(root.path().into(), false, true);
        assert_eq!(cx.blocking(|| 42).await, Err(ContextError::WorkerStopped));
    });
}
