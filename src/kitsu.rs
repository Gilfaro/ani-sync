// Rust guideline compliant 2026-02-21

use crate::client::{BaseClient, TokenRefresher};
use crate::models::{MediaType, SyncStatus, TrackerClient, TrackerEntry, UpdateOptions};
use async_trait::async_trait;
use color_eyre::{Result, eyre::eyre};
use reqwest::{Method, header};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{Level, event};

/// Base URL for the Kitsu GraphQL API.
pub const KITSU_GRAPHQL_URL: &str = "https://kitsu.app/api/graphql";
/// URL for obtaining OAuth 2.0 tokens from Kitsu.
pub const KITSU_OAUTH_TOKEN_URL: &str = "https://kitsu.app/api/oauth/token";
/// Client ID for the Ani-Sync application on Kitsu.
pub const KITSU_CLIENT_ID: &str =
    "dd031b32d2f56c990b1425efe6c42ad847e7fe3ab46bf1299f05ecd856bdb7dd";
/// Client secret for the Ani-Sync application on Kitsu.
pub const KITSU_CLIENT_SECRET: &str =
    "54d7307928f63414defd96399fc31ba847961ceaecef3a5fd93144e960c0e151";

#[derive(Debug, Serialize)]
struct GraphQLRequest<'a> {
    query: &'a str,
    variables: HashMap<&'a str, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GraphQLResponse {
    data: Option<serde_json::Value>,
    errors: Option<Vec<serde_json::Value>>,
}

/// A client for the Kitsu tracker.
pub struct KitsuClient {
    /// The underlying HTTP client.
    pub client: Arc<BaseClient>,
    access_token: Arc<RwLock<String>>,
}

/// A token refresher for Kitsu.
pub struct KitsuTokenRefresher;

#[async_trait]
impl TokenRefresher for KitsuTokenRefresher {
    /// Refreshes the Kitsu access token using a refresh token.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No token bundle is found in storage.
    /// - No refresh token is available in the bundle.
    /// - The refresh request fails.
    async fn refresh(&self) -> Result<String> {
        let bundle = crate::storage::get_token_bundle("kitsu")?
            .ok_or_else(|| eyre!("No token bundle found for Kitsu"))?;

        let refresh_token = bundle
            .refresh_token
            .as_deref()
            .ok_or_else(|| eyre!("No refresh token available in bundle"))?;

        let client = crate::client::create_reqwest_client()?;
        let res = client
            .post(KITSU_OAUTH_TOKEN_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", KITSU_CLIENT_ID),
                ("client_secret", KITSU_CLIENT_SECRET),
            ])
            .send()
            .await?;

        if res.status().is_success() {
            let json: serde_json::Value = res.json().await?;
            if let Some(new_access_token) = json.get("access_token").and_then(|t| t.as_str()) {
                let new_bundle = crate::storage::TokenBundle {
                    access_token: new_access_token.to_string(),
                    refresh_token: json
                        .get("refresh_token")
                        .and_then(|t| t.as_str())
                        .map(ToString::to_string)
                        .or(bundle.refresh_token),
                    expires_at: json
                        .get("expires_in")
                        .and_then(serde_json::Value::as_i64)
                        .map(|expires_in| chrono::Utc::now().timestamp() + expires_in),
                };
                crate::storage::set_token_bundle("kitsu", &new_bundle)?;
                event!(
                    name: "kitsu.token.refresh_success",
                    Level::INFO,
                    "Successfully refreshed Kitsu token",
                );
                Ok(new_access_token.to_string())
            } else {
                Err(eyre!("Kitsu refresh response missing access_token"))
            }
        } else {
            let body = res.text().await?;
            Err(eyre!("Kitsu token refresh failed: {body}"))
        }
    }
}

/// A wrapper around `KitsuTokenRefresher` that also updates the client's local access token.
pub struct KitsuClientRefresher {
    inner: KitsuTokenRefresher,
    access_token: Arc<RwLock<String>>,
}

