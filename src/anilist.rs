// Rust guideline compliant 2026-02-21

use crate::client::{BaseClient, OAuthProvider};
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

const ANILIST_CLIENT_ID: &str = "38728";
const ANILIST_BASE_URL: &str = "https://graphql.anilist.co";
const ANILIST_OAUTH_AUTHORIZE_URL: &str = "https://anilist.co/api/v2/oauth/authorize";

const ANILIST_COLLECTION_QUERY: &str = r"
query ($userName: String, $type: MediaType) {
  MediaListCollection(userName: $userName, type: $type) {
    lists {
      name
      isCustomList
      status
      entries {
        id
        status
        score(format: POINT_100)
        progress
        progressVolumes
        updatedAt
        startedAt { year month day }
        completedAt { year month day }
        repeat
        notes
        private
        media {
          id
          idMal
          type
          episodes
          chapters
          volumes
          title {
            romaji
            english
            native
          }
          coverImage {
            medium
            large
          }
        }
      }
    }
  }
}
";

const ANILIST_UPDATE_MUTATION: &str = r"
mutation (
  $mediaId: Int,
  $status: MediaListStatus,
  $scoreRaw: Int,
  $progress: Int,
  $progressVolumes: Int,
  $startedAt: FuzzyDateInput,
  $completedAt: FuzzyDateInput,
  $repeat: Int,
  $notes: String
) {
  SaveMediaListEntry(
    mediaId: $mediaId,
    status: $status,
    scoreRaw: $scoreRaw,
    progress: $progress,
    progressVolumes: $progressVolumes,
    startedAt: $startedAt,
    completedAt: $completedAt,
    repeat: $repeat,
    notes: $notes
  ) {
    id
    mediaId
    status
    score
    progress
  }
}
";

/// Represents a media title in various languages/formats on `AniList`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AniListTitle {
    /// Title in romaji.
    pub romaji: Option<String>,
    /// Title in English.
    pub english: Option<String>,
    /// Native language title.
    pub native: Option<String>,
}

/// Cover image URLs for a media entry on `AniList`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListCoverImage {
    /// Medium size cover image URL.
    pub medium: Option<String>,
    /// Large size cover image URL.
    pub large: Option<String>,
}

/// Metadata for a media entry on `AniList`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListMedia {
    /// Internal `AniList` ID.
    pub id: i64,
    /// `MyAnimeList` ID mapping.
    pub id_mal: Option<i64>,
    /// Media type (ANIME or MANGA).
    pub r#type: Option<String>,
    /// Titles for the media.
    pub title: AniListTitle,
    /// Cover images for the media.
    pub cover_image: Option<AniListCoverImage>,
    /// Total episodes (for anime).
    pub episodes: Option<i32>,
    /// Total chapters (for manga).
    pub chapters: Option<i32>,
    /// Total volumes (for manga).
    pub volumes: Option<i32>,
}

/// A fuzzy date representation used by the `AniList` API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AniListFuzzyDate {
    /// The year.
    pub year: Option<i32>,
    /// The month.
    pub month: Option<i32>,
    /// The day.
    pub day: Option<i32>,
}

/// An individual entry in a user's media list on `AniList`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AniListEntry {
    /// The entry's unique ID.
    pub id: i64,
    /// The associated media metadata.
    pub media: AniListMedia,
    /// The user's status for this entry.
    pub status: String,
    /// The user's score.
    pub score: i32,
    /// Current episode/chapter progress.
    pub progress: i32,
    /// Current volume progress (for manga).
    pub progress_volumes: Option<i32>,
    /// Last update timestamp.
    pub updated_at: Option<i64>,
    /// When the user started the media.
    pub started_at: Option<AniListFuzzyDate>,
    /// When the user completed the media.
    pub completed_at: Option<AniListFuzzyDate>,
    /// Rewatch/reread count.
    pub repeat: Option<i32>,
    /// Personal notes.
    pub notes: Option<String>,
    /// Whether the entry is private.
    pub private: Option<bool>,
}

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

/// A client for the `AniList` GraphQL API.
pub struct AniListClient {
    /// The underlying HTTP client.
    pub client: Arc<BaseClient>,
    access_token: Arc<RwLock<String>>,
}

