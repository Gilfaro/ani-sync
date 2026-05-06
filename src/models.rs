use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct UpdateOptions {
    pub status: Option<SyncStatus>,
    pub score: Option<i32>,
    pub progress: Option<i32>,
    pub volumes: Option<i32>,
    pub started_at: Option<HashMap<String, Option<i64>>>,
    pub completed_at: Option<HashMap<String, Option<i64>>>,
    pub repeat: Option<i32>,
    pub notes: Option<String>,
    pub is_add: bool,
}

impl UpdateOptions {
    #[must_use]
    pub fn parse_date(date_str: Option<&String>) -> Option<HashMap<String, Option<i64>>> {
        date_str.and_then(|ds| {
            let parts: Vec<&str> = ds.split('T').next()?.split('-').collect();
            if parts.is_empty() {
                None
            } else {
                let year = parts[0].parse::<i64>().ok()?;
                let mut map = HashMap::new();
                map.insert("year".to_string(), Some(year));
                map.insert(
                    "month".to_string(),
                    parts.get(1).and_then(|m| m.parse().ok()),
                );
                map.insert("day".to_string(), parts.get(2).and_then(|d| d.parse().ok()));
                Some(map)
            }
        })
    }

    #[must_use]
    pub fn format_date(date: &Option<HashMap<String, Option<i64>>>) -> Option<String> {
        if let Some(date_map) = date
            && let Some(Some(y)) = date_map.get("year")
        {
            let m = date_map.get("month").and_then(|x| *x).unwrap_or(1);
            let day = date_map.get("day").and_then(|x| *x).unwrap_or(1);
            return Some(format!("{y:04}-{m:02}-{day:02}"));
        }
        None
    }

    #[must_use]
    fn normalize_date(
        date: Option<&HashMap<String, Option<i64>>>,
    ) -> Option<HashMap<String, Option<i64>>> {
        if let Some(d) = date
            && d.values().all(std::option::Option::is_none)
        {
            return None;
        }
        date.cloned()
    }

