//! One filesystem resolver for catalog, installation admission and inference.
//! Legacy locations are read in place; new downloads retain their owned root.
use std::path::{Path, PathBuf};

pub(crate) struct Locations {
    pub model_roots: Vec<PathBuf>,
    pub download_root: PathBuf,
    pub coreml_root: PathBuf,
}

impl Locations {
    pub(crate) fn for_identity(
        home: &Path,
        support: &Path,
        identifier: &str,
        custom: Option<PathBuf>,
    ) -> Self {
        let data = crate::identity::namespace(support, identifier);
        let internal = identifier != crate::identity::PRODUCTION;
        let app = support.join(identifier);
        let download_root = data.join("local-dictation/models");
        let mut model_roots = Vec::new();
        // An isolated preview must never discover production Whisper/sherpa
        // artifacts through an inherited shell override or real HOME cache.
        if let Some(custom) =
            custom.filter(|p| !internal || p.starts_with(&data) || p.starts_with(&app))
        {
            model_roots.push(custom);
        }
        let cache_home = if internal { data.as_path() } else { home };
        for root in [
            download_root.clone(),
            app.join("models"),
            data.join("models"),
            data.join("com.local-dictation.app/models"),
            data.join("murmur/models"),
            data.join("pywhispercpp/models"),
            cache_home.join(".cache/whisper.cpp"),
            cache_home.join(".cache/whisper"),
            cache_home.join(".whisper/models"),
        ] {
            if !model_roots.contains(&root) {
                model_roots.push(root);
            }
        }
        Self {
            model_roots,
            download_root,
            // FluidAudio's SDK owns this shared cache, independently of the
            // bundle identity. Isolated bundles may only reuse complete models.
            coreml_root: support.join("FluidAudio/Models"),
        }
    }

    pub(crate) fn current() -> Option<Self> {
        let support = crate::identity::native_support()?;
        let home = support.parent()?.parent()?;
        #[allow(unused_mut)]
        let mut locations = Self::for_identity(
            home,
            support,
            crate::identity::identifier(),
            std::env::var_os("WHISPER_MODEL_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        );
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("MURMUR_MODEL_DIR_OVERRIDE")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute() && !crate::identity::internal())
        {
            locations.model_roots = vec![path.clone()];
            locations.download_root = path;
        }
        Some(locations)
    }

    pub(crate) fn resolve(&self, model: &str) -> Option<PathBuf> {
        if crate::transcriber::is_coreml_model(model) {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                let path = self
                    .coreml_root
                    .join(crate::transcriber::coreml::CACHE_DIR_NAME);
                return crate::transcriber::coreml::model_exists_at(&path).then_some(path);
            }
            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            return None;
        }
        if let Some((_, bundle)) = crate::transcriber::parakeet::download_spec(model) {
            return self.model_roots.iter().find_map(|root| {
                crate::transcriber::parakeet::specific_model_exists_in(model, root)
                    .then(|| root.join(&bundle))
            });
        }
        if crate::model_runtime::model_definition(model)
            .ok()?
            .install_kind
            != crate::model_runtime::InstallKind::Whisper
        {
            return None;
        }
        self.model_roots.iter().find_map(|root| {
            let path = root.join(format!("ggml-{model}.bin"));
            crate::model_validation::validate_whisper(&path, model)
                .is_ok()
                .then_some(path)
        })
    }
}

