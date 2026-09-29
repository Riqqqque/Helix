//! Keeps server API token secrets encrypted so the owner can view them again.
//!
//! Tokens are still authenticated by their one-way verifier. This only holds an
//! encrypted copy for display. The master key lives in `secrets/` under the data
//! directory, outside `state/`, so a copied state database or a `helixctl`
//! state backup does not reveal any token.

use helix_secrets::{MasterKeyCredential, SecretIdentity, SecretStore, SecretValue};
use helix_state::StateDatabase;
use secrecy::{ExposeSecret, SecretBox};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use uuid::Uuid;

const KEY_DIR: &str = "secrets";
const KEY_FILE: &str = "token-vault.key";
const MAX_KEY_FILE_BYTES: u64 = 4 * 1024;
pub(crate) const SCOPE_TYPE: &str = "api_token";
pub(crate) const SECRET_TYPE: &str = "server_api_token";
pub(crate) const PURPOSE: &str = "owner_view";

pub(crate) struct TokenVault {
    encoded_key: SecretBox<Vec<u8>>,
}

impl TokenVault {
    /// Loads the vault key, creating it on first use. The key is bound to this installation.
    pub(crate) fn load_or_create(data_dir: &Path, state: &StateDatabase) -> Result<Self, String> {
        let installation = Uuid::parse_str(state.installation_id())
            .map_err(|_| "the Helix installation identifier is invalid".to_owned())?;
        let dir = data_dir.join(KEY_DIR);
        create_private_dir(&dir)?;
        let path = dir.join(KEY_FILE);
        let encoded = match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_file() || metadata.len() > MAX_KEY_FILE_BYTES {
                    return Err("the token vault key is not a regular file".to_owned());
                }
                let mut bytes = Vec::new();
                fs::File::open(&path)
                    .and_then(|file| file.take(MAX_KEY_FILE_BYTES).read_to_end(&mut bytes))
                    .map_err(|_| "the token vault key could not be read".to_owned())?;
                bytes
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let credential = MasterKeyCredential::generate(installation, 1)
                    .map_err(|_| "could not generate a token vault key".to_owned())?;
                let bytes = credential.encode().with_secret(<[u8]>::to_vec);
                write_private_file(&path, &bytes)?;
                bytes
            }
            Err(_) => return Err("the token vault key could not be read".to_owned()),
        };
        let vault = Self {
            encoded_key: SecretBox::new(Box::new(encoded)),
        };
        // Installs the key record on first use and proves the key matches afterwards.
        vault.store(state)?;
        Ok(vault)
    }

    fn store<'a>(&self, state: &'a StateDatabase) -> Result<SecretStore<'a>, String> {
        let credential =
            MasterKeyCredential::decode(SecretValue::new(self.encoded_key.expose_secret().clone()))
                .map_err(|_| "the token vault key is damaged".to_owned())?;
        SecretStore::open(state, credential)
            .map_err(|_| "the token vault key does not match this Helix installation".to_owned())
    }

    /// Stores a copy of a newly issued token for later viewing.
    pub(crate) fn remember(
        &self,
        state: &StateDatabase,
        token_id: &str,
        token: &str,
    ) -> Result<(), String> {
        let scope = Uuid::parse_str(token_id).map_err(|_| "invalid token id".to_owned())?;
        let identity = SecretIdentity::new(SECRET_TYPE, SCOPE_TYPE, scope, PURPOSE)
            .map_err(|_| "invalid token vault identity".to_owned())?;
        self.store(state)?
            .put(&identity, SecretValue::new(token.as_bytes().to_vec()))
            .map(|_| ())
            .map_err(|_| "could not store the token for viewing".to_owned())
    }

    /// Decrypts a stored token copy by its secret record id.
    pub(crate) fn reveal(&self, state: &StateDatabase, secret_id: &str) -> Result<String, String> {
        let id = Uuid::parse_str(secret_id).map_err(|_| "invalid secret id".to_owned())?;
        self.store(state)?
            .with_secret(id, |bytes| String::from_utf8(bytes.to_vec()))
            .map_err(|_| "the stored token could not be decrypted".to_owned())?
            .map_err(|_| "the stored token is damaged".to_owned())
    }
}

fn create_private_dir(dir: &Path) -> Result<(), String> {
    match fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Err("the token vault directory is not a directory".to_owned()),
        Err(_) => {
            fs::create_dir(dir)
                .map_err(|_| "could not create the token vault directory".to_owned())?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not protect the token vault directory".to_owned())?;
    }
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "could not create the token vault key".to_owned())?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "could not write the token vault key".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_state::DatabaseSet;

    #[test]
    fn tokens_round_trip_and_the_key_stays_out_of_the_state_folder() {
        let temp = crate::private_test_directory("token vault data directory");
        let databases = DatabaseSet::open_for_daemon(temp.path()).expect("state");
        let vault = TokenVault::load_or_create(temp.path(), databases.state()).expect("vault");
        let key = temp.path().join(KEY_DIR).join(KEY_FILE);
        assert!(key.is_file());
        assert!(!key.starts_with(temp.path().join("state")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&key).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let token_id = Uuid::new_v4().to_string();
        vault
            .remember(databases.state(), &token_id, "hxs_example-secret")
            .expect("remember");
        let secret_id = databases
            .state()
            .api_token_secret_id(&token_id)
            .expect("lookup")
            .expect("stored");
        assert_eq!(
            vault.reveal(databases.state(), &secret_id).expect("reveal"),
            "hxs_example-secret"
        );
        // A second start reads the same key instead of creating another.
        let again = TokenVault::load_or_create(temp.path(), databases.state()).expect("reload");
        assert_eq!(
            again
                .reveal(databases.state(), &secret_id)
                .expect("reveal again"),
            "hxs_example-secret"
        );
    }
}
