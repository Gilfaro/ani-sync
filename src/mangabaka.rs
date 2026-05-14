// Rust guideline compliant 2026-02-21

use crate::client::{BaseClient, OAuthProvider, TokenRefresher};
use crate::models::{MediaType, SyncStatus, TrackerClient, TrackerEntry, UpdateOptions};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use color_eyre::{Result, eyre::eyre};
use rand::Rng;
use reqwest::{Method, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{Level, event};

/// Base URL for the `MangaBaka` API.
pub const MANGABAKA_BASE_URL: &str = "https://api.mangabaka.dev";
/// Client ID for the Ani-Sync application on `MangaBaka`.
pub const MANGABAKA_CLIENT_ID: &str = "dhFSCMtpNCDkJLdpJcyGEMleMWkMoGOw";
/// URL for redirecting users to authorize via OAuth 2.0.
pub const MANGABAKA_OAUTH_AUTHORIZE_URL: &str = "https://mangabaka.org/auth/oauth2/authorize";
/// URL for exchanging and refreshing tokens via OAuth 2.0.
pub const MANGABAKA_OAUTH_TOKEN_URL: &str = "https://mangabaka.org/auth/oauth2/token";

/// Helper to deserialize strings or integers into an `Option<i32>`.
///
/// # Errors
///
/// Returns an error if the value is not a string or integer, or if the string cannot be parsed as an integer.
pub fn deserialize_string_or_i32<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    use std::fmt;

    struct StringOrI32;

    impl<'de> de::Visitor<'de> for StringOrI32 {
        type Value = Option<i32>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an integer or a string representing an integer")
        }

        fn visit_i32<E>(self, value: i32) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(value))
        }

        fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            use std::convert::TryFrom;
            i32::try_from(value).map(Some).map_err(de::Error::custom)
        }

        fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            use std::convert::TryFrom;
            i32::try_from(value).map(Some).map_err(de::Error::custom)
        }

        fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value.is_empty() {
                return Ok(None);
            }
            value.parse::<i32>().map(Some).map_err(de::Error::custom)
        }

        fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }

        fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }

        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_any(StringOrI32)
}

/// Metadata for a series on `MangaBaka`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaBakaSeries {
    /// Internal series ID.
    pub id: Option<i64>,
    /// Primary title.
    pub title: Option<String>,
    /// Romanized title.
    pub romanized_title: Option<String>,
    /// Total chapters available.
    #[serde(default, deserialize_with = "deserialize_string_or_i32")]
    pub total_chapters: Option<i32>,
    /// Final volume number.
    #[serde(default, deserialize_with = "deserialize_string_or_i32")]
    pub final_volume: Option<i32>,
    /// External source mappings.
    pub source: Option<MangaBakaSource>,
}

/// External source mappings for a series on `MangaBaka`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaBakaSource {
    /// `MyAnimeList` ID mapping.
    pub my_anime_list: Option<MangaBakaExternalId>,
    /// `AniList` ID mapping.
    pub anilist: Option<MangaBakaExternalId>,
    /// Kitsu ID mapping.
    pub kitsu: Option<MangaBakaExternalId>,
}

/// An external ID mapping on `MangaBaka`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaBakaExternalId {
    /// The external ID.
    pub id: Option<i64>,
}

/// A library item entry on `MangaBaka`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaBakaLibraryItem {
    /// The user's status for the series (e.g., "reading").
    pub state: Option<String>,
    /// The user's rating.
    #[serde(default, deserialize_with = "deserialize_string_or_i32")]
    pub rating: Option<i32>,
    /// Current chapter progress.
    #[serde(default, deserialize_with = "deserialize_string_or_i32")]
    pub progress_chapter: Option<i32>,
    /// Current volume progress.
    #[serde(default, deserialize_with = "deserialize_string_or_i32")]
    pub progress_volume: Option<i32>,
    /// When the user started the series.
    pub start_date: Option<String>,
    /// When the user finished the series.
    pub finish_date: Option<String>,
    /// User notes.
    pub note: Option<String>,
    /// Associated series metadata.
    #[serde(rename = "Series")]
    pub series: MangaBakaSeries,
}

/// Pagination metadata for `MangaBaka` library responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaBakaPagination {
    /// URL for the next page of results.
    pub next: Option<String>,
}

