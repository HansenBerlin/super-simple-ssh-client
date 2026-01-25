use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as Base64;
use base64::Engine;
use pbkdf2::pbkdf2_hmac;
use rand_core::OsRng;
use rand_core::TryRngCore;
use rpassword::prompt_password;
use sha2::Sha256;

use crate::model::{
    ConnectionConfig, EncryptedBlob, MasterConfig, StoreFile, StoredConnection,
};

pub(crate) fn config_path() -> Result<PathBuf> {
    if let Some(mut dir) = dirs::config_dir() {
        dir.push("ssh-client");
        dir.push("config.json");
        return Ok(dir);
    }
    let mut fallback = std::env::current_dir().context("current dir")?;
    fallback.push("ssh-client-config.json");
    Ok(fallback)
}

pub(crate) fn load_or_init_store(
    path: &Path,
) -> Result<(MasterConfig, Vec<u8>, Vec<ConnectionConfig>)> {
    if path.exists() {
        let store = load_store(path)?;
        let master_key = prompt_existing_master(&store.master)?;
        let connections = store
            .connections
            .into_iter()
            .map(|conn| decrypt_connection(conn, &master_key))
            .collect::<Result<Vec<_>>>()?;
        return Ok((store.master, master_key, connections));
    }

    let (master, master_key) = setup_master()?;
    let store = StoreFile {
        master: master.clone(),
        connections: vec![],
    };
    save_store(path, &store)?;
    Ok((master, master_key, vec![]))
}

pub(crate) fn load_store(path: &Path) -> Result<StoreFile> {
    let content = fs::read_to_string(path).context("read config file")?;
    let store = serde_json::from_str(&content).context("parse config file")?;
    Ok(store)
}

pub(crate) fn save_store(path: &Path, store: &StoreFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create config dir")?;
    }
    let content = serde_json::to_string_pretty(store).context("serialize config")?;
    fs::write(path, content).context("write config file")?;
    Ok(())
}

pub(crate) fn prompt_existing_master(master: &MasterConfig) -> Result<Vec<u8>> {
    loop {
        let password = prompt_password("Master password: ").context("read master password")?;
        let salt = Base64.decode(&master.salt_b64).context("decode salt")?;
        let key = derive_key(&password, &salt);
        match decrypt_string(&master.check, &key) {
            Ok(check) if check == "ssh-client-check" => return Ok(key),
            _ => {
                eprintln!("Invalid master password.");
            }
        }
    }
}

pub(crate) fn setup_master() -> Result<(MasterConfig, Vec<u8>)> {
    loop {
        let password = prompt_password("Set master password: ").context("read master password")?;
        let confirm = prompt_password("Confirm master password: ").context("read confirm password")?;
        if password != confirm {
            eprintln!("Passwords do not match.");
            continue;
        }
        if password.is_empty() {
            eprintln!("Master password cannot be empty.");
            continue;
        }
        return create_master_from_password(&password);
    }
}

pub(crate) fn create_master_from_password(password: &str) -> Result<(MasterConfig, Vec<u8>)> {
    let mut salt = [0u8; 16];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut salt)
        .map_err(|err| anyhow::anyhow!("random salt failed: {err:?}"))?;
    let key = derive_key(password, &salt);
    let check = encrypt_string("ssh-client-check", &key)?;
    let master = MasterConfig {
        salt_b64: Base64.encode(salt),
        check,
    };
    Ok((master, key))
}

pub(crate) fn derive_key(password: &str, salt: &[u8]) -> Vec<u8> {
    let mut key = vec![0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, 100_000, &mut key);
    key
}

pub(crate) fn encrypt_string(plaintext: &str, key: &[u8]) -> Result<EncryptedBlob> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut nonce_bytes)
        .map_err(|err| anyhow::anyhow!("random nonce failed: {err:?}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|err| anyhow::anyhow!("encrypt failed: {err:?}"))?;
    Ok(EncryptedBlob {
        nonce: Base64.encode(nonce_bytes),
        ciphertext: Base64.encode(ciphertext),
    })
}

pub(crate) fn decrypt_string(blob: &EncryptedBlob, key: &[u8]) -> Result<String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce_bytes = Base64.decode(&blob.nonce).context("decode nonce")?;
    let ciphertext = Base64
        .decode(&blob.ciphertext)
        .context("decode ciphertext")?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|err| anyhow::anyhow!("decrypt failed: {err:?}"))?;
    let text = String::from_utf8(plaintext).context("decode utf8")?;
    Ok(text)
}

pub(crate) fn encrypt_connection(conn: &ConnectionConfig, key: &[u8]) -> Result<StoredConnection> {
    let auth = match &conn.auth {
        crate::model::AuthConfig::Password { password } => crate::model::StoredAuthConfig::Password {
            password: encrypt_string(password, key)?,
        },
        crate::model::AuthConfig::PrivateKey { path, password } => {
            crate::model::StoredAuthConfig::PrivateKey {
                path: path.clone(),
                password: match password {
                    Some(pass) => Some(encrypt_string(pass, key)?),
                    None => None,
                },
            }
        }
    };
    Ok(StoredConnection {
        user: conn.user.clone(),
        host: conn.host.clone(),
        auth,
        history: conn.history.clone(),
    })
}

pub(crate) fn decrypt_connection(conn: StoredConnection, key: &[u8]) -> Result<ConnectionConfig> {
    let auth = match conn.auth {
        crate::model::StoredAuthConfig::Password { password } => crate::model::AuthConfig::Password {
            password: decrypt_string(&password, key)?,
        },
        crate::model::StoredAuthConfig::PrivateKey { path, password } => {
            crate::model::AuthConfig::PrivateKey {
                path,
                password: match password {
                    Some(pass) => Some(decrypt_string(&pass, key)?),
                    None => None,
                },
            }
        }
    };
    Ok(ConnectionConfig {
        user: conn.user,
        host: conn.host,
        auth,
        history: conn.history,
    })
}
