use crate::client::{BaseClient, OAuthProvider};
use crate::models::{MediaType, SyncStatus, TrackerClient, TrackerEntry};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use color_eyre::{Result, eyre::eyre};
use rand::Rng;
use reqwest::{Method, header};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use url::Url;

pub const MAL_CLIENT_ID: &str = "2d9228ddbcb6f5693edbb8132b9e8183";
pub const MAL_BASE_URL: &str = "https://api.myanimelist.net/v2";
pub const MAL_OAUTH_AUTHORIZE_URL: &str = "https://myanimelist.net/v1/oauth2/authorize";
pub const MAL_OAUTH_TOKEN_URL: &str = "https://myanimelist.net/v1/oauth2/token";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MalPicture {
    pub medium: Option<String>,
    pub large: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MalNode {
    pub id: i64,
    pub title: String,
    pub main_picture: Option<MalPicture>,
    pub num_episodes: Option<i32>,
    pub num_chapters: Option<i32>,
    pub num_volumes: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MalListStatus {
    pub status: String,
    pub score: i32,
    pub num_episodes_watched: Option<i32>,
    pub num_chapters_read: Option<i32>,
    pub num_volumes_read: Option<i32>,
    pub is_rewatching: Option<bool>,
    pub is_rereading: Option<bool>,
    pub num_times_rewatched: Option<i32>,
    pub num_times_reread: Option<i32>,
    pub rewatch_value: Option<i32>,
    pub reread_value: Option<i32>,
    pub priority: Option<i32>,
    pub tags: Option<Vec<String>>,
    pub start_date: Option<String>,
    pub finish_date: Option<String>,
    pub comments: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MalEntry {
    pub node: MalNode,
    pub list_status: MalListStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MalListResponse {
    pub data: Vec<MalEntry>,
    pub paging: Option<MalPaging>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MalPaging {
    pub next: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MalTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i32,
}

use std::sync::Arc;
use tokio::sync::RwLock;

pub struct MalClient {
    pub client: Arc<BaseClient>,
    access_token: Arc<RwLock<String>>,
}

pub struct MalTokenRefresher {
    pub oauth: MalOAuth,
}

#[async_trait]
impl crate::client::TokenRefresher for MalTokenRefresher {
    async fn refresh(&self) -> Result<String> {
        let bundle = crate::storage::get_token_bundle("mal")?
            .ok_or_else(|| eyre!("No token bundle found for MAL"))?;

        let refresh_token = bundle
            .refresh_token
            .as_deref()
            .ok_or_else(|| eyre!("No refresh token available in bundle"))?;

        self.oauth.refresh_token(refresh_token).await?;

        let new_bundle = crate::storage::get_token_bundle("mal")?
            .ok_or_else(|| eyre!("No token bundle found after MAL refresh"))?;

        Ok(new_bundle.access_token)
    }
}

pub struct MalClientRefresher {
    inner: MalTokenRefresher,
    access_token: Arc<RwLock<String>>,
}

#[async_trait]
impl crate::client::TokenRefresher for MalClientRefresher {
    async fn refresh(&self) -> Result<String> {
        let new_token = self.inner.refresh().await?;
        let mut lock = self.access_token.write().await;
        (*lock).clone_from(&new_token);
        Ok(new_token)
    }
}

impl MalClient {
    /// Creates a new `MalClient`.
    ///
    /// # Errors
    ///
    /// Returns an error if the base client cannot be initialized.
    pub fn new(access_token: &str) -> Result<Self> {
        let client = Arc::new(BaseClient::new(
            "mal",
            MAL_BASE_URL,
            1,
            Duration::from_secs(1),
        )?);
        let access_token_arc = Arc::new(RwLock::new(access_token.to_string()));

        let refresher = Arc::new(MalClientRefresher {
            inner: MalTokenRefresher {
                oauth: MalOAuth::new(),
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

    fn map_mal_status(status: &str) -> SyncStatus {
        const STATUS_WATCHING: &str = "watching";
        const STATUS_READING: &str = "reading";
        const STATUS_COMPLETED: &str = "completed";
        const STATUS_ON_HOLD: &str = "on_hold";
        const STATUS_DROPPED: &str = "dropped";

        match status {
            STATUS_WATCHING | STATUS_READING => SyncStatus::Current,
            STATUS_COMPLETED => SyncStatus::Completed,
            STATUS_ON_HOLD => SyncStatus::Paused,
            STATUS_DROPPED => SyncStatus::Dropped,
            _ => SyncStatus::Planning,
        }
    }

    fn parse_date(date_str: Option<&String>) -> Option<HashMap<String, Option<i64>>> {
        crate::models::UpdateOptions::parse_date(date_str)
    }
}

#[async_trait]
impl TrackerClient for MalClient {
    async fn get_viewer_name(&self) -> Result<String> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.get_access_token().await).parse()?,
        );

        let res = self
            .client
            .request(Method::GET, "users/@me", Some(headers))
            .await?;

        let json: serde_json::Value = res.json().await?;

        if let Some(name) = json.get("name").and_then(|n| n.as_str()) {
            Ok(name.to_string())
        } else {
            Err(eyre!("Could not extract viewer name from MAL response"))
        }
    }

    async fn get_viewer_id(&self) -> Result<String> {
        Ok("@me".to_string())
    }

    fn supported_ids(&self) -> Vec<&'static str> {
        vec!["mal_id"]
    }
    fn supports_anime(&self) -> bool {
        true
    }
    fn supports_manga(&self) -> bool {
        true
    }

    fn get_round_trip_score(&self, internal_score: i32) -> i32 {
        let mut mal_score = if internal_score == 0 {
            0
        } else {
            #[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let s = (internal_score as f32 / 10.0).round() as i32;
            s
        };
        if mal_score == 0 && internal_score > 0 {
            mal_score = 1;
        }
        mal_score * 10
    }

    async fn fetch_anime_list(&self, user_name: &str) -> Result<Vec<TrackerEntry>> {
        let mut all_entries = Vec::new();
        let fields = "list_status,num_episodes,main_picture,start_date,finish_date,tags,comments,is_rewatching,num_times_rewatched,rewatch_value,priority,private,updated_at";

        let mut next_url = Some(format!(
            "users/{user_name}/animelist?nsfw=true&fields={fields}&limit=1000"
        ));

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.get_access_token().await).parse()?,
        );

        while let Some(url) = next_url {
            let response = self
                .client
                .request(Method::GET, &url, Some(headers.clone()))
                .await?;
            let data: MalListResponse = response.json().await?;

            for entry in data.data {
                let mal_entry = entry;

                let started_at = Self::parse_date(mal_entry.list_status.start_date.as_ref());
                let completed_at = Self::parse_date(mal_entry.list_status.finish_date.as_ref());

                all_entries.push(TrackerEntry {
                    id: mal_entry.node.id,
                    mal_id: Some(mal_entry.node.id),
                    ani_id: None,
                    kitsu_id: None,
                    title: mal_entry.node.title,
                    media_type: MediaType::Anime,
                    status: Self::map_mal_status(&mal_entry.list_status.status),
                    score: mal_entry.list_status.score * 10,
                    progress: mal_entry.list_status.num_episodes_watched.unwrap_or(0),
                    volumes: 0,
                    max_progress: mal_entry.node.num_episodes.unwrap_or(0),
                    max_volumes: 0,
                    started_at,
                    completed_at,
                    repeat: mal_entry.list_status.num_times_rewatched.unwrap_or(0),
                    notes: mal_entry.list_status.comments.unwrap_or_default(),
                });
            }

            next_url = data.paging.and_then(|p| p.next).map(|mut url| {
                if url.starts_with(MAL_BASE_URL) {
                    url = url
                        .replace(MAL_BASE_URL, "")
                        .trim_start_matches('/')
                        .to_string();
                }
                url
            });
        }

        Ok(all_entries)
    }

    async fn fetch_manga_list(&self, user_name: &str) -> Result<Vec<TrackerEntry>> {
        let mut all_entries = Vec::new();
        let fields = "list_status,num_chapters,num_volumes,main_picture,start_date,finish_date,tags,comments,is_rereading,num_times_reread,reread_value,priority,private,updated_at";

        let mut next_url = Some(format!(
            "users/{user_name}/mangalist?nsfw=true&fields={fields}&limit=1000"
        ));

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.get_access_token().await).parse()?,
        );

        while let Some(url) = next_url {
            let response = self
                .client
                .request(Method::GET, &url, Some(headers.clone()))
                .await?;
            let data: MalListResponse = response.json().await?;

            for entry in data.data {
                let mal_entry = entry;

                let started_at = Self::parse_date(mal_entry.list_status.start_date.as_ref());
                let completed_at = Self::parse_date(mal_entry.list_status.finish_date.as_ref());

                all_entries.push(TrackerEntry {
                    id: mal_entry.node.id,
                    mal_id: Some(mal_entry.node.id),
                    ani_id: None,
                    kitsu_id: None,
                    title: mal_entry.node.title,
                    media_type: MediaType::Manga,
                    status: Self::map_mal_status(&mal_entry.list_status.status),
                    score: mal_entry.list_status.score * 10,
                    progress: mal_entry.list_status.num_chapters_read.unwrap_or(0),
                    volumes: mal_entry.list_status.num_volumes_read.unwrap_or(0),
                    max_progress: mal_entry.node.num_chapters.unwrap_or(0),
                    max_volumes: mal_entry.node.num_volumes.unwrap_or(0),
                    started_at,
                    completed_at,
                    repeat: mal_entry.list_status.num_times_reread.unwrap_or(0),
                    notes: mal_entry.list_status.comments.unwrap_or_default(),
                });
            }

            next_url = data.paging.and_then(|p| p.next).map(|mut url| {
                if url.starts_with(MAL_BASE_URL) {
                    url = url
                        .replace(MAL_BASE_URL, "")
                        .trim_start_matches('/')
                        .to_string();
                }
                url
            });
        }

        Ok(all_entries)
    }

    async fn update_entry(
        &self,
        entry_id: i64,
        media_type: MediaType,
        options: crate::models::UpdateOptions,
    ) -> Result<bool> {
        const STATUS_WATCHING: &str = "watching";
        const STATUS_READING: &str = "reading";
        const STATUS_COMPLETED: &str = "completed";
        const STATUS_ON_HOLD: &str = "on_hold";
        const STATUS_DROPPED: &str = "dropped";
        const STATUS_PLAN_TO_WATCH: &str = "plan_to_watch";
        const STATUS_PLAN_TO_READ: &str = "plan_to_read";

        let endpoint = match media_type {
            MediaType::Anime => format!("anime/{entry_id}/my_list_status"),
            MediaType::Manga => format!("manga/{entry_id}/my_list_status"),
        };

        let mut data = HashMap::new();

        if let Some(s) = options.status {
            let mal_status = match s {
                SyncStatus::Current => {
                    if media_type == MediaType::Anime {
                        STATUS_WATCHING
                    } else {
                        STATUS_READING
                    }
                }
                SyncStatus::Completed => STATUS_COMPLETED,
                SyncStatus::Paused => STATUS_ON_HOLD,
                SyncStatus::Dropped => STATUS_DROPPED,
                SyncStatus::Planning => {
                    if media_type == MediaType::Anime {
                        STATUS_PLAN_TO_WATCH
                    } else {
                        STATUS_PLAN_TO_READ
                    }
                }
            };
            data.insert("status".to_string(), mal_status.to_string());
        }

        if let Some(s) = options.score {
            let mut mal_score = if s == 0 {
                0
            } else {
                #[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
                let val = (s as f32 / 10.0).round() as i32;
                val
            };
            if mal_score == 0 && s > 0 {
                mal_score = 1;
            }
            data.insert("score".to_string(), mal_score.to_string());
        }

        if let Some(p) = options.progress {
            if media_type == MediaType::Anime {
                data.insert("num_watched_episodes".to_string(), p.to_string());
            } else {
                data.insert("num_chapters_read".to_string(), p.to_string());
            }
        }

        if let Some(v) = options.volumes
            && media_type == MediaType::Manga
        {
            data.insert("num_volumes_read".to_string(), v.to_string());
        }

        let map_date = |d: Option<HashMap<String, Option<i64>>>| -> Option<String> {
            crate::models::UpdateOptions::format_date(&d)
        };

        if let Some(sd) = map_date(options.started_at) {
            data.insert("start_date".to_string(), sd);
        }

        if let Some(cd) = map_date(options.completed_at) {
            data.insert("finish_date".to_string(), cd);
        }

        if let Some(r) = options.repeat {
            if media_type == MediaType::Anime {
                data.insert("num_times_rewatched".to_string(), r.to_string());
            } else {
                data.insert("num_times_reread".to_string(), r.to_string());
            }
        }

        if let Some(n) = options.notes {
            data.insert("comments".to_string(), n);
        }

        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", self.get_access_token().await).parse()?,
        );
        headers.insert(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse()?,
        );

        let res = self
            .client
            .request_with_form(Method::PUT, &endpoint, Some(headers), &data)
            .await?;

        Ok(res.status().is_success())
    }

    async fn get_media_id_by_mal_id(
        &self,
        mal_id: i64,
        _media_type: MediaType,
    ) -> Result<Option<i64>> {
        Ok(Some(mal_id))
    }

    async fn get_media_id_by_ani_id(
        &self,
        _ani_id: i64,
        _media_type: MediaType,
    ) -> Result<Option<i64>> {
        Ok(None)
    }

    async fn get_media_id_by_kitsu_id(
        &self,
        _kitsu_id: i64,
        _media_type: MediaType,
    ) -> Result<Option<i64>> {
        Ok(None)
    }
}