/// Response from the `MangaBaka` library endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MangaBakaLibraryResponse {
    /// The list of library items.
    pub data: Vec<MangaBakaLibraryItem>,
    /// Pagination information.
    pub pagination: Option<MangaBakaPagination>,
}

/// A client for the `MangaBaka` API.
pub struct MangaBakaClient {
    /// The underlying HTTP client.
    pub client: Arc<BaseClient>,
    access_token: Arc<RwLock<String>>,
}

/// OAuth 2.0 provider for `MangaBaka` using PKCE.
pub struct MangaBakaOAuth {
    code_verifier: String,
    state: String,
}

impl Default for MangaBakaOAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl MangaBakaOAuth {
    /// Creates a new `MangaBakaOAuth` with random PKCE verifier and state.
    #[must_use]
    pub fn new() -> Self {
        let mut rng = rand::rng();

        let mut verifier_bytes = [0u8; 96];
        rng.fill_bytes(&mut verifier_bytes);
        let code_verifier = general_purpose::URL_SAFE_NO_PAD.encode(verifier_bytes);

        let mut state_bytes = [0u8; 16];
        rng.fill_bytes(&mut state_bytes);
        let state = general_purpose::URL_SAFE_NO_PAD.encode(state_bytes);

        Self {
            code_verifier,
            state,
        }
    }

    /// Verifies that the returned state matches the generated state.
    #[must_use]
    pub fn verify_state(&self, state: &str) -> bool {
        self.state == state
    }
}

#[async_trait]
impl OAuthProvider for MangaBakaOAuth {
    /// Returns the `MangaBaka` authorization URL with PKCE challenge.
    fn get_auth_url(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.code_verifier.as_bytes());
        let code_challenge = general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());

        let mut url = url::Url::parse(MANGABAKA_OAUTH_AUTHORIZE_URL).unwrap();
        url.query_pairs_mut()
            .append_pair("client_id", MANGABAKA_CLIENT_ID)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", "http://127.0.0.1:9145")
            .append_pair("code_challenge", &code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair(
                "scope",
                "openid profile library.read library.write offline_access",
            )
            .append_pair("state", &self.state);

        url.to_string()
    }

    /// Exchanges the authorization code for an access token.
    ///
    /// # Errors
    ///
    /// Returns an error if the exchange fails or the response is invalid.
    async fn exchange_token(&self, code: &str) -> Result<()> {
        let client = crate::client::create_reqwest_client()?;
        let mut data = HashMap::new();
        data.insert("grant_type", "authorization_code");
        data.insert("client_id", MANGABAKA_CLIENT_ID);
        data.insert("code", code);
        data.insert("redirect_uri", "http://127.0.0.1:9145");
        data.insert("code_verifier", &self.code_verifier);

        let res = client
            .post(MANGABAKA_OAUTH_TOKEN_URL)
            .form(&data)
            .send()
            .await?;

        if res.status().is_success() {
            let token_data: serde_json::Value = res.json().await?;
            if let Some(access_token) = token_data.get("access_token").and_then(|t| t.as_str()) {
                let bundle = crate::storage::TokenBundle {
                    access_token: access_token.to_string(),
                    refresh_token: token_data
                        .get("refresh_token")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string),
                    expires_at: None,
                };
                crate::storage::set_token_bundle("mangabaka", &bundle)?;
                event!(
                    name: "mangabaka.auth.token_exchanged",
                    Level::INFO,
                    "Successfully exchanged token for `MangaBaka`",
                );
            } else {
                return Err(eyre!("No access_token found in response"));
            }
            Ok(())
        } else {
            Err(eyre!("Token exchange failed: {}", res.text().await?))
        }
    }

    /// Refreshes the `MangaBaka` access token.
    ///
    /// # Errors
    ///
    /// Returns an error if the refresh fails or the response is invalid.
    async fn refresh_token(&self, refresh_token: &str) -> Result<()> {
        let client = crate::client::create_reqwest_client()?;
        let mut data = HashMap::new();
        data.insert("grant_type", "refresh_token");
        data.insert("client_id", MANGABAKA_CLIENT_ID);
        data.insert("refresh_token", refresh_token);

        let res = client
            .post(MANGABAKA_OAUTH_TOKEN_URL)
            .form(&data)
            .send()
            .await?;

        if res.status().is_success() {
            let token_data: serde_json::Value = res.json().await?;
            if let Some(access_token) = token_data.get("access_token").and_then(|t| t.as_str()) {
                let bundle = crate::storage::TokenBundle {
                    access_token: access_token.to_string(),
                    refresh_token: token_data
                        .get("refresh_token")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string),
                    expires_at: None,
                };
                crate::storage::set_token_bundle("mangabaka", &bundle)?;
                event!(
                    name: "mangabaka.auth.token_refreshed",
                    Level::INFO,
                    "Successfully refreshed token for `MangaBaka`",
                );
            } else {
                return Err(eyre!("No access_token found in response"));
            }
            Ok(())
        } else {
            Err(eyre!("Token refresh failed: {}", res.text().await?))
        }
    }
}

