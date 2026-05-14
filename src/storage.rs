// Rust guideline compliant 2026-02-21

use color_eyre::Result;
use keyring::Entry;
use serde::{Deserialize, Serialize};
use tracing::{Level, event};

/// The base name of the service used when storing secrets in the OS keyring.
const SERVICE_BASE: &str = "ani-sync";

/// A bundle of OAuth 2.0 tokens and metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenBundle {
    /// The access token used for authentication.
    pub access_token: String,
    /// The optional refresh token used to obtain new access tokens.
    pub refresh_token: Option<String>,
    /// The optional expiration timestamp (Unix epoch in seconds).
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

/// Retrieves a `TokenBundle` for a specific account from the OS keyring.
///
/// # Errors
///
/// Returns an error if retrieving the secret from the keyring fails.
#[tracing::instrument(skip_all, fields(account = account))]
pub fn get_token_bundle(account: &str) -> Result<Option<TokenBundle>> {
    event!(
        name: "storage.token_bundle.get.started",
        Level::DEBUG,
        account = account,
        "Attempting to retrieve token bundle from keyring (split mode)",
    );

    let access_entry = match get_entry(&format!("{account}_access")) {
        Ok(e) => e,
        Err(e) => return Err(color_eyre::eyre::Report::from(e)),
    };

    let access_token = match access_entry.get_password() {
        Ok(token) => token,
        Err(keyring::Error::NoEntry) => {
            event!(
                name: "storage.token_bundle.get.not_found",
                Level::DEBUG,
                account = account,
                "No access token found",
            );
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
    event!(
        name: "storage.token_bundle.set.started",
        Level::DEBUG,
        account = account,
        "Attempting to save token bundle to keyring (split mode)",
    );

    let access_entry = match get_entry(&format!("{account}_access")) {
        Ok(e) => e,
        Err(e) => return Err(color_eyre::eyre::Report::from(e)),
    };

    if let Err(e) = access_entry.set_password(&bundle.access_token) {
        event!(
            name: "storage.token_bundle.set.error",
            Level::ERROR,
            account = account,
            error = ?e,
            "Failed to save access token: {:?}",
            e
        );
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
        event!(
            name: "storage.token_bundle.set_meta.error",
            Level::ERROR,
            account = account,
            error = ?e,
            "Failed to save token metadata: {:?}",
            e
        );
        return Err(color_eyre::eyre::Report::from(e));
    }

    event!(
        name: "storage.token_bundle.set.success",
        Level::DEBUG,
        account = account,
        "Successfully saved token bundle",
    );
    Ok(())
}

/// Retrieves a secret for a specific account from the OS keyring.
///
/// # Errors
///
/// Returns an error if retrieving the secret from the keyring fails.
#[tracing::instrument(skip_all, fields(account = account))]
pub fn get_secret(account: &str) -> Result<Option<String>> {
    event!(
        name: "storage.secret.get.started",
        Level::DEBUG,
        account = account,
        "Attempting to retrieve secret from keyring",
    );
    let entry = match get_entry(account) {
        Ok(e) => e,
        Err(e) => {
            event!(
                name: "storage.secret.get.entry_error",
                Level::ERROR,
                account = account,
                error = ?e,
                "Failed to create keyring entry: {:?}",
                e
            );
            return Err(color_eyre::eyre::Report::from(e));
        }
    };
    match entry.get_password() {
        Ok(password) => {
            event!(
                name: "storage.secret.get.success",
                Level::DEBUG,
                account = account,
                "Successfully retrieved secret",
            );
            Ok(Some(password))
        }
        Err(keyring::Error::NoEntry) => {
            event!(
                name: "storage.secret.get.not_found",
                Level::DEBUG,
                account = account,
                "No secret found in keyring",
            );
            Ok(None)
        }
        Err(e) => {
            event!(
                name: "storage.secret.get.error",
                Level::ERROR,
                account = account,
                error = ?e,
                "Failed to retrieve secret: {:?}",
                e
            );
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
    event!(
        name: "storage.secret.set.started",
        Level::DEBUG,
        account = account,
        "Attempting to save secret to keyring",
    );
    let entry = match get_entry(account) {
        Ok(e) => e,
        Err(e) => {
            event!(
                name: "storage.secret.set.entry_error",
                Level::ERROR,
                account = account,
                error = ?e,
                "Failed to create keyring entry: {:?}",
                e
            );
            return Err(color_eyre::eyre::Report::from(e));
        }
    };
    if let Err(e) = entry.set_password(secret) {
        event!(
            name: "storage.secret.set.error",
            Level::ERROR,
            account = account,
            error = ?e,
            "Failed to save secret: {:?}",
            e
        );
        return Err(color_eyre::eyre::Report::from(e));
    }
    event!(
        name: "storage.secret.set.success",
        Level::DEBUG,
        account = account,
        "Successfully saved secret",
    );
    Ok(())
}

/// Deletes a secret for a specific account from the OS keyring.
///
/// # Errors
///
/// Returns an error if deleting the secret from the keyring fails.
#[tracing::instrument(skip_all, fields(account = account))]
pub fn delete_secret(account: &str) -> Result<()> {
    event!(
        name: "storage.secret.delete.started",
        Level::DEBUG,
        account = account,
        "Attempting to delete secret from keyring",
    );
    let entry = match get_entry(account) {
        Ok(e) => e,
        Err(e) => {
            event!(
                name: "storage.secret.delete.entry_error",
                Level::ERROR,
                account = account,
                error = ?e,
                "Failed to create keyring entry: {:?}",
                e
            );
            return Err(color_eyre::eyre::Report::from(e));
        }
    };
    match entry.delete_credential() {
        Ok(()) => {
            event!(
                name: "storage.secret.delete.success",
                Level::DEBUG,
                account = account,
                "Successfully deleted secret",
            );
            Ok(())
        }
        Err(keyring::Error::NoEntry) => {
            event!(
                name: "storage.secret.delete.not_found",
                Level::DEBUG,
                account = account,
                "Secret not found, nothing to delete",
            );
            Ok(())
        }
        Err(e) => {
            event!(
                name: "storage.secret.delete.error",
                Level::ERROR,
                account = account,
                error = ?e,
                "Failed to delete secret: {:?}",
                e
            );
            Err(color_eyre::eyre::Report::from(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Modifies OS keyring; should not be run in CI environments directly."]
    fn test_secret_storage() {
        let account = "test_account";
        let secret = "test_secret";

        set_secret(account, secret).unwrap();
        assert_eq!(get_secret(account).unwrap(), Some(secret.to_string()));

        delete_secret(account).unwrap();
        assert_eq!(get_secret(account).unwrap(), None);
    }

    #[test]
    #[ignore = "Modifies OS keyring; should not be run in CI environments directly."]
    fn test_token_bundle_storage() {
        let account = "test_account_bundle";
        let bundle = TokenBundle {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: Some(123_456_789),
        };

        set_token_bundle(account, &bundle).unwrap();
        let retrieved = get_token_bundle(account).unwrap().unwrap();
        assert_eq!(retrieved, bundle);

        let _ = delete_secret(account);
        assert_eq!(get_token_bundle(account).unwrap(), None);

        set_token_bundle(account, &bundle).unwrap();
        let _ = get_token_bundle(account);
        let _ = delete_secret(account);
    }
}
