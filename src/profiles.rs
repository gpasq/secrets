//! Discover AWS profile names from the shared config and credentials files.

use std::path::PathBuf;

/// Common regions offered by the TUI region picker (any region can be passed via --region).
pub const REGIONS: &[&str] = &[
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
    "ca-central-1",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-central-1",
    "eu-central-2",
    "eu-north-1",
    "eu-south-1",
    "ap-south-1",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "sa-east-1",
    "me-central-1",
    "af-south-1",
];

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("AWS_CONFIG_FILE")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".aws").join("config")))
}

fn credentials_path() -> Option<PathBuf> {
    std::env::var_os("AWS_SHARED_CREDENTIALS_FILE")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".aws").join("credentials")))
}

/// Extract profile names from an INI-style AWS file. In the config file, sections are
/// `[default]` or `[profile NAME]` (and non-profile sections such as `[sso-session x]` are
/// skipped); in the credentials file, every section is a profile.
pub fn parse_profile_names(text: &str, is_config_file: bool) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let inner = line.strip_prefix('[')?.strip_suffix(']')?.trim();
            if !is_config_file || inner == "default" {
                return Some(inner.to_string());
            }
            inner
                .strip_prefix("profile")
                .filter(|rest| rest.starts_with(char::is_whitespace))
                .map(|rest| rest.trim().to_string())
        })
        .filter(|n| !n.is_empty())
        .collect()
}

/// All known profile names, `default` first, then alphabetical.
pub fn list_profiles() -> Vec<String> {
    let mut names = Vec::new();
    for (path, is_config) in [(config_path(), true), (credentials_path(), false)] {
        if let Some(text) = path.and_then(|p| std::fs::read_to_string(p).ok()) {
            names.extend(parse_profile_names(&text, is_config));
        }
    }
    sort_profiles(names)
}

pub fn sort_profiles(mut names: Vec<String>) -> Vec<String> {
    names.sort_by(|a, b| (a != "default", a).cmp(&(b != "default", b)));
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_file() {
        let text = "\
[default]
region = us-east-1

[profile dev]
sso_session = corp
[profile   prod-admin ]
[sso-session corp]
sso_start_url = https://example.awsapps.com/start
[services local]
[profilex]
";
        assert_eq!(
            parse_profile_names(text, true),
            vec!["default", "dev", "prod-admin"]
        );
    }

    #[test]
    fn parses_credentials_file() {
        let text = "[default]\naws_access_key_id=x\n[ci]\n";
        assert_eq!(parse_profile_names(text, false), vec!["default", "ci"]);
    }

    #[test]
    fn sorts_default_first_and_dedups() {
        let v = sort_profiles(vec!["b".into(), "default".into(), "a".into(), "b".into()]);
        assert_eq!(v, vec!["default", "a", "b"]);
    }
}
