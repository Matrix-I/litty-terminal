//! Release lookup on the hosting service's API and downloads, both over `curl`.

use std::path::Path;
use std::process::Command;

use super::install::platform_asset_candidates;
use super::verify::SIG_NAMES;
use super::{resolve_system_bin, AutoUpdateChannel, ReleaseAsset, ReleaseInfo, Version, SUMS_NAME};
use crate::json::Json;

pub(super) const API_LATEST_URL: &str = "https://api.github.com/repos/Matrix-I/litty-terminal/releases/latest";
pub(super) const API_RELEASES_URL: &str = "https://api.github.com/repos/Matrix-I/litty-terminal/releases";
pub(super) const WEB_LATEST_URL: &str = "https://github.com/Matrix-I/litty-terminal/releases/latest";
pub(super) const WEB_TAG_PREFIX: &str = "https://github.com/Matrix-I/litty-terminal/releases/tag/";
pub(super) const WEB_DOWNLOAD_PREFIX: &str = "https://github.com/Matrix-I/litty-terminal/releases/download/";

/// `curl` with the flags every update request shares: fail on HTTP errors, report them on
/// stderr, follow redirects over HTTPS only, and give up on unreachable hosts.
pub(super) fn curl() -> Command {
    let mut cmd = Command::new(resolve_system_bin("curl"));
    cmd.args([
        "--silent",
        "--show-error",
        "--fail",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "10",
    ]);
    cmd
}

/// Run a `curl` command and return its stdout, or its stderr as the error.
pub(super) fn run_curl(cmd: &mut Command, what: &str) -> Result<Vec<u8>, String> {
    run_curl_with_stdin(cmd, what, None)
}

pub(super) fn run_curl_with_stdin(cmd: &mut Command, what: &str, input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use std::process::Stdio;

    let output = if let Some(data) = input {
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("cannot run curl: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(data);
        }
        child.wait_with_output().map_err(|e| format!("cannot read curl output: {e}"))?
    } else {
        cmd.output().map_err(|e| format!("cannot run curl: {e}"))?
    };

    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = stderr.trim();
    if reason.is_empty() {
        Err(format!("{what} failed (curl exit code {:?})", output.status.code()))
    } else {
        Err(format!("{what} failed: {reason}"))
    }
}

/// A tag is used as a cache directory name, so allow only plain version-like characters.
pub(super) fn is_safe_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 64
        && !tag.starts_with('.')
        && tag.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
}

