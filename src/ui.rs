use crate::models::{MediaType, SyncAction, TrackerEntry};
use anstream::{eprintln, println};
use owo_colors::{AnsiColors, OwoColorize};
use tabled::settings::{Alignment, Modify, Style, Width, object::Rows, peaker::Priority};
use tabled::{Table, Tabled};
use terminal_size::{Width as TerminalWidth, terminal_size};

pub fn print_success(msg: &str) {
    println!(
        "{} {}",
        "✔".color(AnsiColors::Green),
        msg.color(AnsiColors::Green)
    );
}

pub fn print_error(msg: &str) {
    eprintln!(
        "{} {}",
        "✖".color(AnsiColors::Red),
        msg.color(AnsiColors::Red)
    );
}

pub fn print_warning(msg: &str) {
    println!(
        "{} {}",
        "⚠".color(AnsiColors::Yellow),
        msg.color(AnsiColors::Yellow)
    );
}

pub fn print_info(msg: &str) {
    println!(
        "{} {}",
        "ℹ".color(AnsiColors::Default),
        msg.color(AnsiColors::Default)
    );
}

#[must_use]
pub fn format_ids(entry: &TrackerEntry) -> String {
    let mal_id = entry
        .mal_id
        .map_or_else(|| "-".to_string(), |id| id.to_string());
    let ani_id = entry
        .ani_id
        .map_or_else(|| "-".to_string(), |id| id.to_string());
    let kitsu_id = entry
        .kitsu_id
        .map_or_else(|| "-".to_string(), |id| id.to_string());
    format!("M:{mal_id} A:{ani_id} K:{kitsu_id}")
}

#[derive(Tabled)]
struct ActionRow {
    #[tabled(rename = "TYPE")]
    media_type: String,
    #[tabled(rename = "TITLE")]
    title: String,
    #[tabled(rename = "IDs")]
    ids: String,
    #[tabled(rename = "CHANGES")]
    changes: String,
}

#[derive(Tabled)]
struct UpdateActionRow {
    #[tabled(rename = "TYPE")]
    media_type: String,
    #[tabled(rename = "TITLE")]
    title: String,
    #[tabled(rename = "SOURCE IDs")]
    source_ids: String,
    #[tabled(rename = "TARGET IDs")]
    target_ids: String,
    #[tabled(rename = "CHANGES")]
    changes: String,
}

/// Renders the action table to the terminal.
///
/// # Panics
///
/// Panics if an action is missing both its source and target entry.
pub fn render_action_table(title: &str, actions: &[&SyncAction], theme_color: AnsiColors) {
    if actions.is_empty() {
        return;
    }

    let mut rows = Vec::new();

    for action in actions {
        let entry = action
            .target_entry
            .as_ref()
            .or(action.source_entry.as_ref())
            .expect("Action must have at least one entry");

        let media_type = match action.media_type {
            MediaType::Anime => "Anime".color(AnsiColors::Magenta).to_string(),
            MediaType::Manga => "Manga".color(AnsiColors::Blue).to_string(),
        };

        let ids_str = format_ids(entry);

        let changes = action
            .reasons
            .iter()
            .map(|r| {
                let mut old_str = format!("{}", r.old_value).replace('"', "");
                let mut new_str = format!("{}", r.new_value).replace('"', "");

                if r.field_name == "score"
                    && let (Ok(old_score), Ok(new_score)) =
                        (old_str.parse::<i32>(), new_str.parse::<i32>())
                {
                    let target = action.target.as_str();
                    let format_score = |score: i32| -> String {
                        if score == 0 {
                            return "0".to_string();
                        }
                        match target {
                            "mal" => {
                                #[expect(
                                    clippy::cast_possible_truncation,
                                    clippy::cast_precision_loss
                                )]
                                let mut s = (score as f32 / 10.0).round() as i32;
                                if s == 0 {
                                    s = 1;
                                }
                                format!("{s}/10")
                            }
                            "kitsu" => {
                                #[expect(
                                    clippy::cast_possible_truncation,
                                    clippy::cast_precision_loss
                                )]
                                let mut s = (score as f32 / 5.0).round() as i32;
                                if s == 0 {
                                    s = 1;
                                }
                                format!("{s}/20")
                            }
                            "anilist" | "mangabaka" => format!("{score}/100"),
                            _ => format!("{score}"),
                        }
                    };
                    old_str = format_score(old_score);
                    new_str = format_score(new_score);
                }

                format!(
                    "{}: {} -> {}",
                    r.field_name,
                    old_str.color(AnsiColors::Red).strikethrough(),
                    new_str.color(AnsiColors::Green)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        rows.push(ActionRow {
            media_type,
            title: entry.title.clone(),
            ids: ids_str.color(theme_color).to_string(),
            changes,
        });
    }

    let mut table = Table::new(rows);
    table.with(Style::rounded());
    table.with(Modify::new(Rows::first()).with(Alignment::center()));

    if let Some((TerminalWidth(w), _)) = terminal_size() {
        let max_width = w as usize;
        table.with((
            Width::wrap(max_width)
                .keep_words(true)
                .priority(Priority::max(true)),
            Width::increase(max_width).priority(Priority::min(true)),
        ));
    }
    println!("{}", format!("\n=== {title} ===").color(theme_color).bold());
    println!("{table}");
}

