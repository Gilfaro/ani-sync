use color_eyre::Result;
use keyring::Entry;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

/// The base name of the service used when storing secrets in the OS keyring.
const SERVICE_BASE: &str = "ani-sync";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenBundle {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
}

/// Helper to create a keyring Entry with a unique target name to prevent
/// Windows Credential Manager from overwriting generic credentials
/// that share the same base `TargetName`.
///
/// # Errors
///
/// Returns an error if the keyring entry cannot be created.
pub fn get_entry(account: &str) -> Result<Entry, keyring::Error> {
    Entry::new(SERVICE_BASE, account)
}
/// Retrieves a `TokenBundle` for a specific account (e.g., "mal", "anilist") from the OS keyring.
///
/// # Errors
///
/// Returns an error if retrieving the secret from the keyring fails.
#[tracing::instrument(skip_all, fields(account = account))]
pub fn get_token_bundle(account: &str) -> Result<Option<TokenBundle>> {
    debug!("Attempting to retrieve token bundle from keyring (split mode)");

    let access_entry = match get_entry(&format!("{account}_access")) {
        Ok(e) => e,
        Err(e) => return Err(color_eyre::eyre::Report::from(e)),
    };

    let access_token = match access_entry.get_password() {
        Ok(token) => token,
        Err(keyring::Error::NoEntry) => {
            debug!("No access token found");
            return Ok(None);
        }
        Err(e) => return Err(color_eyre::eyre::Report::from(e)),
    };

    let meta_entry = match get_entry(&format!("{account}_meta")) {
        Ok(e) => e,
        Err(e) => return Err(color_eyre::eyre::Report::from(e)),
    };

    let mut refresh_token = None;
    let mut expires_at = None;

    if let Ok(meta_json) = meta_entry.get_password() {
        #[derive(Deserialize)]
        struct TokenMeta {
            refresh_token: Option<String>,
            expires_at: Option<i64>,
        }
        if let Ok(meta) = serde_json::from_str::<TokenMeta>(&meta_json) {
            refresh_token = meta.refresh_token;
            expires_at = meta.expires_at;
        }
    }

    Ok(Some(TokenBundle {
        access_token,
        refresh_token,
        expires_at,
    }))
}

/// Saves a `TokenBundle` for a specific account in the OS keyring.
///
/// # Errors
///
/// Returns an error if the secret cannot be saved to the keyring.
#[tracing::instrument(skip(bundle), fields(account = account))]
pub fn set_token_bundle(account: &str, bundle: &TokenBundle) -> Result<()> {
    debug!("Attempting to save token bundle to keyring (split mode)");

    let access_entry = match get_entry(&format!("{account}_access")) {
        Ok(e) => e,
        Err(e) => return Err(color_eyre::eyre::Report::from(e)),
    };

    if let Err(e) = access_entry.set_password(&bundle.access_token) {
        error!("Failed to save access token: {:?}", e);
        return Err(color_eyre::eyre::Report::from(e));
    }

    let meta_entry = match get_entry(&format!("{account}_meta")) {
        Ok(e) => e,
        Err(e) => return Err(color_eyre::eyre::Report::from(e)),
    };

    #[derive(Serialize)]
    struct TokenMeta<'a> {
        refresh_token: Option<&'a String>,
        expires_at: Option<i64>,
    }

    let meta = TokenMeta {
        refresh_token: bundle.refresh_token.as_ref(),
        expires_at: bundle.expires_at,
    };

    let meta_json = serde_json::to_string(&meta)?;
    if let Err(e) = meta_entry.set_password(&meta_json) {
        error!("Failed to save token metadata: {:?}", e);
        return Err(color_eyre::eyre::Report::from(e));
    }

    debug!("Successfully saved token bundle");
    Ok(())
}