/// Read one release object from the GitHub API. Drafts and unparseable tags are skipped.
pub(super) fn parse_release(v: &Json) -> Option<ReleaseInfo> {
    if v.get("draft") == Some(&Json::Bool(true)) {
        return None;
    }
    let tag_name = v.get("tag_name")?.as_str()?;
    if !is_safe_tag(tag_name) {
        return None;
    }
    let version = Version::parse(tag_name)?;
    let html_url = v
        .get("html_url")
        .and_then(Json::as_str)
        .filter(|u| u.starts_with("https://github.com/"))
        .map_or_else(|| format!("{WEB_TAG_PREFIX}{tag_name}"), str::to_string);
    let assets = match v.get("assets") {
        Some(Json::Arr(list)) => list
            .iter()
            .filter_map(|a| {
                Some(ReleaseAsset {
                    name: a.get("name")?.as_str()?.to_string(),
                    download_url: a
                        .get("browser_download_url")?
                        .as_str()
                        // Release assets are served from github.com only; anything
                        // else (look-alike hosts included) is ignored.
                        .filter(|u| u.starts_with("https://github.com/"))?
                        .to_string(),
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(ReleaseInfo { tag_name: tag_name.to_string(), version, html_url, assets })
}

/// Parse an API response: `/releases/latest` (one object) for `stable`, or `/releases`
/// (a list, prereleases included) for `tip`, where the highest version wins.
pub(crate) fn parse_release_json(body: &str, channel: AutoUpdateChannel) -> Result<ReleaseInfo, String> {
    let json = Json::parse(body)?;
    match channel {
        AutoUpdateChannel::Stable => {
            parse_release(&json).ok_or_else(|| "GitHub response contains no usable release".to_string())
        }
        AutoUpdateChannel::Tip => {
            let Json::Arr(list) = &json else {
                return Err("GitHub response is not a release list".to_string());
            };
            // Keep the first of equal versions: the list is newest first, so a final
            // release wins over an older prerelease of the same version.
            list.iter()
                .filter_map(parse_release)
                .filter(|r| !r.assets.is_empty())
                .reduce(|best, r| if r.version > best.version { r } else { best })
                .ok_or_else(|| "GitHub response contains no usable release".to_string())
        }
    }
}

/// Fetch the latest GitHub release metadata.
pub fn fetch_latest_release(channel: AutoUpdateChannel) -> Result<ReleaseInfo, String> {
    match fetch_release_from_api(channel) {
        Ok(release) => Ok(release),
        // The API is rate limited for unauthenticated clients (403); the web redirect is not.
        // It only knows the latest stable release, which is still a valid answer for `tip`.
        Err(api_err) => fetch_latest_release_redirect().map_err(|e| format!("{api_err}\n(fallback: {e})")),
    }
}

pub(super) fn fetch_release_from_api(channel: AutoUpdateChannel) -> Result<ReleaseInfo, String> {
    let url = match channel {
        AutoUpdateChannel::Stable => API_LATEST_URL,
        AutoUpdateChannel::Tip => API_RELEASES_URL,
    };

    let mut cmd = curl();
    cmd.args([
        "--max-time",
        "30",
        "-H",
        "User-Agent: Litty-Terminal",
        "-H",
        "Accept: application/vnd.github+json",
    ]);
    let mut stdin_config: Option<Vec<u8>> = None;
    if let Ok(token) = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN"))
        && !token.is_empty() {
            cmd.arg("-K").arg("-");
            stdin_config = Some(format!("header = \"Authorization: Bearer {token}\"\n").into_bytes());
        }
    cmd.arg(url);

    let body = run_curl_with_stdin(&mut cmd, "GitHub API request", stdin_config.as_deref())?;
    let body = String::from_utf8(body).map_err(|_| "GitHub API returned invalid UTF-8".to_string())?;
    parse_release_json(&body, channel)
}

/// Tag name from the final URL of the `/releases/latest` redirect.
pub(super) fn tag_from_release_url(url: &str) -> Option<&str> {
    url.trim().strip_prefix(WEB_TAG_PREFIX).filter(|tag| is_safe_tag(tag))
}

/// Fallback method: follow `https://github.com/Matrix-I/litty-terminal/releases/latest`,
/// which redirects to the newest tag and does not suffer from GitHub API rate limits.
pub(super) fn fetch_latest_release_redirect() -> Result<ReleaseInfo, String> {
    let mut cmd = curl();
    cmd.args(["--max-time", "30", "--head", "--output", "/dev/null", "--write-out", "%{url_effective}"]);
    cmd.arg(WEB_LATEST_URL);
    let output = run_curl(&mut cmd, "github.com release lookup")?;
    let final_url = String::from_utf8_lossy(&output);

    let tag_name = tag_from_release_url(&final_url)
        .ok_or_else(|| format!("could not find a release tag in `{}`", final_url.trim()))?
        .to_string();
    let version = Version::parse(&tag_name).ok_or_else(|| format!("invalid version tag `{tag_name}`"))?;

    // Without the API the asset list is unknown: offer this platform's names and let
    // `SHA256SUMS.txt` decide which of them the release really has.
    let assets = platform_asset_candidates()
        .into_iter()
        .chain([SUMS_NAME.to_string()])
        .chain(SIG_NAMES.iter().map(|s| (*s).to_string()))
        .map(|name| ReleaseAsset { download_url: format!("{WEB_DOWNLOAD_PREFIX}{tag_name}/{name}"), name })
        .collect();

    Ok(ReleaseInfo { html_url: format!("{WEB_TAG_PREFIX}{tag_name}"), tag_name, version, assets })
}

/// Download a file to `dest` using `curl`.
pub(super) fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let mut cmd = curl();
    // Abort stalled transfers (< 1 KiB/s for a minute) and anything slower than 15 minutes.
    cmd.args(["--max-time", "900", "--speed-limit", "1024", "--speed-time", "60", "--output"]);
    cmd.arg(dest).arg(url);
    run_curl(&mut cmd, &format!("download of {url}")).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::MAX_JSON_DEPTH;

    /// Shape of a real `/releases/latest` response: each asset nests an `uploader` object
    /// between `name` and `browser_download_url`, which defeated the old regex.
    const LATEST_JSON: &str = r#"{
        "html_url": "https://github.com/Matrix-I/litty-terminal/releases/tag/v0.4.0",
        "author": {"login": "Matrix-I", "html_url": "https://github.com/Matrix-I", "id": 1},
        "tag_name": "v0.4.0", "draft": false, "prerelease": false, "name": "Litty ⚡ v0.4.0",
        "assets": [
            {"id": 1, "name": "litty-linux", "label": "", "size": 123,
             "uploader": {"login": "Matrix-I", "site_admin": false},
             "browser_download_url": "https://github.com/Matrix-I/litty-terminal/releases/download/v0.4.0/litty-linux"},
            {"id": 2, "name": "SHA256SUMS.txt", "uploader": {"login": "Matrix-I"},
             "browser_download_url": "https://github.com/Matrix-I/litty-terminal/releases/download/v0.4.0/SHA256SUMS.txt"}
        ],
        "body": "Notes with \"quotes\", braces {} and a newline\n"
    }"#;

    #[test]
    fn test_json_parse_values_and_escapes() {
        let v = Json::parse(r#" {"a": [1, -2.5e3, true, false, null], "s": "x\"\\\/\né😀"} "#).unwrap();
        assert_eq!(
            v.get("a"),
            Some(&Json::Arr(vec![Json::Num("1".into()), Json::Num("-2.5e3".into()), Json::Bool(true), Json::Bool(false), Json::Null]))
        );
        assert_eq!(v.get("s").and_then(Json::as_str), Some("x\"\\/\né😀"));
        assert_eq!(Json::parse("[]"), Ok(Json::Arr(Vec::new())));
        assert_eq!(Json::parse("{}"), Ok(Json::Obj(Vec::new())));
    }

    #[test]
    fn test_json_rejects_malformed_input() {
        for bad in ["", "{", r#"{"a" 1}"#, "[1,]", r#""abc"#, r#""\x""#, r#""\ud83d""#, "{} x", "tru"] {
            assert!(Json::parse(bad).is_err(), "accepted {bad:?}");
        }
        let deep = "[".repeat(MAX_JSON_DEPTH + 2) + &"]".repeat(MAX_JSON_DEPTH + 2);
        assert!(Json::parse(&deep).is_err());
    }

    #[test]
    fn test_parse_latest_release_with_nested_uploader() {
        let r = parse_release_json(LATEST_JSON, AutoUpdateChannel::Stable).unwrap();
        assert_eq!(r.tag_name, "v0.4.0");
        assert_eq!(r.version, Version { major: 0, minor: 4, patch: 0 });
        assert_eq!(r.html_url, "https://github.com/Matrix-I/litty-terminal/releases/tag/v0.4.0");
        let names: Vec<_> = r.assets.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["litty-linux", "SHA256SUMS.txt"]);
        assert!(r.assets[0].download_url.ends_with("/v0.4.0/litty-linux"));
    }

    #[test]
    fn test_parse_tip_picks_highest_version_and_skips_drafts() {
        let asset = r#""assets": [{"name": "a", "browser_download_url": "https://github.com/x/a"}]"#;
        let body = format!(
            r#"[
                {{"tag_name": "v0.9.0", "draft": true, {asset}}},
                {{"tag_name": "v0.5.0", {asset}}},
                {{"tag_name": "v0.5.0-rc1", "prerelease": true, {asset}}},
                {{"tag_name": "v0.6.0-beta.1", "prerelease": true, "assets": []}},
                {{"tag_name": "v0.4.0", {asset}}}
            ]"#
        );
        let r = parse_release_json(&body, AutoUpdateChannel::Tip).unwrap();
        // The draft and the asset-less beta are ignored; the final 0.5.0 beats its rc.
        assert_eq!(r.tag_name, "v0.5.0");
        assert!(parse_release_json("{}", AutoUpdateChannel::Tip).is_err());
        assert!(parse_release_json("[]", AutoUpdateChannel::Tip).is_err());
    }

    #[test]
    fn test_parse_release_rejects_unsafe_tags_and_urls() {
        let body = r#"{"tag_name": "v1.2.3-../../evil", "assets": []}"#;
        assert!(parse_release_json(body, AutoUpdateChannel::Stable).is_err());
        let body = r#"{"tag_name": "v1.2.3", "html_url": "javascript:alert(1)",
            "assets": [{"name": "a", "browser_download_url": "http://insecure/a"}]}"#;
        let r = parse_release_json(body, AutoUpdateChannel::Stable).unwrap();
        assert_eq!(r.html_url, format!("{WEB_TAG_PREFIX}v1.2.3"));
        assert!(r.assets.is_empty());
    }

    #[test]
    fn test_tag_from_release_url() {
        assert_eq!(tag_from_release_url(&format!("{WEB_TAG_PREFIX}v0.3.0\n")), Some("v0.3.0"));
        assert_eq!(tag_from_release_url("https://github.com/Matrix-I/litty-terminal/releases"), None);
        assert_eq!(tag_from_release_url(&format!("{WEB_TAG_PREFIX}..")), None);
        assert_eq!(tag_from_release_url("https://evil.example/releases/tag/v9.9.9"), None);
    }
}