/// Renders the update action table to the terminal.
///
/// # Panics
///
/// Panics if an action is missing its source or target entry.
pub fn render_update_action_table(title: &str, actions: &[&SyncAction], theme_color: AnsiColors) {
    if actions.is_empty() {
        return;
    }

    let mut rows = Vec::new();

    for action in actions {
        let source_entry = action
            .source_entry
            .as_ref()
            .expect("Update action must have source entry");
        let target_entry = action
            .target_entry
            .as_ref()
            .expect("Update action must have target entry");

        let media_type = match action.media_type {
            MediaType::Anime => "Anime".color(AnsiColors::Magenta).to_string(),
            MediaType::Manga => "Manga".color(AnsiColors::Blue).to_string(),
        };

        let source_ids_str = format_ids(source_entry);
        let target_ids_str = format_ids(target_entry);

        let changes = action
            .reasons
            .iter()
            .map(|r| {
                let mut old_str = format!("{}", r.old_value).replace('"', "");
                let mut new_str = format!("{}", r.new_value).replace('"', "");

                if r.field_name == "score"
                    && let (Ok(old_score), Ok(new_score)) =
                        (old_str.parse::<i32>(), new_str.parse::<i32>())
                {
                    let target = action.target.as_str();
                    let format_score = |score: i32| -> String {
                        if score == 0 {
                            return "0".to_string();
                        }
                        match target {
                            "mal" => {
                                #[expect(
                                    clippy::cast_possible_truncation,
                                    clippy::cast_precision_loss
                                )]
                                let mut s = (score as f32 / 10.0).round() as i32;
                                if s == 0 {
                                    s = 1;
                                }
                                format!("{s}/10")
                            }
                            "kitsu" => {
                                #[expect(
                                    clippy::cast_possible_truncation,
                                    clippy::cast_precision_loss
                                )]
                                let mut s = (score as f32 / 5.0).round() as i32;
                                if s == 0 {
                                    s = 1;
                                }
                                format!("{s}/20")
                            }
                            "anilist" | "mangabaka" => format!("{score}/100"),
                            _ => format!("{score}"),
                        }
                    };
                    old_str = format_score(old_score);
                    new_str = format_score(new_score);
                }

                format!(
                    "{}: {} -> {}",
                    r.field_name,
                    old_str.color(AnsiColors::Red).strikethrough(),
                    new_str.color(AnsiColors::Green)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        rows.push(UpdateActionRow {
            media_type,
            title: source_entry.title.clone(),
            source_ids: source_ids_str.color(theme_color).to_string(),
            target_ids: target_ids_str.color(theme_color).to_string(),
            changes,
        });
    }

    let mut table = Table::new(rows);
    table.with(Style::rounded());
    table.with(Modify::new(Rows::first()).with(Alignment::center()));

    if let Some((TerminalWidth(w), _)) = terminal_size() {
        let max_width = w as usize;
        table.with((
            Width::wrap(max_width)
                .keep_words(true)
                .priority(Priority::max(true)),
            Width::increase(max_width).priority(Priority::min(true)),
        ));
    }
    println!("{}", format!("\n=== {title} ===").color(theme_color).bold());
    println!("{table}");
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActionType, DiffField, SyncStatus, TrackerEntry};
    use serde_json::json;

    #[test]
    fn test_print_functions_dont_panic() {
        print_success("test success");
        print_error("test error");
        print_warning("test warning");
        print_info("test info");
    }

    #[test]
    fn test_render_action_table_empty() {
        let actions: Vec<&SyncAction> = vec![];
        render_action_table("Empty", &actions, AnsiColors::Cyan);
    }

    #[test]
    fn test_render_action_table_with_data() {
        let entry = TrackerEntry {
            id: 1,
            mal_id: Some(1),
            ani_id: Some(2),
            kitsu_id: Some(3),
            title: "Test Anime".to_string(),
            media_type: MediaType::Anime,
            status: SyncStatus::Current,
            score: 80,
            progress: 5,
            volumes: 0,
            started_at: None,
            completed_at: None,
            repeat: 0,
            notes: String::new(),
            max_progress: 12,
            max_volumes: 0,
        };

        let action = SyncAction {
            action: ActionType::Update,
            source: "mal".to_string(),
            target: "anilist".to_string(),
            media_type: MediaType::Anime,
            source_entry: Some(entry.clone()),
            target_entry: Some(entry),
            reasons: vec![DiffField {
                field_name: "progress".to_string(),
                old_value: json!(4),
                new_value: json!(5),
            }],
        };

        let actions = vec![&action];
        render_action_table("Test Table", &actions, AnsiColors::Yellow);
    }

    #[test]
    fn test_render_update_action_table_with_dual_ids() {
        let source_entry = TrackerEntry {
            id: 1,
            mal_id: Some(10),
            ani_id: Some(20),
            kitsu_id: Some(30),
            title: "Test Anime".to_string(),
            media_type: MediaType::Anime,
            status: SyncStatus::Current,
            score: 80,
            progress: 5,
            volumes: 0,
            started_at: None,
            completed_at: None,
            repeat: 0,
            notes: String::new(),
            max_progress: 12,
            max_volumes: 0,
        };

        let mut target_entry = source_entry.clone();
        target_entry.id = 2;
        target_entry.mal_id = Some(11);
        target_entry.ani_id = Some(21);
        target_entry.kitsu_id = Some(31);

        let action = SyncAction {
            action: ActionType::Update,
            source: "mal".to_string(),
            target: "anilist".to_string(),
            media_type: MediaType::Anime,
            source_entry: Some(source_entry),
            target_entry: Some(target_entry),
            reasons: vec![DiffField {
                field_name: "progress".to_string(),
                old_value: json!(4),
                new_value: json!(5),
            }],
        };

        let actions = vec![&action];
        render_update_action_table("Test Update Table", &actions, AnsiColors::Yellow);
    }
}
