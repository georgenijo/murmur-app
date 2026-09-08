use std::fs::{self, File, OpenOptions};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

pub const MAX_AUDIO_SECONDS: u64 = 2 * 60 * 60;
const MAX_SAMPLES: u64 = MAX_AUDIO_SECONDS * 16_000;

/// Owns the only temporary whole-session copy. Moving the owner transfers cleanup.
pub struct RemoteAudio {
    path: PathBuf,
    writer: Option<hound::WavWriter<BufWriter<File>>>,
    samples: u64,
    pub origin_ms: u64,
}

impl RemoteAudio {
    pub fn create(root: &Path) -> Option<Self> {
        let directory = root.join("diarization");
        fs::create_dir_all(&directory).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            if !fs::symlink_metadata(&directory).ok()?.is_dir() {
                return None;
            }
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).ok()?;
            let path = directory.join(format!("{}.wav", uuid::Uuid::new_v4()));
            let file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&path)
                .ok()?;
            let writer = match hound::WavWriter::new(
                BufWriter::new(file),
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16_000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            ) {
                Ok(writer) => writer,
                Err(_) => {
                    let _ = fs::remove_file(&path);
                    return None;
                }
            };
            Some(Self {
                path,
                writer: Some(writer),
                samples: 0,
                origin_ms: 0,
            })
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    pub fn append(&mut self, samples: &[f32]) -> bool {
        if self.samples + samples.len() as u64 > MAX_SAMPLES {
            return false;
        }
        let Some(writer) = self.writer.as_mut() else {
            return false;
        };
        for sample in samples {
            if !sample.is_finite()
                || writer
                    .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                    .is_err()
            {
                return false;
            }
        }
        self.samples += samples.len() as u64;
        true
    }

    pub fn finish(mut self, origin_ms: u64) -> Option<Self> {
        self.origin_ms = origin_ms;
        if self.samples == 0 || self.writer.take()?.finalize().is_err() {
            return None;
        }
        Some(self)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn duration_ms(&self) -> u64 {
        self.samples * 1000 / 16_000
    }
}

impl Drop for RemoteAudio {
    fn drop(&mut self) {
        self.writer.take();
        let _ = fs::remove_file(&self.path);
    }
}

pub fn sweep_abandoned(root: &Path) {
    // Called once before capture can start. This directory contains only disposable audio.
    let _ = fs::remove_dir_all(root.join("diarization"));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_remote_audio_is_bounded_private_and_deleted_by_owner() {
        let root = tempfile::tempdir().unwrap();
        let mut audio = RemoteAudio::create(root.path()).unwrap();
        let path = audio.path().to_owned();
        assert!(audio.append(&[0.25, -0.25]));
        audio.samples = MAX_SAMPLES;
        assert!(!audio.append(&[0.0]));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(audio);
        assert!(!path.exists());
    }
    #[test]
    fn finalized_audio_has_exact_samples_and_origin() {
        let root = tempfile::tempdir().unwrap();
        let mut audio = RemoteAudio::create(root.path()).unwrap();
        assert!(audio.append(&vec![0.2; 16000]));
        let audio = audio.finish(1234).unwrap();
        assert_eq!(audio.origin_ms, 1234);
        assert_eq!(audio.duration_ms(), 1000);
        assert_eq!(
            hound::WavReader::open(audio.path()).unwrap().duration(),
            16000
        );
    }
    #[test]
    fn startup_only_sweeps_disposable_diarization_audio() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("preserve.txt"), b"transcript evidence").unwrap();
        fs::create_dir(root.path().join("diarization")).unwrap();
        fs::write(
            root.path().join("diarization/orphan.wav"),
            b"abandoned audio",
        )
        .unwrap();
        sweep_abandoned(root.path());
        assert!(!root.path().join("diarization").exists());
        assert!(root.path().join("preserve.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn audio_owner_refuses_a_symlinked_staging_directory() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("diarization")).unwrap();
        assert!(RemoteAudio::create(root.path()).is_none());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
