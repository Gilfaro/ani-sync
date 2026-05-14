// Rust guideline compliant 2026-02-21

//! `Ani-Sync` is a CLI tool to synchronize anime and manga tracking across multiple services.
//!
//! Supported services include `MyAnimeList`, `AniList`, `Kitsu`, and `MangaBaka`.

pub mod anilist;
pub mod auth;
pub mod cli;
pub mod client;
pub mod kitsu;
pub mod mal;
pub mod mangabaka;
pub mod models;
pub mod storage;
pub mod sync;
pub mod ui;
