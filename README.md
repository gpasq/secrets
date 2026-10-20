# Secrets

A desktop app for creating and managing **AWS Secrets Manager** secrets on macOS and
Windows. It's a single native binary built with Rust and [egui](https://github.com/emilk/egui),
with no browser, Node or runtime to install.

- Pick any AWS profile and region from dropdowns, including IAM Identity Center (SSO)
  and `aws login` profiles
- Search secrets by name, description or tag. **Refresh** (next to the search box, or
  Cmd+R / Ctrl+R / F5) reloads the list and the selected secret. New and deleted secrets
  show up straight away, even though AWS's list can take a few seconds to catch up.
- **Overview**: ARN, dates, rotation status, and editable description and KMS key
- **Value**: shown as soon as you select a secret. JSON is pretty-printed and
  syntax-highlighted in the theme's colors, with a *Format JSON* button and a warning for
  invalid JSON. It's saved compactly, the way the AWS console stores it. Formatting alone
  doesn't count as a change. Key/value (JSON) secrets appear as an
  editable table, plain text as a text box, and binary as base64. Saving creates a new
  version.
- **Versions**: every version and its staging labels. View any version's value, or
  *Make current* to roll back.
- **Code**: snippets for reading the secret with the official AWS SDK in JavaScript,
  TypeScript, Python, Go, Java, C#, Rust, Ruby and PHP. Each comes with its install
  command and is filled in with the secret's name and region. For JSON secrets the snippet
  parses the value and reads a real key (`password` if there is one); TypeScript gets an
  interface generated from the keys. Snippets are highlighted in the current theme.
- **Tags**: add, edit and remove
- **Rotation**: enable or reconfigure (Lambda ARN, every N days or a schedule expression,
  window), rotate now, or disable
- **New secret**: key/value pairs, plain text or no value, plus generated passwords, a KMS
  key and tags
- **Delete** with a 7–30 day recovery window, or immediately. You must type the name to
  confirm. Tick *Show deleted* to see secrets pending deletion and *Restore* them.

## Themes

Theming works the same way as in FlexFTP. Open **Settings → Appearance**, or use the
System / Light / Dark switch in the bottom-right corner.

- **Appearance:** *System* (the default) follows the macOS or Windows light/dark setting.
  *Light* and *Dark* force one or the other.
- **Dark theme / Light theme:** the theme used in each mode. The defaults are VS Code's
  Dark Modern and Light Modern. The choices are:
  - six preloaded themes: Dark Modern, Monokai, Night Owl, Solarized Dark, Light Modern
    and Solarized Light (MIT licensed; see [assets/themes/NOTICE.md](assets/themes/NOTICE.md))
  - themes you import with **Import VS Code theme…**, which accepts any theme `.json`,
    including comments, trailing commas and `"include"` chains
  - every color theme installed in VS Code, VS Code Insiders, Cursor, Windsurf or VSCodium
- **Colorful accents** (on by default) use the theme's own terminal colors. Create
  buttons are green, Save is blue and confirming a delete is red. Version labels are
  colored chips. Each secret gets a colored dot and heading marker: red for `prod…`,
  yellow for staging, green for dev/test, and blue, magenta or cyan for anything else.
  Turn it off for a plainer look.

Everything else comes from the theme's workbench colors: editor, sidebar, title bar,
status bar, inputs, buttons, list selection, links and error colors. JSON values and code
samples use the theme's `tokenColors`, the same as VS Code. Colors a theme doesn't define
fall back to Dark Modern or Light Modern. Your choices are remembered between launches.

## Install

You need Rust 1.95 or newer, from <https://rustup.rs>. [`just`](https://github.com/casey/just)
is optional but handy: `brew install just` or `winget install Casey.Just`.

**macOS:**

```bash
just install
```

This builds `Secrets.app` and copies it to `~/Applications`, so you can open it from
Spotlight, Launchpad or the Dock. To build without installing, run `just app`; the result
is `target/Secrets.app`.

**Windows:**

```powershell
cargo build --release
```

Run `target\release\secrets.exe`, or copy it anywhere and pin it to the Start menu. Release
builds open no console window.

**From source, any OS:** `just run` or `cargo run`. To pick the starting profile, run
`just run daikin` or set `AWS_PROFILE`.

## AWS sign-in

There are two ways to give the app credentials, and you can use both. Pick between them
in the **Account** menu.

