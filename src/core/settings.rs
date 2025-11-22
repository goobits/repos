use serde::Deserialize;
use std::collections::HashSet;
use std::sync::OnceLock;

/// Global settings instance
static SETTINGS: OnceLock<Settings> = OnceLock::new();

/// Application configuration settings
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub timeouts: TimeoutSettings,
    #[serde(default)]
    pub patterns: PatternSettings,
    #[serde(default)]
    pub discovery: DiscoverySettings,
}

impl Settings {
    /// Load settings from configuration file or defaults
    pub fn load() -> &'static Settings {
        SETTINGS.get_or_init(|| {
            // Try to find config file
            let config_names = [".goobits.toml", "goobits.toml", "repos.toml"];

            for name in &config_names {
                if let Ok(content) = std::fs::read_to_string(name) {
                    if let Ok(settings) = toml::from_str::<Settings>(&content) {
                        return settings;
                    }
                }
            }

            // Fallback to defaults
            Settings::default()
        })
    }

    /// Get a reference to the global settings
    pub fn get() -> &'static Settings {
        Self::load()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimeoutSettings {
    pub git_operation: u64,
    pub npm_operation: u64,
    pub cargo_operation: u64,
    pub python_operation: u64,
}

impl Default for TimeoutSettings {
    fn default() -> Self {
        Self {
            git_operation: 180,
            npm_operation: 300,
            cargo_operation: 600,
            python_operation: 300,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PatternSettings {
    pub hygiene_bad_patterns: Vec<String>,
}

impl Default for PatternSettings {
    fn default() -> Self {
        Self {
            hygiene_bad_patterns: vec![
                "node_modules/".to_string(),
                "vendor/".to_string(),
                "dist/".to_string(),
                "build/".to_string(),
                "target/debug/".to_string(),
                "target/release/".to_string(),
                ".env".to_string(),
                "*.log".to_string(),
                ".DS_Store".to_string(),
                "Thumbs.db".to_string(),
                "*.tmp".to_string(),
                "*.cache".to_string(),
                "__pycache__/".to_string(),
                ".venv/".to_string(),
                ".idea/".to_string(),
                ".vscode/settings.json".to_string(),
                "*.key".to_string(),
                "*.pem".to_string(),
                "*.p12".to_string(),
                "*.jks".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoverySettings {
    pub skip_directories: HashSet<String>,
    pub max_depth: usize,
}

impl Default for DiscoverySettings {
    fn default() -> Self {
        let skip = vec![
            "node_modules",
            "vendor",
            "target",
            "build",
            ".next",
            "dist",
            "__pycache__",
            ".venv",
            "venv",
        ];

        Self {
            skip_directories: skip.into_iter().map(String::from).collect(),
            max_depth: 10,
        }
    }
}
