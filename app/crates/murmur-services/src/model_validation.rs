//! Read-only, constant-I/O admission for the exact artifacts Murmur downloads.
//! Sizes and GGML hyperparameters are from ggerganov/whisper.cpp at
//! 5359861c739e955e79d9a303bcbc70fb988958b1 (Hugging Face tree API).
//! Sherpa sizes are from csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-fp16
//! at a6cc4e24ed264a93a17575dad416dacc4238497a, the release bundle's mirror.
//! This checks completeness and format, not a cryptographic integrity claim.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

fn whisper_spec(model: &str) -> Option<(u64, [u32; 12])> {
    let (bytes, state, heads, layers, text_layers, vocab, mels) = match model {
        "tiny.en" => (77_704_715, 384, 6, 4, 4, 51_864, 80),
        "base.en" => (147_964_211, 512, 8, 6, 6, 51_864, 80),
        "small.en" => (487_614_201, 768, 12, 12, 12, 51_864, 80),
        "medium.en" => (1_533_774_781, 1024, 16, 24, 24, 51_864, 80),
        "large-v3-turbo" => (1_624_555_275, 1280, 20, 32, 4, 51_866, 128),
        _ => return None,
    };
    Some((
        bytes,
        [
            0x67676d6c,
            vocab,
            1500,
            state,
            heads,
            layers,
            448,
            state,
            heads,
            text_layers,
            mels,
            1,
        ],
    ))
}

pub(crate) fn validate_whisper(path: &Path, model: &str) -> Result<(), String> {
    let invalid = || format!("Whisper model '{model}' is incomplete or has an invalid GGML header");
    let (bytes, header) = whisper_spec(model).ok_or_else(invalid)?;
    let mut file = open_exact(path, bytes).ok_or_else(invalid)?;
    let mut actual = [0; 48];
    file.read_exact(&mut actual).map_err(|_| invalid())?;
    let expected: Vec<_> = header.into_iter().flat_map(u32::to_le_bytes).collect();
    if actual.as_slice() != expected {
        return Err(invalid());
    }
    Ok(())
}

fn open_exact(path: &Path, bytes: u64) -> Option<File> {
    let file = File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    (meta.is_file() && meta.len() == bytes).then_some(file)
}

fn parakeet_bytes(component: &str) -> Option<u64> {
    match component {
        "encoder.fp16.onnx" => Some(1_239_245_548),
        "decoder.fp16.onnx" => Some(14_446_596),
        "joiner.fp16.onnx" => Some(3_456_459),
        "tokens.txt" => Some(9384),
        _ => None,
    }
}

fn varint(file: &mut File) -> Option<u64> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let mut byte = [0];
        file.read_exact(&mut byte).ok()?;
        if shift == 63 && byte[0] > 1 {
            return None;
        }
        value |= u64::from(byte[0] & 127) << shift;
        if byte[0] < 128 {
            return Some(value);
        }
    }
    None
}

// Walk only the ModelProto envelope, seeking over tensor/graph payloads.
// Bad wire types, overflowing lengths and a truncated graph all fail closed.
// No tensor scan, allocation proportional to file size, or native ONNX load.
fn onnx_envelope(file: &mut File, bytes: u64) -> Option<()> {
    let mut ir = false;
    let mut graph = false;
    let mut opset = false;
    for _ in 0..128 {
        if file.stream_position().ok()? == bytes {
            return (ir && graph && opset).then_some(());
        }
        let tag = varint(file)?;
        let field = tag >> 3;
        if field == 0 {
            return None;
        }
        match tag & 7 {
            0 => {
                let value = varint(file)?;
                if field == 1 {
                    if ir || !(7..=12).contains(&value) {
                        return None;
                    }
                    ir = true;
                }
            }
            2 => {
                let length = varint(file)?;
                let start = file.stream_position().ok()?;
                let end = start.checked_add(length)?;
                if end > bytes {
                    return None;
                }
                if field == 7 {
                    if graph || length < 1024 {
                        return None;
                    }
                    // A GraphProto begins with its first NodeProto.
                    if varint(file)? != 10 {
                        return None;
                    }
                    let node_length = varint(file)?;
                    if node_length == 0
                        || file.stream_position().ok()?.checked_add(node_length)? > end
                    {
                        return None;
                    }
                    graph = true;
                }
                if field == 8 {
                    opset = length > 0;
                }
                file.seek(SeekFrom::Start(end)).ok()?;
            }
            _ => return None,
        }
    }
    None
}

