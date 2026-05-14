// Rust guideline compliant 2026-02-21

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Options for updating an entry on a tracker.
///
/// This struct encapsulates all possible fields that can be updated during a
/// synchronization operation.
#[derive(Debug, Default, Clone)]
pub struct UpdateOptions {
    /// The new status of the entry.
    pub status: Option<SyncStatus>,
    /// The new score of the entry (0-100).
    pub score: Option<i32>,
    /// The current episode/chapter progress.
    pub progress: Option<i32>,
    /// The current volume progress (for manga).
    pub volumes: Option<i32>,
    /// The start date of the media.
    pub started_at: Option<HashMap<String, Option<i64>>>,
    /// The completion date of the media.
    pub completed_at: Option<HashMap<String, Option<i64>>>,
    /// The number of times the media has been rewatched/reread.
    pub repeat: Option<i32>,
    /// Personal notes for the entry.
    pub notes: Option<String>,
    /// Whether this is a new entry being added.
    pub is_add: bool,
}

impl UpdateOptions {
    /// Creates a new `UpdateOptions` with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses a date string into a structured format.
    ///
    /// The input string is expected to be in ISO 8601 format (e.g., "2023-04-14T00:00:00Z").
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

    /// Formats a structured date into a string.
    ///
    /// The output string is in "YYYY-MM-DD" format.
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

    /// Creates `UpdateOptions` from a `SyncAction`.
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

/// A client for interacting with an anime/manga tracker.
#[async_trait::async_trait]
pub trait TrackerClient: Send + Sync {
    /// Returns the list of external source IDs supported by this tracker.
    fn supported_ids(&self) -> Vec<&'static str>;
    /// Returns whether this tracker supports anime.
    fn supports_anime(&self) -> bool;
    /// Returns whether this tracker supports manga.
    fn supports_manga(&self) -> bool;

    /// Converts an internal 100-scale score to the tracker's native scale, then back to 100-scale.
    ///
    /// This simulates how the tracker will interpret and save the score.
    fn get_round_trip_score(&self, internal_score: i32) -> i32;

    /// Returns the current viewer's username.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    async fn get_viewer_name(&self) -> color_eyre::Result<String>;

    /// Returns the current viewer's unique identifier.
    ///
    /// Defaults to `get_viewer_name`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    async fn get_viewer_id(&self) -> color_eyre::Result<String> {
        self.get_viewer_name().await
    }

    /// Fetches the user's anime list.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    async fn fetch_anime_list(&self, user_name: &str) -> color_eyre::Result<Vec<TrackerEntry>>;

    /// Fetches the user's manga list.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    async fn fetch_manga_list(&self, user_name: &str) -> color_eyre::Result<Vec<TrackerEntry>>;

    /// Updates or adds an entry on the tracker.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    async fn update_entry(
        &self,
        entry_id: i64,
        media_type: MediaType,
        options: UpdateOptions,
    ) -> color_eyre::Result<bool>;

    /// Resolves a media ID on the tracker using its `MyAnimeList` ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    async fn get_media_id_by_mal_id(
        &self,
        mal_id: i64,
        media_type: MediaType,
    ) -> color_eyre::Result<Option<i64>>;

    /// Resolves a media ID on the tracker using its `AniList` ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    async fn get_media_id_by_ani_id(
        &self,
        ani_id: i64,
        media_type: MediaType,
    ) -> color_eyre::Result<Option<i64>>;

    /// Resolves a media ID on the tracker using its Kitsu ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    async fn get_media_id_by_kitsu_id(
        &self,
        kitsu_id: i64,
        media_type: MediaType,
    ) -> color_eyre::Result<Option<i64>>;
}

/// The status of an entry in a user's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SyncStatus {
    /// Currently watching/reading.
    Current,
    /// Finished watching/reading.
    Completed,
    /// On hold.
    Paused,
    /// Stopped watching/reading.
    Dropped,
    /// Planning to watch/read.
    Planning,
}

impl SyncStatus {
    /// Returns a numeric rank for comparing statuses.
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

/// The type of media (Anime or Manga).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum MediaType {
    /// Animation.
    Anime,
    /// Comics/Graphic novels.
    Manga,
}

/// A normalized entry from a tracker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackerEntry {
    /// Internal ID on the tracker.
    pub id: i64,
    /// `MyAnimeList` ID.
    pub mal_id: Option<i64>,
    /// `AniList` ID.
    pub ani_id: Option<i64>,
    /// Kitsu ID.
    pub kitsu_id: Option<i64>,
    /// The primary title of the media.
    pub title: String,
    /// The type of media.
    pub media_type: MediaType,
    /// The user's status for this entry.
    pub status: SyncStatus,
    /// The user's score (0-100).
    pub score: i32,
    /// Current episode/chapter progress.
    pub progress: i32,
    /// Current volume progress (for manga).
    #[serde(default)]
    pub volumes: i32,
    /// The start date of the media.
    #[serde(default)]
    pub started_at: Option<HashMap<String, Option<i64>>>,
    /// The completion date of the media.
    #[serde(default)]
    pub completed_at: Option<HashMap<String, Option<i64>>>,
    /// Number of times rewatched/reread.
    #[serde(default)]
    pub repeat: i32,
    /// Personal notes.
    #[serde(default)]
    pub notes: String,
    /// Total episodes/chapters available.
    #[serde(default)]
    pub max_progress: i32,
    /// Total volumes available.
    #[serde(default)]
    pub max_volumes: i32,
}

/// Represents a change in a specific field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffField {
    /// Name of the field.
    pub field_name: String,
    /// Value before the change.
    pub old_value: serde_json::Value,
    /// Value after the change.
    pub new_value: serde_json::Value,
}

/// The result of a synchronization comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncResult {
    /// Entry from the source tracker.
    pub source_entry: Option<TrackerEntry>,
    /// Entry from the target tracker.
    pub target_entry: Option<TrackerEntry>,
    /// Whether the two entries are currently in sync.
    pub is_in_sync: bool,
    /// List of differences found.
    pub diff: Vec<DiffField>,
}

/// The type of action to perform during synchronization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ActionType {
    /// Add a new entry to the target tracker.
    Add,
    /// Update an existing entry on the target tracker.
    Update,
    /// No changes needed.
    Skip,
    /// Remove an entry (not currently implemented/used for safety).
    Delete,
}

/// A planned synchronization action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncAction {
    /// The type of action to perform.
    pub action: ActionType,
    /// Name of the source tracker.
    pub source: String,
    /// Name of the target tracker.
    pub target: String,
    /// The type of media.
    pub media_type: MediaType,
    /// The source entry.
    pub source_entry: Option<TrackerEntry>,
    /// The target entry.
    pub target_entry: Option<TrackerEntry>,
    /// Reasons for the action (list of differences).
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