/// A token refresher for `MangaBaka`.
pub struct MangaBakaTokenRefresher {
    /// The OAuth provider used for refreshing.
    pub oauth: MangaBakaOAuth,
}

#[async_trait]
impl TokenRefresher for MangaBakaTokenRefresher {
    /// Refreshes the `MangaBaka` access token.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No token bundle is found in storage.
    /// - No refresh token is available in the bundle.
    /// - The refresh request fails.
    async fn refresh(&self) -> Result<String> {
        let bundle = crate::storage::get_token_bundle("mangabaka")?
            .ok_or_else(|| eyre!("No token bundle found for `MangaBaka`"))?;

        let refresh_token = bundle
            .refresh_token
            .as_deref()
            .ok_or_else(|| eyre!("No refresh token available in bundle"))?;

        self.oauth.refresh_token(refresh_token).await?;

        let new_bundle = crate::storage::get_token_bundle("mangabaka")?
            .ok_or_else(|| eyre!("No token bundle found after `MangaBaka` refresh"))?;

        Ok(new_bundle.access_token)
    }
}

/// A wrapper around `MangaBakaTokenRefresher` that updates the client's local access token.
pub struct MangaBakaClientRefresher {
    inner: MangaBakaTokenRefresher,
    access_token: Arc<RwLock<String>>,
}

#[async_trait]
impl TokenRefresher for MangaBakaClientRefresher {
    /// Refreshes the token and updates the local cache.
    ///
    /// # Errors
    ///
    /// Returns an error if the inner refresh fails.
    async fn refresh(&self) -> Result<String> {
        let new_token = self.inner.refresh().await?;
        let mut lock = self.access_token.write().await;
        (*lock).clone_from(&new_token);
        Ok(new_token)
    }
}

impl MangaBakaClient {
    /// Creates a new `MangaBakaClient`.
    ///
    /// # Errors
    ///
    /// Returns an error if the base client cannot be initialized.
    pub fn new(access_token: &str) -> Result<Self> {
        let client = Arc::new(BaseClient::new(
            "mangabaka",
            MANGABAKA_BASE_URL,
            30,
            Duration::from_mins(1),
        )?);
        let access_token_arc = Arc::new(RwLock::new(access_token.to_string()));

        let refresher = Arc::new(MangaBakaClientRefresher {
            inner: MangaBakaTokenRefresher {
                oauth: MangaBakaOAuth::new(),
            },
            access_token: access_token_arc.clone(),
        });

        let c = client.clone();
        tokio::spawn(async move {
            c.set_refresher(refresher).await;
        });

        Ok(Self {
            client,
            access_token: access_token_arc,
        })
    }

    async fn get_access_token(&self) -> String {
        self.access_token.read().await.clone()
    }

    fn map_status(status: &str) -> SyncStatus {
        const STATUS_COMPLETED: &str = "completed";
        const STATUS_PAUSED: &str = "paused";
        const STATUS_DROPPED: &str = "dropped";
        const STATUS_PLAN_TO_READ: &str = "plan_to_read";
        const STATUS_CONSIDERING: &str = "considering";

        match status {
            STATUS_COMPLETED => SyncStatus::Completed,
            STATUS_PAUSED => SyncStatus::Paused,
            STATUS_DROPPED => SyncStatus::Dropped,
            STATUS_PLAN_TO_READ | STATUS_CONSIDERING => SyncStatus::Planning,
            _ => SyncStatus::Current,
        }
    }