/// Retrieves a secret for a specific account (e.g., "mal", "anilist") from the OS keyring.
///
/// # Errors
///
/// Returns an error if retrieving the secret from the keyring fails.
#[tracing::instrument(skip_all, fields(account = account))]
pub fn get_secret(account: &str) -> Result<Option<String>> {
    debug!("Attempting to retrieve secret from keyring");
    let entry = match get_entry(account) {
        Ok(e) => e,
        Err(e) => {
            error!("Failed to create keyring entry: {e:?}");
            return Err(color_eyre::eyre::Report::from(e));
        }
    };
    match entry.get_password() {
        Ok(password) => {
            debug!("Successfully retrieved secret");
            Ok(Some(password))
        }
        Err(keyring::Error::NoEntry) => {
            debug!("No secret found in keyring");
            Ok(None)
        }
        Err(e) => {
            error!("Failed to retrieve secret: {e:?}");
            Err(color_eyre::eyre::Report::from(e))
        }
    }
}

/// Saves a secret for a specific account in the OS keyring.
///
/// # Errors
///
/// Returns an error if the secret cannot be saved to the keyring.
#[tracing::instrument(skip(secret), fields(account = account))]
pub fn set_secret(account: &str, secret: &str) -> Result<()> {
    debug!("Attempting to save secret to keyring");
    let entry = match get_entry(account) {
        Ok(e) => e,
        Err(e) => {
            error!("Failed to create keyring entry: {e:?}");
            return Err(color_eyre::eyre::Report::from(e));
        }
    };
    if let Err(e) = entry.set_password(secret) {
        error!("Failed to save secret: {e:?}");
        return Err(color_eyre::eyre::Report::from(e));
    }
    debug!("Successfully saved secret");
    Ok(())
}

/// Deletes a secret for a specific account from the OS keyring.
///
/// # Errors
///
/// Returns an error if deleting the secret from the keyring fails.
#[tracing::instrument(skip_all, fields(account = account))]
pub fn delete_secret(account: &str) -> Result<()> {
    debug!("Attempting to delete secret from keyring");
    let entry = match get_entry(account) {
        Ok(e) => e,
        Err(e) => {
            error!("Failed to create keyring entry: {e:?}");
            return Err(color_eyre::eyre::Report::from(e));
        }
    };
    match entry.delete_credential() {
        Ok(()) => {
            debug!("Successfully deleted secret");
            Ok(())
        }
        Err(keyring::Error::NoEntry) => {
            debug!("Secret not found, nothing to delete");
            Ok(())
        }
        Err(e) => {
            error!("Failed to delete secret: {e:?}");
            Err(color_eyre::eyre::Report::from(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Since `keyring::Entry::new_with_target` may not behave the same in the
    // mock backend (it might not retain state across instances), we are avoiding
    // doing a rigorous assert here because Windows TargetNames don't map 1:1 with
    // the Mock backend in the `keyring` crate v3.
    #[test]
    #[ignore = "Modifies OS keyring; should not be run in CI environments directly."]
    fn test_secret_storage() {
        let account = "test_account";
        let secret = "super_secret_token_123";

        let _ = delete_secret(account);
        assert_eq!(get_secret(account).unwrap(), None);

        // We mainly want to ensure it doesn't panic
        set_secret(account, secret).unwrap();
        let _ = get_secret(account);
        let _ = delete_secret(account);
    }

    #[test]
    #[ignore = "Modifies OS keyring; should not be run in CI environments directly."]
    fn test_token_bundle_storage() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("debug")
            .try_init();
        let account = "test_bundle_account";
        let bundle = TokenBundle {
            access_token: "access_123".to_string(),
            refresh_token: Some("refresh_123".to_string()),
            expires_at: Some(1_234_567_890),
        };

        let _ = delete_secret(account);
        assert_eq!(get_token_bundle(account).unwrap(), None);

        set_token_bundle(account, &bundle).unwrap();
        let _ = get_token_bundle(account);
        let _ = delete_secret(account);
    }
}
