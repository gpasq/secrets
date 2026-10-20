//! Thin wrapper around the Secrets Manager SDK that returns plain domain types.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use aws_config::{BehaviorVersion, Region};
use aws_sdk_secretsmanager::Client;
use aws_sdk_secretsmanager::error::DisplayErrorContext;
use aws_sdk_secretsmanager::primitives::Blob;
use aws_sdk_secretsmanager::types::{Filter, FilterNameStringType, RotationRulesType, Tag};
use chrono::{DateTime, Utc};

/// How to connect: everything optional, falling back to the standard AWS
/// credential/region chain (env vars, ~/.aws/config, SSO, IMDS, ...).
#[derive(Clone, Debug, Default)]
pub struct ConnOpts {
    pub profile: Option<String>,
    pub region: Option<String>,
    pub endpoint_url: Option<String>,
    /// Explicit keys (a saved connection) instead of the profile's credentials.
    pub keys: Option<StaticKeys>,
    /// Role to assume with the credentials above.
    pub role_arn: Option<String>,
    pub external_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct StaticKeys {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

impl std::fmt::Debug for StaticKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticKeys")
            .field("access_key_id", &self.access_key_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct Sm {
    client: Client,
    pub region: String,
}

#[derive(Clone, Debug)]
pub struct SecretSummary {
    pub name: String,
    pub description: Option<String>,
    pub last_changed: Option<DateTime<Utc>>,
    pub deleted: Option<DateTime<Utc>>,
    pub rotation_enabled: bool,
    pub tags: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct SecretDetail {
    pub name: String,
    pub arn: String,
    pub description: Option<String>,
    pub kms_key_id: Option<String>,
    pub created: Option<DateTime<Utc>>,
    pub last_changed: Option<DateTime<Utc>>,
    pub last_accessed: Option<DateTime<Utc>>,
    pub deleted: Option<DateTime<Utc>>,
    pub rotation_enabled: bool,
    pub rotation_lambda: Option<String>,
    pub rotation_days: Option<i64>,
    pub rotation_schedule: Option<String>,
    pub rotation_window: Option<String>,
    pub last_rotated: Option<DateTime<Utc>>,
    pub next_rotation: Option<DateTime<Utc>>,
    pub owning_service: Option<String>,
    pub primary_region: Option<String>,
    pub tags: Vec<(String, String)>,
    /// version id -> staging labels
    pub versions: BTreeMap<String, Vec<String>>,
}

impl SecretDetail {
    /// Version id currently carrying the given staging label.
    pub fn version_with_stage(&self, stage: &str) -> Option<&str> {
        self.versions
            .iter()
            .find(|(_, stages)| stages.iter().any(|s| s == stage))
            .map(|(id, _)| id.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretValue {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct FetchedValue {
    pub value: SecretValue,
    pub version_id: Option<String>,
    pub stages: Vec<String>,
    pub created: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct VersionInfo {
    pub id: String,
    pub stages: Vec<String>,
    pub created: Option<DateTime<Utc>>,
    pub last_accessed: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default)]
pub struct ListFilter {
    pub all: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub tag_key: Option<String>,
    pub tag_value: Option<String>,
    pub include_deleted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RotationConfig {
    pub lambda_arn: Option<String>,
    pub days: Option<i64>,
    pub schedule: Option<String>,
    pub window: Option<String>,
    pub rotate_immediately: bool,
}

pub enum Deletion {
    RecoveryWindow(i64),
    Force,
}

/// Format SDK errors with their full context (the default Display is just "service error"),
/// adding a hint for the most common SSO and `aws login` failures.
pub fn sdk_err<E: std::error::Error>(e: E) -> anyhow::Error {
    let msg = DisplayErrorContext(&e).to_string();
    let lower = msg.to_lowercase();
    if lower.contains("login token") && lower.contains("expired") {
        anyhow!(
            "{msg}\n\nhint: your console login session has expired; run `aws login --profile <profile>`"
        )
    } else if lower.contains("sso") && (lower.contains("token") || lower.contains("expired")) {
        anyhow!(
            "{msg}\n\nhint: your SSO session may have expired; run `aws sso login --profile <profile>`"
        )
    } else {
        anyhow!(msg)
    }
}

fn ts(d: Option<&aws_smithy_types::DateTime>) -> Option<DateTime<Utc>> {
    d.and_then(|d| DateTime::from_timestamp(d.secs(), d.subsec_nanos()))
}

fn tags_of(tags: &[Tag]) -> Vec<(String, String)> {
    tags.iter()
        .map(|t| {
            (
                t.key().unwrap_or_default().to_string(),
                t.value().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn to_tags(tags: &[(String, String)]) -> Vec<Tag> {
    tags.iter()
        .map(|(k, v)| Tag::builder().key(k).value(v).build())
        .collect()
}

impl Sm {
    pub async fn connect(opts: &ConnOpts) -> Result<Self> {
        let mut loader = aws_config::defaults(BehaviorVersion::latest());
        if let Some(p) = &opts.profile {
            loader = loader.profile_name(p);
        }
        if let Some(r) = &opts.region {
            loader = loader.region(Region::new(r.clone()));
        }
        if let Some(e) = &opts.endpoint_url {
            loader = loader.endpoint_url(e);
        }
        if let Some(k) = &opts.keys {
            loader = loader.credentials_provider(aws_sdk_secretsmanager::config::Credentials::new(
                k.access_key_id.trim(),
                k.secret_access_key.trim(),
                k.session_token.clone().filter(|t| !t.trim().is_empty()),
                None,
                "secrets-app",
            ));
        }
        let mut conf = loader.load().await;
        if let Some(role) = opts.role_arn.as_deref().filter(|r| !r.trim().is_empty()) {
            let base = conf
                .credentials_provider()
                .ok_or_else(|| anyhow!("no credentials available to assume {role}"))?;
            let mut builder = aws_config::sts::AssumeRoleProvider::builder(role.trim())
                .session_name("secrets-app")
                .configure(&conf);
            if let Some(id) = opts.external_id.as_deref().filter(|i| !i.trim().is_empty()) {
                builder = builder.external_id(id.trim());
            }
            let provider = builder.build_from_provider(base).await;
            conf = conf
                .into_builder()
                .credentials_provider(
                    aws_sdk_secretsmanager::config::SharedCredentialsProvider::new(provider),
                )
                .build();
        }
        let region = conf
            .region()
            .map(|r| r.to_string())
            .filter(|r| !r.is_empty())
            .ok_or_else(|| {
            anyhow!(
                "no AWS region configured; pass --region, set AWS_REGION, or add `region` to your profile"
            )
        })?;
        Ok(Self {
            client: Client::new(&conf),
            region,
        })
    }

    /// Cheap call to confirm the credentials work. Returns how many secrets are visible
    /// on the first page.
    pub async fn check(&self) -> Result<usize> {
        let r = self
            .client
            .list_secrets()
            .max_results(20)
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(r.secret_list().len())
    }

    pub async fn list(&self, f: &ListFilter) -> Result<Vec<SecretSummary>> {
        let mut filters = Vec::new();
        let mut add = |key: FilterNameStringType, v: &Option<String>| {
            if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
                filters.push(Filter::builder().key(key).values(v).build());
            }
        };
        add(FilterNameStringType::All, &f.all);
        add(FilterNameStringType::Name, &f.name);
        add(FilterNameStringType::Description, &f.description);
        add(FilterNameStringType::TagKey, &f.tag_key);
        add(FilterNameStringType::TagValue, &f.tag_value);

        let entries = self
            .client
            .list_secrets()
            .set_filters((!filters.is_empty()).then_some(filters))
            .include_planned_deletion(f.include_deleted)
            .max_results(100)
            .into_paginator()
            .send()
            .collect::<Result<Vec<_>, _>>()
            .await
            .map_err(sdk_err)?;

        let mut out: Vec<SecretSummary> = entries
            .iter()
            .flat_map(|page| page.secret_list())
            .map(|e| SecretSummary {
                name: e.name().unwrap_or_default().to_string(),
                description: e.description().map(str::to_string),
                last_changed: ts(e.last_changed_date()),
                deleted: ts(e.deleted_date()),
                rotation_enabled: e.rotation_enabled().unwrap_or(false),
                tags: tags_of(e.tags()),
            })
            .collect();
        out.sort_by_key(|s| s.name.to_lowercase());
        Ok(out)
    }

    pub async fn describe(&self, id: &str) -> Result<SecretDetail> {
        let d = self
            .client
            .describe_secret()
            .secret_id(id)
            .send()
            .await
            .map_err(sdk_err)?;
        let rules = d.rotation_rules();
        Ok(SecretDetail {
            name: d.name().unwrap_or_default().to_string(),
            arn: d.arn().unwrap_or_default().to_string(),
            description: d.description().map(str::to_string),
            kms_key_id: d.kms_key_id().map(str::to_string),
            created: ts(d.created_date()),
            last_changed: ts(d.last_changed_date()),
            last_accessed: ts(d.last_accessed_date()),
            deleted: ts(d.deleted_date()),
            rotation_enabled: d.rotation_enabled().unwrap_or(false),
            rotation_lambda: d.rotation_lambda_arn().map(str::to_string),
            rotation_days: rules.and_then(|r| r.automatically_after_days()),
            rotation_schedule: rules
                .and_then(|r| r.schedule_expression())
                .map(str::to_string),
            rotation_window: rules.and_then(|r| r.duration()).map(str::to_string),
            last_rotated: ts(d.last_rotated_date()),
            next_rotation: ts(d.next_rotation_date()),
            owning_service: d.owning_service().map(str::to_string),
            primary_region: d.primary_region().map(str::to_string),
            tags: tags_of(d.tags()),
            versions: d
                .version_ids_to_stages()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default(),
        })
    }

    pub async fn get_value(
        &self,
        id: &str,
        version_id: Option<&str>,
        stage: Option<&str>,
    ) -> Result<FetchedValue> {
        let r = self
            .client
            .get_secret_value()
            .secret_id(id)
            .set_version_id(version_id.map(str::to_string))
            .set_version_stage(stage.map(str::to_string))
            .send()
            .await
            .map_err(sdk_err)?;
        let value = if let Some(s) = r.secret_string() {
            SecretValue::Text(s.to_string())
        } else if let Some(b) = r.secret_binary() {
            SecretValue::Binary(b.as_ref().to_vec())
        } else {
            SecretValue::Text(String::new())
        };
        Ok(FetchedValue {
            value,
            version_id: r.version_id().map(str::to_string),
            stages: r.version_stages().to_vec(),
            created: ts(r.created_date()),
        })
    }

    /// Returns the ARN and the initial version id.
    pub async fn create(
        &self,
        name: &str,
        description: Option<&str>,
        kms_key_id: Option<&str>,
        tags: &[(String, String)],
        value: Option<&SecretValue>,
    ) -> Result<(String, Option<String>)> {
        let mut req = self
            .client
            .create_secret()
            .name(name)
            .set_description(description.map(str::to_string))
            .set_kms_key_id(kms_key_id.map(str::to_string))
            .set_tags((!tags.is_empty()).then(|| to_tags(tags)));
        match value {
            Some(SecretValue::Text(s)) => req = req.secret_string(s),
            Some(SecretValue::Binary(b)) => req = req.secret_binary(Blob::new(b.clone())),
            None => {}
        }
        let r = req.send().await.map_err(sdk_err)?;
        Ok((
            r.arn().unwrap_or_default().to_string(),
            r.version_id().map(str::to_string),
        ))
    }

    /// Store a new version. Returns the new version id.
    pub async fn put_value(
        &self,
        id: &str,
        value: &SecretValue,
        stages: &[String],
    ) -> Result<Option<String>> {
        let mut req = self
            .client
            .put_secret_value()
            .secret_id(id)
            .set_version_stages((!stages.is_empty()).then(|| stages.to_vec()));
        req = match value {
            SecretValue::Text(s) => req.secret_string(s),
            SecretValue::Binary(b) => req.secret_binary(Blob::new(b.clone())),
        };
        let r = req.send().await.map_err(sdk_err)?;
        Ok(r.version_id().map(str::to_string))
    }

    pub async fn update_metadata(
        &self,
        id: &str,
        description: Option<&str>,
        kms_key_id: Option<&str>,
    ) -> Result<()> {
        self.client
            .update_secret()
            .secret_id(id)
            .set_description(description.map(str::to_string))
            .set_kms_key_id(kms_key_id.map(str::to_string))
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(())
    }

    pub async fn tag(&self, id: &str, tags: &[(String, String)]) -> Result<()> {
        if tags.is_empty() {
            return Ok(());
        }
        self.client
            .tag_resource()
            .secret_id(id)
            .set_tags(Some(to_tags(tags)))
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(())
    }

    pub async fn untag(&self, id: &str, keys: &[String]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        self.client
            .untag_resource()
            .secret_id(id)
            .set_tag_keys(Some(keys.to_vec()))
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(())
    }

    pub async fn versions(&self, id: &str, include_deprecated: bool) -> Result<Vec<VersionInfo>> {
        let entries = self
            .client
            .list_secret_version_ids()
            .secret_id(id)
            .include_deprecated(include_deprecated)
            .into_paginator()
            .send()
            .collect::<Result<Vec<_>, _>>()
            .await
            .map_err(sdk_err)?;
        let mut out: Vec<VersionInfo> = entries
            .iter()
            .flat_map(|page| page.versions())
            .map(|v| VersionInfo {
                id: v.version_id().unwrap_or_default().to_string(),
                stages: v.version_stages().to_vec(),
                created: ts(v.created_date()),
                last_accessed: ts(v.last_accessed_date()),
            })
            .collect();
        out.sort_by_key(|v| std::cmp::Reverse(v.created));
        Ok(out)
    }

    /// Move a staging label to `to_version`. If the label is currently attached to another
    /// version, `from_version` must name it (AWS requires this to move a label).
    pub async fn move_stage(
        &self,
        id: &str,
        stage: &str,
        to_version: Option<&str>,
        from_version: Option<&str>,
    ) -> Result<()> {
        self.client
            .update_secret_version_stage()
            .secret_id(id)
            .version_stage(stage)
            .set_move_to_version_id(to_version.map(str::to_string))
            .set_remove_from_version_id(from_version.map(str::to_string))
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(())
    }

    /// Make `version` the AWSCURRENT version (rollback/restore a previous version).
    pub async fn promote(&self, id: &str, version: &str) -> Result<()> {
        let detail = self.describe(id).await?;
        let current = detail.version_with_stage("AWSCURRENT").map(str::to_string);
        if current.as_deref() == Some(version) {
            return Ok(());
        }
        self.move_stage(id, "AWSCURRENT", Some(version), current.as_deref())
            .await
    }

    /// Returns the scheduled deletion date (if any).
    pub async fn delete(&self, id: &str, how: Deletion) -> Result<Option<DateTime<Utc>>> {
        let req = self.client.delete_secret().secret_id(id);
        let req = match how {
            Deletion::RecoveryWindow(days) => req.recovery_window_in_days(days),
            Deletion::Force => req.force_delete_without_recovery(true),
        };
        let r = req.send().await.map_err(sdk_err)?;
        Ok(ts(r.deletion_date()))
    }

    pub async fn restore(&self, id: &str) -> Result<()> {
        self.client
            .restore_secret()
            .secret_id(id)
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(())
    }

    /// Configure rotation (and optionally rotate immediately). With an empty config this
    /// simply triggers a rotation using the existing configuration.
    pub async fn rotate(&self, id: &str, cfg: &RotationConfig) -> Result<Option<String>> {
        let rules =
            (cfg.days.is_some() || cfg.schedule.is_some() || cfg.window.is_some()).then(|| {
                RotationRulesType::builder()
                    .set_automatically_after_days(cfg.days)
                    .set_schedule_expression(cfg.schedule.clone())
                    .set_duration(cfg.window.clone())
                    .build()
            });
        let r = self
            .client
            .rotate_secret()
            .secret_id(id)
            .set_rotation_lambda_arn(cfg.lambda_arn.clone())
            .set_rotation_rules(rules)
            .rotate_immediately(cfg.rotate_immediately)
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(r.version_id().map(str::to_string))
    }

    pub async fn cancel_rotation(&self, id: &str) -> Result<()> {
        self.client
            .cancel_rotate_secret()
            .secret_id(id)
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(())
    }

    pub async fn random_password(&self, length: i64, exclude_punctuation: bool) -> Result<String> {
        let r = self
            .client
            .get_random_password()
            .password_length(length)
            .exclude_punctuation(exclude_punctuation)
            .send()
            .await
            .map_err(sdk_err)?;
        Ok(r.random_password().unwrap_or_default().to_string())
    }
}