    fn reverse_map_status(status: SyncStatus) -> &'static str {
        const STATUS_READING: &str = "reading";
        const STATUS_COMPLETED: &str = "completed";
        const STATUS_PAUSED: &str = "paused";
        const STATUS_DROPPED: &str = "dropped";
        const STATUS_PLAN_TO_READ: &str = "plan_to_read";

        match status {
            SyncStatus::Current => STATUS_READING,
            SyncStatus::Completed => STATUS_COMPLETED,
            SyncStatus::Paused => STATUS_PAUSED,
            SyncStatus::Dropped => STATUS_DROPPED,
            SyncStatus::Planning => STATUS_PLAN_TO_READ,
        }
    }

    fn parse_date(date_str: Option<&String>) -> Option<HashMap<String, Option<i64>>> {
        UpdateOptions::parse_date(date_str)
    }

    /// Gets the media ID for a series given an external source and its ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be parsed.
    async fn get_media_id_by_source(&self, source: &str, source_id: i64) -> Result<Option<i64>> {
        let endpoint = format!("/v1/source/{source}/{source_id}");
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.get_access_token().await).parse()?,
        );
        headers.insert(header::ACCEPT, "application/json".parse()?);

        let Ok(res) = self
            .client
            .request(Method::GET, &endpoint, Some(headers))
            .await
        else {
            return Ok(None);
        };

        if res.status() != reqwest::StatusCode::OK {
            return Ok(None);
        }

        if let Ok(data) = res.json::<serde_json::Value>().await
            && let Some(series_list) = data
                .get("data")
                .and_then(|d| d.get("series"))
                .and_then(serde_json::Value::as_array)
        {
            for series in series_list {
                if series.get("state").and_then(serde_json::Value::as_str) == Some("active") {
                    return Ok(series.get("id").and_then(serde_json::Value::as_i64));
                }
            }
            if !series_list.is_empty() {
                return Ok(series_list[0].get("id").and_then(serde_json::Value::as_i64));
            }
        }
        Ok(None)
    }
}