pub(crate) fn parakeet_component_is_valid(path: &Path, component: &str) -> bool {
    let Some(bytes) = parakeet_bytes(component) else {
        return false;
    };
    let Some(mut file) = open_exact(path, bytes) else {
        return false;
    };
    if component != "tokens.txt" {
        return onnx_envelope(&mut file, bytes).is_some();
    }
    let mut tokens = String::new();
    if file.read_to_string(&mut tokens).is_err() || !tokens.ends_with('\n') {
        return false;
    }
    tokens.lines().count() == 1025
        && tokens.lines().enumerate().all(|(id, line)| {
            line.rsplit_once(' ').is_some_and(|(token, number)| {
                !token.is_empty() && number.parse::<usize>() == Ok(id)
            })
        })
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use std::io::Write;

    pub(crate) fn whisper(path: &Path, model: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let (bytes, header) = whisper_spec(model).unwrap();
        let mut file = File::create(path).unwrap();
        for value in header {
            file.write_all(&value.to_le_bytes()).unwrap();
        }
        file.set_len(bytes).unwrap();
    }

    fn write_varint(file: &mut File, mut value: u64) {
        while value > 127 {
            file.write_all(&[(value as u8 & 127) | 128]).unwrap();
            value >>= 7;
        }
        file.write_all(&[value as u8]).unwrap();
    }

    pub(crate) fn parakeet(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        for name in ["encoder.fp16.onnx", "decoder.fp16.onnx", "joiner.fp16.onnx"] {
            let bytes = parakeet_bytes(name).unwrap();
            let mut file = File::create(path.join(name)).unwrap();
            file.write_all(b"\x08\x08\x12\x07pytorch\x1a\x052.7.0\x3a")
                .unwrap();
            // All three pinned lengths use a four/five-byte graph length.
            let width = if bytes > 1 << 28 { 5 } else { 4 };
            let graph_bytes = bytes - file.stream_position().unwrap() - width - 4;
            write_varint(&mut file, graph_bytes);
            file.write_all(b"\x0a\x01\0").unwrap();
            file.seek(SeekFrom::Start(bytes - 4)).unwrap();
            file.write_all(b"\x42\x02\x10\x11").unwrap();
        }
        let mut tokens: String = (0..1025).map(|id| format!("token {id}\n")).collect();
        // Keep IDs realistic and the pinned length; padding belongs to a token.
        if tokens.len() > 9384 {
            tokens = (0..1025).map(|id| format!("t {id}\n")).collect();
        }
        tokens.insert_str(0, &"x".repeat(9384 - tokens.len()));
        std::fs::write(path.join("tokens.txt"), tokens).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn whisper_requires_model_specific_size_magic_and_hyperparameters() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("model.bin");
        for model in [
            "tiny.en",
            "base.en",
            "small.en",
            "medium.en",
            "large-v3-turbo",
        ] {
            fixtures::whisper(&path, model);
            assert!(validate_whisper(&path, model).is_ok());
            assert!(validate_whisper(&path, "unknown").is_err());
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.write_all(b"nope").unwrap();
            assert!(validate_whisper(&path, model).is_err());
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(b"lmgg").unwrap();
            file.seek(SeekFrom::Start(12)).unwrap();
            file.write_all(&123_u32.to_le_bytes()).unwrap();
            assert!(validate_whisper(&path, model).is_err());
            fixtures::whisper(&path, model);
            file.set_len(20 * 1024 * 1024).unwrap();
            assert!(validate_whisper(&path, model).is_err());
        }
    }

    #[test]
    fn sherpa_rejects_short_components_corrupt_protobuf_and_invalid_tokens() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "encoder.fp16.onnx",
            "decoder.fp16.onnx",
            "joiner.fp16.onnx",
            "tokens.txt",
        ] {
            fixtures::parakeet(root.path());
            let path = root.path().join(name);
            assert!(parakeet_component_is_valid(&path, name));
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.write_all(b"\xffsynthetic").unwrap();
            assert!(!parakeet_component_is_valid(&path, name));
            fixtures::parakeet(root.path());
            file.set_len(parakeet_bytes(name).unwrap() - 1).unwrap();
            assert!(!parakeet_component_is_valid(&path, name));
        }
    }
}
