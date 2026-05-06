use ani_sync::anilist;
use ani_sync::auth;
use ani_sync::cli::{AuthProvider, Cli, Commands};
use ani_sync::client::{OAuthProvider, create_reqwest_client};
use ani_sync::kitsu;
use ani_sync::mal;
use ani_sync::mangabaka;
use ani_sync::models;
use ani_sync::storage;
use ani_sync::sync;
use ani_sync::ui::{
    print_error, print_info, print_success, render_action_table, render_update_action_table,
};

use clap::Parser;
use color_eyre::Result;
use owo_colors::AnsiColors;
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};

#[derive(Deserialize, Default)]
struct Config {
    source: Option<String>,
    target: Option<String>,
    sync_anime: Option<bool>,
    sync_manga: Option<bool>,
}

fn load_config() -> Config {
    let mut path = std::env::current_dir().unwrap_or_default();
    path.push("config.json");
    if let Ok(content) = fs::read_to_string(path)
        && let Ok(config) = serde_json::from_str(&content)
    {
        return config;
    }
    Config::default()
}

fn prompt_for_password() -> Result<String> {
    print!("Password: ");
    io::stdout().flush()?;
    let password = rpassword::read_password()?;
    println!();
    Ok(password)
}

fn prompt_for_input(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

#[tokio::main]
#[expect(clippy::too_many_lines)]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Auth { provider } => match provider {
            AuthProvider::Mal => {
                print_info("Initiating MyAnimeList authentication flow...");
                let oauth = mal::MalOAuth::new();
                let url = oauth.get_auth_url();
                print_info(&format!("Please visit: {url}"));
                if webbrowser::open(&url).is_ok() {
                    print_info("Waiting for authorization on port 9145...");
                    if let Ok(callback_path) = auth::capture_oauth_callback(9145).await {
                        let parsed_url =
                            url::Url::parse(&format!("http://localhost{callback_path}")).unwrap();
                        let query: std::collections::HashMap<_, _> =
                            parsed_url.query_pairs().into_owned().collect();

                        let Some(state) = query.get("state") else {
                            print_error("Authorization state missing! Possible CSRF attack.");
                            return Ok(());
                        };

                        if !oauth.verify_state(state) {
                            print_error("Authorization state mismatch! Possible CSRF attack.");
                            return Ok(());
                        }

                        if let Some(code) = query.get("code") {
                            if oauth.exchange_token(code).await.is_ok() {
                                print_success("MyAnimeList authorization successful!");
                            } else {
                                print_error("Failed to exchange token.");
                            }
                        }
                    }
                } else {
                    print_error("Failed to open browser.");
                }
            }
            AuthProvider::Anilist => {
                print_info("Initiating AniList authentication flow...");
                let oauth = anilist::AniListOAuth;
                let url = oauth.get_auth_url();
                print_info(&format!("Please visit: {url}"));
                if webbrowser::open(&url).is_ok() {
                    print_info("Waiting for authorization on port 9145...");
                    if let Ok(callback_path) = auth::capture_oauth_callback(9145).await {
                        let parsed_url =
                            url::Url::parse(&format!("http://localhost{callback_path}")).unwrap();
                        let query: std::collections::HashMap<_, _> =
                            parsed_url.query_pairs().into_owned().collect();
                        if let Some(fragment) = query.get("forwarded_fragment") {
                            if oauth.exchange_token(fragment).await.is_ok() {
                                print_success("AniList authorization successful!");
                            } else {
                                print_error("Failed to extract token.");
                            }
                        }
                    }
                } else {
                    print_error("Failed to open browser.");
                }
            }
            AuthProvider::Kitsu => {
                print_info("--- Kitsu Login ---");
                print_info(
                    "Disclaimer: Your email and password are used ONLY for this initial token exchange and are NOT saved.",
                );
                let username = prompt_for_input("Email: ")?;
                if username.is_empty() {
                    print_error("Email cannot be empty.");
                    std::process::exit(1);
                }
                let password = prompt_for_password()?;
                if password.is_empty() {
                    print_error("Password cannot be empty.");
                    std::process::exit(1);
                }

                print_info("Authenticating...");
                let client = create_reqwest_client()?;
                let res = client
                    .post(kitsu::KITSU_OAUTH_TOKEN_URL)
                    .header("Accept", "application/json")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .form(&[
                        ("grant_type", "password"),
                        ("username", &username),
                        ("password", &password),
                        ("client_id", kitsu::KITSU_CLIENT_ID),
                        ("client_secret", kitsu::KITSU_CLIENT_SECRET),
                    ])
                    .send()
                    .await;

                match res {
                    Ok(response) => {
                        if response.status() == 401 {
                            print_error("Invalid username or password.");
                        } else if response.status().is_success() {
                            if let Ok(json) = response.json::<serde_json::Value>().await
                                && let Some(token) = json["access_token"].as_str()
                            {
                                let bundle = crate::storage::TokenBundle {
                                    access_token: token.to_string(),
                                    refresh_token: json["refresh_token"]
                                        .as_str()
                                        .map(ToString::to_string),
                                    expires_at: json["created_at"]
                                        .as_i64()
                                        .and_then(|c| json["expires_in"].as_i64().map(|e| c + e)),
                                };
                                storage::set_token_bundle("kitsu", &bundle).unwrap();
                                print_success("Kitsu authorization successful!");
                            }
                        } else {
                            print_error(&format!("Authentication failed: {}", response.status()));
                        }
                    }
                    Err(e) => print_error(&format!("Authentication failed: {e}")),
                }
            }
            AuthProvider::Mangabaka => {
                print_info("MangaBaka auth not fully implemented via CLI yet.");
                let oauth = mangabaka::MangaBakaOAuth::new();
                let url = oauth.get_auth_url();
                print_info(&format!("Please visit: {url}"));
                if webbrowser::open(&url).is_ok() {
                    print_info("Waiting for authorization on port 9145...");
                    if let Ok(callback_path) = auth::capture_oauth_callback(9145).await {
                        let parsed_url =
                            url::Url::parse(&format!("http://localhost{callback_path}")).unwrap();
                        let query: std::collections::HashMap<_, _> =
                            parsed_url.query_pairs().into_owned().collect();

                        let Some(state) = query.get("state") else {
                            print_error("Authorization state missing! Possible CSRF attack.");
                            return Ok(());
                        };

                        if !oauth.verify_state(state) {
                            print_error("Authorization state mismatch! Possible CSRF attack.");
                            return Ok(());
                        }

                        if let Some(code) = query.get("code") {
                            if oauth.exchange_token(code).await.is_ok() {
                                print_success("MangaBaka authorization successful!");
                            } else {
                                print_error("Failed to exchange token.");
                            }
                        }
                    }
                } else {
                    print_error("Failed to open browser.");
                }
            }
        },
        Commands::Sync {
            source,
            target,
            anime,
            no_anime,
            manga,
            no_manga,
            yes,
            no_downgrade,
            preserve_existing,
        } => {
            let config = load_config();
            let source_svc = source
                .clone()
                .or(config.source)
                .unwrap_or_default()
                .to_lowercase();
            let target_svc = target
                .clone()
                .or(config.target)
                .unwrap_or_default()
                .to_lowercase();

            if source_svc.is_empty() || target_svc.is_empty() {
                print_error("Error: Must specify both source and target services.");
                std::process::exit(1);
            }

            let valid_providers = ["mal", "anilist", "kitsu", "mangabaka"];
            if !valid_providers.contains(&source_svc.as_str())
                || !valid_providers.contains(&target_svc.as_str())
                || source_svc == target_svc
            {
                print_error(&format!(
                    "Error: Invalid sync pair {source_svc} -> {target_svc}."
                ));
                std::process::exit(1);
            }

            print_info(&format!(
                "Initializing sync from {source_svc} to {target_svc}..."
            ));

            let source_token = storage::get_token_bundle(&source_svc)
                .unwrap_or_default()
                .map(|b| b.access_token)
                .unwrap_or_default();
            let target_token = storage::get_token_bundle(&target_svc)
                .unwrap_or_default()
                .map(|b| b.access_token)
                .unwrap_or_default();

            if source_token.is_empty() || target_token.is_empty() {
                print_error("Cannot sync: Missing tokens. Run `auth` first.");
                std::process::exit(1);
            }

            print_info("Synchronizing lists...");
            let source_client: std::sync::Arc<dyn models::TrackerClient> = match source_svc.as_str()
            {
                "mal" => std::sync::Arc::new(mal::MalClient::new(&source_token)?),
                "anilist" => std::sync::Arc::new(anilist::AniListClient::new(&source_token)?),
                "kitsu" => std::sync::Arc::new(kitsu::KitsuClient::new(&source_token)?),
                "mangabaka" => std::sync::Arc::new(mangabaka::MangaBakaClient::new(&source_token)?),
                _ => {
                    print_error(&format!("Unsupported source service: {source_svc}"));
                    std::process::exit(1);
                }
            };

            let target_client: std::sync::Arc<dyn models::TrackerClient> = match target_svc.as_str()
            {
                "mal" => std::sync::Arc::new(mal::MalClient::new(&target_token)?),
                "anilist" => std::sync::Arc::new(anilist::AniListClient::new(&target_token)?),
                "kitsu" => std::sync::Arc::new(kitsu::KitsuClient::new(&target_token)?),
                "mangabaka" => std::sync::Arc::new(mangabaka::MangaBakaClient::new(&target_token)?),
                _ => {
                    print_error(&format!("Unsupported target service: {target_svc}"));
                    std::process::exit(1);
                }
            };

            let sync_anime_pref = if *anime {
                true
            } else if *no_anime {
                false
            } else {
                config.sync_anime.unwrap_or(true)
            } && source_client.supports_anime()
                && target_client.supports_anime();

            let sync_manga_pref = if *manga {
                true
            } else if *no_manga {
                false
            } else {
                config.sync_manga.unwrap_or(true)
            } && source_client.supports_manga()
                && target_client.supports_manga();

            if !sync_anime_pref && !sync_manga_pref {
                print_error("Error: No media types enabled for sync or common between services.");
                std::process::exit(1);
            }

            let sync_config = sync::SyncConfig {
                preserve_existing: *preserve_existing,
                no_downgrade: *no_downgrade,
            };

            let source_viewer_name = source_client.get_viewer_name().await?;
            let target_viewer_name = target_client.get_viewer_name().await?;

            let source_viewer_id = source_client.get_viewer_id().await?;
            let target_viewer_id = target_client.get_viewer_id().await?;

            print_info(&format!("Source User: {source_viewer_name}"));
            print_info(&format!("Target User: {target_viewer_name}"));

            if sync_anime_pref {
                print_info("  - Anime: ENABLED");
            }
            if sync_manga_pref {
                print_info("  - Manga: ENABLED");
            }

            let mut sync_actions = Vec::new();

            if sync_anime_pref {
                print_info(&format!("Fetching {source_svc} Anime data..."));
                let source_anime = source_client.fetch_anime_list(&source_viewer_id).await?;

                print_info(&format!("Fetching {target_svc} Anime data..."));
                let target_anime = target_client.fetch_anime_list(&target_viewer_id).await?;

                let results_anime = sync::SyncManager::compare_lists(
                    &source_anime,
                    &target_anime,
                    target_client.as_ref(),
                    sync_config,
                );
                for res in results_anime {
                    if let Some(action) = sync::SyncManager::generate_actions(
                        &source_svc,
                        &target_svc,
                        &res,
                        Some(target_client.supported_ids()),
                        sync_config,
                    ) {
                        sync_actions.push(action);
                    }
                }
            }

            if sync_manga_pref {
                print_info(&format!("Fetching {source_svc} Manga data..."));
                let source_manga = source_client.fetch_manga_list(&source_viewer_id).await?;

                print_info(&format!("Fetching {target_svc} Manga data..."));
                let target_manga = target_client.fetch_manga_list(&target_viewer_id).await?;

                let results_manga = sync::SyncManager::compare_lists(
                    &source_manga,
                    &target_manga,
                    target_client.as_ref(),
                    sync_config,
                );
                for res in results_manga {
                    if let Some(action) = sync::SyncManager::generate_actions(
                        &source_svc,
                        &target_svc,
                        &res,
                        Some(target_client.supported_ids()),
                        sync_config,
                    ) {
                        sync_actions.push(action);
                    }
                }
            }
            if sync_actions.is_empty() {
                print_success("No changes needed. All lists are in sync.");
                return Ok(());
            }

            print_info("--- SYNC PLAN ---");

            let adds: Vec<_> = sync_actions
                .iter()
                .filter(|a| a.action == models::ActionType::Add)
                .collect();
            let updates: Vec<_> = sync_actions
                .iter()
                .filter(|a| a.action == models::ActionType::Update)
                .collect();
            let skips: Vec<_> = sync_actions
                .iter()
                .filter(|a| a.action == models::ActionType::Skip)
                .collect();

            let error_skips: Vec<_> = skips
                .into_iter()
                .filter(|s| {
                    s.reasons.iter().any(|r| r.field_name != "status")
                        || s.reasons.iter().any(|r| {
                            r.new_value == "Downgrade Prevented"
                                || r.new_value == "Preserved existing target"
                        })
                })
                .collect();

            render_action_table("SKIPPED", &error_skips, AnsiColors::Red);
            render_action_table("ADD", &adds, AnsiColors::Cyan);
            render_update_action_table("UPDATE", &updates, AnsiColors::Yellow);

            print_info("-----------------");

            if adds.is_empty() && updates.is_empty() {
                print_info("No actionable changes to apply.");
                return Ok(());
            }

            if !*yes {
                let ans = prompt_for_input("Proceed with these changes? [y/N]: ")?;
                if ans.to_lowercase() != "y" && ans.to_lowercase() != "yes" {
                    print_info("Sync aborted.");
                    std::process::exit(1);
                }
            }

            print_info("Executing sync plan...");
            sync::SyncManager::execute_sync(sync_actions, target_client.as_ref()).await;

            print_success("Sync complete!");
        }
        Commands::Status => {
            print_info("Checking status...");
            let mal_status = match storage::get_token_bundle("mal") {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(e) => {
                    tracing::error!("Error retrieving MAL token bundle: {:?}", e);
                    false
                }
            };
            let anilist_status = match storage::get_token_bundle("anilist") {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(e) => {
                    tracing::error!("Error retrieving AniList token bundle: {:?}", e);
                    false
                }
            };
            let kitsu_status = match storage::get_token_bundle("kitsu") {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(e) => {
                    tracing::error!("Error retrieving Kitsu token bundle: {:?}", e);
                    false
                }
            };
            let mangabaka_status = match storage::get_token_bundle("mangabaka") {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(e) => {
                    tracing::error!("Error retrieving MangaBaka token bundle: {:?}", e);
                    false
                }
            };

            print_info(&format!(
                "MyAnimeList: {}",
                if mal_status {
                    "Authenticated"
                } else {
                    "Not Authenticated"
                }
            ));
            print_info(&format!(
                "AniList: {}",
                if anilist_status {
                    "Authenticated"
                } else {
                    "Not Authenticated"
                }
            ));
            print_info(&format!(
                "Kitsu: {}",
                if kitsu_status {
                    "Authenticated"
                } else {
                    "Not Authenticated"
                }
            ));
            print_info(&format!(
                "MangaBaka: {}",
                if mangabaka_status {
                    "Authenticated"
                } else {
                    "Not Authenticated"
                }
            ));
        }
    }

    Ok(())
}