#[async_trait]
impl TokenRefresher for KitsuClientRefresher {
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

impl KitsuClient {
    /// Create a new Kitsu client.
    ///
    /// # Errors
    ///
    /// Returns an error if the base client fails to initialize.
    pub fn new(access_token: &str) -> Result<Self> {
        Self::with_base_url(KITSU_GRAPHQL_URL, access_token)
    }

    /// Creates a new `KitsuClient` with a custom base URL.
    ///
    /// # Errors
    ///
    /// Returns an error if the base client cannot be initialized.
    pub fn with_base_url(base_url: &str, access_token: &str) -> Result<Self> {
        let client = Arc::new(BaseClient::new(
            "kitsu",
            base_url,
            2,
            Duration::from_secs(1),
        )?);
        let access_token_arc = Arc::new(RwLock::new(access_token.to_string()));

        let refresher = Arc::new(KitsuClientRefresher {
            inner: KitsuTokenRefresher,
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

    async fn query<'a>(
        &self,
        query: &'a str,
        variables: HashMap<&'a str, serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.get_access_token().await).parse()?,
        );
        headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
        headers.insert(header::ACCEPT, "application/json".parse()?);

        let req_body = GraphQLRequest { query, variables };
        let res = self
            .client
            .request_with_json(Method::POST, "", Some(headers), &req_body)
            .await?;

        let status = res.status();
        let res_json: GraphQLResponse = res.json().await?;

        if let Some(errs) = res_json.errors {
            return Err(eyre!("Kitsu GraphQL Error (Status: {status}): {errs:?}",));
        }

        if let Some(ref data) = res_json.data
            && data
                .get("currentAccount")
                .is_some_and(serde_json::Value::is_null)
        {
            // Kitsu returns 200 OK with currentAccount: null when token is invalid
            event!(
                name: "kitsu.query.auth_error",
                Level::DEBUG,
                "Kitsu currentAccount is null, attempting token refresh",
            );
            if self.client.trigger_refresh().await.is_ok() {
                // Retry once with new token
                let mut new_headers = header::HeaderMap::new();
                new_headers.insert(
                    header::AUTHORIZATION,
                    format!("Bearer {}", self.get_access_token().await).parse()?,
                );
                new_headers.insert(header::CONTENT_TYPE, "application/json".parse()?);
                new_headers.insert(header::ACCEPT, "application/json".parse()?);

                let retry_res = self
                    .client
                    .request_with_json(Method::POST, "", Some(new_headers), &req_body)
                    .await?;
                let retry_json: GraphQLResponse = retry_res.json().await?;
                return retry_json.data.ok_or_else(|| {
                    eyre!("No data returned from Kitsu GraphQL API after refresh retry")
                });
            }
        }

        res_json
            .data
            .ok_or_else(|| eyre!("No data returned from Kitsu GraphQL API (Status: {status})"))
    }

    fn map_kitsu_status(status: &str) -> SyncStatus {
        const STATUS_CURRENT: &str = "CURRENT";
        const STATUS_COMPLETED: &str = "COMPLETED";
        const STATUS_ON_HOLD: &str = "ON_HOLD";
        const STATUS_DROPPED: &str = "DROPPED";

        match status {
            STATUS_CURRENT => SyncStatus::Current,
            STATUS_COMPLETED => SyncStatus::Completed,
            STATUS_ON_HOLD => SyncStatus::Paused,
            STATUS_DROPPED => SyncStatus::Dropped,
            _ => SyncStatus::Planning,
        }
    }

    fn parse_date(date_str: Option<&String>) -> Option<HashMap<String, Option<i64>>> {
        UpdateOptions::parse_date(date_str)
    }

    #[expect(clippy::too_many_lines)]
    fn parse_kitsu_node(node: &serde_json::Value, media_kind: &str) -> Result<TrackerEntry> {
        let media = node
            .get("media")
            .ok_or_else(|| eyre!("Library entry has no associated media"))?;

        let mut mal_id = None;
        let mut ani_id = None;

        if let Some(mappings) = media.get("mappings").and_then(|m| m.get("nodes"))
            && let Some(nodes) = mappings.as_array()
        {
            for m in nodes {
                let site = m
                    .get("externalSite")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_lowercase();
                let ext_id = m.get("externalId").and_then(serde_json::Value::as_str);

                if site.contains("myanimelist") {
                    mal_id = ext_id.and_then(|id| id.parse().ok());
                } else if site.contains("anilist") {
                    ani_id = ext_id.and_then(|id| id.parse().ok());
                }
            }
        }

        let title = media
            .get("titles")
            .and_then(|t| {
                t.get("canonical")
                    .or_else(|| t.get("en"))
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or("Unknown")
            .to_string();

        let kitsu_id = media
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| id.parse().ok())
            .unwrap_or(0);

        let entry_id = node
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| id.parse().ok())
            .unwrap_or(0);

        let raw_score = node.get("rating").and_then(serde_json::Value::as_i64);
        #[expect(clippy::cast_possible_truncation)]
        let score = raw_score.map_or(0, |s| (s * 5) as i32);

        let started_at_str = node
            .get("startedAt")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        let started_at = Self::parse_date(started_at_str.as_ref());
        let completed_at_str = node
            .get("finishedAt")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        let completed_at = Self::parse_date(completed_at_str.as_ref());

        #[expect(clippy::cast_possible_truncation)]
        let progress = node
            .get("progress")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32;
        #[expect(clippy::cast_possible_truncation)]
        let volumes = node
            .get("volumesOwned")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32;
        #[expect(clippy::cast_possible_truncation)]
        let max_progress = media
            .get("episodeCount")
            .or_else(|| media.get("chapterCount"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32;
        #[expect(clippy::cast_possible_truncation)]
        let max_volumes = media
            .get("volumeCount")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32;
        #[expect(clippy::cast_possible_truncation)]
        let repeat = node
            .get("reconsumeCount")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32;

        Ok(TrackerEntry {
            id: entry_id,
            mal_id,
            ani_id,
            kitsu_id: Some(kitsu_id),
            title,
            media_type: if media_kind == "ANIME" {
                MediaType::Anime
            } else {
                MediaType::Manga
            },
            status: Self::map_kitsu_status(
                node.get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            ),
            score,
            progress,
            volumes,
            max_progress,
            max_volumes,
            started_at,
            completed_at,
            repeat,
            notes: node
                .get("notes")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
    }
}

#[async_trait]
impl TrackerClient for KitsuClient {
    /// Gets the current viewer's name from Kitsu.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the profile data is missing.
    async fn get_viewer_name(&self) -> Result<String> {
        let query = r"
        query {
          currentAccount {
            profile {
              id
              slug
              name
            }
          }
        }
        ";

        let variables = HashMap::new();
        let data = self.query(query, variables).await?;

        if let Some(account) = data.get("currentAccount")
            && !account.is_null()
            && let Some(profile) = account.get("profile")
            && let Some(name_val) = profile.get("name")
            && let Some(name_str) = name_val.as_str()
        {
            return Ok(name_str.to_string());
        }

        Err(eyre!(
            "Could not extract viewer name from Kitsu GraphQL response: {:?}",
            data
        ))
    }

    /// Gets the current viewer's unique ID from Kitsu.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the profile data is missing.
    async fn get_viewer_id(&self) -> Result<String> {
        let query = r"
        query {
          currentAccount {
            profile {
              id
            }
          }
        }
        ";

        let variables = HashMap::new();
        let data = self.query(query, variables).await?;

        if let Some(account) = data.get("currentAccount")
            && !account.is_null()
            && let Some(profile) = account.get("profile")
            && let Some(id_val) = profile.get("id")
            && let Some(id_str) = id_val.as_str()
        {
            return Ok(id_str.to_string());
        }

        Err(eyre!(
            "Could not extract viewer ID from Kitsu GraphQL response"
        ))
    }

    fn supported_ids(&self) -> Vec<&'static str> {
        vec!["mal_id", "ani_id", "kitsu_id"]
    }
    fn supports_anime(&self) -> bool {
        true
    }
    fn supports_manga(&self) -> bool {
        true
    }

    fn get_round_trip_score(&self, internal_score: i32) -> i32 {
        let mut score_val = if internal_score == 0 {
            0
        } else {
            #[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let s = (internal_score as f32 / 5.0).round() as i32;
            s
        };
        if score_val == 0 && internal_score > 0 {
            score_val = 1;
        }
        score_val * 5
    }

    async fn fetch_anime_list(&self, user_id: &str) -> Result<Vec<TrackerEntry>> {
        self.fetch_list(user_id, "ANIME").await
    }

    async fn fetch_manga_list(&self, user_id: &str) -> Result<Vec<TrackerEntry>> {
        self.fetch_list(user_id, "MANGA").await
    }

    #[expect(clippy::too_many_lines)]
    async fn update_entry(
        &self,
        entry_id: i64,
        media_type: MediaType,
        options: UpdateOptions,
    ) -> Result<bool> {
        const STATUS_CURRENT: &str = "CURRENT";
        const STATUS_COMPLETED: &str = "COMPLETED";
        const STATUS_ON_HOLD: &str = "ON_HOLD";
        const STATUS_DROPPED: &str = "DROPPED";
        const STATUS_PLANNED: &str = "PLANNED";

        let mut variables = HashMap::new();

        let mutation_name = if options.is_add { "create" } else { "update" };
        let input_type = if options.is_add {
            "LibraryEntryCreateInput"
        } else {
            "LibraryEntryUpdateInput"
        };

        let mut args_defs: Vec<String> = Vec::new();
        args_defs.push(format!("$input: {input_type}!"));

        if options.is_add {
            variables.insert("mediaId", serde_json::json!(entry_id.to_string()));
            variables.insert(
                "mediaType",
                serde_json::json!(if media_type == MediaType::Anime {
                    "ANIME"
                } else {
                    "MANGA"
                }),
            );
        } else {
            variables.insert("id", serde_json::json!(entry_id.to_string()));
        }

        if let Some(s) = options.status {
            let kitsu_status = match s {
                SyncStatus::Current => STATUS_CURRENT,
                SyncStatus::Completed => STATUS_COMPLETED,
                SyncStatus::Paused => STATUS_ON_HOLD,
                SyncStatus::Dropped => STATUS_DROPPED,
                SyncStatus::Planning => STATUS_PLANNED,
            };
            variables.insert("status", serde_json::json!(kitsu_status));
        } else if options.is_add {
            variables.insert("status", serde_json::json!(STATUS_PLANNED));
        }

        let mut current_progress = options.progress;
        let mut current_volumes = options.volumes;

        if (current_progress.is_some() || current_volumes.is_some())
            && let Ok((max_p, max_v)) = self
                .get_max_progress_and_volumes(entry_id, media_type, options.is_add)
                .await
        {
            if let Some(p) = current_progress
                && let Some(max_val) = max_p
                && max_val > 0
                && p > max_val
            {
                event!(
                    name: "kitsu.update.cap_progress",
                    Level::DEBUG,
                    entry_id = entry_id,
                    progress = p,
                    max = max_val,
                    "Capping progress for entry {} from {} to {}",
                    entry_id,
                    p,
                    max_val
                );
                current_progress = Some(max_val);
            }

            if let Some(v) = current_volumes
                && let Some(max_val) = max_v
                && max_val > 0
                && v > max_val
            {
                event!(
                    name: "kitsu.update.cap_volumes",
                    Level::DEBUG,
                    entry_id = entry_id,
                    volumes = v,
                    max = max_val,
                    "Capping volumes for entry {} from {} to {}",
                    entry_id,
                    v,
                    max_val
                );
                current_volumes = Some(max_val);
            }
        }

        if let Some(p) = current_progress {
            variables.insert("progress", serde_json::json!(p));
        }

        if let Some(v) = current_volumes {
            variables.insert("volumesOwned", serde_json::json!(v));
        }

        if let Some(s) = options.score {
            let mut score_val = if s == 0 {
                0
            } else {
                #[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                let val = (s as f32 / 5.0).round() as i32;
                val
            };
            if score_val == 0 && s > 0 {
                score_val = 1;
            }
            if score_val > 0 {
                variables.insert("rating", serde_json::json!(score_val));
            } else {
                variables.insert("rating", serde_json::Value::Null);
            }
        }

        if let Some(r) = options.repeat {
            variables.insert("reconsumeCount", serde_json::json!(r));
        }

        if let Some(n) = options.notes {
            variables.insert("notes", serde_json::json!(n));
        }

        let map_date = |d: Option<HashMap<String, Option<i64>>>| -> Option<serde_json::Value> {
            d.map(|date_map| {
                if let Some(date_str) = UpdateOptions::format_date(&Some(date_map)) {
                    serde_json::json!(date_str)
                } else {
                    serde_json::Value::Null
                }
            })
        };

        if let Some(sd) = map_date(options.started_at) {
            variables.insert("startedAt", sd);
        }

        if let Some(cd) = map_date(options.completed_at) {
            variables.insert("finishedAt", cd);
        }

        let mut input_vars = HashMap::new();
        input_vars.insert("input", serde_json::json!(variables));

        let args_str = args_defs.join(", ");

        let query = if options.is_add {
            format!(
                r"
                mutation SyncLibraryEntry({args_str}) {{
                  libraryEntry {{
                    create(input: $input) {{
                      libraryEntry {{ id }}
                      errors {{ message path }}
                    }}
                  }}
                }}
                "
            )
        } else {
            format!(
                r"
                mutation SyncLibraryEntry({args_str}) {{
                  libraryEntry {{
                    update(input: $input) {{
                      libraryEntry {{ id }}
                      errors {{ message path }}
                    }}
                  }}
                }}
                "
            )
        };

        event!(
            name: "kitsu.update.request",
            Level::DEBUG,
            query = query,
            variables = ?input_vars,
            "Sending Kitsu GraphQL update request",
        );

        let data = self.query(&query, input_vars).await?;
        let payload = data.get("libraryEntry").and_then(|l| l.get(mutation_name));

        if let Some(p) = payload {
            if let Some(errors) = p.get("errors").and_then(|e| e.as_array())
                && !errors.is_empty()
            {
                event!(
                    name: "kitsu.update.errors",
                    Level::WARN,
                    mutation = mutation_name,
                    errors = ?errors,
                    "Kitsu {} errors: {:?}",
                    mutation_name,
                    errors
                );
                return Ok(false);
            }
            Ok(p.get("libraryEntry").is_some())
        } else {
            Ok(false)
        }
    }

    async fn get_media_id_by_mal_id(
        &self,
        mal_id: i64,
        media_type: MediaType,
    ) -> Result<Option<i64>> {
        let site = format!(
            "MYANIMELIST_{}",
            if media_type == MediaType::Anime {
                "ANIME"
            } else {
                "MANGA"
            }
        );
        self.get_media_id_by_external_id(&site, &mal_id.to_string())
            .await
    }

    async fn get_media_id_by_ani_id(
        &self,
        ani_id: i64,
        media_type: MediaType,
    ) -> Result<Option<i64>> {
        let site = format!(
            "ANILIST_{}",
            if media_type == MediaType::Anime {
                "ANIME"
            } else {
                "MANGA"
            }
        );
        self.get_media_id_by_external_id(&site, &ani_id.to_string())
            .await
    }

    async fn get_media_id_by_kitsu_id(
        &self,
        kitsu_id: i64,
        _media_type: MediaType,
    ) -> Result<Option<i64>> {
        Ok(Some(kitsu_id))
    }
}

impl KitsuClient {
    async fn get_max_progress_and_volumes(
        &self,
        id: i64,
        media_type: MediaType,
        is_add: bool,
    ) -> Result<(Option<i32>, Option<i32>)> {
        let query = if is_add {
            r"
            query GetMediaMaxProgress($id: ID!) {
              findAnimeById(id: $id) { episodeCount }
              findMangaById(id: $id) { chapterCount volumeCount }
            }
            "
        } else {
            r"
            query GetLibraryEntryMaxProgress($id: ID!) {
              findLibraryEntryById(id: $id) {
                media {
                  __typename
                  ... on Anime { episodeCount }
                  ... on Manga { chapterCount volumeCount }
                }
              }
            }
            "
        };

        let mut variables = HashMap::new();
        variables.insert("id", serde_json::json!(id.to_string()));

        let data = self.query(query, variables).await?;

        let (count_val, vol_val) = if is_add {
            if media_type == MediaType::Anime {
                (
                    data.get("findAnimeById")
                        .and_then(|node| node.get("episodeCount")),
                    None,
                )
            } else {
                (
                    data.get("findMangaById")
                        .and_then(|node| node.get("chapterCount")),
                    data.get("findMangaById")
                        .and_then(|node| node.get("volumeCount")),
                )
            }
        } else {
            let media = data
                .get("findLibraryEntryById")
                .and_then(|node| node.get("media"));
            (
                media.and_then(|m| m.get("episodeCount").or_else(|| m.get("chapterCount"))),
                media.and_then(|m| m.get("volumeCount")),
            )
        };

        #[expect(clippy::cast_possible_truncation)]
        let max_progress = count_val
            .and_then(serde_json::Value::as_i64)
            .map(|c| c as i32);
        #[expect(clippy::cast_possible_truncation)]
        let max_volumes = vol_val
            .and_then(serde_json::Value::as_i64)
            .map(|c| c as i32);

        Ok((max_progress, max_volumes))
    }

    async fn get_media_id_by_external_id(
        &self,
        site: &str,
        external_id: &str,
    ) -> Result<Option<i64>> {
        let query = r"
        query GetMediaByMapping($site: MappingExternalSiteEnum!, $id: ID!) {
          lookupMapping(externalSite: $site, externalId: $id) {
            __typename
            ... on Anime { id }
            ... on Manga { id }
          }
        }
        ";

        let mut variables = HashMap::new();
        variables.insert("site", serde_json::json!(site));
        variables.insert("id", serde_json::json!(external_id));

        let data = self.query(query, variables).await?;

        if let Some(mapping) = data.get("lookupMapping")
            && let Some(id_str) = mapping.get("id").and_then(|id| id.as_str())
        {
            return Ok(id_str.parse().ok());
        }
        Ok(None)
    }

    async fn fetch_list(&self, user_id: &str, media_kind: &str) -> Result<Vec<TrackerEntry>> {
        let query = r"
        query GetUserLibrary($id: ID!, $mediaType: MediaTypeEnum!, $after: String) {
          findProfileById(id: $id) {
            library {
              all(first: 1000, mediaType: $mediaType, after: $after) {
                nodes {
                  id status progress rating startedAt finishedAt reconsumeCount notes volumesOwned
                  media {
                    __typename
                    ... on Anime {
                      id titles { canonical } episodeCount
                      mappings(first: 20) { nodes { externalSite externalId } }
                    }
                    ... on Manga {
                      id titles { canonical } chapterCount volumeCount
                      mappings(first: 20) { nodes { externalSite externalId } }
                    }
                  }
                }
                pageInfo { hasNextPage endCursor }
              }
            }
          }
        }
        ";

        let mut all_entries = Vec::new();
        let mut after = serde_json::Value::Null;
        let mut has_next = true;

        while has_next {
            let mut variables = HashMap::new();
            variables.insert("id", serde_json::json!(user_id));
            variables.insert("mediaType", serde_json::json!(media_kind));
            variables.insert("after", after.clone());

            let data = self.query(query, variables).await?;
            let profile = data.get("findProfileById");
            if profile.is_none() {
                break;
            }

            let library_all = profile.unwrap().get("library").and_then(|l| l.get("all"));
            if library_all.is_none() {
                break;
            }

            let library_all = library_all.unwrap();
            if let Some(nodes) = library_all.get("nodes").and_then(|n| n.as_array()) {
                for node in nodes {
                    match Self::parse_kitsu_node(node, media_kind) {
                        Ok(entry) => all_entries.push(entry),
                        Err(e) => {
                            event!(
                                name: "kitsu.fetch_list.parse_error",
                                Level::WARN,
                                error = %e,
                                node = ?node,
                                "Failed to parse Kitsu node: {} - Node data: {:?}",
                                e,
                                node
                            );
                        }
                    }
                }
            }

            let page_info = library_all.get("pageInfo");
            has_next = page_info
                .and_then(|p| p.get("hasNextPage"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            after = page_info
                .and_then(|p| p.get("endCursor"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
        }

        Ok(all_entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MediaType, SyncStatus, TrackerClient, UpdateOptions};
    use mockito::Server;

    #[tokio::test]
    async fn test_kitsu_client_init() {
        let client = KitsuClient::new("dummy_token").unwrap();
        assert_eq!(*client.access_token.read().await, "dummy_token");
        assert_eq!(client.client.rate_limit_calls, 2);
    }

    #[tokio::test]
    async fn test_kitsu_round_trip() {
        let client = KitsuClient::new("dummy").unwrap();
        assert_eq!(client.get_round_trip_score(85), 85);
        assert_eq!(client.get_round_trip_score(82), 80);
        assert_eq!(client.get_round_trip_score(83), 85);
        assert_eq!(client.get_round_trip_score(3), 5); // Min 1 rule
        assert_eq!(client.get_round_trip_score(0), 0);
    }

    #[test]
    fn test_map_kitsu_status() {
        assert_eq!(
            KitsuClient::map_kitsu_status("CURRENT"),
            SyncStatus::Current
        );
        assert_eq!(
            KitsuClient::map_kitsu_status("COMPLETED"),
            SyncStatus::Completed
        );
        assert_eq!(KitsuClient::map_kitsu_status("ON_HOLD"), SyncStatus::Paused);
        assert_eq!(
            KitsuClient::map_kitsu_status("DROPPED"),
            SyncStatus::Dropped
        );
        assert_eq!(
            KitsuClient::map_kitsu_status("PLANNED"),
            SyncStatus::Planning
        );
    }

    #[test]
    fn test_parse_date() {
        let date_str = Some("2023-04-14".to_string());
        let parsed = KitsuClient::parse_date(date_str.as_ref()).unwrap();
        assert_eq!(parsed.get("year").unwrap(), &Some(2023));
        assert_eq!(parsed.get("month").unwrap(), &Some(4));
        assert_eq!(parsed.get("day").unwrap(), &Some(14));
    }

    #[tokio::test]
    async fn test_get_viewer_name_graphql() {
        let mut server = Server::new_async().await;
        let client = KitsuClient::with_base_url(&server.url(), "dummy").unwrap();

        let _m = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"query":"\n        query {\n          currentAccount {\n            profile {\n              id\n              slug\n              name\n            }\n          }\n        }\n        ","variables":{}}"#.to_string(),
            ))
            .match_header("authorization", "Bearer dummy")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{
                "data": {
                    "currentAccount": {
                        "profile": {
                            "id": "123",
                            "slug": "testuser",
                            "name": "Test User"
                        }
                    }
                }
            }"#)
            .create_async()
            .await;

        let name = client.get_viewer_name().await.unwrap();
        assert_eq!(name, "Test User");
    }

    #[tokio::test]
    async fn test_kitsu_update_entry_exceeds_max_progress() {
        let mut server = Server::new_async().await;
        let client = KitsuClient::with_base_url(&server.url(), "token").unwrap();

        // 1. Mock max progress check
        let _m1 = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": {
                    "findLibraryEntryById": {
                        "media": {
                            "__typename": "Anime",
                            "episodeCount": 12
                        }
                    }
                }
            }"#,
            )
            .create_async()
            .await;

        // 2. Mock update mutation (progress should be capped at 12)
        let _m2 = server
            .mock("POST", "/")
            .match_body(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "data": {
                    "libraryEntry": {
                        "update": {
                            "libraryEntry": { "id": "100" },
                            "errors": []
                        }
                    }
                }
            }"#,
            )
            .create_async()
            .await;

        let options = UpdateOptions {
            progress: Some(24), // Exceeds max 12
            is_add: false,
            ..Default::default()
        };

        let result = TrackerClient::update_entry(&client, 100, MediaType::Anime, options)
            .await
            .unwrap();

        assert!(result);
    }
}