impl AniListClient {
    /// Create a new `AniList` client.
    ///
    /// # Errors
    ///
    /// Returns an error if the base client fails to initialize.
    pub fn new(access_token: &str) -> Result<Self> {
        let client = Arc::new(BaseClient::new(
            "anilist",
            ANILIST_BASE_URL,
            90,
            Duration::from_mins(1),
        )?);
        Ok(Self {
            client,
            access_token: Arc::new(RwLock::new(access_token.to_string())),
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
        let body_text = res.text().await?;
        let res_json_result: std::result::Result<GraphQLResponse, _> =
            serde_json::from_str(&body_text);

        match res_json_result {
            Ok(res_json) => {
                if let Some(errs) = res_json.errors
                    && !errs.is_empty()
                {
                    let err_msgs: Vec<String> = errs
                        .iter()
                        .filter_map(|e| {
                            e.get("message")
                                .and_then(|m| m.as_str())
                                .map(ToString::to_string)
                        })
                        .collect();
                    let joined = if err_msgs.is_empty() {
                        format!("{errs:?}")
                    } else {
                        err_msgs.join(", ")
                    };
                    if !status.is_success() {
                        return Err(eyre!("GraphQL Error ({}): {joined}", status.as_u16()));
                    }
                    return Err(eyre!("GraphQL Error: {joined}"));
                }

                if !status.is_success() {
                    return Err(eyre!("HTTP Error {status}: {body_text}"));
                }

                res_json
                    .data
                    .ok_or_else(|| eyre!("No data field in response"))
            }
            Err(e) => {
                if status.is_success() {
                    Err(eyre!("Failed to parse response: {}", e))
                } else {
                    Err(eyre!("HTTP Error {status}: {body_text}"))
                }
            }
        }
    }

    fn map_anilist_status(status: &str) -> SyncStatus {
        const STATUS_CURRENT: &str = "CURRENT";
        const STATUS_REPEATING: &str = "REPEATING";
        const STATUS_COMPLETED: &str = "COMPLETED";
        const STATUS_PAUSED: &str = "PAUSED";
        const STATUS_DROPPED: &str = "DROPPED";

        match status {
            STATUS_CURRENT | STATUS_REPEATING => SyncStatus::Current,
            STATUS_COMPLETED => SyncStatus::Completed,
            STATUS_PAUSED => SyncStatus::Paused,
            STATUS_DROPPED => SyncStatus::Dropped,
            _ => SyncStatus::Planning,
        }
    }

    fn parse_anilist_entry(
        entry: serde_json::Value,
        media_type: MediaType,
    ) -> Result<TrackerEntry> {
        let entry: AniListEntry = serde_json::from_value(entry)?;

        let title = entry
            .media
            .title
            .english
            .or(entry.media.title.romaji)
            .or(entry.media.title.native)
            .unwrap_or_else(|| "Unknown Title".to_string());

        let mut started_at = None;
        if let Some(ref d) = entry.started_at
            && d.year.is_some()
        {
            let mut map = HashMap::new();
            map.insert("year".to_string(), d.year.map(i64::from));
            map.insert("month".to_string(), d.month.map(i64::from));
            map.insert("day".to_string(), d.day.map(i64::from));
            started_at = Some(map);
        }

        let mut completed_at = None;
        if let Some(ref d) = entry.completed_at
            && d.year.is_some()
        {
            let mut map = HashMap::new();
            map.insert("year".to_string(), d.year.map(i64::from));
            map.insert("month".to_string(), d.month.map(i64::from));
            map.insert("day".to_string(), d.day.map(i64::from));
            completed_at = Some(map);
        }

        Ok(TrackerEntry {
            id: entry.media.id,
            mal_id: entry.media.id_mal,
            ani_id: Some(entry.media.id),
            kitsu_id: None,
            title,
            media_type,
            status: Self::map_anilist_status(&entry.status),
            score: entry.score,
            progress: entry.progress,
            volumes: if media_type == MediaType::Manga {
                entry.progress_volumes.unwrap_or(0)
            } else {
                0
            },
            max_progress: if media_type == MediaType::Anime {
                entry.media.episodes.unwrap_or(0)
            } else {
                entry.media.chapters.unwrap_or(0)
            },
            max_volumes: if media_type == MediaType::Manga {
                entry.media.volumes.unwrap_or(0)
            } else {
                0
            },
            started_at,
            completed_at,
            repeat: entry.repeat.unwrap_or(0),
            notes: entry.notes.unwrap_or_default(),
        })
    }

    async fn fetch_list(
        &self,
        user_name: &str,
        media_type: MediaType,
    ) -> Result<Vec<TrackerEntry>> {
        let mut vars = HashMap::new();
        vars.insert("userName", serde_json::json!(user_name));
        vars.insert(
            "type",
            serde_json::json!(if media_type == MediaType::Anime {
                "ANIME"
            } else {
                "MANGA"
            }),
        );

        let data = self.query(ANILIST_COLLECTION_QUERY, vars).await?;

        let mut entries = Vec::new();
        if let Some(lists) = data
            .get("MediaListCollection")
            .and_then(|c| c.get("lists"))
            .and_then(|l| l.as_array())
        {
            for list in lists {
                if let Some(list_entries) = list.get("entries").and_then(|e| e.as_array()) {
                    for entry_val in list_entries {
                        let tracker_entry =
                            Self::parse_anilist_entry(entry_val.clone(), media_type)?;
                        entries.push(tracker_entry);
                    }
                }
            }
        }
        Ok(entries)
    }
}

#[async_trait]
impl TrackerClient for AniListClient {
    /// Get the current viewer's name from `AniList`.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or the response is invalid.
    async fn get_viewer_name(&self) -> Result<String> {
        let query = "query { Viewer { name } }";
        let data = self.query(query, HashMap::new()).await?;
        if let Some(name) = data
            .get("Viewer")
            .and_then(|v| v.get("name"))
            .and_then(|n| n.as_str())
        {
            Ok(name.to_string())
        } else {
            Err(eyre!("Failed to get Viewer name"))
        }
    }

    fn supported_ids(&self) -> Vec<&'static str> {
        vec!["mal_id", "ani_id"]
    }
    fn supports_anime(&self) -> bool {
        true
    }
    fn supports_manga(&self) -> bool {
        true
    }

    fn get_round_trip_score(&self, internal_score: i32) -> i32 {
        internal_score // `AniList` POINT_100 is 1:1
    }

    async fn fetch_anime_list(&self, user_name: &str) -> Result<Vec<TrackerEntry>> {
        self.fetch_list(user_name, MediaType::Anime).await
    }

    async fn fetch_manga_list(&self, user_name: &str) -> Result<Vec<TrackerEntry>> {
        self.fetch_list(user_name, MediaType::Manga).await
    }

    async fn update_entry(
        &self,
        entry_id: i64,
        _media_type: MediaType,
        options: UpdateOptions,
    ) -> Result<bool> {
        const STATUS_CURRENT: &str = "CURRENT";
        const STATUS_COMPLETED: &str = "COMPLETED";
        const STATUS_PAUSED: &str = "PAUSED";
        const STATUS_DROPPED: &str = "DROPPED";
        const STATUS_PLANNING: &str = "PLANNING";

        let mut vars: HashMap<&str, serde_json::Value> = HashMap::new();
        vars.insert("mediaId", serde_json::json!(entry_id));

        if let Some(s) = options.status {
            let status_str = match s {
                SyncStatus::Current => STATUS_CURRENT,
                SyncStatus::Completed => STATUS_COMPLETED,
                SyncStatus::Paused => STATUS_PAUSED,
                SyncStatus::Dropped => STATUS_DROPPED,
                SyncStatus::Planning => STATUS_PLANNING,
            };
            vars.insert("status", serde_json::json!(status_str));
        }

        if let Some(s) = options.score {
            vars.insert("scoreRaw", serde_json::json!(s));
        }

        if let Some(p) = options.progress {
            vars.insert("progress", serde_json::json!(p));
        }

        if let Some(v) = options.volumes {
            vars.insert("progressVolumes", serde_json::json!(v));
        }

        let map_fuzzy_date =
            |d: Option<HashMap<String, Option<i64>>>| -> Option<serde_json::Value> {
                if let Some(date_map) = d {
                    let mut obj = serde_json::Map::new();
                    obj.insert(
                        "year".to_string(),
                        serde_json::json!(date_map.get("year").copied().flatten()),
                    );
                    obj.insert(
                        "month".to_string(),
                        serde_json::json!(date_map.get("month").copied().flatten()),
                    );
                    obj.insert(
                        "day".to_string(),
                        serde_json::json!(date_map.get("day").copied().flatten()),
                    );
                    Some(serde_json::Value::Object(obj))
                } else {
                    None
                }
            };

        if let Some(sd) = map_fuzzy_date(options.started_at) {
            vars.insert("startedAt", sd);
        }

        if let Some(cd) = map_fuzzy_date(options.completed_at) {
            vars.insert("completedAt", cd);
        }

        if let Some(r) = options.repeat {
            vars.insert("repeat", serde_json::json!(r));
        }

        if let Some(n) = options.notes {
            vars.insert("notes", serde_json::json!(n));
        }

        let _data = self.query(ANILIST_UPDATE_MUTATION, vars).await?;
        Ok(true)
    }

    async fn get_media_id_by_mal_id(
        &self,
        mal_id: i64,
        media_type: MediaType,
    ) -> Result<Option<i64>> {
        let query = r"
        query ($idMal: Int, $type: MediaType) {
            Media(idMal: $idMal, type: $type) {
                id
            }
        }
        ";

        let mut vars = HashMap::new();
        vars.insert("idMal", serde_json::json!(mal_id));
        vars.insert(
            "type",
            serde_json::json!(if media_type == MediaType::Anime {
                "ANIME"
            } else {
                "MANGA"
            }),
        );

        match self.query(query, vars).await {
            Ok(data) => {
                if let Some(id) = data
                    .get("Media")
                    .and_then(|m| m.get("id"))
                    .and_then(serde_json::Value::as_i64)
                {
                    Ok(Some(id))
                } else {
                    Ok(None)
                }
            }
            Err(e) if e.to_string().contains("GraphQL Error (404)") => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn get_media_id_by_ani_id(
        &self,
        ani_id: i64,
        _media_type: MediaType,
    ) -> Result<Option<i64>> {
        Ok(Some(ani_id))
    }

    async fn get_media_id_by_kitsu_id(
        &self,
        _kitsu_id: i64,
        _media_type: MediaType,
    ) -> Result<Option<i64>> {
        Ok(None)
    }
}

/// OAuth 2.0 provider for `AniList`.
pub struct AniListOAuth;

#[async_trait]
impl OAuthProvider for AniListOAuth {
    /// Returns the `AniList` authorization URL.
    fn get_auth_url(&self) -> String {
        format!("{ANILIST_OAUTH_AUTHORIZE_URL}?client_id={ANILIST_CLIENT_ID}&response_type=token")
    }

    /// Exchanges the implicit grant fragment for an access token.
    ///
    /// # Errors
    ///
    /// Returns an error if the access token is missing from the fragment.
    async fn exchange_token(&self, code: &str) -> Result<()> {
        // `AniList` uses implicit grant. The token is sent back in the hash fragment.
        // The callback server script extracts the fragment and forwards it as a query param `forwarded_fragment`.
        // `code` here will actually be the full `forwarded_fragment` value like `access_token=123&...`
        let parsed = url::form_urlencoded::parse(code.as_bytes())
            .into_owned()
            .collect::<HashMap<String, String>>();

        if let Some(token) = parsed.get("access_token") {
            let bundle = crate::storage::TokenBundle {
                access_token: token.clone(),
                refresh_token: None,
                expires_at: None,
            };
            crate::storage::set_token_bundle("anilist", &bundle)?;
            event!(
                name: "anilist.auth.success",
                Level::INFO,
                "Successfully exchanged token for `AniList`",
            );
            Ok(())
        } else {
            Err(eyre!(
                "No access_token found in `AniList` implicit grant response fragment."
            ))
        }
    }

    /// Token refresh is not supported for `AniList` implicit grant.
    ///
    /// # Errors
    ///
    /// Always returns an error as `AniList` implicit grant does not provide refresh tokens.
    async fn refresh_token(&self, _refresh_token: &str) -> Result<()> {
        Err(eyre!(
            "`AniList` Implicit Grant does not provide refresh tokens."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_anilist_client_init() {
        let client = AniListClient::new("dummy_token").unwrap();
        assert_eq!(*client.access_token.read().await, "dummy_token");
        assert_eq!(client.client.rate_limit_calls, 90);
    }

    #[test]
    fn test_anilist_oauth_url() {
        let oauth = AniListOAuth;
        let url = oauth.get_auth_url();
        assert!(url.contains("response_type=token"));
        assert!(url.contains("client_id="));
    }

    #[test]
    fn test_anilist_round_trip() {
        let client = AniListClient::new("dummy").unwrap();
        assert_eq!(client.get_round_trip_score(85), 85);
    }
}
