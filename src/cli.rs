use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authenticate with a service
    Auth {
        #[command(subcommand)]
        provider: AuthProvider,
    },
    /// Sync lists between two services
    Sync {
        #[arg(short, long, value_name = "SOURCE")]
        source: Option<String>,

        #[arg(short, long, value_name = "TARGET")]
        target: Option<String>,

        #[arg(long, help = "Sync anime lists")]
        anime: bool,

        #[arg(long, help = "Do not sync anime lists", conflicts_with = "anime")]
        no_anime: bool,

        #[arg(long, help = "Sync manga lists")]
        manga: bool,

        #[arg(long, help = "Do not sync manga lists", conflicts_with = "manga")]
        no_manga: bool,

        #[arg(
            short,
            long,
            help = "Apply changes immediately without prompting",
            alias = "apply"
        )]
        yes: bool,

        #[arg(
            long,
            help = "Prevent overwriting a higher target status/progress with a lower source value"
        )]
        no_downgrade: bool,

        #[arg(long, help = "Skip syncing an item if it already exists on the target")]
        preserve_existing: bool,
    },
    /// Show authentication status for services
    Status,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum AuthProvider {
    /// Authenticate with `MyAnimeList`
    Mal,
    /// Authenticate with `AniList`
    Anilist,
    /// Authenticate with `Kitsu`
    Kitsu,
    /// Authenticate with `MangaBaka`
    Mangabaka,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing_auth() {
        let args = vec!["ani_sync", "auth", "mal"];
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Auth { provider } => assert_eq!(provider, AuthProvider::Mal),
            _ => panic!("Expected Auth command"),
        }
    }

    #[test]
    fn test_cli_parsing_sync() {
        let args = vec!["ani_sync", "sync", "-s", "mal", "-t", "anilist"];
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Sync { source, target, .. } => {
                assert_eq!(source, Some("mal".to_string()));
                assert_eq!(target, Some("anilist".to_string()));
            }
            _ => panic!("Expected Sync command"),
        }
    }

    #[test]
    fn test_cli_parsing_sync_conditional_flags() {
        let args = vec![
            "ani_sync",
            "sync",
            "-s",
            "mal",
            "-t",
            "anilist",
            "--no-downgrade",
            "--preserve-existing",
        ];
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Sync {
                source,
                target,
                no_downgrade,
                preserve_existing,
                ..
            } => {
                assert_eq!(source, Some("mal".to_string()));
                assert_eq!(target, Some("anilist".to_string()));
                assert!(no_downgrade);
                assert!(preserve_existing);
            }
            _ => panic!("Expected Sync command"),
        }
    }
}