#[async_trait]
impl TrackerClient for MangaBakaClient {
    /// Gets the viewer's name from `MangaBaka`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the profile data cannot be parsed.
    async fn get_viewer_name(&self) -> Result<String> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.get_access_token().await).parse()?,
        );
        headers.insert(header::ACCEPT, "application/json".parse()?);

        let res = self
            .client
            .request(Method::GET, "/v1/my/profile", Some(headers))
            .await?;
        let res_json: serde_json::Value = res.json().await?;
        let data = res_json.get("data").unwrap_or(&serde_json::Value::Null);

        if let Some(name) = data
            .get("preferred_username")
            .and_then(serde_json::Value::as_str)
        {
            return Ok(name.to_string());
        }
        if let Some(name) = data.get("nickname").and_then(serde_json::Value::as_str) {
            return Ok(name.to_string());
        }
        if let Some(id) = data.get("id").and_then(serde_json::Value::as_str) {
            return Ok(id.to_string());
        }

        Err(eyre!(
            "Could not extract viewer name from `MangaBaka` profile"
        ))
    }

    fn supported_ids(&self) -> Vec<&'static str> {
        vec!["mal_id", "ani_id", "kitsu_id"]
    }
    fn supports_anime(&self) -> bool {
        false
    }
    fn supports_manga(&self) -> bool {
        true
    }

    fn get_round_trip_score(&self, internal_score: i32) -> i32 {
        internal_score // `MangaBaka` is 1:1
    }

    async fn fetch_anime_list(&self, _user_name: &str) -> Result<Vec<TrackerEntry>> {
        Ok(vec![])
    }

    /// Fetches the user's manga library from `MangaBaka`.
    ///
    /// # Errors
    ///
    /// Returns an error if any request fails or the response cannot be parsed.
    ///
    /// # Panics
    ///
    /// Panics if the pagination URL is malformed.
    async fn fetch_manga_list(&self, _user_id: &str) -> Result<Vec<TrackerEntry>> {
        let mut all_entries = Vec::new();
        let mut next_url = Some("/v1/my/library?limit=100".to_string());

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.get_access_token().await).parse()?,
        );
        headers.insert(header::ACCEPT, "application/json".parse()?);

        while let Some(url) = next_url {
            let res = self
                .client
                .request(Method::GET, &url, Some(headers.clone()))
                .await?;
            let data: MangaBakaLibraryResponse = res.json().await?;

            for item in data.data {
                let series = item.series;
                let Some(series_id) = series.id else { continue };
                let source = series.source.unwrap_or(MangaBakaSource {
                    my_anime_list: None,
                    anilist: None,
                    kitsu: None,
                });

                let started_at = Self::parse_date(item.start_date.as_ref());
                let completed_at = Self::parse_date(item.finish_date.as_ref());

                all_entries.push(TrackerEntry {
                    id: series_id,
                    mal_id: source.my_anime_list.and_then(|m| m.id),
                    ani_id: source.anilist.and_then(|a| a.id),
                    kitsu_id: source.kitsu.and_then(|k| k.id),
                    title: series
                        .title
                        .or(series.romanized_title)
                        .unwrap_or_else(|| "Unknown".to_string()),
                    media_type: MediaType::Manga,
                    status: Self::map_status(item.state.as_deref().unwrap_or("reading")),
                    score: item.rating.unwrap_or(0),
                    progress: item.progress_chapter.unwrap_or(0),
                    volumes: item.progress_volume.unwrap_or(0),
                    max_progress: series.total_chapters.unwrap_or(0),
                    max_volumes: series.final_volume.unwrap_or(0),
                    started_at,
                    completed_at,
                    repeat: 0, // `MangaBaka` doesn't seem to expose repeat count in library
                    notes: item.note.unwrap_or_default(),
                });
            }

            next_url = data.pagination.and_then(|p| p.next).and_then(|n| {
                if let Ok(parsed) = url::Url::parse(&n) {
                    let query = parsed.query().map(|q| format!("?{q}")).unwrap_or_default();
                    Some(format!("{}{}", parsed.path(), query))
                } else {
                    None
                }
            });
        }

        Ok(all_entries)
    }

    /// Updates or adds a manga entry in the user's `MangaBaka` library.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response cannot be parsed.
    async fn update_entry(
        &self,
        entry_id: i64,
        media_type: MediaType,
        options: UpdateOptions,
    ) -> Result<bool> {
        if media_type != MediaType::Manga {
            return Ok(false);
        }

        let mut data = HashMap::new();

        if let Some(s) = options.status {
            data.insert(
                "state".to_string(),
                serde_json::json!(Self::reverse_map_status(s)),
            );
        } else if options.is_add {
            data.insert("state".to_string(), serde_json::json!("reading"));
        }

        if let Some(p) = options.progress {
            data.insert("progress_chapter".to_string(), serde_json::json!(p));
        }

        if let Some(v) = options.volumes {
            data.insert("progress_volume".to_string(), serde_json::json!(v));
        }

        if let Some(s) = options.score {
            data.insert("rating".to_string(), serde_json::json!(s));
        }

        if let Some(ref n) = options.notes {
            data.insert("note".to_string(), serde_json::json!(n));
        }

        let map_date = |d: &Option<HashMap<String, Option<i64>>>| -> serde_json::Value {
            if let Some(date_str) = UpdateOptions::format_date(d) {
                serde_json::json!(date_str)
            } else {
                serde_json::Value::Null
            }
        };

        if options.started_at.is_some() {
            data.insert("start_date".to_string(), map_date(&options.started_at));
        }

        if options.completed_at.is_some() {
            data.insert("finish_date".to_string(), map_date(&options.completed_at));
        }

        if data.is_empty() {
            return Ok(true);
        }

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.get_access_token().await).parse()?,
        );
        headers.insert(header::ACCEPT, "application/json".parse()?);
        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);

        let endpoint = format!("/v1/my/library/{entry_id}");
        let method = if options.is_add {
            Method::POST
        } else {
            Method::PUT
        };

        let res = self
            .client
            .request_with_json(method, &endpoint, Some(headers), &data)
            .await?;
        Ok(res.status().is_success())
    }

    async fn get_media_id_by_mal_id(
        &self,
        mal_id: i64,
        media_type: MediaType,
    ) -> Result<Option<i64>> {
        if media_type != MediaType::Manga {
            return Ok(None);
        }
        self.get_media_id_by_source("my-anime-list", mal_id).await
    }

    async fn get_media_id_by_ani_id(
        &self,
        ani_id: i64,
        media_type: MediaType,
    ) -> Result<Option<i64>> {
        if media_type != MediaType::Manga {
            return Ok(None);
        }
        self.get_media_id_by_source("anilist", ani_id).await
    }

    async fn get_media_id_by_kitsu_id(
        &self,
        kitsu_id: i64,
        media_type: MediaType,
    ) -> Result<Option<i64>> {
        if media_type != MediaType::Manga {
            return Ok(None);
        }
        self.get_media_id_by_source("kitsu", kitsu_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_mangabaka_client_init() {
        let client = MangaBakaClient::new("dummy_token").unwrap();
        // Since access_token is Arc<RwLock<String>>, we need to read it
        assert_eq!(*client.access_token.read().await, "dummy_token");
        assert_eq!(client.client.rate_limit_calls, 30);
    }

    #[test]
    fn test_map_mangabaka_status() {
        assert_eq!(MangaBakaClient::map_status("reading"), SyncStatus::Current);
        assert_eq!(
            MangaBakaClient::map_status("completed"),
            SyncStatus::Completed
        );
        assert_eq!(MangaBakaClient::map_status("paused"), SyncStatus::Paused);
        assert_eq!(MangaBakaClient::map_status("dropped"), SyncStatus::Dropped);
        assert_eq!(
            MangaBakaClient::map_status("plan_to_read"),
            SyncStatus::Planning
        );
    }

    #[test]
    fn test_parse_date() {
        let date_str = Some("2023-04-14".to_string());
        let parsed = MangaBakaClient::parse_date(date_str.as_ref()).unwrap();
        assert_eq!(parsed.get("year").unwrap(), &Some(2023));
        assert_eq!(parsed.get("month").unwrap(), &Some(4));
        assert_eq!(parsed.get("day").unwrap(), &Some(14));
    }

    #[test]
    fn test_mangabaka_string_int_parsing() {
        let json_data = r#"{
            "data": [
                {
                    "state": "reading",
                    "rating": "8",
                    "progress_chapter": "13",
                    "progress_volume": "1",
                    "start_date": "2023-04-14T00:00:00Z",
                    "finish_date": null,
                    "note": "test note",
                    "Series": {
                        "id": 1234,
                        "title": "Test Manga",
                        "romanized_title": "Test Manga Romaji",
                        "total_chapters": "100",
                        "final_volume": "10",
                        "source": {
                            "my_anime_list": { "id": 5678 }
                        }
                    }
                }
            ],
            "pagination": {
                "next": null
            }
        }"#;

        let parsed: Result<MangaBakaLibraryResponse, _> = serde_json::from_str(json_data);
        assert!(
            parsed.is_ok(),
            "Failed to parse MangaBakaLibraryResponse with string integers: {:?}",
            parsed.err()
        );

        let item = &parsed.unwrap().data[0];
        assert_eq!(item.progress_chapter, Some(13));
        assert_eq!(item.rating, Some(8));
        assert_eq!(item.series.total_chapters, Some(100));
    }

    #[tokio::test]
    async fn test_mangabaka_round_trip() {
        let client = MangaBakaClient::new("dummy").unwrap();
        assert_eq!(client.get_round_trip_score(85), 85);
    }

    #[test]
    fn test_mangabaka_oauth_state() {
        let oauth = MangaBakaOAuth::new();
        let auth_url = oauth.get_auth_url();

        let parsed_url = url::Url::parse(&auth_url).unwrap();
        let mut state_param = None;
        for (key, value) in parsed_url.query_pairs() {
            if key == "state" {
                state_param = Some(value.into_owned());
                break;
            }
        }

        let state = state_param.expect("auth_url must contain a 'state' parameter");
        assert!(
            oauth.verify_state(&state),
            "verify_state should return true for the generated state"
        );
        assert!(
            !oauth.verify_state("invalid_state"),
            "verify_state should return false for an invalid state"
        );
    }
}