    #[must_use]
    #[expect(clippy::too_many_lines)]
    pub fn from_sync_action(action: &SyncAction) -> Self {
        let mut update_options = UpdateOptions {
            is_add: action.action == ActionType::Add,
            ..Default::default()
        };

        let Some(source_entry) = &action.source_entry else {
            return update_options;
        };

        if action.action == ActionType::Add {
            update_options.status = Some(source_entry.status);
            update_options.score = Some(source_entry.score);
            update_options.repeat = Some(source_entry.repeat);
            update_options.notes = Some(source_entry.notes.clone());

            // Progress Maxing Out Logic for ADD
            if source_entry.status == SyncStatus::Completed && source_entry.max_progress > 0 {
                update_options.progress = Some(source_entry.max_progress);
            } else if source_entry.progress > 0 {
                update_options.progress = Some(source_entry.progress);
            }

            if action.media_type == MediaType::Manga {
                if source_entry.status == SyncStatus::Completed && source_entry.max_volumes > 0 {
                    update_options.volumes = Some(source_entry.max_volumes);
                } else if source_entry.volumes > 0 {
                    update_options.volumes = Some(source_entry.volumes);
                }
            }

            if let Some(started) = Self::normalize_date(source_entry.started_at.as_ref()) {
                update_options.started_at = Some(started);
            } else {
                let mut empty = HashMap::new();
                empty.insert("year".to_string(), None);
                empty.insert("month".to_string(), None);
                empty.insert("day".to_string(), None);
                update_options.started_at = Some(empty);
            }

            if let Some(completed) = Self::normalize_date(source_entry.completed_at.as_ref()) {
                update_options.completed_at = Some(completed);
            } else {
                let mut empty = HashMap::new();
                empty.insert("year".to_string(), None);
                empty.insert("month".to_string(), None);
                empty.insert("day".to_string(), None);
                update_options.completed_at = Some(empty);
            }
        } else {
            for reason in &action.reasons {
                match reason.field_name.as_str() {
                    "status" => {
                        update_options.status =
                            serde_json::from_value(reason.new_value.clone()).ok();
                    }
                    "score" => {
                        update_options.score =
                            serde_json::from_value(reason.new_value.clone()).ok();
                    }
                    "progress" => {
                        update_options.progress =
                            serde_json::from_value(reason.new_value.clone()).ok();
                    }
                    "volumes" => {
                        update_options.volumes =
                            serde_json::from_value(reason.new_value.clone()).ok();
                    }
                    "repeat" => {
                        update_options.repeat =
                            serde_json::from_value(reason.new_value.clone()).ok();
                    }
                    "notes" => {
                        update_options.notes =
                            serde_json::from_value(reason.new_value.clone()).ok();
                    }
                    "started_at" => {
                        if reason.new_value.is_null() {
                            let mut empty = HashMap::new();
                            empty.insert("year".to_string(), None);
                            empty.insert("month".to_string(), None);
                            empty.insert("day".to_string(), None);
                            update_options.started_at = Some(empty);
                        } else {
                            update_options.started_at =
                                serde_json::from_value(reason.new_value.clone()).ok();
                        }
                    }
                    "completed_at" => {
                        if reason.new_value.is_null() {
                            let mut empty = HashMap::new();
                            empty.insert("year".to_string(), None);
                            empty.insert("month".to_string(), None);
                            empty.insert("day".to_string(), None);
                            update_options.completed_at = Some(empty);
                        } else {
                            update_options.completed_at =
                                serde_json::from_value(reason.new_value.clone()).ok();
                        }
                    }
                    _ => {}
                }
            }
        }

        if action.action == ActionType::Update {
            // If date missing from source, ensure cleared on target
            if Self::normalize_date(source_entry.started_at.as_ref()).is_none()
                && update_options.started_at.is_none()
            {
                let mut empty = HashMap::new();
                empty.insert("year".to_string(), None);
                empty.insert("month".to_string(), None);
                empty.insert("day".to_string(), None);
                update_options.started_at = Some(empty);
            }
            if Self::normalize_date(source_entry.completed_at.as_ref()).is_none()
                && update_options.completed_at.is_none()
            {
                let mut empty = HashMap::new();
                empty.insert("year".to_string(), None);
                empty.insert("month".to_string(), None);
                empty.insert("day".to_string(), None);
                update_options.completed_at = Some(empty);
            }
        }

        update_options
    }
}

#[async_trait::async_trait]
pub trait TrackerClient: Send + Sync {
    fn supported_ids(&self) -> Vec<&'static str>;
    fn supports_anime(&self) -> bool;
    fn supports_manga(&self) -> bool;

    /// Converts an internal 100-scale score to the tracker's native scale, then back to 100-scale.
    /// This simulates how the tracker will interpret and save the score.
    fn get_round_trip_score(&self, internal_score: i32) -> i32;

    async fn get_viewer_name(&self) -> color_eyre::Result<String>;
    async fn get_viewer_id(&self) -> color_eyre::Result<String> {
        self.get_viewer_name().await
    }
    async fn fetch_anime_list(&self, user_name: &str) -> color_eyre::Result<Vec<TrackerEntry>>;
    async fn fetch_manga_list(&self, user_name: &str) -> color_eyre::Result<Vec<TrackerEntry>>;