### Saved connections (no AWS config file needed)

Open **Settings → Connections → Add connection** and enter:

- a name
- an access key ID and secret access key
- a session token, only for temporary `ASIA…` keys
- a region
- optionally, a role ARN to assume (plus an external ID if the role needs one)

**Test connection** checks the keys before you save. Saved connections appear at the top
of the Account menu. When there are no AWS config profiles, a first-run screen offers to
add one.

The secret access key and session token are stored in the **macOS Keychain** or **Windows
Credential Manager**, under the service `net.pasq.secrets`. Only the name, access key ID,
region and role are saved in the app's settings file. To use saved connections only, untick
*Also list profiles from ~/.aws/config* in Settings.

### AWS config profiles

The app uses the standard AWS credential chain, the same one the AWS CLI uses. It reads
profiles from `~/.aws/config` and `~/.aws/credentials` (on Windows,
`%USERPROFILE%\.aws\...`) and lists them in the **Profile** dropdown. Supported profile
types:

- Static access keys
- `role_arn` assume-role chains
- `credential_process`
- IAM Identity Center (SSO): run `aws sso login --profile NAME` first
- `aws login` console sessions: run `aws login --profile NAME` first

The app starts with `$AWS_PROFILE`, else `default`, else the first profile it finds. The
**Region** dropdown defaults to the profile's own region. If you get an expired-token
error, sign in again with the AWS CLI and click **Refresh**.

To use an emulator such as LocalStack, set `SECRETS_ENDPOINT_URL=http://localhost:4566`
before launching.

**IAM permissions.** Browsing needs `secretsmanager:ListSecrets`, `DescribeSecret`,
`ListSecretVersionIds` and `GetSecretValue`, plus `kms:Decrypt` for customer-managed
keys. Editing needs `CreateSecret`, `PutSecretValue`, `UpdateSecret`, `TagResource`,
`UntagResource`, `UpdateSecretVersionStage`, `DeleteSecret`, `RestoreSecret`,
`RotateSecret`, `CancelRotateSecret` and `GetRandomPassword`.

## Development

```bash
just            # list recipes
just run        # launch from source
just check      # fmt --check, clippy -D warnings, tests (same as CI)
```

The tests make no AWS calls. They cover the parsing and formatting helpers, profile
discovery, JSON key/value round-tripping, and tag diffs. They also render every tab and
dialog headlessly with [egui_kittest](https://crates.io/crates/egui_kittest), and check
that a late response for a previously selected secret is never shown. To look at the rendered screens, run:

```bash
SECRETS_UI_SHOTS=/tmp/shots cargo test
```

Source layout:

- `src/app.rs`: the UI. AWS calls run on a background tokio runtime and report back over
  a channel, so the window never blocks.
- `src/theme.rs`: light/dark palettes, and VS Code theme loading and discovery
- `src/connections.rs`: saved connections and Keychain/Credential Manager storage
- `src/aws.rs`: Secrets Manager SDK wrapper
- `src/util.rs`, `src/profiles.rs`: pure helpers
- `assets/Info.plist`: macOS app bundle metadata
- `assets/icon.svg`: the app icon source. After editing it, run `just icon` (needs
  `cargo install resvg`) to regenerate `AppIcon.icns` (macOS), `icon.ico` (Windows exe,
  embedded by `build.rs`) and `icon-256.png` (window icon).

## Building for Windows

**On Windows (recommended):** install Rust with `rustup-init.exe`, keeping the default
MSVC toolchain. That needs the Visual Studio Build Tools with the "Desktop development
with C++" workload. Then run `cargo build --release`. The TLS library (`aws-lc-rs`)
occasionally needs [NASM](https://www.nasm.us) and [CMake](https://cmake.org) on `PATH`;
install them if the build asks.

**CI:** `.github/workflows/ci.yml` runs fmt, clippy, tests and a release build on macOS
(arm64 and x86_64) and Windows (x64 and arm64). The binaries are uploaded as workflow
artifacts. Push the repo to GitHub to get a Windows `.exe` without a Windows machine.

**Cross-compiling from macOS (optional):**

```bash
brew install llvm nasm cmake
```

```bash
cargo install --locked cargo-xwin
```

```bash
just windows
```

The result is `target/x86_64-pc-windows-msvc/release/secrets.exe`. CI is the more
reliable route, because `aws-lc-sys` cross builds depend on the local LLVM and CMake
versions.