pub struct MalOAuth {
    code_verifier: String,
    state: String,
}

impl Default for MalOAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl MalOAuth {
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

    #[must_use]
    pub fn verify_state(&self, state: &str) -> bool {
        self.state == state
    }
}

#[async_trait]
impl OAuthProvider for MalOAuth {
    fn get_auth_url(&self) -> String {
        let mut url = Url::parse(MAL_OAUTH_AUTHORIZE_URL).unwrap();
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", MAL_CLIENT_ID)
            .append_pair("code_challenge", &self.code_verifier)
            .append_pair("state", &self.state);

        url.to_string()
    }

    async fn exchange_token(&self, code: &str) -> Result<()> {
        let client = crate::client::create_reqwest_client()?;
        let mut params = HashMap::new();
        params.insert("client_id", MAL_CLIENT_ID);
        params.insert("code", code);
        params.insert("code_verifier", &self.code_verifier);
        params.insert("grant_type", "authorization_code");

        let res = client
            .post(MAL_OAUTH_TOKEN_URL)
            .form(&params)
            .send()
            .await?;

        if res.status().is_success() {
            let data: MalTokenResponse = res.json().await?;
            let bundle = crate::storage::TokenBundle {
                access_token: data.access_token,
                refresh_token: Some(data.refresh_token),
                expires_at: Some(chrono::Utc::now().timestamp() + i64::from(data.expires_in)),
            };
            crate::storage::set_token_bundle("mal", &bundle)?;
            Ok(())
        } else {
            Err(eyre!("Failed to exchange token: {}", res.text().await?))
        }
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<()> {
        let client = crate::client::create_reqwest_client()?;
        let mut params = HashMap::new();
        params.insert("client_id", MAL_CLIENT_ID);
        params.insert("grant_type", "refresh_token");
        params.insert("refresh_token", refresh_token);

        let res = client
            .post(MAL_OAUTH_TOKEN_URL)
            .form(&params)
            .send()
            .await?;

        if res.status().is_success() {
            let data: MalTokenResponse = res.json().await?;
            let bundle = crate::storage::TokenBundle {
                access_token: data.access_token,
                refresh_token: Some(data.refresh_token),
                expires_at: Some(chrono::Utc::now().timestamp() + i64::from(data.expires_in)),
            };
            crate::storage::set_token_bundle("mal", &bundle)?;
            Ok(())
        } else {
            Err(eyre!("Failed to refresh token: {}", res.text().await?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mal_client_init() {
        let client = MalClient::new("dummy_token").unwrap();
        assert_eq!(*client.access_token.read().await, "dummy_token");
    }

    #[test]
    fn test_mal_oauth_url() {
        let oauth = MalOAuth::new();
        let url = oauth.get_auth_url();
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id="));
        assert!(url.contains("code_challenge="));
        assert!(url.contains("state="));
    }

    #[tokio::test]
    async fn test_mal_round_trip() {
        let client = MalClient::new("dummy").unwrap();
        assert_eq!(client.get_round_trip_score(80), 80);
        assert_eq!(client.get_round_trip_score(85), 90);
        assert_eq!(client.get_round_trip_score(84), 80);
        assert_eq!(client.get_round_trip_score(5), 10); // Min 1 rule
        assert_eq!(client.get_round_trip_score(0), 0);
    }
}