    async fn update_entry(
        &self,
        entry_id: i64,
        media_type: MediaType,
        options: UpdateOptions,
    ) -> color_eyre::Result<bool>;
    async fn get_media_id_by_mal_id(
        &self,
        mal_id: i64,
        media_type: MediaType,
    ) -> color_eyre::Result<Option<i64>>;
    async fn get_media_id_by_ani_id(
        &self,
        ani_id: i64,
        media_type: MediaType,
    ) -> color_eyre::Result<Option<i64>>;
    async fn get_media_id_by_kitsu_id(
        &self,
        kitsu_id: i64,
        media_type: MediaType,
    ) -> color_eyre::Result<Option<i64>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SyncStatus {
    Current,
    Completed,
    Paused,
    Dropped,
    Planning,
}

impl SyncStatus {
    #[must_use]
    pub fn rank(&self) -> u8 {
        match self {
            Self::Completed => 5,
            Self::Current => 4,
            Self::Paused => 3,
            Self::Dropped => 2,
            Self::Planning => 1,
        }
    }
}

impl PartialOrd for SyncStatus {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SyncStatus {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum MediaType {
    Anime,
    Manga,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackerEntry {
    pub id: i64,
    pub mal_id: Option<i64>,
    pub ani_id: Option<i64>,
    pub kitsu_id: Option<i64>,
    pub title: String,
    pub media_type: MediaType,
    pub status: SyncStatus,
    pub score: i32,
    pub progress: i32,
    #[serde(default)]
    pub volumes: i32,
    #[serde(default)]
    pub started_at: Option<HashMap<String, Option<i64>>>,
    #[serde(default)]
    pub completed_at: Option<HashMap<String, Option<i64>>>,
    #[serde(default)]
    pub repeat: i32,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub max_progress: i32,
    #[serde(default)]
    pub max_volumes: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffField {
    pub field_name: String,
    pub old_value: serde_json::Value,
    pub new_value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncResult {
    pub source_entry: Option<TrackerEntry>,
    pub target_entry: Option<TrackerEntry>,
    pub is_in_sync: bool,
    pub diff: Vec<DiffField>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ActionType {
    Add,
    Update,
    Skip,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncAction {
    pub action: ActionType,
    pub source: String,
    pub target: String,
    pub media_type: MediaType,
    pub source_entry: Option<TrackerEntry>,
    pub target_entry: Option<TrackerEntry>,
    #[serde(default)]
    pub reasons: Vec<DiffField>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracker_entry_serialization() {
        let entry = TrackerEntry {
            id: 1,
            mal_id: Some(2),
            ani_id: None,
            kitsu_id: None,
            title: "Test Anime".to_string(),
            media_type: MediaType::Anime,
            status: SyncStatus::Current,
            score: 85,
            progress: 12,
            volumes: 0,
            started_at: None,
            completed_at: None,
            repeat: 0,
            notes: String::new(),
            max_progress: 24,
            max_volumes: 0,
        };

        let serialized = serde_json::to_string(&entry).unwrap();

        // Assert expected keys exist and are properly serialized
        assert!(serialized.contains("\"id\":1"));
        assert!(serialized.contains("\"mal_id\":2"));
        assert!(serialized.contains("\"media_type\":\"ANIME\""));
        assert!(serialized.contains("\"status\":\"CURRENT\""));
        assert!(serialized.contains("\"score\":85"));
        assert!(serialized.contains("\"progress\":12"));

        let deserialized: TrackerEntry = serde_json::from_str(&serialized).unwrap();
        assert_eq!(entry, deserialized);
    }

    #[test]
    fn test_update_options_from_sync_action_add() {
        let entry = TrackerEntry {
            id: 1,
            mal_id: Some(2),
            ani_id: None,
            kitsu_id: None,
            title: "Test Anime".to_string(),
            media_type: MediaType::Anime,
            status: SyncStatus::Current,
            score: 85,
            progress: 12,
            volumes: 0,
            started_at: None,
            completed_at: None,
            repeat: 0,
            notes: "Test notes".to_string(),
            max_progress: 24,
            max_volumes: 0,
        };

        let action = SyncAction {
            action: ActionType::Add,
            source: "mal".to_string(),
            target: "anilist".to_string(),
            media_type: MediaType::Anime,
            source_entry: Some(entry),
            target_entry: None,
            reasons: vec![],
        };

        let options = UpdateOptions::from_sync_action(&action);

        assert!(options.is_add);
        assert_eq!(options.status, Some(SyncStatus::Current));
        assert_eq!(options.score, Some(85));
        assert_eq!(options.progress, Some(12));
        assert_eq!(options.notes, Some("Test notes".to_string()));

        // Assert dates are normalized to empty if None
        let empty_date = options.started_at.unwrap();
        assert_eq!(empty_date.get("year").unwrap(), &None);
    }
}
