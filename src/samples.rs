//! Sample code for reading a secret with each language's official AWS SDK.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    JavaScript,
    TypeScript,
    Java,
    Rust,
    Go,
    Python,
    CSharp,
    Ruby,
    Php,
}

impl Lang {
    pub const ALL: [Lang; 9] = [
        Lang::JavaScript,
        Lang::TypeScript,
        Lang::Python,
        Lang::Go,
        Lang::Java,
        Lang::CSharp,
        Lang::Rust,
        Lang::Ruby,
        Lang::Php,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Lang::JavaScript => "JavaScript",
            Lang::TypeScript => "TypeScript",
            Lang::Java => "Java",
            Lang::Rust => "Rust",
            Lang::Go => "Go",
            Lang::Python => "Python",
            Lang::CSharp => "C#",
            Lang::Ruby => "Ruby",
            Lang::Php => "PHP",
        }
    }

    /// How to add the SDK to a project.
    pub fn install(self) -> &'static str {
        match self {
            Lang::JavaScript => "npm install @aws-sdk/client-secrets-manager",
            Lang::TypeScript => "npm install @aws-sdk/client-secrets-manager",
            Lang::Java => {
                "<!-- pom.xml (use the AWS SDK BOM to manage versions) -->\n\
                 <dependency>\n  \
                   <groupId>software.amazon.awssdk</groupId>\n  \
                   <artifactId>secretsmanager</artifactId>\n\
                 </dependency>\n\
                 <dependency>\n  \
                   <groupId>com.fasterxml.jackson.core</groupId>\n  \
                   <artifactId>jackson-databind</artifactId>\n\
                 </dependency>"
            }
            Lang::Rust => {
                "cargo add aws-config aws-sdk-secretsmanager serde_json\n\
                 cargo add tokio --features full"
            }
            Lang::Go => {
                "go get github.com/aws/aws-sdk-go-v2/config \\\n  \
                   github.com/aws/aws-sdk-go-v2/service/secretsmanager"
            }
            Lang::Python => "pip install boto3",
            Lang::CSharp => "dotnet add package AWSSDK.SecretsManager",
            Lang::Ruby => "gem install aws-sdk-secretsmanager",
            Lang::Php => "composer require aws/aws-sdk-php",
        }
    }
}

/// What the secret holds, which decides how the sample reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// A JSON object: its keys and a sample JSON value per key.
    Json(Vec<(String, Value)>),
    Text,
    Binary,
}

impl Shape {
    /// The key the sample reads: `password` if there is one, else the first key.
    fn key(&self) -> Option<&str> {
        match self {
            Shape::Json(keys) => keys
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("password"))
                .or_else(|| keys.first())
                .map(|(k, _)| k.as_str()),
            _ => None,
        }
    }
}

/// A double-quoted string literal (secret names and keys never contain newlines).
fn dq(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A single-quoted string literal (PHP).
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn ts_field(key: &str) -> String {
    let ident = !key.is_empty()
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if ident { key.to_string() } else { dq(key) }
}

fn ts_type(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        _ => "unknown",
    }
}

/// `prod/db-credentials` -> `ProdDbCredentials` (for TypeScript interface names).
fn pascal(name: &str) -> String {
    let mut out = String::new();
    for part in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert_str(0, "Secret");
    }
    out
}

