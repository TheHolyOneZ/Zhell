use std::borrow::Cow;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    AwsKeyId,
    AwsSecret,
    GitHubToken,
    GitLabToken,
    SlackToken,
    StripeKey,
    GoogleApiKey,
    AiApiKey,
    Jwt,
    PrivateKey,
    Assignment,
    UrlPassword,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::AwsKeyId => "aws-key-id",
            Kind::AwsSecret => "aws-secret",
            Kind::GitHubToken => "github-token",
            Kind::GitLabToken => "gitlab-token",
            Kind::SlackToken => "slack-token",
            Kind::StripeKey => "stripe-key",
            Kind::GoogleApiKey => "google-api-key",
            Kind::AiApiKey => "api-key",
            Kind::Jwt => "jwt",
            Kind::PrivateKey => "private-key",
            Kind::Assignment => "secret",
            Kind::UrlPassword => "password",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub range: Range<usize>,
    pub kind: Kind,
}

struct Detector {
    kind: Kind,
    re: Regex,

    group: usize,
}

static DETECTORS: LazyLock<Vec<Detector>> = LazyLock::new(|| {
    let d = |kind, pattern: &str, group| Detector { kind, re: Regex::new(pattern).expect("secret regex"), group };
    vec![
        d(Kind::PrivateKey, r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----[\s\S]*?(?:-----END (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----|\z)", 0),
        d(Kind::AwsKeyId, r"\b(?:AKIA|ASIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA)[0-9A-Z]{16}\b", 0),
        d(Kind::AwsSecret, r#"(?i)aws_?secret_?access_?key["']?\s*[=:]\s*["']?([A-Za-z0-9/+=]{40})"#, 1),
        d(Kind::GitHubToken, r"\b(?:gh[pousr]_[A-Za-z0-9]{36,255}|github_pat_[A-Za-z0-9_]{22,255})\b", 0),
        d(Kind::GitLabToken, r"\bglpat-[A-Za-z0-9_\-]{20,}", 0),
        d(Kind::SlackToken, r"\bxox[baprs]-[A-Za-z0-9-]{10,}", 0),
        d(Kind::StripeKey, r"\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{16,}\b", 0),
        d(Kind::GoogleApiKey, r"\bAIza[0-9A-Za-z_\-]{35}\b", 0),
        d(Kind::AiApiKey, r"\bsk-(?:proj-|ant-(?:api|admin)\d\d-)?[A-Za-z0-9_\-]{20,}", 0),
        d(Kind::Jwt, r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}", 0),
        d(Kind::UrlPassword, r"\b[a-zA-Z][a-zA-Z0-9+.\-]*://[^/\s:@]+:([^@\s/]+)@", 1),
        d(
            Kind::Assignment,
            r#"(?i)\b(?:password|passwd|pwd|secret|client_secret|token|auth_token|access_token|api[_-]?key|private[_-]?key)\b["']?\s*[=:]\s*["']?([^\s"',;]{6,})"#,
            1,
        ),
    ]
});

pub fn find(text: &str) -> Vec<Match> {
    let mut out: Vec<Match> = Vec::new();
    for det in DETECTORS.iter() {
        for caps in det.re.captures_iter(text) {
            let Some(m) = caps.get(det.group) else { continue };
            let range = m.range();
            if out.iter().any(|o| range.start < o.range.end && o.range.start < range.end) {
                continue;
            }
            out.push(Match { range, kind: det.kind });
        }
    }
    out.sort_by_key(|m| m.range.start);
    out
}

pub fn redact(text: &str) -> Cow<'_, str> {
    let found = find(text);
    if found.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for m in found {
        out.push_str(&text[pos..m.range.start]);
        out.push_str("‹redacted:");
        out.push_str(m.kind.label());
        out.push('›');
        pos = m.range.end;
    }
    out.push_str(&text[pos..]);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(t: &str) -> Vec<Kind> {
        find(t).into_iter().map(|m| m.kind).collect()
    }

    #[test]
    fn known_token_formats() {
        assert_eq!(kinds("export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE"), vec![Kind::AwsKeyId]);
        assert_eq!(
            kinds("aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            vec![Kind::AwsSecret]
        );
        assert_eq!(kinds("ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789"), vec![Kind::GitHubToken]);
        assert_eq!(kinds("glpat-abcdefghij1234567890"), vec![Kind::GitLabToken]);
        assert_eq!(kinds("xoxb-1234567890-abcdefghij"), vec![Kind::SlackToken]);
        assert_eq!(kinds("sk_live_abcdefghijklmnop1234"), vec![Kind::StripeKey]);
        assert_eq!(kinds("key=AIzaSyA1234567890abcdefghijklmnopqrstuv"), vec![Kind::GoogleApiKey]);
        assert_eq!(kinds("ANTHROPIC_API_KEY=sk-ant-api03-abcdefghijklmnopqrstuvwxyz"), vec![Kind::AiApiKey]);
        assert_eq!(
            kinds("Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"),
            vec![Kind::Jwt]
        );
    }

    #[test]
    fn private_keys_even_truncated() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----";
        let text = format!("cat id\n{pem}\ndone");
        let r = redact(&text);
        assert_eq!(r, "cat id\n‹redacted:private-key›\ndone");

        assert_eq!(redact("-----BEGIN RSA PRIVATE KEY-----\nMIIE"), "‹redacted:private-key›");
    }

    #[test]
    fn assignments_and_urls_keep_context() {
        assert_eq!(redact("mysql -u root --password=hunter2pass"), "mysql -u root --password=‹redacted:secret›");
        assert_eq!(redact("TOKEN: 'abc123def456'"), "TOKEN: '‹redacted:secret›'");
        assert_eq!(
            redact("git clone https://user:s3cretPass@example.com/repo.git"),
            "git clone https://user:‹redacted:password›@example.com/repo.git"
        );
    }

    #[test]
    fn ordinary_text_is_untouched() {
        for t in [
            "commit 3f786850e387550fdab836ed7e6dc881de23001b",
            "sha256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "password is required",
            "cargo build --release",
            "https://example.com/path:8080",
        ] {
            assert_eq!(redact(t), t, "{t}");
        }
    }
}