pub(crate) fn resolve(model: &str) -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if crate::transcriber::is_coreml_model(model)
        && std::env::var("MURMUR_COREML_FORCE_NOT_INSTALLED").as_deref() == Ok("1")
    {
        return None;
    }
    Locations::current()?.resolve(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whisper(path: &Path) {
        crate::model_validation::fixtures::whisper(path, "base.en");
    }

    #[test]
    fn every_production_legacy_and_custom_root_resolves_without_copying() {
        let home = tempfile::tempdir().unwrap();
        let support = home.path().join("Library/Application Support");
        let locations = Locations::for_identity(
            home.path(),
            &support,
            crate::identity::PRODUCTION,
            Some(home.path().join("user-configured-models")),
        );
        assert_eq!(locations.model_roots.len(), 10);
        for root in &locations.model_roots {
            let path = root.join("ggml-base.en.bin");
            whisper(&path);
            assert_eq!(locations.resolve("base.en"), Some(path.clone()));
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 147_964_211);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn partial_and_error_documents_do_not_hide_a_valid_legacy_copy() {
        let home = tempfile::tempdir().unwrap();
        let locations = Locations::for_identity(
            home.path(),
            &home.path().join("support"),
            crate::identity::PRODUCTION,
            None,
        );
        let partial = locations.model_roots[0].join("ggml-base.en.bin");
        std::fs::create_dir_all(partial.parent().unwrap()).unwrap();
        whisper(&partial);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&partial)
            .unwrap()
            .set_len(20 * 1024 * 1024)
            .unwrap();
        assert!(locations.resolve("base.en").is_none());
        whisper(&partial);
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&partial)
            .unwrap()
            .write_all(b"<html>Error")
            .unwrap();
        assert!(locations.resolve("base.en").is_none());
        let valid = locations.model_roots[1].join("ggml-base.en.bin");
        whisper(&valid);
        assert_eq!(locations.resolve("base.en"), Some(valid));
        assert!(locations.resolve("../base.en").is_none());
    }

    #[test]
    fn sherpa_uses_the_same_legacy_roots_and_requires_a_complete_bundle() {
        let home = tempfile::tempdir().unwrap();
        let locations = Locations::for_identity(
            home.path(),
            &home.path().join("support"),
            crate::identity::PRODUCTION,
            Some(home.path().join("custom")),
        );
        let model = "parakeet-tdt-0.6b-v2-fp16";
        let (_, bundle) = crate::transcriber::parakeet::download_spec(model).unwrap();
        for root in &locations.model_roots {
            let dir = root.join(&bundle);
            std::fs::create_dir_all(&dir).unwrap();
            crate::model_validation::fixtures::parakeet(&dir);
            std::fs::remove_file(dir.join("tokens.txt")).unwrap();
            assert!(locations.resolve(model).is_none());
            crate::model_validation::fixtures::parakeet(&dir);
            assert_eq!(locations.resolve(model), Some(dir.clone()));
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn invalid_candidates_never_mask_later_complete_models_or_change_legacy_files() {
        use std::io::Write;
        let home = tempfile::tempdir().unwrap();
        let locations = Locations::for_identity(
            home.path(),
            &home.path().join("support"),
            crate::identity::PRODUCTION,
            None,
        );
        let early = locations.model_roots.first().unwrap();
        let later = locations.model_roots.last().unwrap();
        let invalid = early.join("ggml-base.en.bin");
        whisper(&invalid);
        let valid = later.join("ggml-base.en.bin");
        whisper(&valid);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&invalid)
            .unwrap();
        file.set_len(20 * 1024 * 1024).unwrap();
        assert_eq!(locations.resolve("base.en"), Some(valid.clone()));
        assert_eq!(file.metadata().unwrap().len(), 20 * 1024 * 1024);
        whisper(&invalid);
        file.write_all(b"nope").unwrap();
        assert_eq!(locations.resolve("base.en"), Some(valid));
        let mut magic = [0; 4];
        std::io::Read::read_exact(&mut std::fs::File::open(&invalid).unwrap(), &mut magic).unwrap();
        assert_eq!(&magic, b"nope");

        let model = "parakeet-tdt-0.6b-v2-fp16";
        let (_, bundle) = crate::transcriber::parakeet::download_spec(model).unwrap();
        let invalid = early.join(&bundle);
        let valid = later.join(&bundle);
        crate::model_validation::fixtures::parakeet(&invalid);
        crate::model_validation::fixtures::parakeet(&valid);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(invalid.join("decoder.fp16.onnx"))
            .unwrap();
        file.write_all(b"corrupt").unwrap();
        assert_eq!(locations.resolve(model), Some(valid.clone()));
        crate::model_validation::fixtures::parakeet(&invalid);
        file.set_len(100).unwrap();
        assert_eq!(locations.resolve(model), Some(valid));
        assert_eq!(file.metadata().unwrap().len(), 100);
    }

    #[test]
    fn isolated_legacy_roots_never_reach_production_whisper_or_sherpa() {
        let home = tempfile::tempdir().unwrap();
        let support = home.path().join("Library/Application Support");
        let production =
            Locations::for_identity(home.path(), &support, crate::identity::PRODUCTION, None);
        let preview = Locations::for_identity(
            home.path(),
            &support,
            "test.preview",
            Some(production.download_root.clone()),
        );
        assert_eq!(preview.coreml_root, production.coreml_root);
        for root in &production.model_roots {
            whisper(&root.join("ggml-base.en.bin"));
        }
        assert!(preview.resolve("base.en").is_none());
        for root in &preview.model_roots {
            let file = root.join("ggml-base.en.bin");
            whisper(&file);
            assert_eq!(preview.resolve("base.en"), Some(file.clone()));
            std::fs::remove_file(file).unwrap();
        }
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn complete_coreml_cache_is_shared_but_incomplete_cache_is_not_installed() {
        let home = tempfile::tempdir().unwrap();
        let support = home.path().join("support");
        let production =
            Locations::for_identity(home.path(), &support, crate::identity::PRODUCTION, None);
        let preview = Locations::for_identity(home.path(), &support, "test.preview", None);
        let dir = production
            .coreml_root
            .join(crate::transcriber::coreml::CACHE_DIR_NAME);
        for name in [
            "Preprocessor.mlmodelc",
            "Encoder.mlmodelc",
            "Decoder.mlmodelc",
            "JointDecisionv3.mlmodelc",
        ] {
            let compiled = dir.join(name);
            std::fs::create_dir_all(compiled.join("weights")).unwrap();
            std::fs::write(compiled.join("coremldata.bin"), b"synthetic").unwrap();
            std::fs::write(compiled.join("weights/weight.bin"), b"synthetic").unwrap();
        }
        assert!(preview
            .resolve(crate::transcriber::COREML_MODEL_NAME)
            .is_none());
        std::fs::write(dir.join("parakeet_vocab.json"), b"{}").unwrap();
        for locations in [&production, &preview] {
            assert_eq!(
                locations.resolve(crate::transcriber::COREML_MODEL_NAME),
                Some(dir.clone())
            );
        }
    }
}