pub fn sample(lang: Lang, name: &str, region: &str, shape: &Shape) -> String {
    let n = dq(name);
    let r = dq(region);
    let key = shape.key();
    match lang {
        Lang::JavaScript => {
            let read = match (shape, key) {
                (Shape::Binary, _) => "const secret = response.SecretBinary; // Uint8Array\n\
                     console.log(secret.length, \"bytes\");"
                    .to_string(),
                (Shape::Json(_), Some(k)) => format!(
                    "const secret = JSON.parse(response.SecretString);\nconsole.log(secret[{}]);",
                    dq(k)
                ),
                _ => "const secret = response.SecretString;\nconsole.log(secret);".to_string(),
            };
            format!(
                "import {{\n  SecretsManagerClient,\n  GetSecretValueCommand,\n}} from \"@aws-sdk/client-secrets-manager\";\n\n\
                 // Credentials come from the default AWS chain (env vars, ~/.aws, IAM role).\n\
                 const client = new SecretsManagerClient({{ region: {r} }});\n\n\
                 const response = await client.send(\n  new GetSecretValueCommand({{ SecretId: {n} }}),\n);\n\
                 {read}\n"
            )
        }
        Lang::TypeScript => {
            let (iface, read) = match (shape, key) {
                (Shape::Json(keys), Some(k)) => {
                    let ty = pascal(name);
                    let fields: String = keys
                        .iter()
                        .map(|(k, v)| format!("  {}: {};\n", ts_field(k), ts_type(v)))
                        .collect();
                    (
                        format!("interface {ty} {{\n{fields}}}\n\n"),
                        format!(
                            "const secret: {ty} = JSON.parse(response.SecretString ?? \"{{}}\");\nconsole.log(secret[{}]);",
                            dq(k)
                        ),
                    )
                }
                (Shape::Binary, _) => (
                    String::new(),
                    "const secret: Uint8Array | undefined = response.SecretBinary;\nconsole.log(secret?.length, \"bytes\");"
                        .to_string(),
                ),
                _ => (
                    String::new(),
                    "const secret: string | undefined = response.SecretString;\nconsole.log(secret);"
                        .to_string(),
                ),
            };
            format!(
                "import {{\n  SecretsManagerClient,\n  GetSecretValueCommand,\n}} from \"@aws-sdk/client-secrets-manager\";\n\n\
                 {iface}\
                 // Credentials come from the default AWS chain (env vars, ~/.aws, IAM role).\n\
                 const client = new SecretsManagerClient({{ region: {r} }});\n\n\
                 const response = await client.send(\n  new GetSecretValueCommand({{ SecretId: {n} }}),\n);\n\
                 {read}\n"
            )
        }
        Lang::Python => {
            let (imports, read) = match (shape, key) {
                (Shape::Json(_), Some(k)) => (
                    "import json\n\nimport boto3\n",
                    format!(
                        "secret = json.loads(response[\"SecretString\"])\nprint(secret[{}])",
                        dq(k)
                    ),
                ),
                (Shape::Binary, _) => (
                    "import boto3\n",
                    "secret = response[\"SecretBinary\"]  # bytes\nprint(len(secret), \"bytes\")"
                        .to_string(),
                ),
                _ => (
                    "import boto3\n",
                    "secret = response[\"SecretString\"]\nprint(secret)".to_string(),
                ),
            };
            format!(
                "{imports}\n\
                 # Credentials come from the default AWS chain (env vars, ~/.aws, IAM role).\n\
                 client = boto3.client(\"secretsmanager\", region_name={r})\n\n\
                 response = client.get_secret_value(SecretId={n})\n\
                 {read}\n"
            )
        }
        Lang::Go => {
            let (extra_import, read) = match (shape, key) {
                (Shape::Json(_), Some(k)) => (
                    "\t\"encoding/json\"\n",
                    format!(
                        "\tvar secret map[string]any\n\
                         \tif err := json.Unmarshal([]byte(*out.SecretString), &secret); err != nil {{\n\
                         \t\tlog.Fatal(err)\n\t}}\n\
                         \tfmt.Println(secret[{}])",
                        dq(k)
                    ),
                ),
                (Shape::Binary, _) => (
                    "",
                    "\tsecret := out.SecretBinary // []byte\n\tfmt.Println(len(secret), \"bytes\")"
                        .to_string(),
                ),
                _ => (
                    "",
                    "\tsecret := aws.ToString(out.SecretString)\n\tfmt.Println(secret)".to_string(),
                ),
            };
            format!(
                "package main\n\nimport (\n\t\"context\"\n{extra_import}\t\"fmt\"\n\t\"log\"\n\n\
                 \t\"github.com/aws/aws-sdk-go-v2/aws\"\n\
                 \t\"github.com/aws/aws-sdk-go-v2/config\"\n\
                 \t\"github.com/aws/aws-sdk-go-v2/service/secretsmanager\"\n)\n\n\
                 func main() {{\n\
                 \tctx := context.Background()\n\
                 \t// Credentials come from the default AWS chain (env vars, ~/.aws, IAM role).\n\
                 \tcfg, err := config.LoadDefaultConfig(ctx, config.WithRegion({r}))\n\
                 \tif err != nil {{\n\t\tlog.Fatal(err)\n\t}}\n\
                 \tclient := secretsmanager.NewFromConfig(cfg)\n\n\
                 \tout, err := client.GetSecretValue(ctx, &secretsmanager.GetSecretValueInput{{\n\
                 \t\tSecretId: aws.String({n}),\n\t}})\n\
                 \tif err != nil {{\n\t\tlog.Fatal(err)\n\t}}\n\
                 {read}\n}}\n"
            )
        }
        Lang::Java => {
            let (imports, read) = match (shape, key) {
                (Shape::Json(_), Some(k)) => (
                    "import com.fasterxml.jackson.databind.JsonNode;\nimport com.fasterxml.jackson.databind.ObjectMapper;\n",
                    format!(
                        "            JsonNode secret = new ObjectMapper().readTree(response.secretString());\n\
                         \x20           System.out.println(secret.get({}).asText());",
                        dq(k)
                    ),
                ),
                (Shape::Binary, _) => (
                    "",
                    "            byte[] secret = response.secretBinary().asByteArray();\n\
                     \x20           System.out.println(secret.length + \" bytes\");"
                        .to_string(),
                ),
                _ => (
                    "",
                    "            String secret = response.secretString();\n\
                     \x20           System.out.println(secret);"
                        .to_string(),
                ),
            };
            format!(
                "{imports}import software.amazon.awssdk.regions.Region;\n\
                 import software.amazon.awssdk.services.secretsmanager.SecretsManagerClient;\n\
                 import software.amazon.awssdk.services.secretsmanager.model.GetSecretValueRequest;\n\
                 import software.amazon.awssdk.services.secretsmanager.model.GetSecretValueResponse;\n\n\
                 public class GetSecret {{\n\
                 \x20   public static void main(String[] args) throws Exception {{\n\
                 \x20       // Credentials come from the default AWS chain (env vars, ~/.aws, IAM role).\n\
                 \x20       try (SecretsManagerClient client = SecretsManagerClient.builder()\n\
                 \x20               .region(Region.of({r}))\n\
                 \x20               .build()) {{\n\
                 \x20           GetSecretValueResponse response = client.getSecretValue(\n\
                 \x20               GetSecretValueRequest.builder().secretId({n}).build());\n\
                 {read}\n\
                 \x20       }}\n\
                 \x20   }}\n\
                 }}\n"
            )
        }
        Lang::CSharp => {
            let (usings, read) = match (shape, key) {
                (Shape::Json(_), Some(k)) => (
                    "using System.Text.Json;\n",
                    format!(
                        "using var secret = JsonDocument.Parse(response.SecretString);\n\
                         Console.WriteLine(secret.RootElement.GetProperty({}));",
                        dq(k)
                    ),
                ),
                (Shape::Binary, _) => (
                    "",
                    "var secret = response.SecretBinary.ToArray();\nConsole.WriteLine($\"{secret.Length} bytes\");"
                        .to_string(),
                ),
                _ => (
                    "",
                    "var secret = response.SecretString;\nConsole.WriteLine(secret);".to_string(),
                ),
            };
            format!(
                "{usings}using Amazon;\nusing Amazon.SecretsManager;\nusing Amazon.SecretsManager.Model;\n\n\
                 // Credentials come from the default AWS chain (env vars, ~/.aws, IAM role).\n\
                 var client = new AmazonSecretsManagerClient(RegionEndpoint.GetBySystemName({r}));\n\n\
                 var response = await client.GetSecretValueAsync(new GetSecretValueRequest\n\
                 {{\n    SecretId = {n},\n}});\n\
                 {read}\n"
            )
        }
        Lang::Rust => {
            let read = match (shape, key) {
                (Shape::Json(_), Some(k)) => format!(
                    "    let secret: serde_json::Value =\n        serde_json::from_str(response.secret_string().unwrap_or(\"{{}}\"))?;\n    println!(\"{{}}\", secret[{}]);",
                    dq(k)
                ),
                (Shape::Binary, _) => "    let secret = response.secret_binary().map(|b| b.as_ref()).unwrap_or_default();\n    println!(\"{} bytes\", secret.len());".to_string(),
                _ => "    let secret = response.secret_string().unwrap_or_default();\n    println!(\"{secret}\");".to_string(),
            };
            format!(
                "use aws_config::{{BehaviorVersion, Region}};\n\n\
                 #[tokio::main]\n\
                 async fn main() -> Result<(), Box<dyn std::error::Error>> {{\n\
                 \x20   // Credentials come from the default AWS chain (env vars, ~/.aws, IAM role).\n\
                 \x20   let config = aws_config::defaults(BehaviorVersion::latest())\n\
                 \x20       .region(Region::new({r}))\n\
                 \x20       .load()\n\
                 \x20       .await;\n\
                 \x20   let client = aws_sdk_secretsmanager::Client::new(&config);\n\n\
                 \x20   let response = client.get_secret_value().secret_id({n}).send().await?;\n\
                 {read}\n\
                 \x20   Ok(())\n\
                 }}\n"
            )
        }
        Lang::Ruby => {
            let (requires, read) = match (shape, key) {
                (Shape::Json(_), Some(k)) => (
                    "require \"aws-sdk-secretsmanager\"\nrequire \"json\"\n",
                    format!(
                        "secret = JSON.parse(response.secret_string)\nputs secret[{}]",
                        dq(k)
                    ),
                ),
                (Shape::Binary, _) => (
                    "require \"aws-sdk-secretsmanager\"\n",
                    "secret = response.secret_binary # binary String\nputs \"#{secret.bytesize} bytes\""
                        .to_string(),
                ),
                _ => (
                    "require \"aws-sdk-secretsmanager\"\n",
                    "secret = response.secret_string\nputs secret".to_string(),
                ),
            };
            format!(
                "{requires}\n\
                 # Credentials come from the default AWS chain (env vars, ~/.aws, IAM role).\n\
                 client = Aws::SecretsManager::Client.new(region: {r})\n\n\
                 response = client.get_secret_value(secret_id: {n})\n\
                 {read}\n"
            )
        }
        Lang::Php => {
            let read = match (shape, key) {
                (Shape::Json(_), Some(k)) => format!(
                    "$secret = json_decode($result['SecretString'], true);\necho $secret[{}], PHP_EOL;",
                    sq(k)
                ),
                (Shape::Binary, _) => {
                    "$secret = $result['SecretBinary']; // binary string\necho strlen($secret), ' bytes', PHP_EOL;"
                        .to_string()
                }
                _ => "$secret = $result['SecretString'];\necho $secret, PHP_EOL;".to_string(),
            };
            format!(
                "<?php\nrequire 'vendor/autoload.php';\n\n\
                 use Aws\\SecretsManager\\SecretsManagerClient;\n\n\
                 // Credentials come from the default AWS chain (env vars, ~/.aws, IAM role).\n\
                 $client = new SecretsManagerClient([\n    'region' => {},\n]);\n\n\
                 $result = $client->getSecretValue(['SecretId' => {}]);\n\
                 {read}\n",
                sq(region),
                sq(name)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_shape() -> Shape {
        Shape::Json(vec![
            ("username".into(), Value::String("app".into())),
            ("password".into(), Value::String("x".into())),
            ("port".into(), serde_json::json!(5432)),
            ("db-name".into(), Value::String("main".into())),
        ])
    }

    #[test]
    fn every_language_names_the_secret_and_region() {
        for lang in Lang::ALL {
            for shape in [json_shape(), Shape::Text, Shape::Binary] {
                let code = sample(lang, "prod/db", "eu-west-2", &shape);
                assert!(code.contains("prod/db"), "{lang:?}");
                assert!(code.contains("eu-west-2"), "{lang:?}");
                assert!(!lang.install().is_empty());
            }
        }
    }

    #[test]
    fn json_samples_read_the_password_key() {
        for lang in Lang::ALL {
            let code = sample(lang, "prod/db", "us-east-1", &json_shape());
            assert!(code.contains("password"), "{lang:?}: {code}");
        }
        let one_key = Shape::Json(vec![("token".into(), Value::String("t".into()))]);
        assert!(sample(Lang::Python, "s", "us-east-1", &one_key).contains("secret[\"token\"]"));
    }

    #[test]
    fn typescript_interface_from_keys() {
        let code = sample(
            Lang::TypeScript,
            "prod/db-creds",
            "us-east-1",
            &json_shape(),
        );
        assert!(code.contains("interface ProdDbCreds {"));
        assert!(code.contains("  username: string;"));
        assert!(code.contains("  port: number;"));
        assert!(code.contains("  \"db-name\": string;"));
    }

    #[test]
    fn literals_are_escaped() {
        let shape = Shape::Json(vec![("we\"ird'key".into(), Value::Null)]);
        assert!(sample(Lang::Python, "n", "r", &shape).contains(r#"secret["we\"ird'key"]"#));
        assert!(sample(Lang::Php, "n", "r", &shape).contains(r#"$secret['we"ird\'key']"#));
    }

    #[test]
    fn names() {
        assert_eq!(pascal("prod/db-creds"), "ProdDbCreds");
        assert_eq!(pascal("1password"), "Secret1password");
        assert_eq!(pascal("--"), "Secret");
    }
}

#[cfg(test)]
mod dump {
    /// Write every sample to $SECRETS_SAMPLES_DIR for checking with real toolchains.
    #[test]
    #[ignore]
    fn dump_samples() {
        use super::*;
        let dir = std::path::PathBuf::from(std::env::var("SECRETS_SAMPLES_DIR").unwrap());
        let shapes = [
            (
                "json",
                Shape::Json(vec![
                    ("username".into(), Value::String("app".into())),
                    ("password".into(), Value::String("x".into())),
                ]),
            ),
            ("text", Shape::Text),
            ("binary", Shape::Binary),
        ];
        for lang in Lang::ALL {
            for (sname, shape) in &shapes {
                let ext = match lang {
                    Lang::JavaScript => "mjs",
                    Lang::TypeScript => "ts",
                    Lang::Java => "java",
                    Lang::Rust => "rs",
                    Lang::Go => "go",
                    Lang::Python => "py",
                    Lang::CSharp => "cs",
                    Lang::Ruby => "rb",
                    Lang::Php => "php",
                };
                let f = dir.join(format!(
                    "{}-{sname}.{ext}",
                    lang.label().replace('#', "sharp")
                ));
                std::fs::write(f, sample(lang, "prod/db", "us-east-1", shape)).unwrap();
            }
        }
    }
}
