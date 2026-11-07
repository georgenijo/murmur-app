//! Tauri-compatible base64(Minisign text) signatures, without the Tauri CLI.
//! Secrets are accepted only through the release environment; never argv/files.
use base64::{engine::general_purpose::STANDARD, Engine};
use minisign::{PublicKeyBox, SecretKeyBox};
use std::{env, fs, io::Cursor, path::Path};

fn sign(
    bytes: &[u8],
    encoded_key: &str,
    password: String,
    encoded_public: &str,
) -> Result<String, ()> {
    let key =
        String::from_utf8(STANDARD.decode(encoded_key.trim()).map_err(|_| ())?).map_err(|_| ())?;
    let secret = SecretKeyBox::from_string(&key)
        .and_then(|key| key.into_secret_key(Some(password)))
        .map_err(|_| ())?;
    let public = String::from_utf8(STANDARD.decode(encoded_public.trim()).map_err(|_| ())?)
        .map_err(|_| ())?;
    let public = PublicKeyBox::from_string(&public)
        .and_then(|key| key.into_public_key())
        .map_err(|_| ())?;
    let signature =
        minisign::sign(Some(&public), &secret, Cursor::new(bytes), None, None).map_err(|_| ())?;
    // Check the supplied public key before writing an artifact. Wrong keys and
    // wrong passwords fail without printing key material or library errors.
    minisign::verify(&public, &signature, Cursor::new(bytes), true, false, false)
        .map_err(|_| ())?;
    Ok(STANDARD.encode(signature.to_string()))
}

fn main() -> Result<(), &'static str> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: murmur-updater-signer ARCHIVE (keys/password via environment)");
    }
    let path = Path::new(&args[0]);
    let bytes = fs::read(path).map_err(|_| "could not read archive")?;
    let key = env::var("TAURI_SIGNING_PRIVATE_KEY").map_err(|_| "missing signing key")?;
    let password = env::var("TAURI_SIGNING_PRIVATE_KEY_PASSWORD").unwrap_or_default();
    let public = env::var("MURMUR_SIGNING_PUBLIC_KEY").map_err(|_| "missing public key")?;
    let signature = sign(&bytes, &key, password, &public).map_err(|_| "updater signing failed")?;
    let mut output = path.as_os_str().to_os_string();
    output.push(".sig");
    fs::write(output, signature).map_err(|_| "could not write signature")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minisign::KeyPair;

    #[test]
    fn encrypted_tauri_key_and_signature_wire_format_work_in_both_clients() {
        let pair = KeyPair::generate_encrypted_keypair(Some("synthetic password".into())).unwrap();
        let key = STANDARD.encode(pair.sk.to_box(None).unwrap().to_string());
        let public = STANDARD.encode(pair.pk.to_box().unwrap().to_string());
        let data = b"synthetic updater archive";
        let wire = sign(data, &key, "synthetic password".into(), &public).unwrap();
        let text = String::from_utf8(STANDARD.decode(wire).unwrap()).unwrap();
        let pk_text = String::from_utf8(STANDARD.decode(&public).unwrap()).unwrap();
        let key_line = pk_text.lines().nth(1).unwrap();
        let old = installed_client_verifier::PublicKey::from_base64(key_line).unwrap();
        let old_sig = installed_client_verifier::Signature::decode(&text).unwrap();
        old.verify(data, &old_sig, false).unwrap();
        assert!(old.verify(b"tampered", &old_sig, false).is_err());
        let new = minisign_verify::PublicKey::from_base64(key_line).unwrap();
        let new_sig = minisign_verify::Signature::decode(&text).unwrap();
        new.verify(data, &new_sig, false).unwrap();
        assert!(new.verify(b"tampered", &new_sig, false).is_err());
        assert!(sign(data, &key, "wrong password".into(), &public).is_err());
        let other = KeyPair::generate_unencrypted_keypair().unwrap();
        let wrong_public = STANDARD.encode(other.pk.to_box().unwrap().to_string());
        assert!(sign(data, &key, "synthetic password".into(), &wrong_public).is_err());
        assert!(sign(data, "invalid key", String::new(), &public).is_err());
    }
}
